use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::brush::{BrushProvinceClassification, StateBrushMode};
use super::lasso::LassoSelectionMode;
use super::properties::{EditableProvinceData, EditableStateProperties};
use crate::app::map::{Map, ProvinceKind};
use crate::app::project::{
    DiagnosticSeverity, Hoi4Project, ProjectDiagnostic, ProjectDiagnosticKind,
};
use crate::app::state::{StateData, VictoryPoint};

#[derive(Debug, Clone)]
pub struct StateEditSession {
    baseline: StateWorkingSet,
    working: StateWorkingSet,
    known_state_ids: BTreeSet<u32>,
    valid_state_ids: BTreeSet<u32>,
    origins_by_state: BTreeMap<u32, WorkingStateOrigin>,
    removed_state_ids: BTreeSet<u32>,
    province_kinds: BTreeMap<u32, ProvinceKind>,
    ambiguous_provinces: BTreeSet<u32>,
    selected_provinces: BTreeSet<u32>,
    target_state_id: Option<u32>,
    undo_stack: Vec<StateEditCommand>,
    redo_stack: Vec<StateEditCommand>,
    dirty_state_ids: BTreeSet<u32>,
    session_diagnostics: Vec<ProjectDiagnostic>,
    last_timings: StateEditTimings,
    last_changed_provinces: BTreeSet<u32>,
    revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct StateWorkingSet {
    state_by_province: HashMap<u32, u32>,
    provinces_by_state: BTreeMap<u32, BTreeSet<u32>>,
    properties_by_state: BTreeMap<u32, EditableStateProperties>,
    victory_points_by_state: BTreeMap<u32, Vec<VictoryPoint>>,
    province_buildings_by_state: BTreeMap<u32, BTreeMap<u32, BTreeMap<String, i64>>>,
    detached_victory_points: BTreeMap<u32, Vec<VictoryPoint>>,
    detached_province_buildings: BTreeMap<u32, BTreeMap<String, i64>>,
    unassigned_land_provinces: BTreeSet<u32>,
    dated_history_states: BTreeSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingStateOrigin {
    Loaded { document_path: PathBuf },
    CreatedInSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingStateLifecycle {
    Active,
    RemovedInSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRemovalPolicy {
    MoveToState(u32),
    Unassign,
}

#[derive(Debug, Clone, PartialEq)]
enum StateEditCommand {
    ReassignProvinces {
        deltas: Vec<ProvinceEditDelta>,
    },
    UpdateStateProperties {
        change: Box<StatePropertyChange>,
    },
    UpdateProvinceData {
        province_id: u32,
        state_id: u32,
        before: EditableProvinceData,
        after: EditableProvinceData,
    },
    CreateState {
        state: Box<WorkingStateSnapshot>,
        deltas: Vec<ProvinceEditDelta>,
    },
    RemoveState {
        state: Box<WorkingStateSnapshot>,
        policy: StateRemovalPolicy,
        deltas: Vec<ProvinceEditDelta>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct StatePropertyChange {
    state_id: u32,
    before: EditableStateProperties,
    after: EditableStateProperties,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProvinceEditDelta {
    province_id: u32,
    from_state_id: Option<u32>,
    to_state_id: Option<u32>,
    victory_points: Vec<VictoryPoint>,
    province_buildings: Option<BTreeMap<String, i64>>,
}

#[derive(Debug, Clone, PartialEq)]
struct WorkingStateSnapshot {
    state_id: u32,
    properties: EditableStateProperties,
    provinces: BTreeSet<u32>,
    victory_points: Vec<VictoryPoint>,
    province_buildings: BTreeMap<u32, BTreeMap<String, i64>>,
    origin: WorkingStateOrigin,
    has_dated_history: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateEditTimings {
    pub command_preflight: Duration,
    pub command_apply: Duration,
    pub index_update: Duration,
    pub state_texture_update: Duration,
    pub state_boundary_update: Duration,
    pub undo: Duration,
    pub redo: Duration,
    pub discard: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateEditSummary {
    pub assigned_provinces: usize,
    pub unassigned_land_provinces: usize,
    pub commands: usize,
    pub redo_commands: usize,
    pub modified_states: usize,
    pub selected_provinces: usize,
    pub target_state_id: Option<u32>,
    pub session_errors: usize,
    pub session_warnings: usize,
    pub active_states: usize,
    pub created_states: usize,
    pub removed_states: usize,
    pub reserved_state_ids: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateEditError {
    EmptySelection,
    ProvinceNotFound(u32),
    ProvinceIsNotLand(u32),
    AmbiguousProvince(u32),
    ProvinceInInvalidState {
        province_id: u32,
        state_id: u32,
    },
    ProvinceUnassigned(u32),
    ProvinceStateChanged {
        province_id: u32,
        expected_state_id: u32,
        actual_state_id: Option<u32>,
    },
    InvalidProvinceData(String),
    InvalidStateProperties(String),
    StateIdInvalid(u32),
    StateIdReserved(u32),
    StateAlreadyRemoved(u32),
    StateRemovalTargetMatches(u32),
    TargetStateNotFound(u32),
    TargetStateInvalid(u32),
    ProvincialDataConflict {
        province_id: u32,
        target_state_id: u32,
    },
    InvariantViolation(String),
}

impl StateEditSession {
    pub fn new(project: &Hoi4Project, map: &Map) -> Self {
        let province_kinds = map
            .iter_province_data()
            .filter_map(|(_, province)| province.preserved_id.map(|id| (id, province.kind)))
            .collect::<BTreeMap<_, _>>();
        let known_state_ids = project
            .states
            .iter()
            .filter_map(|document| document.data.as_ref())
            .filter_map(|data| data.id)
            .filter(|&id| id != 0)
            .chain(
                project
                    .states
                    .iter()
                    .filter_map(|document| state_id_from_filename(&document.path)),
            )
            .collect::<BTreeSet<_>>();
        let mut state_id_counts = BTreeMap::<u32, usize>::new();
        for state_id in project
            .states
            .iter()
            .filter_map(|document| document.data.as_ref())
            .filter_map(|data| data.id)
        {
            *state_id_counts.entry(state_id).or_default() += 1;
        }
        let valid_state_ids = project
            .states_by_id
            .iter()
            .filter_map(|(&state_id, &document_index)| {
                let document = project.states.get(document_index)?;
                let structurally_valid = document.data.is_some()
                    && !document
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
                (structurally_valid && state_id_counts.get(&state_id) == Some(&1))
                    .then_some(state_id)
            })
            .collect::<BTreeSet<_>>();
        let origins_by_state = valid_state_ids
            .iter()
            .filter_map(|state_id| {
                let document_index = project.states_by_id.get(state_id)?;
                let document = project.states.get(*document_index)?;
                Some((
                    *state_id,
                    WorkingStateOrigin::Loaded {
                        document_path: document.path.clone(),
                    },
                ))
            })
            .collect();
        let baseline = StateWorkingSet::from_project(project, &province_kinds);

        Self {
            working: baseline.clone(),
            baseline,
            known_state_ids,
            valid_state_ids,
            origins_by_state,
            removed_state_ids: BTreeSet::new(),
            province_kinds,
            ambiguous_provinces: project.ambiguous_provinces.keys().copied().collect(),
            selected_provinces: BTreeSet::new(),
            target_state_id: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty_state_ids: BTreeSet::new(),
            session_diagnostics: Vec::new(),
            last_timings: StateEditTimings::default(),
            last_changed_provinces: BTreeSet::new(),
            revision: 0,
        }
    }

    pub fn state_by_province(&self) -> &HashMap<u32, u32> {
        &self.working.state_by_province
    }

    pub fn unassigned_land_provinces(&self) -> &BTreeSet<u32> {
        &self.working.unassigned_land_provinces
    }

    pub fn selected_provinces(&self) -> &BTreeSet<u32> {
        &self.selected_provinces
    }

    pub fn valid_state_ids(&self) -> &BTreeSet<u32> {
        &self.valid_state_ids
    }

    pub fn removed_state_ids(&self) -> &BTreeSet<u32> {
        &self.removed_state_ids
    }

    pub fn state_origin(&self, state_id: u32) -> Option<&WorkingStateOrigin> {
        self.origins_by_state.get(&state_id)
    }

    pub fn state_lifecycle(&self, state_id: u32) -> Option<WorkingStateLifecycle> {
        if self.valid_state_ids.contains(&state_id) {
            Some(WorkingStateLifecycle::Active)
        } else if self.removed_state_ids.contains(&state_id) {
            Some(WorkingStateLifecycle::RemovedInSession)
        } else {
            None
        }
    }

    pub fn is_state_id_reserved(&self, state_id: u32) -> bool {
        self.reserved_state_ids().contains(&state_id)
    }

    pub fn validate_new_state_id(&self, state_id: u32) -> Result<(), StateEditError> {
        if state_id == 0 {
            Err(StateEditError::StateIdInvalid(state_id))
        } else if self.is_state_id_reserved(state_id) {
            Err(StateEditError::StateIdReserved(state_id))
        } else {
            Ok(())
        }
    }

    pub fn suggest_next_state_id(&self) -> u32 {
        self.reserved_state_ids()
            .last()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .unwrap_or(0)
    }

    pub fn is_state_active(&self, state_id: u32) -> bool {
        self.valid_state_ids.contains(&state_id)
    }

    pub fn state_province_count(&self, state_id: u32) -> usize {
        self.working
            .provinces_by_state
            .get(&state_id)
            .map_or(0, BTreeSet::len)
    }

    pub fn validate_removable_state(&self, state_id: u32) -> Result<(), StateEditError> {
        if self.removed_state_ids.contains(&state_id) {
            Err(StateEditError::StateAlreadyRemoved(state_id))
        } else {
            self.validate_target_state(state_id)
        }
    }

    pub fn validate_state_creation(
        &self,
        state_id: u32,
        properties: &EditableStateProperties,
        use_selected: bool,
    ) -> Result<(), StateEditError> {
        let province_ids = if use_selected {
            self.selected_provinces.iter().copied().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.build_create_command(state_id, properties.clone(), &province_ids)
            .map(drop)
    }

    pub fn validate_state_removal(
        &self,
        state_id: u32,
        policy: StateRemovalPolicy,
    ) -> Result<(), StateEditError> {
        self.build_remove_command(state_id, policy).map(drop)
    }

    pub fn selection_sources(&self) -> BTreeMap<Option<u32>, usize> {
        let mut sources = BTreeMap::new();
        for province_id in &self.selected_provinces {
            *sources
                .entry(self.working.state_by_province.get(province_id).copied())
                .or_default() += 1;
        }
        sources
    }

    pub fn take_last_changed_provinces(&mut self) -> BTreeSet<u32> {
        std::mem::take(&mut self.last_changed_provinces)
    }

    pub fn target_state_id(&self) -> Option<u32> {
        self.target_state_id
    }

    pub fn dirty_state_ids(&self) -> &BTreeSet<u32> {
        &self.dirty_state_ids
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn can_move_selection_to_target(&self) -> bool {
        let Some(target_state_id) = self.target_state_id else {
            return false;
        };
        self.can_reassign_selected(Some(target_state_id))
    }

    pub fn can_unassign_selection(&self) -> bool {
        self.can_reassign_selected(None)
    }

    pub fn diagnostics(&self) -> &[ProjectDiagnostic] {
        &self.session_diagnostics
    }

    pub fn last_timings(&self) -> StateEditTimings {
        self.last_timings
    }

    pub fn state_data(&self, state_id: u32) -> Option<StateData> {
        self.valid_state_ids
            .contains(&state_id)
            .then(|| self.working.state_data(state_id))
    }

    pub fn province_state_id(&self, province_id: u32) -> Option<u32> {
        self.working.state_by_province.get(&province_id).copied()
    }

    pub fn province_data(&self, province_id: u32) -> Option<EditableProvinceData> {
        self.province_kinds.contains_key(&province_id).then(|| {
            self.working
                .province_data(self.province_state_id(province_id), province_id)
        })
    }

    pub fn editable_province_state(&self, province_id: u32) -> Result<u32, StateEditError> {
        self.validate_selectable_province(province_id)?;
        let Some(state_id) = self.province_state_id(province_id) else {
            return Err(StateEditError::ProvinceUnassigned(province_id));
        };
        self.validate_target_state(state_id)?;
        self.working
            .properties_by_state
            .get(&state_id)
            .ok_or(StateEditError::TargetStateNotFound(state_id))?;
        Ok(state_id)
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_state_ids.is_empty()
    }

    pub fn is_state_dirty(&self, state_id: u32) -> bool {
        self.dirty_state_ids.contains(&state_id)
    }

    pub fn last_command_description(&self) -> Option<String> {
        self.undo_stack.last().map(StateEditCommand::description)
    }

    pub fn summary(&self) -> StateEditSummary {
        StateEditSummary {
            assigned_provinces: self.working.state_by_province.len(),
            unassigned_land_provinces: self.working.unassigned_land_provinces.len(),
            commands: self.undo_stack.len(),
            redo_commands: self.redo_stack.len(),
            modified_states: self.dirty_state_ids.len(),
            selected_provinces: self.selected_provinces.len(),
            target_state_id: self.target_state_id,
            session_errors: self
                .session_diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
                .count(),
            session_warnings: self
                .session_diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
                .count(),
            active_states: self.valid_state_ids.len(),
            created_states: self
                .valid_state_ids
                .iter()
                .filter(|state_id| {
                    self.origins_by_state.get(state_id)
                        == Some(&WorkingStateOrigin::CreatedInSession)
                })
                .count(),
            removed_states: self.removed_state_ids.len(),
            reserved_state_ids: self.reserved_state_ids().len(),
        }
    }

    pub fn set_target_state(&mut self, state_id: Option<u32>) -> Result<(), StateEditError> {
        if let Some(state_id) = state_id {
            self.validate_target_state(state_id)?;
        }
        self.target_state_id = state_id;
        Ok(())
    }

    pub fn validate_brush_target(
        &self,
        mode: StateBrushMode,
        target_state_id: Option<u32>,
    ) -> Result<Option<u32>, StateEditError> {
        match mode {
            StateBrushMode::AssignToTarget => {
                let state_id = target_state_id.ok_or(StateEditError::TargetStateInvalid(0))?;
                self.validate_target_state(state_id)?;
                Ok(Some(state_id))
            }
            StateBrushMode::Unassign => Ok(None),
        }
    }

    pub fn classify_brush_province(
        &self,
        province_id: u32,
        mode: StateBrushMode,
        target_state_id: Option<u32>,
    ) -> BrushProvinceClassification {
        if province_id == 0 {
            return BrushProvinceClassification::IgnoredNonLand;
        }
        match self.province_kinds.get(&province_id) {
            Some(ProvinceKind::Land) => {}
            Some(_) => return BrushProvinceClassification::IgnoredNonLand,
            None => return BrushProvinceClassification::Unknown,
        }
        if self.ambiguous_provinces.contains(&province_id) {
            return BrushProvinceClassification::BlockedAmbiguous;
        }
        if let Some(state_id) = self.working.state_by_province.get(&province_id)
            && !self.valid_state_ids.contains(state_id)
        {
            return BrushProvinceClassification::BlockedInvalidState;
        }
        let destination = match mode {
            StateBrushMode::AssignToTarget => target_state_id,
            StateBrushMode::Unassign => None,
        };
        if self.working.state_by_province.get(&province_id).copied() == destination {
            BrushProvinceClassification::NoOp
        } else {
            BrushProvinceClassification::Selectable
        }
    }

    pub fn set_visual_timings(&mut self, texture: Duration, boundaries: Duration) {
        self.last_timings.state_texture_update = texture;
        self.last_timings.state_boundary_update = boundaries;
    }

    pub fn toggle_selected_province(&mut self, province_id: u32) -> Result<bool, StateEditError> {
        self.validate_selectable_province(province_id)?;
        if !self.selected_provinces.insert(province_id) {
            self.selected_provinces.remove(&province_id);
            Ok(false)
        } else {
            Ok(true)
        }
    }

    pub fn apply_lasso_selection(
        &mut self,
        province_ids: &BTreeSet<u32>,
        mode: LassoSelectionMode,
    ) -> Result<usize, StateEditError> {
        for &province_id in province_ids {
            self.validate_selectable_province(province_id)?;
        }
        match mode {
            LassoSelectionMode::Replace => self.selected_provinces = province_ids.clone(),
            LassoSelectionMode::Add => self.selected_provinces.extend(province_ids),
            LassoSelectionMode::Remove => {
                self.selected_provinces
                    .retain(|province_id| !province_ids.contains(province_id));
            }
        }
        Ok(self.selected_provinces.len())
    }

    pub fn clear_selected_provinces(&mut self) -> bool {
        let had_selection = !self.selected_provinces.is_empty();
        self.selected_provinces.clear();
        had_selection
    }

    pub fn select_target_state_provinces(&mut self) -> Result<usize, StateEditError> {
        let Some(target_state_id) = self.target_state_id else {
            return Err(StateEditError::TargetStateInvalid(0));
        };
        self.validate_target_state(target_state_id)?;
        self.selected_provinces = self
            .working
            .provinces_by_state
            .get(&target_state_id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|province_id| self.validate_selectable_province(*province_id).is_ok())
            .collect();
        Ok(self.selected_provinces.len())
    }

    pub fn clear_target_state(&mut self) {
        self.target_state_id = None;
    }

    pub fn move_selection_to_target(&mut self) -> Result<(), StateEditError> {
        let Some(target_state_id) = self.target_state_id else {
            return Err(StateEditError::TargetStateInvalid(0));
        };
        self.reassign_selected(Some(target_state_id))
    }

    pub fn unassign_selection(&mut self) -> Result<(), StateEditError> {
        self.reassign_selected(None)
    }

    pub fn reassign_selected(
        &mut self,
        target_state_id: Option<u32>,
    ) -> Result<(), StateEditError> {
        let provinces = self.selected_provinces.iter().copied().collect::<Vec<_>>();
        self.reassign_provinces(&provinces, target_state_id)
    }

    fn can_reassign_selected(&self, target_state_id: Option<u32>) -> bool {
        let provinces = self.selected_provinces.iter().copied().collect::<Vec<_>>();
        self.build_command(&provinces, target_state_id)
            .is_ok_and(|command| !command.is_noop())
    }

    pub fn reassign_provinces(
        &mut self,
        province_ids: &[u32],
        target_state_id: Option<u32>,
    ) -> Result<(), StateEditError> {
        self.last_changed_provinces.clear();
        let preflight_started = Instant::now();
        let command = self.build_command(province_ids, target_state_id);
        self.last_timings.command_preflight = preflight_started.elapsed();
        let command = command?;
        if command.is_noop() {
            return Ok(());
        }

        let apply_started = Instant::now();
        self.apply_command(&command, Direction::Forward);
        self.last_timings.command_apply = apply_started.elapsed();

        let index_started = Instant::now();
        if let Err(err) = self.validate_invariants() {
            self.apply_command(&command, Direction::Backward);
            return Err(err);
        }
        self.recompute_dirty();
        self.last_timings.index_update = index_started.elapsed();

        self.push_command_diagnostics(&command);
        self.last_changed_provinces
            .extend(command.changed_provinces());
        self.undo_stack.push(command);
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn update_state_properties(
        &mut self,
        state_id: u32,
        after: EditableStateProperties,
    ) -> Result<bool, StateEditError> {
        self.validate_target_state(state_id)?;
        self.last_changed_provinces.clear();
        let before = self
            .working
            .properties_by_state
            .get(&state_id)
            .cloned()
            .ok_or(StateEditError::TargetStateNotFound(state_id))?;
        if before == after {
            return Ok(false);
        }
        let command = StateEditCommand::UpdateStateProperties {
            change: Box::new(StatePropertyChange {
                state_id,
                before,
                after,
            }),
        };
        self.apply_command(&command, Direction::Forward);
        self.recompute_dirty();
        self.push_command_diagnostics(&command);
        self.undo_stack.push(command);
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn create_state(
        &mut self,
        state_id: u32,
        properties: EditableStateProperties,
        use_selected: bool,
    ) -> Result<(), StateEditError> {
        let province_ids = if use_selected {
            self.selected_provinces.iter().copied().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.create_state_with_provinces(state_id, properties, &province_ids)
    }

    pub fn create_state_with_provinces(
        &mut self,
        state_id: u32,
        properties: EditableStateProperties,
        province_ids: &[u32],
    ) -> Result<(), StateEditError> {
        self.last_changed_provinces.clear();
        let command = self.build_create_command(state_id, properties, province_ids)?;
        self.apply_new_command(command)?;
        self.target_state_id = Some(state_id);
        self.selected_provinces.clear();
        Ok(())
    }

    pub fn create_state_from_selection(
        &mut self,
        state_id: u32,
        properties: EditableStateProperties,
    ) -> Result<(), StateEditError> {
        self.create_state(state_id, properties, true)
    }

    pub fn remove_state(
        &mut self,
        state_id: u32,
        policy: StateRemovalPolicy,
    ) -> Result<(), StateEditError> {
        self.last_changed_provinces.clear();
        let command = self.build_remove_command(state_id, policy)?;
        let target = match policy {
            StateRemovalPolicy::MoveToState(target_state_id) => Some(target_state_id),
            StateRemovalPolicy::Unassign => None,
        };
        self.apply_new_command(command)?;
        if self.target_state_id == Some(state_id) {
            self.target_state_id = target;
        }
        self.selected_provinces.retain(|province_id| {
            self.working.state_by_province.contains_key(province_id)
                || self.working.unassigned_land_provinces.contains(province_id)
        });
        Ok(())
    }

    fn build_create_command(
        &self,
        state_id: u32,
        properties: EditableStateProperties,
        province_ids: &[u32],
    ) -> Result<StateEditCommand, StateEditError> {
        self.validate_new_state_id(state_id)?;
        properties
            .validate()
            .map_err(StateEditError::InvalidStateProperties)?;
        Ok(StateEditCommand::CreateState {
            state: Box::new(WorkingStateSnapshot {
                state_id,
                properties,
                provinces: BTreeSet::new(),
                victory_points: Vec::new(),
                province_buildings: BTreeMap::new(),
                origin: WorkingStateOrigin::CreatedInSession,
                has_dated_history: false,
            }),
            deltas: self.build_province_deltas(province_ids, Some(state_id))?,
        })
    }

    fn build_remove_command(
        &self,
        state_id: u32,
        policy: StateRemovalPolicy,
    ) -> Result<StateEditCommand, StateEditError> {
        self.validate_removable_state(state_id)?;
        let target = match policy {
            StateRemovalPolicy::MoveToState(target_state_id) => {
                if target_state_id == state_id {
                    return Err(StateEditError::StateRemovalTargetMatches(state_id));
                }
                self.validate_target_state(target_state_id)?;
                Some(target_state_id)
            }
            StateRemovalPolicy::Unassign => None,
        };
        let state = self.snapshot_state(state_id)?;
        let province_ids = state.provinces.iter().copied().collect::<Vec<_>>();
        let deltas = self.build_province_deltas(&province_ids, target)?;
        Ok(StateEditCommand::RemoveState {
            state: Box::new(state),
            policy,
            deltas,
        })
    }

    pub fn update_province_data(
        &mut self,
        province_id: u32,
        state_id: u32,
        after: EditableProvinceData,
    ) -> Result<bool, StateEditError> {
        self.last_changed_provinces.clear();
        after
            .validate()
            .map_err(StateEditError::InvalidProvinceData)?;
        let actual_state_id = self.editable_province_state(province_id)?;
        if actual_state_id != state_id {
            return Err(StateEditError::ProvinceStateChanged {
                province_id,
                expected_state_id: state_id,
                actual_state_id: Some(actual_state_id),
            });
        }
        let before = self.working.province_data(Some(state_id), province_id);
        if before == after {
            return Ok(false);
        }
        let command = StateEditCommand::UpdateProvinceData {
            province_id,
            state_id,
            before,
            after,
        };
        self.apply_command(&command, Direction::Forward);
        self.recompute_dirty();
        self.push_command_diagnostics(&command);
        self.undo_stack.push(command);
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        let started = Instant::now();
        self.last_changed_provinces.clear();
        let Some(command) = self.undo_stack.pop() else {
            return false;
        };
        self.apply_command(&command, Direction::Backward);
        if let Err(error) = self.validate_invariants() {
            self.apply_command(&command, Direction::Forward);
            self.undo_stack.push(command);
            self.push_invariant_diagnostic(error);
            self.last_timings.undo = started.elapsed();
            return false;
        }
        self.recompute_dirty();
        self.last_changed_provinces
            .extend(command.changed_provinces());
        match &command {
            StateEditCommand::CreateState { state, .. }
                if self.target_state_id == Some(state.state_id) =>
            {
                self.target_state_id = None;
            }
            StateEditCommand::RemoveState { state, .. } => {
                self.target_state_id = Some(state.state_id);
            }
            _ => {}
        }
        self.redo_stack.push(command);
        self.rebuild_session_diagnostics();
        self.last_timings.undo = started.elapsed();
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let started = Instant::now();
        self.last_changed_provinces.clear();
        let Some(command) = self.redo_stack.pop() else {
            return false;
        };
        self.apply_command(&command, Direction::Forward);
        if let Err(error) = self.validate_invariants() {
            self.apply_command(&command, Direction::Backward);
            self.redo_stack.push(command);
            self.push_invariant_diagnostic(error);
            self.last_timings.redo = started.elapsed();
            return false;
        }
        self.recompute_dirty();
        self.last_changed_provinces
            .extend(command.changed_provinces());
        match &command {
            StateEditCommand::CreateState { state, .. } => {
                self.target_state_id = Some(state.state_id);
            }
            StateEditCommand::RemoveState { state, policy, .. }
                if self.target_state_id == Some(state.state_id) =>
            {
                self.target_state_id = match policy {
                    StateRemovalPolicy::MoveToState(target_state_id) => Some(*target_state_id),
                    StateRemovalPolicy::Unassign => None,
                };
            }
            _ => {}
        }
        self.undo_stack.push(command);
        self.rebuild_session_diagnostics();
        self.last_timings.redo = started.elapsed();
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn discard(&mut self) {
        let started = Instant::now();
        self.last_changed_provinces = self
            .baseline
            .state_by_province
            .keys()
            .chain(self.working.state_by_province.keys())
            .copied()
            .filter(|province_id| {
                self.baseline.state_by_province.get(province_id)
                    != self.working.state_by_province.get(province_id)
            })
            .collect();
        self.working = self.baseline.clone();
        self.origins_by_state
            .retain(|_, origin| matches!(origin, WorkingStateOrigin::Loaded { .. }));
        self.valid_state_ids = self.origins_by_state.keys().copied().collect();
        self.removed_state_ids.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty_state_ids.clear();
        self.selected_provinces.clear();
        self.target_state_id = None;
        self.session_diagnostics.clear();
        self.last_timings.discard = started.elapsed();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn validate_invariants(&self) -> Result<(), StateEditError> {
        for (province_id, state_id) in &self.working.state_by_province {
            if !self
                .working
                .provinces_by_state
                .get(state_id)
                .is_some_and(|provinces| provinces.contains(province_id))
            {
                return Err(StateEditError::InvariantViolation(format!(
                    "province {province_id} points to state {state_id}, but the state province list disagrees"
                )));
            }
        }

        for (state_id, provinces) in &self.working.provinces_by_state {
            for province_id in provinces {
                if self.working.state_by_province.get(province_id) != Some(state_id) {
                    return Err(StateEditError::InvariantViolation(format!(
                        "state {state_id} contains province {province_id}, but the province index disagrees"
                    )));
                }
            }
        }

        Ok(())
    }

    fn build_command(
        &self,
        province_ids: &[u32],
        target_state_id: Option<u32>,
    ) -> Result<StateEditCommand, StateEditError> {
        if province_ids.is_empty() {
            return Err(StateEditError::EmptySelection);
        }
        if let Some(target_state_id) = target_state_id {
            self.validate_target_state(target_state_id)?;
        }

        Ok(StateEditCommand::ReassignProvinces {
            deltas: self.build_province_deltas(province_ids, target_state_id)?,
        })
    }

    fn build_province_deltas(
        &self,
        province_ids: &[u32],
        target_state_id: Option<u32>,
    ) -> Result<Vec<ProvinceEditDelta>, StateEditError> {
        let mut unique = BTreeSet::new();
        let mut deltas = Vec::new();
        for &province_id in province_ids {
            if !unique.insert(province_id) {
                continue;
            }
            self.validate_selectable_province(province_id)?;
            let from_state_id = self.working.state_by_province.get(&province_id).copied();
            if from_state_id == target_state_id {
                continue;
            }
            if let Some(target_state_id) = target_state_id {
                self.validate_no_provincial_data_conflict(
                    province_id,
                    target_state_id,
                    from_state_id,
                )?;
            }
            deltas.push(ProvinceEditDelta {
                province_id,
                from_state_id,
                to_state_id: target_state_id,
                victory_points: self
                    .working
                    .victory_points_owned(from_state_id, province_id),
                province_buildings: self
                    .working
                    .province_buildings_for(from_state_id, province_id)
                    .cloned(),
            });
        }
        Ok(deltas)
    }

    fn apply_new_command(&mut self, command: StateEditCommand) -> Result<(), StateEditError> {
        self.apply_command(&command, Direction::Forward);
        if let Err(error) = self.validate_invariants() {
            self.apply_command(&command, Direction::Backward);
            return Err(error);
        }
        self.recompute_dirty();
        self.push_command_diagnostics(&command);
        self.last_changed_provinces
            .extend(command.changed_provinces());
        self.undo_stack.push(command);
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    fn reserved_state_ids(&self) -> BTreeSet<u32> {
        let mut reserved = self.known_state_ids.clone();
        reserved.extend(self.valid_state_ids.iter().copied());
        for command in self.undo_stack.iter().chain(&self.redo_stack) {
            if let StateEditCommand::CreateState { state, .. } = command {
                reserved.insert(state.state_id);
            }
        }
        reserved
    }

    fn snapshot_state(&self, state_id: u32) -> Result<WorkingStateSnapshot, StateEditError> {
        Ok(WorkingStateSnapshot {
            state_id,
            properties: self
                .working
                .properties_by_state
                .get(&state_id)
                .cloned()
                .ok_or(StateEditError::TargetStateNotFound(state_id))?,
            provinces: self
                .working
                .provinces_by_state
                .get(&state_id)
                .cloned()
                .unwrap_or_default(),
            victory_points: self
                .working
                .victory_points_by_state
                .get(&state_id)
                .cloned()
                .unwrap_or_default(),
            province_buildings: self
                .working
                .province_buildings_by_state
                .get(&state_id)
                .cloned()
                .unwrap_or_default(),
            origin: self
                .origins_by_state
                .get(&state_id)
                .cloned()
                .ok_or(StateEditError::TargetStateNotFound(state_id))?,
            has_dated_history: self.working.dated_history_states.contains(&state_id),
        })
    }

    fn validate_target_state(&self, state_id: u32) -> Result<(), StateEditError> {
        if state_id == 0 {
            return Err(StateEditError::TargetStateInvalid(state_id));
        }
        if self.known_state_ids.contains(&state_id) && !self.valid_state_ids.contains(&state_id) {
            return Err(StateEditError::TargetStateInvalid(state_id));
        }
        if !self.valid_state_ids.contains(&state_id) {
            return Err(StateEditError::TargetStateNotFound(state_id));
        }
        Ok(())
    }

    fn validate_selectable_province(&self, province_id: u32) -> Result<(), StateEditError> {
        let Some(kind) = self.province_kinds.get(&province_id).copied() else {
            return Err(StateEditError::ProvinceNotFound(province_id));
        };
        if kind != ProvinceKind::Land {
            return Err(StateEditError::ProvinceIsNotLand(province_id));
        }
        if self.ambiguous_provinces.contains(&province_id) {
            return Err(StateEditError::AmbiguousProvince(province_id));
        }
        if let Some(&state_id) = self.working.state_by_province.get(&province_id)
            && !self.valid_state_ids.contains(&state_id)
        {
            return Err(StateEditError::ProvinceInInvalidState {
                province_id,
                state_id,
            });
        }
        Ok(())
    }

    fn validate_no_provincial_data_conflict(
        &self,
        province_id: u32,
        target_state_id: u32,
        from_state_id: Option<u32>,
    ) -> Result<(), StateEditError> {
        if from_state_id == Some(target_state_id) {
            return Ok(());
        }
        let incoming_vp = !self
            .working
            .victory_points_owned(from_state_id, province_id)
            .is_empty();
        let target_vp = !self
            .working
            .victory_points_owned(Some(target_state_id), province_id)
            .is_empty();
        let incoming_buildings = self
            .working
            .province_buildings_for(from_state_id, province_id);
        let target_buildings = self
            .working
            .province_buildings_for(Some(target_state_id), province_id);
        if (incoming_vp && target_vp) || incoming_buildings.zip(target_buildings).is_some() {
            return Err(StateEditError::ProvincialDataConflict {
                province_id,
                target_state_id,
            });
        }
        Ok(())
    }

    fn apply_command(&mut self, command: &StateEditCommand, direction: Direction) {
        match command {
            StateEditCommand::ReassignProvinces { deltas } => {
                self.apply_province_deltas(deltas, direction);
            }
            StateEditCommand::UpdateStateProperties { change } => {
                let properties = match direction {
                    Direction::Forward => &change.after,
                    Direction::Backward => &change.before,
                };
                self.working
                    .properties_by_state
                    .insert(change.state_id, properties.clone());
            }
            StateEditCommand::UpdateProvinceData {
                province_id,
                state_id,
                before,
                after,
            } => {
                let data = match direction {
                    Direction::Forward => after,
                    Direction::Backward => before,
                };
                self.working
                    .set_province_data(*state_id, *province_id, data);
            }
            StateEditCommand::CreateState { state, deltas } => match direction {
                Direction::Forward => {
                    self.restore_state(state);
                    self.apply_province_deltas(deltas, Direction::Forward);
                }
                Direction::Backward => {
                    self.apply_province_deltas(deltas, Direction::Backward);
                    self.remove_state_snapshot(state, false);
                }
            },
            StateEditCommand::RemoveState { state, deltas, .. } => match direction {
                Direction::Forward => {
                    self.apply_province_deltas(deltas, Direction::Forward);
                    self.remove_state_snapshot(state, true);
                }
                Direction::Backward => {
                    self.restore_state(state);
                    for delta in deltas {
                        StateWorkingSet::remove_victory_points(
                            &mut self.working.victory_points_by_state,
                            &mut self.working.detached_victory_points,
                            Some(state.state_id),
                            delta.province_id,
                        );
                        StateWorkingSet::remove_province_buildings(
                            &mut self.working.province_buildings_by_state,
                            &mut self.working.detached_province_buildings,
                            Some(state.state_id),
                            delta.province_id,
                        );
                    }
                    self.apply_province_deltas(deltas, Direction::Backward);
                    self.working
                        .provinces_by_state
                        .insert(state.state_id, state.provinces.clone());
                    self.working
                        .victory_points_by_state
                        .insert(state.state_id, state.victory_points.clone());
                    self.working
                        .province_buildings_by_state
                        .insert(state.state_id, state.province_buildings.clone());
                }
            },
        }
    }

    fn apply_province_deltas(&mut self, deltas: &[ProvinceEditDelta], direction: Direction) {
        for delta in deltas {
            let (from, to) = match direction {
                Direction::Forward => (delta.from_state_id, delta.to_state_id),
                Direction::Backward => (delta.to_state_id, delta.from_state_id),
            };
            self.working.move_province(delta.province_id, from, to);
            self.working.move_victory_points(delta, from, to);
            self.working.move_province_buildings(delta, from, to);
        }
        self.working.recompute_unassigned_land(&self.province_kinds);
    }

    fn restore_state(&mut self, state: &WorkingStateSnapshot) {
        self.working
            .properties_by_state
            .insert(state.state_id, state.properties.clone());
        self.working
            .provinces_by_state
            .insert(state.state_id, state.provinces.clone());
        self.working
            .victory_points_by_state
            .insert(state.state_id, state.victory_points.clone());
        self.working
            .province_buildings_by_state
            .insert(state.state_id, state.province_buildings.clone());
        if state.has_dated_history {
            self.working.dated_history_states.insert(state.state_id);
        } else {
            self.working.dated_history_states.remove(&state.state_id);
        }
        self.origins_by_state
            .insert(state.state_id, state.origin.clone());
        self.removed_state_ids.remove(&state.state_id);
        self.valid_state_ids.insert(state.state_id);
    }

    fn remove_state_snapshot(&mut self, state: &WorkingStateSnapshot, tombstone: bool) {
        self.working.properties_by_state.remove(&state.state_id);
        self.working.provinces_by_state.remove(&state.state_id);
        self.working.victory_points_by_state.remove(&state.state_id);
        self.working
            .province_buildings_by_state
            .remove(&state.state_id);
        self.working.dated_history_states.remove(&state.state_id);
        self.valid_state_ids.remove(&state.state_id);
        if tombstone {
            self.removed_state_ids.insert(state.state_id);
        } else {
            self.removed_state_ids.remove(&state.state_id);
            self.origins_by_state.remove(&state.state_id);
        }
    }

    fn recompute_dirty(&mut self) {
        self.dirty_state_ids.clear();
        let ids = self
            .baseline
            .provinces_by_state
            .keys()
            .chain(self.working.provinces_by_state.keys())
            .chain(self.baseline.properties_by_state.keys())
            .chain(self.working.properties_by_state.keys())
            .chain(self.baseline.victory_points_by_state.keys())
            .chain(self.working.victory_points_by_state.keys())
            .chain(self.baseline.province_buildings_by_state.keys())
            .chain(self.working.province_buildings_by_state.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        for id in ids {
            if self.baseline.provinces_by_state.get(&id) != self.working.provinces_by_state.get(&id)
                || self.baseline.properties_by_state.get(&id)
                    != self.working.properties_by_state.get(&id)
                || !victory_points_match(
                    self.baseline.victory_points_by_state.get(&id),
                    self.working.victory_points_by_state.get(&id),
                )
                || self.baseline.province_buildings_by_state.get(&id)
                    != self.working.province_buildings_by_state.get(&id)
            {
                self.dirty_state_ids.insert(id);
            }
        }
    }

    fn push_command_diagnostics(&mut self, command: &StateEditCommand) {
        match command {
            StateEditCommand::ReassignProvinces { deltas } => {
                for delta in deltas {
                    self.session_diagnostics.push(ProjectDiagnostic::new(
                        ProjectDiagnosticKind::StateEditSession,
                        DiagnosticSeverity::Info,
                        None,
                        None,
                        format!(
                            "Province {} was moved from {} to {} in memory.",
                            delta.province_id,
                            state_label(delta.from_state_id),
                            state_label(delta.to_state_id)
                        ),
                    ));
                    if delta.to_state_id.is_none() {
                        self.session_diagnostics.push(ProjectDiagnostic::new(
                            ProjectDiagnosticKind::StateEditSession,
                            DiagnosticSeverity::Warning,
                            None,
                            None,
                            format!(
                                "Province {} is currently unassigned due to an unsaved edit.",
                                delta.province_id
                            ),
                        ));
                    }
                    for state_id in [delta.from_state_id, delta.to_state_id]
                        .into_iter()
                        .flatten()
                    {
                        if self.state_has_dated_history(state_id) {
                            self.session_diagnostics.push(ProjectDiagnostic::new(
              ProjectDiagnosticKind::StateEditSession,
              DiagnosticSeverity::Warning,
              None,
              None,
              format!(
                "State {state_id} contains dated history blocks; unknown references to province {} were not edited.",
                delta.province_id
              ),
            ));
                        }
                    }
                }
            }
            StateEditCommand::UpdateStateProperties { change } => {
                self.session_diagnostics.push(ProjectDiagnostic::new(
                    ProjectDiagnosticKind::StateEditSession,
                    DiagnosticSeverity::Info,
                    None,
                    None,
                    format!(
                        "State {} properties were updated in memory.",
                        change.state_id
                    ),
                ));
            }
            StateEditCommand::UpdateProvinceData {
                province_id,
                state_id,
                ..
            } => {
                self.session_diagnostics.push(ProjectDiagnostic::new(
                    ProjectDiagnosticKind::StateEditSession,
                    DiagnosticSeverity::Info,
                    None,
                    None,
                    format!(
                        "Province {province_id} data in State {state_id} was updated in memory."
                    ),
                ));
            }
            StateEditCommand::CreateState { state, deltas } => {
                self.session_diagnostics.push(ProjectDiagnostic::new(
                    ProjectDiagnosticKind::StateEditSession,
                    DiagnosticSeverity::Info,
                    None,
                    None,
                    format!(
                        "State {} was created in memory and has no backing file.",
                        state.state_id
                    ),
                ));
                if deltas.is_empty() {
                    self.session_diagnostics.push(ProjectDiagnostic::new(
                        ProjectDiagnosticKind::StateEditSession,
                        DiagnosticSeverity::Warning,
                        None,
                        None,
                        format!("State {} currently has no provinces.", state.state_id),
                    ));
                }
            }
            StateEditCommand::RemoveState {
                state,
                policy,
                deltas,
            } => {
                self.session_diagnostics.push(ProjectDiagnostic::new(
                    ProjectDiagnosticKind::StateEditSession,
                    DiagnosticSeverity::Info,
                    None,
                    None,
                    format!(
                        "State {} is marked as removed in the current session.",
                        state.state_id
                    ),
                ));
                let message = match policy {
                    StateRemovalPolicy::MoveToState(target_state_id) => format!(
                        "{} provinces were moved from removed State {} to State {}.",
                        deltas.len(),
                        state.state_id,
                        target_state_id,
                    ),
                    StateRemovalPolicy::Unassign => format!(
                        "{} provinces are temporarily unassigned after removing State {}.",
                        deltas.len(),
                        state.state_id,
                    ),
                };
                self.session_diagnostics.push(ProjectDiagnostic::new(
                    ProjectDiagnosticKind::StateEditSession,
                    if matches!(policy, StateRemovalPolicy::Unassign) && !deltas.is_empty() {
                        DiagnosticSeverity::Warning
                    } else {
                        DiagnosticSeverity::Info
                    },
                    None,
                    None,
                    message,
                ));
            }
        }
    }

    fn push_invariant_diagnostic(&mut self, error: StateEditError) {
        self.session_diagnostics.push(ProjectDiagnostic::new(
            ProjectDiagnosticKind::StateEditSession,
            DiagnosticSeverity::Error,
            None,
            None,
            format!("State edit history was rolled back: {error}."),
        ));
    }

    fn rebuild_session_diagnostics(&mut self) {
        self.session_diagnostics.clear();
        for command in self.undo_stack.clone() {
            self.push_command_diagnostics(&command);
        }
    }

    fn state_has_dated_history(&self, state_id: u32) -> bool {
        self.baseline.dated_history_states.contains(&state_id)
    }
}

impl StateEditCommand {
    fn is_noop(&self) -> bool {
        match self {
            Self::ReassignProvinces { deltas } => deltas.is_empty(),
            Self::UpdateStateProperties { change } => change.before == change.after,
            Self::UpdateProvinceData { before, after, .. } => before == after,
            Self::CreateState { .. } | Self::RemoveState { .. } => false,
        }
    }

    fn changed_provinces(&self) -> Vec<u32> {
        match self {
            Self::ReassignProvinces { deltas } => {
                deltas.iter().map(|delta| delta.province_id).collect()
            }
            Self::UpdateStateProperties { .. } => Vec::new(),
            Self::UpdateProvinceData { .. } => Vec::new(),
            Self::CreateState { deltas, .. } | Self::RemoveState { deltas, .. } => {
                deltas.iter().map(|delta| delta.province_id).collect()
            }
        }
    }

    fn description(&self) -> String {
        match self {
            Self::ReassignProvinces { deltas } => {
                format!("Reassigned {} province(s)", deltas.len())
            }
            Self::UpdateStateProperties { change } => {
                format!("Updated State {} properties", change.state_id)
            }
            Self::UpdateProvinceData { province_id, .. } => {
                format!("Updated Province {province_id} data")
            }
            Self::CreateState { state, deltas } => {
                format!(
                    "Created State {} with {} province(s)",
                    state.state_id,
                    deltas.len()
                )
            }
            Self::RemoveState { state, deltas, .. } => {
                format!(
                    "Removed State {} with {} province(s)",
                    state.state_id,
                    deltas.len()
                )
            }
        }
    }
}

impl StateWorkingSet {
    fn from_project(project: &Hoi4Project, province_kinds: &BTreeMap<u32, ProvinceKind>) -> Self {
        let mut provinces_by_state = BTreeMap::<u32, BTreeSet<u32>>::new();
        for (&province_id, &state_id) in &project.state_by_province {
            provinces_by_state
                .entry(state_id)
                .or_default()
                .insert(province_id);
        }

        let mut victory_points_by_state = BTreeMap::new();
        let mut properties_by_state = BTreeMap::new();
        let mut province_buildings_by_state = BTreeMap::new();
        let mut dated_history_states = BTreeSet::new();
        for (&state_id, &document_index) in &project.states_by_id {
            let Some(data) = project
                .states
                .get(document_index)
                .and_then(|document| document.data.as_ref())
            else {
                continue;
            };
            properties_by_state.insert(state_id, EditableStateProperties::from_state(data));
            victory_points_by_state.insert(state_id, data.history.victory_points.clone());
            province_buildings_by_state.insert(state_id, data.history.province_buildings.clone());
            if !data.history.dated_blocks.is_empty() {
                dated_history_states.insert(state_id);
            }
            provinces_by_state.entry(state_id).or_default();
        }

        let mut out = Self {
            state_by_province: project.state_by_province.clone(),
            provinces_by_state,
            properties_by_state,
            victory_points_by_state,
            province_buildings_by_state,
            detached_victory_points: BTreeMap::new(),
            detached_province_buildings: BTreeMap::new(),
            unassigned_land_provinces: BTreeSet::new(),
            dated_history_states,
        };
        out.recompute_unassigned_land(province_kinds);
        out
    }

    pub fn state_data(&self, state_id: u32) -> StateData {
        let mut data = StateData {
            id: Some(state_id),
            provinces: self
                .provinces_by_state
                .get(&state_id)
                .cloned()
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Some(properties) = self.properties_by_state.get(&state_id) {
            properties.apply_to(&mut data);
        }
        data.history.victory_points = self
            .victory_points_by_state
            .get(&state_id)
            .cloned()
            .unwrap_or_default();
        data.history.province_buildings = self
            .province_buildings_by_state
            .get(&state_id)
            .cloned()
            .unwrap_or_default();
        data
    }

    fn recompute_unassigned_land(&mut self, province_kinds: &BTreeMap<u32, ProvinceKind>) {
        self.unassigned_land_provinces = province_kinds
            .iter()
            .filter_map(|(&province_id, &kind)| {
                (kind == ProvinceKind::Land && !self.state_by_province.contains_key(&province_id))
                    .then_some(province_id)
            })
            .collect();
    }

    fn move_province(&mut self, province_id: u32, from: Option<u32>, to: Option<u32>) {
        if let Some(from) = from
            && let Some(provinces) = self.provinces_by_state.get_mut(&from)
        {
            provinces.remove(&province_id);
        }
        if let Some(to) = to {
            self.provinces_by_state
                .entry(to)
                .or_default()
                .insert(province_id);
            self.state_by_province.insert(province_id, to);
        } else {
            self.state_by_province.remove(&province_id);
        }
    }

    fn victory_points_owned(&self, state_id: Option<u32>, province_id: u32) -> Vec<VictoryPoint> {
        if let Some(state_id) = state_id {
            self.victory_points_by_state
                .get(&state_id)
                .iter()
                .flat_map(|vps| vps.iter())
                .filter(|vp| vp.province_id == province_id)
                .cloned()
                .collect()
        } else {
            self.detached_victory_points
                .get(&province_id)
                .cloned()
                .unwrap_or_default()
        }
    }

    fn province_buildings_for(
        &self,
        state_id: Option<u32>,
        province_id: u32,
    ) -> Option<&BTreeMap<String, i64>> {
        if let Some(state_id) = state_id {
            self.province_buildings_by_state
                .get(&state_id)
                .and_then(|buildings| buildings.get(&province_id))
        } else {
            self.detached_province_buildings.get(&province_id)
        }
    }

    fn province_data(&self, state_id: Option<u32>, province_id: u32) -> EditableProvinceData {
        EditableProvinceData {
            victory_point: self
                .victory_points_owned(state_id, province_id)
                .first()
                .map(|victory_point| victory_point.value),
            buildings: self
                .province_buildings_for(state_id, province_id)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn set_province_data(&mut self, state_id: u32, province_id: u32, data: &EditableProvinceData) {
        let victory_points = self.victory_points_by_state.entry(state_id).or_default();
        let original_position = victory_points
            .iter()
            .position(|victory_point| victory_point.province_id == province_id);
        victory_points.retain(|victory_point| victory_point.province_id != province_id);
        if let Some(value) = data.victory_point {
            let position = original_position
                .unwrap_or(victory_points.len())
                .min(victory_points.len());
            victory_points.insert(position, VictoryPoint { province_id, value });
        }
        let province_buildings = self
            .province_buildings_by_state
            .entry(state_id)
            .or_default();
        if data.buildings.is_empty() {
            province_buildings.remove(&province_id);
        } else {
            province_buildings.insert(province_id, data.buildings.clone());
        }
    }

    fn move_victory_points(
        &mut self,
        delta: &ProvinceEditDelta,
        from: Option<u32>,
        to: Option<u32>,
    ) {
        Self::remove_victory_points(
            &mut self.victory_points_by_state,
            &mut self.detached_victory_points,
            from,
            delta.province_id,
        );
        if delta.victory_points.is_empty() {
            return;
        }
        if let Some(to) = to {
            self.victory_points_by_state
                .entry(to)
                .or_default()
                .extend(delta.victory_points.clone());
        } else {
            self.detached_victory_points
                .insert(delta.province_id, delta.victory_points.clone());
        }
    }

    fn remove_victory_points(
        states: &mut BTreeMap<u32, Vec<VictoryPoint>>,
        detached: &mut BTreeMap<u32, Vec<VictoryPoint>>,
        state_id: Option<u32>,
        province_id: u32,
    ) {
        if let Some(state_id) = state_id {
            if let Some(vps) = states.get_mut(&state_id) {
                vps.retain(|vp| vp.province_id != province_id);
            }
        } else {
            detached.remove(&province_id);
        }
    }

    fn move_province_buildings(
        &mut self,
        delta: &ProvinceEditDelta,
        from: Option<u32>,
        to: Option<u32>,
    ) {
        Self::remove_province_buildings(
            &mut self.province_buildings_by_state,
            &mut self.detached_province_buildings,
            from,
            delta.province_id,
        );
        let Some(buildings) = delta.province_buildings.clone() else {
            return;
        };
        if let Some(to) = to {
            self.province_buildings_by_state
                .entry(to)
                .or_default()
                .insert(delta.province_id, buildings);
        } else {
            self.detached_province_buildings
                .insert(delta.province_id, buildings);
        }
    }

    fn remove_province_buildings(
        states: &mut BTreeMap<u32, BTreeMap<u32, BTreeMap<String, i64>>>,
        detached: &mut BTreeMap<u32, BTreeMap<String, i64>>,
        state_id: Option<u32>,
        province_id: u32,
    ) {
        if let Some(state_id) = state_id {
            if let Some(buildings) = states.get_mut(&state_id) {
                buildings.remove(&province_id);
            }
        } else {
            detached.remove(&province_id);
        }
    }
}

impl fmt::Display for StateEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateEditError::EmptySelection => write!(f, "No provinces are selected"),
            StateEditError::ProvinceNotFound(id) => {
                write!(f, "Province {id} does not exist in the map")
            }
            StateEditError::ProvinceIsNotLand(id) => write!(
                f,
                "Province {id} is not land and cannot be assigned to a state"
            ),
            StateEditError::AmbiguousProvince(id) => {
                write!(f, "Province {id} is ambiguous in the original state files")
            }
            StateEditError::ProvinceInInvalidState {
                province_id,
                state_id,
            } => {
                write!(
                    f,
                    "Province {province_id} belongs to invalid state {state_id}"
                )
            }
            StateEditError::ProvinceUnassigned(id) => write!(
                f,
                "Assign this province to a valid state before adding provincial data. Province {id} is unassigned"
            ),
            StateEditError::ProvinceStateChanged {
                province_id,
                expected_state_id,
                actual_state_id,
            } => write!(
                f,
                "Province {province_id} moved from expected State {expected_state_id} to {} while its draft was open",
                state_label(*actual_state_id)
            ),
            StateEditError::InvalidProvinceData(message) => {
                write!(f, "Invalid province data: {message}")
            }
            StateEditError::InvalidStateProperties(message) => {
                write!(f, "Invalid state properties: {message}")
            }
            StateEditError::StateIdInvalid(id) => write!(f, "State ID {id} is invalid"),
            StateEditError::StateIdReserved(id) => write!(
                f,
                "State ID {id} is already occupied or reserved by edit history"
            ),
            StateEditError::StateAlreadyRemoved(id) => {
                write!(f, "State {id} is already removed from this session")
            }
            StateEditError::StateRemovalTargetMatches(id) => {
                write!(f, "State {id} cannot be its own removal target")
            }
            StateEditError::TargetStateNotFound(id) => write!(f, "State {id} is not indexed"),
            StateEditError::TargetStateInvalid(id) => {
                write!(f, "State {id} is not a valid edit target")
            }
            StateEditError::ProvincialDataConflict {
                province_id,
                target_state_id,
            } => {
                write!(
                    f,
                    "Province {province_id} has conflicting provincial data in target state {target_state_id}"
                )
            }
            StateEditError::InvariantViolation(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for StateEditError {}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

fn state_label(state_id: Option<u32>) -> String {
    state_id.map_or_else(|| "None".to_owned(), |id| format!("State {id}"))
}

fn state_id_from_filename(path: &std::path::Path) -> Option<u32> {
    let filename = path.file_name()?.to_str()?;
    let digits = filename
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    let state_id = digits.parse().ok()?;
    (state_id != 0).then_some(state_id)
}

fn victory_points_match(
    left: Option<&Vec<VictoryPoint>>,
    right: Option<&Vec<VictoryPoint>>,
) -> bool {
    let mut left = left.cloned().unwrap_or_default();
    let mut right = right.cloned().unwrap_or_default();
    left.sort_by_key(|victory_point| (victory_point.province_id, victory_point.value));
    right.sort_by_key(|victory_point| (victory_point.province_id, victory_point.value));
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::map::Bundle;
    use crate::app::project::view::{
        UNASSIGNED_LAND_COLOR, classify_province_color_for, generate_state_view_for, state_color,
    };
    use crate::app::project::{
        ProjectPaths, RoundTripCancellation, RoundTripStatus, RoundTripValidator, StateLoadSummary,
        plan_state_patches,
    };
    use crate::app::state::{StateDocument, parse_text};
    use crate::config::Config;
    use crate::util::files::Location;
    use std::path::PathBuf;

    fn document(id: u32, provinces: &[u32]) -> StateDocument {
        let mut data = StateData {
            id: Some(id),
            ..Default::default()
        };
        data.provinces.extend(provinces.iter().copied());
        StateDocument {
            path: PathBuf::from(format!("{id}.txt")),
            original_bytes: Vec::new().into(),
            exact_utf8: true,
            syntax: parse_text("", ""),
            data: Some(data),
            diagnostics: Vec::new(),
            modified: false,
        }
    }

    fn session() -> StateEditSession {
        let mut project = Hoi4Project {
            paths: ProjectPaths {
                root: PathBuf::new(),
                map_directory: PathBuf::new(),
                provinces_bmp: PathBuf::new(),
                definition_csv: PathBuf::new(),
                adjacencies_csv: None,
                rivers_bmp: None,
                history_directory: PathBuf::new(),
                states_directory: PathBuf::new(),
            },
            states: vec![document(1, &[10, 11, 12]), document(2, &[30])],
            states_by_id: BTreeMap::from([(1, 0), (2, 1)]),
            state_by_province: HashMap::from([(10, 1), (11, 1), (12, 1), (30, 2)]),
            ambiguous_provinces: BTreeMap::new(),
            unassigned_land_provinces: BTreeSet::new(),
            diagnostics: Vec::new(),
            load_summary: StateLoadSummary::default(),
        };
        project.states[0]
            .data
            .as_mut()
            .unwrap()
            .history
            .victory_points
            .push(VictoryPoint {
                province_id: 10,
                value: 5,
            });
        project.states[0]
            .data
            .as_mut()
            .unwrap()
            .history
            .province_buildings
            .insert(11, BTreeMap::from([("bunker".into(), 2)]));
        let province_kinds = BTreeMap::from([
            (10, ProvinceKind::Land),
            (11, ProvinceKind::Land),
            (12, ProvinceKind::Land),
            (20, ProvinceKind::Land),
            (30, ProvinceKind::Land),
            (40, ProvinceKind::Sea),
            (41, ProvinceKind::Lake),
        ]);
        let baseline = StateWorkingSet::from_project(&project, &province_kinds);
        StateEditSession {
            working: baseline.clone(),
            baseline,
            known_state_ids: BTreeSet::from([1, 2]),
            valid_state_ids: BTreeSet::from([1, 2]),
            origins_by_state: BTreeMap::from([
                (
                    1,
                    WorkingStateOrigin::Loaded {
                        document_path: PathBuf::from("1.txt"),
                    },
                ),
                (
                    2,
                    WorkingStateOrigin::Loaded {
                        document_path: PathBuf::from("2.txt"),
                    },
                ),
            ]),
            removed_state_ids: BTreeSet::new(),
            province_kinds,
            ambiguous_provinces: BTreeSet::new(),
            selected_provinces: BTreeSet::new(),
            target_state_id: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty_state_ids: BTreeSet::new(),
            session_diagnostics: Vec::new(),
            last_timings: StateEditTimings::default(),
            last_changed_provinces: BTreeSet::new(),
            revision: 0,
        }
    }

    #[test]
    fn state_edit_moves_one_province_atomically() {
        let mut edit = session();
        edit.reassign_provinces(&[10], Some(2)).unwrap();

        assert_eq!(edit.working.state_by_province.get(&10), Some(&2));
        assert!(!edit.working.provinces_by_state[&1].contains(&10));
        assert!(edit.working.provinces_by_state[&2].contains(&10));
        assert_eq!(edit.dirty_state_ids, BTreeSet::from([1, 2]));
        assert_eq!(edit.undo_stack.len(), 1);
        edit.validate_invariants().unwrap();
    }

    #[test]
    fn state_edit_visual_colors_follow_move_unassign_undo_and_redo() {
        let mut edit = session();
        let ambiguous = BTreeSet::new();
        let color = |edit: &StateEditSession| {
            classify_province_color_for(
                Some(10),
                ProvinceKind::Land,
                edit.state_by_province(),
                &ambiguous,
                edit.unassigned_land_provinces(),
            )
        };

        assert_eq!(color(&edit), state_color(1));
        edit.reassign_provinces(&[10], Some(2)).unwrap();
        assert_eq!(color(&edit), state_color(2));
        assert!(edit.undo());
        assert_eq!(color(&edit), state_color(1));
        assert!(edit.redo());
        assert_eq!(color(&edit), state_color(2));
        edit.reassign_provinces(&[10], None).unwrap();
        assert_eq!(color(&edit), UNASSIGNED_LAND_COLOR);
        assert!(edit.undo());
        assert_eq!(color(&edit), state_color(2));
        assert!(edit.redo());
        assert_eq!(color(&edit), UNASSIGNED_LAND_COLOR);
    }

    #[test]
    fn province_and_property_changes_share_one_ordered_history() {
        let mut edit = session();
        edit.reassign_provinces(&[10, 11, 12], Some(2)).unwrap();
        assert_eq!(edit.undo_stack.len(), 1);
        assert!(matches!(
          &edit.undo_stack[0],
          StateEditCommand::ReassignProvinces { deltas } if deltas.len() == 3
        ));
        let mut properties = EditableStateProperties::from_state(&edit.state_data(2).unwrap());
        properties.manpower = Some(150_000);
        assert!(edit.update_state_properties(2, properties).unwrap());
        assert_eq!(edit.undo_stack.len(), 2);
        assert_eq!(edit.state_data(2).unwrap().manpower, Some(150_000));

        assert!(edit.undo());
        assert_eq!(edit.state_data(2).unwrap().manpower, None);
        assert_eq!(edit.working.state_by_province.get(&10), Some(&2));
        assert!(edit.undo());
        assert_eq!(edit.working.state_by_province.get(&10), Some(&1));
        assert!(edit.redo());
        assert!(edit.redo());
        assert_eq!(edit.state_data(2).unwrap().manpower, Some(150_000));
    }

    #[test]
    fn reassign_can_assign_unassigned_land_to_state() {
        let mut edit = session();
        edit.reassign_provinces(&[20], Some(2)).unwrap();
        assert_eq!(edit.working.state_by_province.get(&20), Some(&2));
        assert!(!edit.working.unassigned_land_provinces.contains(&20));
    }

    #[test]
    fn reassign_can_unassign_land_from_state() {
        let mut edit = session();
        edit.reassign_provinces(&[10], None).unwrap();
        assert_eq!(edit.working.state_by_province.get(&10), None);
        assert!(edit.working.unassigned_land_provinces.contains(&10));
    }

    #[test]
    fn reassign_noop_does_not_create_history() {
        let mut edit = session();
        edit.reassign_provinces(&[10], Some(1)).unwrap();
        assert!(edit.undo_stack.is_empty());
    }

    #[test]
    fn reassign_failure_is_atomic() {
        let mut edit = session();
        let before = edit.working.clone();
        let err = edit.reassign_provinces(&[10, 99999], Some(2)).unwrap_err();
        assert_eq!(err, StateEditError::ProvinceNotFound(99999));
        assert_eq!(edit.working, before);
        assert!(edit.undo_stack.is_empty());
    }

    #[test]
    fn reassign_rejects_invalid_target() {
        let mut edit = session();
        assert_eq!(
            edit.reassign_provinces(&[10], Some(999)).unwrap_err(),
            StateEditError::TargetStateNotFound(999)
        );
        assert_eq!(edit.working.state_by_province.get(&10), Some(&1));
    }

    #[test]
    fn province_selection_rejects_sea_lake_and_ambiguous() {
        let mut edit = session();
        edit.ambiguous_provinces.insert(12);
        assert_eq!(
            edit.toggle_selected_province(40).unwrap_err(),
            StateEditError::ProvinceIsNotLand(40)
        );
        assert_eq!(
            edit.toggle_selected_province(41).unwrap_err(),
            StateEditError::ProvinceIsNotLand(41)
        );
        assert_eq!(
            edit.toggle_selected_province(12).unwrap_err(),
            StateEditError::AmbiguousProvince(12)
        );
    }

    #[test]
    fn province_selection_toggles_and_target_stays_separate_without_dirtying() {
        let mut edit = session();
        assert!(edit.toggle_selected_province(10).unwrap());
        edit.set_target_state(Some(2)).unwrap();
        assert_eq!(edit.selected_provinces, BTreeSet::from([10]));
        assert_eq!(edit.target_state_id, Some(2));
        assert!(!edit.is_dirty());
        assert!(edit.undo_stack.is_empty());
        assert!(!edit.toggle_selected_province(10).unwrap());
    }

    #[test]
    fn province_selection_can_select_all_land_in_target_state() {
        let mut edit = session();
        edit.set_target_state(Some(1)).unwrap();
        assert_eq!(edit.select_target_state_provinces().unwrap(), 3);
        assert_eq!(edit.selected_provinces, BTreeSet::from([10, 11, 12]));
        assert_eq!(edit.target_state_id, Some(1));
        assert!(!edit.is_dirty());
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT"]
    fn real_mod_state_edit_smoke_move_undo_redo_discard_unassign() {
        let root = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
        let paths = ProjectPaths::discover(&root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle =
            Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);
        let invalid_documents = project
            .states
            .iter()
            .filter(|document| document.data.is_none())
            .map(|document| document.path.display().to_string())
            .collect::<Vec<_>>();
        let error_bearing_documents = project
            .states
            .iter()
            .filter(|document| {
                document
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            })
            .map(|document| document.path.display().to_string())
            .collect::<Vec<_>>();
        let warning_kinds = project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
            .fold(
                BTreeMap::<String, usize>::new(),
                |mut counts, diagnostic| {
                    *counts.entry(format!("{:?}", diagnostic.kind)).or_default() += 1;
                    counts
                },
            );
        println!("{}", project.load_summary_message());
        println!("Invalid state documents: {invalid_documents:#?}");
        println!("Documents with errors: {error_bearing_documents:#?}");
        println!(
            "Unassigned land province IDs: {:?}",
            project.unassigned_land_provinces
        );
        println!("Warning kinds: {warning_kinds:#?}");
        for diagnostic in project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            println!(
                "ERROR [{:?}] [{}] {}",
                diagnostic.kind,
                diagnostic
                    .path
                    .as_ref()
                    .map_or_else(|| "<project>".into(), |path| path.display().to_string()),
                diagnostic.message
            );
        }
        let original_assignments = project.state_by_province.clone();
        let mut edit = StateEditSession::new(&project, &bundle.map);
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        let valverde_target = edit
            .valid_state_ids
            .iter()
            .copied()
            .find(|state_id| *state_id != 1)
            .expect("expected another valid state");
        let valverde_after = EditableProvinceData {
            victory_point: Some(10),
            buildings: BTreeMap::from([("custom_test_building".into(), 2)]),
        };
        assert!(
            edit.update_province_data(5144, 1, valverde_after.clone())
                .unwrap()
        );
        assert_eq!(edit.summary().commands, 1);
        assert_eq!(edit.province_data(5144), Some(valverde_after.clone()));
        assert!(edit.undo());
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(edit.redo());
        assert_eq!(edit.province_data(5144), Some(valverde_after.clone()));
        edit.reassign_provinces(&[5144], Some(valverde_target))
            .unwrap();
        assert_eq!(edit.province_data(5144), Some(valverde_after.clone()));
        assert!(edit.undo());
        assert!(edit.undo());
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(edit.redo());
        assert!(edit.redo());
        edit.reassign_provinces(&[5144], None).unwrap();
        assert_eq!(edit.province_data(5144), Some(valverde_after));
        assert!(edit.undo());
        edit.discard();
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(!edit.is_dirty());

        let suggested_state_id = edit.suggest_next_state_id();
        edit.toggle_selected_province(5144).unwrap();
        edit.create_state(
            suggested_state_id,
            EditableStateProperties {
                name: Some("Phase 3C Smoke".into()),
                buildings_max_level_factor: Some(1.0),
                local_supplies: Some(0.0),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        assert_eq!(edit.province_state_id(5144), Some(suggested_state_id));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert!(edit.redo());
        edit.remove_state(suggested_state_id, StateRemovalPolicy::MoveToState(1))
            .unwrap();
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(!edit.is_dirty());
        edit.discard();
        edit.remove_state(1, StateRemovalPolicy::Unassign).unwrap();
        assert_eq!(edit.province_state_id(5144), None);
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(5144), Some(1));
        edit.discard();
        assert!(!edit.is_dirty());

        let brush_state_id = edit.suggest_next_state_id();
        edit.create_state(
            brush_state_id,
            EditableStateProperties {
                name: Some("Phase 3D Brush Smoke".into()),
                buildings_max_level_factor: Some(1.0),
                local_supplies: Some(0.0),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(
            edit.validate_brush_target(StateBrushMode::AssignToTarget, Some(brush_state_id)),
            Ok(Some(brush_state_id))
        );
        assert_eq!(
            edit.classify_brush_province(
                5144,
                StateBrushMode::AssignToTarget,
                Some(brush_state_id)
            ),
            BrushProvinceClassification::Selectable
        );
        edit.reassign_provinces(&[5144], Some(brush_state_id))
            .unwrap();
        assert_eq!(edit.province_state_id(5144), Some(brush_state_id));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert_eq!(edit.summary().commands, 2);
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert!(edit.undo());
        assert!(!edit.is_state_active(brush_state_id));
        assert!(edit.redo());
        assert!(edit.redo());
        assert_eq!(edit.province_state_id(5144), Some(brush_state_id));
        assert_eq!(
            edit.classify_brush_province(
                5144,
                StateBrushMode::AssignToTarget,
                Some(brush_state_id)
            ),
            BrushProvinceClassification::NoOp
        );

        let dimensions = bundle.map.dimensions();
        let mut drag = (
            Vec::new(),
            Vec::new(),
            BTreeSet::new(),
            [0_usize; 4],
            0_usize,
            Duration::default(),
        );
        for row in 0..=16 {
            let y = dimensions[1].saturating_sub(1) * row / 16;
            let collection_started = Instant::now();
            let samples = crate::app::project::sample_segment(
                [0.0, y as f64],
                [dimensions[0].saturating_sub(1) as f64, y as f64],
                1.0,
                dimensions,
            );
            assert!(samples.windows(2).all(|pair| {
                pair[0][0].abs_diff(pair[1][0]) <= 1 && pair[0][1].abs_diff(pair[1][1]) <= 1
            }));
            let mut selectable = BTreeSet::new();
            let mut visited = BTreeSet::new();
            let mut counts = [0_usize; 4];
            for &position in &samples {
                let Some(province_id) = bundle.map.get_province_at(position).preserved_id else {
                    counts[3] += 1;
                    continue;
                };
                if !visited.insert(province_id) {
                    continue;
                }
                match edit.classify_brush_province(
                    province_id,
                    StateBrushMode::AssignToTarget,
                    Some(brush_state_id),
                ) {
                    BrushProvinceClassification::Selectable => {
                        selectable.insert(province_id);
                    }
                    BrushProvinceClassification::NoOp => counts[0] += 1,
                    BrushProvinceClassification::IgnoredNonLand => counts[1] += 1,
                    BrushProvinceClassification::BlockedAmbiguous
                    | BrushProvinceClassification::BlockedInvalidState => counts[2] += 1,
                    BrushProvinceClassification::Unknown => counts[3] += 1,
                }
            }
            if selectable.len() > drag.2.len() {
                let collection_elapsed = collection_started.elapsed();
                let visited_count = visited.len();
                drag = (
                    samples,
                    selectable.iter().copied().take(64).collect(),
                    selectable,
                    counts,
                    visited_count,
                    collection_elapsed,
                );
            }
        }
        assert!(
            drag.1.len() >= 2,
            "expected a sampled path through multiple editable provinces"
        );
        let drag_commands_before = edit.summary().commands;
        let drag_started = Instant::now();
        edit.reassign_provinces(&drag.1, Some(brush_state_id))
            .unwrap();
        let drag_elapsed = drag_started.elapsed();
        let drag_timings = edit.last_timings();
        assert_eq!(edit.summary().commands, drag_commands_before + 1);
        assert!(
            drag.1
                .iter()
                .all(|province_id| edit.province_state_id(*province_id) == Some(brush_state_id))
        );
        assert!(edit.undo());
        assert!(edit.redo());

        let mut unassign_ids = drag.1.clone();
        unassign_ids.push(5144);
        let unassign_commands_before = edit.summary().commands;
        edit.reassign_provinces(&unassign_ids, None).unwrap();
        assert_eq!(edit.summary().commands, unassign_commands_before + 1);
        assert!(
            unassign_ids
                .iter()
                .all(|province_id| edit.province_state_id(*province_id).is_none())
        );
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(5144), Some(brush_state_id));
        edit.discard();
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert!(!edit.is_dirty());

        let ambiguous_provinces = project
            .ambiguous_provinces
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let measure_visual = |edit: &StateEditSession| {
            let view = generate_state_view_for(
                &bundle.map,
                edit.state_by_province(),
                &ambiguous_provinces,
                edit.unassigned_land_provinces(),
            );
            (view.generated_in, view.boundary_scan_in)
        };

        let candidate = bundle
            .map
            .iter_boundaries()
            .find_map(|(boundary, _)| {
                let [source_pos, target_pos] = boundary.into_array();
                let source = bundle.map.get_province_at(source_pos);
                let target = bundle.map.get_province_at(target_pos);
                let (source_id, target_id) = (source.preserved_id?, target.preserved_id?);
                let (source_state, target_state) = (
                    edit.state_by_province().get(&source_id).copied()?,
                    edit.state_by_province().get(&target_id).copied()?,
                );
                if source.kind != ProvinceKind::Land
                    || target.kind != ProvinceKind::Land
                    || source_state == target_state
                    || project.ambiguous_provinces.contains_key(&source_id)
                    || project.ambiguous_provinces.contains_key(&target_id)
                {
                    return None;
                }
                let mut probe = edit.clone();
                probe
                    .reassign_provinces(&[source_id], Some(target_state))
                    .ok()?;
                Some((
                    source_id,
                    source_state,
                    target_id,
                    target_state,
                    source_pos,
                    target_pos,
                ))
            })
            .expect("expected adjacent editable land provinces in different states");

        let (source_id, source_state, target_id, target_state, source_pos, target_pos) = candidate;
        edit.set_target_state(Some(target_state)).unwrap();
        edit.toggle_selected_province(source_id).unwrap();
        edit.move_selection_to_target().unwrap();
        assert_eq!(
            edit.state_by_province().get(&source_id),
            Some(&target_state)
        );
        let move_visual = measure_visual(&edit);
        assert!(edit.undo());
        assert_eq!(
            edit.state_by_province().get(&source_id),
            Some(&source_state)
        );
        let undo_visual = measure_visual(&edit);
        assert!(edit.redo());
        assert_eq!(
            edit.state_by_province().get(&source_id),
            Some(&target_state)
        );
        let redo_visual = measure_visual(&edit);
        edit.discard();
        assert_eq!(
            edit.state_by_province().get(&source_id),
            Some(&source_state)
        );
        assert!(!edit.is_dirty());
        let discard_visual = measure_visual(&edit);

        edit.toggle_selected_province(source_id).unwrap();
        edit.unassign_selection().unwrap();
        assert!(!edit.state_by_province().contains_key(&source_id));
        assert!(edit.unassigned_land_provinces().contains(&source_id));
        let unassign_visual = measure_visual(&edit);
        assert!(edit.undo());
        assert_eq!(
            edit.state_by_province().get(&source_id),
            Some(&source_state)
        );
        let unassign_undo_visual = measure_visual(&edit);
        assert_eq!(project.state_by_province, original_assignments);
        edit.validate_invariants().unwrap();

        println!(
            "Real-mod Province 5144 smoke: State 1, VP 5 -> 10, \
       custom_test_building=2, move target State {valverde_target}, unassign/undo/discard passed.\n\
       Real-mod Phase 3C smoke: suggested/created State {suggested_state_id}, Province 5144 \
       followed create/remove, loaded State 1 unassign/undo returned to baseline.\n\
       Real-mod Phase 3D brush smoke: empty State {brush_state_id} accepted Province 5144, \
       no-op classification, drag with {} input events / {} samples / {} unique editable \
       provinces ({} applied) / {} visited / {} no-op / {} ignored / {} blocked / {} unknown, \
       one command, Unassign, Undo/Redo and Discard returned to baseline; \
       collection {:?}, transaction {:?}, preflight {:?}, apply {:?}.\n\
       Real-mod smoke candidate: province {source_id} at {source_pos:?} \
       (state {source_state}) -> adjacent province {target_id} at {target_pos:?} \
       (state {target_state}); timings: {:?}; visual refreshes: \
       move={move_visual:?}, undo={undo_visual:?}, redo={redo_visual:?}, \
       discard={discard_visual:?}, unassign={unassign_visual:?}, \
       unassign-undo={unassign_undo_visual:?}",
            2,
            drag.0.len(),
            drag.2.len(),
            drag.1.len(),
            drag.4,
            drag.3[0],
            drag.3[1],
            drag.3[2],
            drag.3[3],
            drag.5,
            drag_elapsed,
            drag_timings.command_preflight,
            drag_timings.command_apply,
            edit.last_timings(),
        );
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT"]
    fn real_mod_phase4a_patch_preview_smoke() {
        let root = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
        let paths = ProjectPaths::discover(&root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle =
            Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);
        let mut edit = StateEditSession::new(&project, &bundle.map);

        let original_valverde = edit.state_data(1).expect("State 1");
        assert_eq!(original_valverde.manpower, Some(142_000));
        assert_eq!(original_valverde.state_category.as_deref(), Some("rural"));
        assert_eq!(original_valverde.resources.get("oil"), Some(&8));
        assert_eq!(edit.province_state_id(5144), Some(1));
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));

        let mut valverde = EditableStateProperties::from_state(&original_valverde);
        valverde.manpower = Some(150_000);
        valverde.state_category = Some("town".to_owned());
        valverde.resources.insert("oil".to_owned(), 10);
        edit.update_state_properties(1, valverde).unwrap();
        let commands = edit.summary().commands;
        let dirty = edit.dirty_state_ids().clone();
        let scalar_plan = plan_state_patches(&project, &edit);
        assert_eq!(edit.summary().commands, commands);
        assert_eq!(edit.dirty_state_ids(), &dirty);
        assert_eq!(scalar_plan.modified_files.len(), 1);
        assert!(scalar_plan.modified_files[0].after.is_some());
        assert!(
            scalar_plan.modified_files[0]
                .semantic_changes
                .iter()
                .any(|change| { change.contains("manpower") && change.contains("150000") })
        );
        println!("Phase 4A Scenario A:\n{}", scalar_plan.summary_text());
        println!("{}", scalar_plan.file_report(0).unwrap());
        edit.discard();

        let move_target = edit
            .valid_state_ids()
            .iter()
            .copied()
            .find(|state_id| *state_id != 1)
            .expect("another valid state");
        edit.reassign_provinces(&[5144], Some(move_target)).unwrap();
        let move_plan = plan_state_patches(&project, &edit);
        let move_report = (0..move_plan.files_len())
            .filter_map(|index| move_plan.file_report(index))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(move_report.contains("remove Province 5144"));
        assert!(move_report.contains("add Province 5144"));
        assert!(move_report.contains("remove VP 5144 = 5"));
        assert!(move_report.contains("add VP 5144 = 5"));
        println!(
            "Phase 4A Scenario B (target State {move_target}):\n{}",
            move_plan.summary_text()
        );
        println!("{move_report}");
        edit.discard();

        let new_state_id = edit.suggest_next_state_id();
        let new_state_name = format!("STATE_{new_state_id}");
        edit.toggle_selected_province(5144).unwrap();
        edit.create_state(
            new_state_id,
            EditableStateProperties {
                name: Some(new_state_name.clone()),
                manpower: Some(1_000),
                state_category: Some("rural".to_owned()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let create_plan = plan_state_patches(&project, &edit);
        let created = create_plan
            .created_files
            .iter()
            .find(|file| file.state_id == new_state_id)
            .expect("planned new state file");
        assert_eq!(
            created.path,
            PathBuf::from(format!(
                "history/states/{new_state_id}-State_{new_state_id}.txt"
            ))
        );
        assert!(!project.paths.root.join(&created.path).exists());
        assert_ne!(created.safety, crate::app::project::PatchSafety::Blocked);
        println!("Phase 4A Scenario C:\n{}", create_plan.summary_text());
        println!(
            "{}",
            create_plan
                .file_report(create_plan.modified_files.len())
                .unwrap()
        );
        let stale_plan = create_plan.clone();
        edit.discard();
        assert!(stale_plan.is_stale(edit.revision()));
        let empty_after_discard = plan_state_patches(&project, &edit);
        assert_eq!(empty_after_discard.files_len(), 0);

        let removal_id = edit
            .valid_state_ids()
            .iter()
            .copied()
            .filter(|state_id| *state_id != 1)
            .find(|state_id| {
                let mut probe = edit.clone();
                probe
                    .remove_state(*state_id, StateRemovalPolicy::MoveToState(1))
                    .is_ok()
            })
            .expect("removable loaded state");
        edit.remove_state(removal_id, StateRemovalPolicy::MoveToState(1))
            .unwrap();
        let removal_plan = plan_state_patches(&project, &edit);
        assert!(
            removal_plan
                .removed_files
                .iter()
                .any(|file| file.state_id == removal_id)
        );
        println!(
            "Phase 4A Scenario D (removed State {removal_id}):\n{}",
            removal_plan.summary_text()
        );
        assert!(edit.undo());
        let restored_plan = plan_state_patches(&project, &edit);
        assert!(restored_plan.removed_files.is_empty());
        edit.discard();

        let mut net_zero = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
        net_zero.manpower = Some(150_000);
        edit.update_state_properties(1, net_zero).unwrap();
        assert!(edit.undo());
        let net_zero_plan = plan_state_patches(&project, &edit);
        assert_eq!(net_zero_plan.files_len(), 0);
        assert!(net_zero_plan.diagnostics.is_empty());
        println!(
            "Phase 4A Scenarios E/H: net-zero and stale/discard passed.\n{}",
            net_zero_plan.summary_text()
        );
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT"]
    fn real_mod_phase4b_round_trip_smoke() {
        let root = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
        let paths = ProjectPaths::discover(&root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle =
            Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);
        let validator = RoundTripValidator::default();
        let validate = |label: &str, edit: &StateEditSession| {
            let plan = plan_state_patches(&project, edit);
            let report = validator.validate(
                &project,
                edit,
                &plan,
                &RoundTripCancellation::default(),
                |_| {},
            );
            println!("Phase 4B {label}:\n{}", report.full_text());
            assert_eq!(
                report.status,
                RoundTripStatus::Passed,
                "{label}: {}",
                report.full_text()
            );
            assert!(report.workspace.cleaned || report.no_candidate_changes);
            report
        };

        let mut edit = StateEditSession::new(&project, &bundle.map);
        let mut valverde = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
        valverde.manpower = Some(150_000);
        valverde.state_category = Some("town".to_owned());
        valverde.resources.insert("oil".to_owned(), 10);
        edit.update_state_properties(1, valverde).unwrap();
        let scalar = validate("Scenario A - Valverde properties", &edit);
        assert_eq!(scalar.application.modified_files_applied, 1);
        edit.discard();

        let move_target = edit
            .valid_state_ids()
            .iter()
            .copied()
            .find(|state_id| *state_id != 1)
            .unwrap();
        edit.reassign_provinces(&[5144], Some(move_target)).unwrap();
        let moved = validate("Scenario B - Province 5144", &edit);
        assert_eq!(moved.application.modified_files_applied, 2);
        assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
        edit.discard();

        let new_state_id = edit.suggest_next_state_id();
        let new_state_name = format!("STATE_{new_state_id}");
        edit.toggle_selected_province(5144).unwrap();
        edit.create_state(
            new_state_id,
            EditableStateProperties {
                name: Some(new_state_name.clone()),
                manpower: Some(1_000),
                state_category: Some("rural".to_owned()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let created = validate(&format!("Scenario C - State {new_state_id}"), &edit);
        assert_eq!(created.application.created_files_applied, 1);
        assert!(
            !root
                .join(format!(
                    "history/states/{new_state_id}-{new_state_name}.txt"
                ))
                .exists()
        );
        edit.discard();

        let removal_id = edit
            .valid_state_ids()
            .iter()
            .copied()
            .filter(|state_id| *state_id != 1)
            .find(|state_id| {
                let mut probe = edit.clone();
                probe
                    .remove_state(*state_id, StateRemovalPolicy::MoveToState(1))
                    .is_ok()
            })
            .expect("removable loaded state");
        let removed_source = project.state_document(removal_id).unwrap().path.clone();
        edit.remove_state(removal_id, StateRemovalPolicy::MoveToState(1))
            .unwrap();
        let removed = validate("Scenario D - loaded state removal", &edit);
        assert_eq!(removed.application.removed_files_applied, 1);
        assert!(removed_source.exists());
        edit.discard();

        let mut combined_properties =
            EditableStateProperties::from_state(&edit.state_data(1).unwrap());
        combined_properties.manpower = Some(150_000);
        combined_properties.state_category = Some("town".to_owned());
        combined_properties.resources.insert("oil".to_owned(), 10);
        edit.update_state_properties(1, combined_properties)
            .unwrap();
        let provincial_id = edit
            .state_data(1)
            .unwrap()
            .provinces
            .iter()
            .copied()
            .find(|province_id| *province_id != 5144)
            .expect("another State 1 province");
        edit.update_province_data(
            provincial_id,
            1,
            EditableProvinceData {
                victory_point: Some(1),
                buildings: BTreeMap::from([("bunker".to_owned(), 1)]),
            },
        )
        .unwrap();
        let combined_state_id = edit.suggest_next_state_id();
        edit.toggle_selected_province(5144).unwrap();
        edit.create_state(
            combined_state_id,
            EditableStateProperties {
                name: Some(format!("STATE_{combined_state_id}")),
                state_category: Some("rural".to_owned()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let combined_removal_id = edit
            .valid_state_ids()
            .iter()
            .copied()
            .filter(|state_id| *state_id != 1 && *state_id != combined_state_id)
            .find(|state_id| {
                let mut probe = edit.clone();
                probe
                    .remove_state(*state_id, StateRemovalPolicy::MoveToState(1))
                    .is_ok()
            })
            .expect("combined removable loaded state");
        edit.remove_state(combined_removal_id, StateRemovalPolicy::MoveToState(1))
            .unwrap();
        let combined = validate("Scenario E - combined global candidate", &edit);
        assert!(combined.application.modified_files_applied >= 1);
        assert_eq!(combined.application.created_files_applied, 1);
        assert_eq!(combined.application.removed_files_applied, 1);
        edit.discard();

        let empty = validate("Scenario F/K - discard and net-zero", &edit);
        assert!(empty.no_candidate_changes);
        assert_eq!(edit.summary().commands, 0);
        assert_eq!(edit.summary().modified_states, 0);
    }

    #[test]
    fn state_edit_moves_victory_points_and_undo_restores_them() {
        let mut edit = session();
        edit.reassign_provinces(&[10], Some(2)).unwrap();
        assert!(edit.working.victory_points_by_state[&1].is_empty());
        assert_eq!(
            edit.working.victory_points_by_state[&2],
            vec![VictoryPoint {
                province_id: 10,
                value: 5
            }]
        );
        assert!(edit.undo());
        assert_eq!(
            edit.working.victory_points_by_state[&1],
            vec![VictoryPoint {
                province_id: 10,
                value: 5
            }]
        );
    }

    #[test]
    fn state_edit_moves_province_buildings_and_undo_restores_them() {
        let mut edit = session();
        edit.reassign_provinces(&[11], Some(2)).unwrap();
        assert!(!edit.working.province_buildings_by_state[&1].contains_key(&11));
        assert_eq!(
            edit.working.province_buildings_by_state[&2][&11]["bunker"],
            2
        );
        assert!(edit.undo());
        assert_eq!(
            edit.working.province_buildings_by_state[&1][&11]["bunker"],
            2
        );
    }

    #[test]
    fn state_edit_conflicting_provincial_data_blocks_whole_command() {
        let mut edit = session();
        edit.working
            .province_buildings_by_state
            .entry(2)
            .or_default()
            .insert(11, BTreeMap::from([("dockyard".into(), 1)]));
        let before = edit.working.clone();
        assert_eq!(
            edit.reassign_provinces(&[10, 11], Some(2)).unwrap_err(),
            StateEditError::ProvincialDataConflict {
                province_id: 11,
                target_state_id: 2
            }
        );
        assert_eq!(edit.working, before);
        assert!(edit.undo_stack.is_empty());
    }

    #[test]
    fn state_brush_classifies_samples_and_reuses_atomic_reassign() {
        let samples = crate::app::project::sample_segment([0.0, 0.0], [3.0, 0.0], 1.0, [10, 10]);
        assert_eq!(samples, vec![[0, 0], [1, 0], [2, 0], [3, 0]]);

        let mut edit = session();
        edit.ambiguous_provinces.insert(12);
        edit.working.state_by_province.insert(20, 99);
        let preview_working = edit.working.clone();
        let preview_commands = edit.summary().commands;
        assert_eq!(
            edit.classify_brush_province(10, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::Selectable
        );
        assert_eq!(
            edit.classify_brush_province(30, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::NoOp
        );
        assert_eq!(
            edit.classify_brush_province(40, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::IgnoredNonLand
        );
        assert_eq!(
            edit.classify_brush_province(0, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::IgnoredNonLand
        );
        assert_eq!(
            edit.classify_brush_province(12, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::BlockedAmbiguous
        );
        assert_eq!(
            edit.classify_brush_province(20, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::BlockedInvalidState
        );
        assert_eq!(
            edit.classify_brush_province(999, StateBrushMode::AssignToTarget, Some(2)),
            BrushProvinceClassification::Unknown
        );
        assert_eq!(
            edit.classify_brush_province(10, StateBrushMode::Unassign, None),
            BrushProvinceClassification::Selectable
        );
        assert!(
            edit.validate_brush_target(StateBrushMode::AssignToTarget, None)
                .is_err()
        );
        assert_eq!(
            edit.validate_brush_target(StateBrushMode::Unassign, None),
            Ok(None)
        );
        assert_eq!(edit.working, preview_working);
        assert_eq!(edit.summary().commands, preview_commands);

        edit.working.state_by_province.remove(&20);
        edit.ambiguous_provinces.remove(&12);
        edit.create_state(3, EditableStateProperties::default(), false)
            .unwrap();
        assert_eq!(
            edit.validate_brush_target(StateBrushMode::AssignToTarget, Some(3)),
            Ok(Some(3))
        );
        edit.working
            .province_buildings_by_state
            .entry(3)
            .or_default()
            .insert(11, BTreeMap::from([("dockyard".into(), 1)]));
        let before = edit.working.clone();
        assert_eq!(
            edit.reassign_provinces(&[10, 11], Some(3)).unwrap_err(),
            StateEditError::ProvincialDataConflict {
                province_id: 11,
                target_state_id: 3
            }
        );
        assert_eq!(edit.working, before);

        edit.working
            .province_buildings_by_state
            .get_mut(&3)
            .unwrap()
            .remove(&11);
        edit.reassign_provinces(&[10, 11, 30], Some(3)).unwrap();
        assert_eq!(edit.undo_stack.len(), 2);
        assert_eq!(edit.province_state_id(10), Some(3));
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert_eq!(edit.province_data(11).unwrap().buildings["bunker"], 2);
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(10), Some(1));
        assert!(edit.redo());
        assert_eq!(edit.province_state_id(10), Some(3));
    }

    #[test]
    fn undo_redo_discard_and_edit_invariants_roundtrip() {
        let mut edit = session();
        assert_eq!(edit.revision(), 0);
        edit.reassign_provinces(&[10, 11], Some(2)).unwrap();
        assert!(edit.undo());
        edit.validate_invariants().unwrap();
        assert_eq!(edit.working, edit.baseline);
        assert!(edit.redo());
        edit.validate_invariants().unwrap();
        assert_ne!(edit.working, edit.baseline);
        assert_eq!(edit.undo_stack.len(), 1);
        assert!(edit.undo());
        edit.reassign_provinces(&[12], Some(2)).unwrap();
        assert!(edit.redo_stack.is_empty());
        edit.discard();
        assert_eq!(edit.working, edit.baseline);
        assert!(!edit.is_dirty());
        assert!(edit.undo_stack.is_empty());
        assert!(edit.redo_stack.is_empty());
        assert!(edit.dirty_state_ids.is_empty());
        assert!(edit.session_diagnostics.is_empty());
        assert_eq!(edit.revision(), 6);
    }

    #[test]
    fn province_data_update_is_atomic_and_follows_reassignment() {
        let mut edit = session();
        let after = EditableProvinceData {
            victory_point: Some(10),
            buildings: BTreeMap::from([("custom_test_building".into(), 2)]),
        };
        assert!(edit.update_province_data(10, 1, after.clone()).unwrap());
        assert_eq!(edit.undo_stack.len(), 1);
        assert!(edit.take_last_changed_provinces().is_empty());
        assert_eq!(edit.province_data(10), Some(after.clone()));

        edit.reassign_provinces(&[10], Some(2)).unwrap();
        assert_eq!(edit.province_state_id(10), Some(2));
        assert_eq!(edit.province_data(10), Some(after.clone()));
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(10), Some(1));
        assert_eq!(edit.province_data(10), Some(after.clone()));
        assert!(edit.undo());
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert!(edit.redo());
        assert!(edit.redo());
        assert_eq!(edit.province_data(10), Some(after.clone()));

        edit.reassign_provinces(&[10], None).unwrap();
        assert_eq!(edit.province_state_id(10), None);
        assert_eq!(edit.province_data(10), Some(after));
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(10), Some(2));

        edit.discard();
        assert_eq!(edit.province_state_id(10), Some(1));
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert!(!edit.is_dirty());
        assert!(edit.undo_stack.is_empty());
    }

    #[test]
    fn edit_history_new_command_after_undo_clears_redo() {
        let mut edit = session();
        edit.reassign_provinces(&[10], Some(2)).unwrap();
        assert!(edit.undo());
        assert!(edit.can_redo());
        edit.reassign_provinces(&[12], Some(2)).unwrap();
        assert!(!edit.can_redo());
        edit.validate_invariants().unwrap();
    }

    #[test]
    fn state_lifecycle_create_remove_history_and_data_roundtrip() {
        let mut edit = session();
        assert_eq!(
            state_id_from_filename(std::path::Path::new("111-State_111.txt")),
            Some(111)
        );
        edit.known_state_ids.insert(111);
        assert_eq!(edit.suggest_next_state_id(), 112);
        edit.toggle_selected_province(10).unwrap();
        let properties = EditableStateProperties {
            name: Some("Temporary State".into()),
            buildings_max_level_factor: Some(1.0),
            local_supplies: Some(0.0),
            ..Default::default()
        };
        let before_failed_create = edit.working.clone();
        assert_eq!(
            edit.create_state_with_provinces(113, properties.clone(), &[10, 99999]),
            Err(StateEditError::ProvinceNotFound(99999))
        );
        assert_eq!(edit.working, before_failed_create);
        assert!(!edit.is_state_id_reserved(113));
        assert_eq!(edit.selected_provinces(), &BTreeSet::from([10]));

        edit.create_state(112, properties.clone(), true).unwrap();
        assert!(edit.is_state_active(112));
        assert!(edit.selected_provinces().is_empty());
        assert_eq!(edit.target_state_id(), Some(112));
        assert_eq!(
            edit.state_origin(112),
            Some(&WorkingStateOrigin::CreatedInSession)
        );
        assert_eq!(edit.province_state_id(10), Some(112));
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert_eq!(edit.summary().commands, 1);
        assert!(edit.undo());
        assert!(!edit.is_state_active(112));
        assert!(edit.selected_provinces().is_empty());
        assert_eq!(edit.target_state_id(), None);
        assert_eq!(
            edit.validate_new_state_id(112),
            Err(StateEditError::StateIdReserved(112))
        );
        assert!(edit.redo());
        assert_eq!(edit.target_state_id(), Some(112));
        assert_eq!(
            edit.state_data(112).unwrap().name.as_deref(),
            Some("Temporary State")
        );

        edit.remove_state(112, StateRemovalPolicy::MoveToState(1))
            .unwrap();
        assert_eq!(
            edit.state_lifecycle(112),
            Some(WorkingStateLifecycle::RemovedInSession)
        );
        assert_eq!(edit.province_state_id(10), Some(1));
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert!(!edit.is_dirty());
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(10), Some(112));
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert!(edit.redo());
        assert!(!edit.is_dirty());

        edit.discard();
        let baseline = edit.working.clone();
        edit.remove_state(1, StateRemovalPolicy::Unassign).unwrap();
        assert_eq!(
            edit.state_lifecycle(1),
            Some(WorkingStateLifecycle::RemovedInSession)
        );
        assert_eq!(edit.province_state_id(10), None);
        assert_eq!(edit.province_data(10).unwrap().victory_point, Some(5));
        assert!(edit.undo());
        assert_eq!(edit.working, baseline);
        assert!(edit.redo());
        edit.discard();
        assert_eq!(edit.working, baseline);
        assert!(!edit.is_dirty());
        edit.validate_invariants().unwrap();

        let mut conflict = session();
        conflict
            .working
            .province_buildings_by_state
            .entry(2)
            .or_default()
            .insert(11, BTreeMap::from([("dockyard".into(), 1)]));
        let before_failed_remove = conflict.working.clone();
        assert_eq!(
            conflict.remove_state(1, StateRemovalPolicy::MoveToState(2)),
            Err(StateEditError::ProvincialDataConflict {
                province_id: 11,
                target_state_id: 2,
            })
        );
        assert_eq!(conflict.working, before_failed_remove);
        assert!(conflict.is_state_active(1));
        assert!(!conflict.can_undo());

        let mut reservation = session();
        reservation
            .create_state(3, properties.clone(), false)
            .unwrap();
        assert!(reservation.undo());
        assert_eq!(
            reservation.validate_new_state_id(3),
            Err(StateEditError::StateIdReserved(3))
        );
        reservation.create_state(4, properties, false).unwrap();
        assert!(reservation.validate_new_state_id(3).is_ok());
    }

    #[test]
    fn invalid_duplicate_like_target_is_rejected_as_invalid() {
        let mut edit = session();
        edit.valid_state_ids.remove(&2);
        assert_eq!(
            edit.reassign_provinces(&[10], Some(2)).unwrap_err(),
            StateEditError::TargetStateInvalid(2)
        );
        assert_eq!(edit.state_by_province().get(&10), Some(&1));
    }
}

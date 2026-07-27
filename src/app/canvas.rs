use ahash::AHashSet;
use graphics::Transformed;
use graphics::types::Color as DrawColor;
use graphics::context::Context;
use graphics::ellipse::Ellipse;
use image::RgbImage;
use itertools::Itertools;
use opengl_graphics::{Filter, GlGraphics, Texture, TextureSettings};
use uord::UOrd2 as UOrd;
use vecmath::{Matrix2x3, Vector2};

use super::{colors, FontGlyphCache};
use super::alerts::Alerts;
use super::map::*;
use super::interface::{Interface, StateActionAvailability};
use super::format::DefinitionKind;
use super::project::{
  DiagnosticSeverity, Hoi4Project, LassoSelectionMode, MapViewMode,
  ProvinceInclusionMode, StateEditSession, StateLassoPhase, StateSelection,
  EditableStateProperties, ProvinceDataDraft, StatePropertyDraft, StateRemovalPolicy,
  BrushProvinceClassification, StateBrushMode, WorkingStateOrigin, sample_segment,
  boundaries_for_state, classify_state_lasso, generate_state_view,
  generate_state_view_for, generate_state_view_region_for,
  select_state_at_for as resolve_state_at_for, selection_overlay_for
};
use crate::config::Config;
use crate::font::{self, FONT_SIZE};
use crate::util::stringify_color;
use crate::util::files::Location;
use crate::error::Error;

use std::path::Path;
use std::io::BufWriter;
use std::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const ZOOM_SENSITIVITY: f64 = 0.125;
const STATE_PROPERTY_LABELS: [&str; StatePropertyDraft::TEXT_FIELD_COUNT] = [
  "Name",
  "Manpower",
  "State category",
  "Max level factor",
  "Local supplies",
  "Owner",
  "Controller",
  "Cores (comma-separated)",
  "Claims (comma-separated)",
  "Resources (name=value)",
  "State buildings (name=value)",
];
const STATE_PROPERTY_FIELD_KEYS: [&str; StatePropertyDraft::TEXT_FIELD_COUNT] = [
  "Name",
  "Manpower",
  "State category",
  "Buildings max level factor",
  "Local supplies",
  "Owner",
  "Controller",
  "Cores",
  "Claims",
  "Resources",
  "State buildings",
];

pub struct Canvas {
  bundle: Bundle,
  history: History,
  texture: Texture,
  state_texture: Option<Texture>,
  state_boundaries: Vec<UOrd<Vector2<u32>>>,
  province_boundaries: BTreeMap<u32, Vec<UOrd<Vector2<u32>>>>,
  selected_state_boundaries: Vec<UOrd<Vector2<u32>>>,
  selected_province_boundaries: Vec<UOrd<Vector2<u32>>>,
  lasso_preview_boundaries: Vec<UOrd<Vector2<u32>>>,
  lasso_blocked_boundaries: Vec<UOrd<Vector2<u32>>>,
  brush_preview_boundaries: Vec<UOrd<Vector2<u32>>>,
  brush_blocked_boundaries: Vec<UOrd<Vector2<u32>>>,
  selection_texture: Option<Texture>,
  texture_overlay: Option<Texture>,
  view_mode: ViewMode,
  map_view_mode: MapViewMode,
  state_selection: Option<StateSelection>,
  active_state_id: Option<u32>,
  active_province_id: Option<u32>,
  project_status: Option<String>,
  selection_info: Option<String>,
  problems: Vec<Problem>,
  unknown_terrains: Option<AHashSet<String>>,
  location: Location,
  project: Option<Hoi4Project>,
  state_edit_session: Option<StateEditSession>,
  state_lifecycle_draft: Option<StateLifecycleDraft>,
  state_property_draft: Option<StatePropertyDraft>,
  province_data_draft: Option<ProvinceDataDraft>,
  property_editor_field: usize,
  property_editor_replace_field: bool,
  province_editor_page: usize,
  state_lasso_phase: StateLassoPhase,
  state_lasso_mode: LassoSelectionMode,
  state_lasso_inclusion: ProvinceInclusionMode,
  state_brush_phase: StateBrushPhase,
  state_brush_mode: StateBrushMode,
  last_state_brush_result: Option<String>,
  state_province_extents: Option<BTreeMap<u32, Extents>>,
  last_state_visual_update_ms: u128,
  last_state_visual_update_kind: &'static str,
  map_access_mode: MapAccessMode,
  show_province_ids: bool,
  show_province_boundaries: bool,
  show_river_overlay: bool,
  pub tool: ToolSettings,
  pub modified: bool,
  pub camera: Camera
}

#[derive(Debug, Clone)]
enum StateLifecycleDraft {
  Create {
    id: String,
    properties: Box<StatePropertyDraft>,
  },
  Remove {
    state_id: u32,
    target_id: String,
    unassign: bool,
    province_count: usize,
  },
}

#[derive(Debug, Clone, Default)]
enum StateBrushPhase {
  #[default]
  Inactive,
  Ready,
  Stroking(Box<StateBrushStroke>),
}

#[derive(Debug, Clone)]
struct StateBrushStroke {
  mode: StateBrushMode,
  target_state_id: Option<u32>,
  visited_provinces: BTreeSet<u32>,
  selectable_provinces: BTreeSet<u32>,
  no_op_provinces: BTreeSet<u32>,
  blocked_ambiguous: BTreeSet<u32>,
  blocked_invalid_state: BTreeSet<u32>,
  ignored_non_land: BTreeSet<u32>,
  encountered_unknown: bool,
  previous_map_position: Vector2<f64>,
  last_editable_province: Option<u32>,
  input_events: usize,
  sampled_points: usize,
  started: Instant,
}

impl StateLifecycleDraft {
  fn text_field_count(&self) -> usize {
    match self {
      Self::Create { .. } => StatePropertyDraft::TEXT_FIELD_COUNT + 1,
      Self::Remove { .. } => 1,
    }
  }

  fn field(&self, index: usize) -> Option<&str> {
    match self {
      Self::Create { id, properties } => match index {
        0 => Some(id),
        _ => properties.field(index - 1),
      },
      Self::Remove { target_id, .. } => (index == 0).then_some(target_id),
    }
  }

  fn field_mut(&mut self, index: usize) -> Option<&mut String> {
    match self {
      Self::Create { id, properties } => match index {
        0 => Some(id),
        _ => properties.field_mut(index - 1),
      },
      Self::Remove { target_id, .. } => (index == 0).then_some(target_id),
    }
  }
}

impl Canvas {
  pub fn load(location: Location) -> Result<Canvas, Error> {
    Self::load_with_access(
      location,
      None,
      MapAccessMode::EditableProvinceMap,
      Config::load()?
    )
  }

  pub fn load_project(project: Hoi4Project) -> Result<Canvas, Error> {
    let location = Location::Directory(project.paths.map_directory.clone());
    let mut config = Config::load()?;
    config.preserve_ids = true;
    Self::load_with_access(location, Some(project), MapAccessMode::ReadOnly, config)
  }

  fn load_with_access(
    location: Location,
    mut project: Option<Hoi4Project>,
    map_access_mode: MapAccessMode,
    config: Config
  ) -> Result<Canvas, Error> {
    let bundle = Bundle::load(&location, config)?;
    if let Some(project) = project.as_mut() {
      let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
      let land_province_ids = bundle.map.iter_province_data()
        .filter(|(_, province)| province.kind == ProvinceKind::Land)
        .filter_map(|(_, province)| province.preserved_id)
        .collect::<BTreeSet<_>>();
      project.load_states(&province_ids, &land_province_ids);
    }
    let history = History::new(bundle.config.max_undo_states, &bundle.map);
    let texture_settings = TextureSettings::new().mag(Filter::Nearest);
    let texture = Texture::from_image(&bundle.texture_buffer_color(), &texture_settings);
    let state_view = project.as_ref()
      .map(|project| generate_state_view(&bundle.map, project));
    if let (Some(project), Some(state_view)) = (project.as_mut(), state_view.as_ref()) {
      project.load_summary.state_texture_generation_ms = state_view.generated_in.as_millis();
      project.load_summary.state_boundary_generation_ms = state_view.boundary_scan_in.as_millis();
    }
    let state_texture = state_view.as_ref()
      .map(|state_view| Texture::from_image(&state_view.image, &texture_settings));
    let state_boundaries = state_view
      .map(|state_view| state_view.state_boundaries)
      .unwrap_or_default();
    let mut province_boundaries = BTreeMap::<u32, Vec<_>>::new();
    if project.is_some() {
      for (boundary, _) in bundle.map.iter_boundaries() {
        for position in boundary.into_array() {
          if let Some(province_id) = bundle.map.get_province_at(position).preserved_id {
            province_boundaries.entry(province_id).or_default().push(boundary);
          }
        }
      }
    }
    let map_view_mode = if state_texture.is_some() {
      MapViewMode::States
    } else {
      MapViewMode::Provinces
    };
    let state_edit_session = project
      .as_ref()
      .map(|project| StateEditSession::new(project, &bundle.map));
    let project_status = project.as_ref().map(|project| {
      project_status_message_with_session(project, state_edit_session.as_ref(), 0, "initial")
    });
    if let Some(project) = project.as_ref() {
      println!("{}", project.load_summary_message());
      println!(
        "Generated state map texture in {} ms.\nGenerated state boundaries in {} ms.",
        project.load_summary.state_texture_generation_ms,
        project.load_summary.state_boundary_generation_ms
      );
      println!("State project diagnostics:\n{}", project.diagnostic_report());
    }
    // The test map is very small with large ocean provinces, the 'too large box' errors go nuts
    let problems = if cfg!(any(debug_assertions, feature = "debug-mode")) { Vec::new() } else { bundle.generate_problems() };
    let unknown_terrains = bundle.search_unknown_terrains();
    let show_province_ids = bundle.config.preserve_ids;
    let camera = Camera::new(&texture);

    Ok(Canvas {
      bundle,
      history,
      texture,
      state_texture,
      state_boundaries,
      province_boundaries,
      selected_state_boundaries: Vec::new(),
      selected_province_boundaries: Vec::new(),
      lasso_preview_boundaries: Vec::new(),
      lasso_blocked_boundaries: Vec::new(),
      brush_preview_boundaries: Vec::new(),
      brush_blocked_boundaries: Vec::new(),
      selection_texture: None,
      texture_overlay: None,
      view_mode: ViewMode::default(),
      map_view_mode,
      state_selection: None,
      active_state_id: None,
      active_province_id: None,
      project_status,
      selection_info: None,
      tool: ToolSettings::default(),
      problems,
      unknown_terrains,
      location,
      project,
      state_edit_session,
      state_lifecycle_draft: None,
      state_property_draft: None,
      province_data_draft: None,
      property_editor_field: 0,
      property_editor_replace_field: false,
      province_editor_page: 0,
      state_lasso_phase: StateLassoPhase::Inactive,
      state_lasso_mode: LassoSelectionMode::default(),
      state_lasso_inclusion: ProvinceInclusionMode::default(),
      state_brush_phase: StateBrushPhase::Inactive,
      state_brush_mode: StateBrushMode::AssignToTarget,
      last_state_brush_result: None,
      state_province_extents: None,
      last_state_visual_update_ms: 0,
      last_state_visual_update_kind: "initial",
      map_access_mode,
      show_province_ids,
      show_province_boundaries: false,
      show_river_overlay: false,
      modified: false,
      camera
    })
  }

  pub fn save(&mut self, location: &Location) -> Result<SaveOperation, Error> {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      return Err("geographic map files are read-only in state projects".into());
    };

    if self.bundle.config.generate_coastal_on_save {
      self.history.calculate_coastal_provinces(&mut self.bundle);
    };

    let save_operation = self.bundle.save(location)?;
    self.location = location.clone();
    self.modified = false;

    Ok(save_operation)
  }

  pub fn location(&self) -> &Location {
    &self.location
  }

  pub fn view_mode(&self) -> ViewMode {
    self.view_mode
  }

  pub fn map_view_mode(&self) -> MapViewMode {
    self.map_view_mode
  }

  pub fn map_access_mode(&self) -> MapAccessMode {
    self.map_access_mode
  }

  pub fn project(&self) -> Option<&Hoi4Project> {
    self.project.as_ref()
  }

  pub fn has_unsaved_state_edits(&self) -> bool {
    self.state_edit_session
      .as_ref()
      .is_some_and(StateEditSession::is_dirty)
  }

  pub fn property_editor_is_open(&self) -> bool {
    self.state_lifecycle_draft.is_some()
      || self.state_property_draft.is_some()
      || self.province_data_draft.is_some()
  }

  pub fn property_draft_is_modified(&self) -> bool {
    self.state_lifecycle_draft.is_some()
      || self.state_property_draft
      .as_ref()
      .is_some_and(StatePropertyDraft::is_modified)
      || self.province_data_draft
        .as_ref()
        .is_some_and(ProvinceDataDraft::is_modified)
  }

  pub fn province_data_editor_is_open(&self) -> bool {
    self.province_data_draft.is_some()
  }

  pub fn open_state_property_editor(&mut self, alerts: &mut Alerts) {
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso before editing properties"));
      return;
    }
    let Some(state_id) = self.active_state_id else {
      alerts.push(Err("Select a valid state before editing properties"));
      return;
    };
    let Some(data) = self.state_edit_session
      .as_ref()
      .and_then(|edit| edit.state_data(state_id))
    else {
      alerts.push(Err("This state is invalid and remains read-only"));
      return;
    };
    let properties = EditableStateProperties::from_state(&data);
    self.state_property_draft = Some(StatePropertyDraft::new(state_id, &properties));
    self.property_editor_field = 0;
    self.property_editor_replace_field = false;
    self.refresh_state_information();
    alerts.push(Ok(format!("Editing State {state_id} properties in a temporary draft")));
  }

  pub fn open_new_state_editor(&mut self, alerts: &mut Alerts) {
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso before creating a state"));
      return;
    }
    let Some(edit) = self.state_edit_session.as_ref() else {
      alerts.push(Err("State creation is available only for loaded state projects"));
      return;
    };
    let suggested_id = edit.suggest_next_state_id();
    let properties = EditableStateProperties {
      manpower: Some(0),
      buildings_max_level_factor: Some(1.0),
      local_supplies: Some(0.0),
      ..Default::default()
    };
    self.state_lifecycle_draft = Some(StateLifecycleDraft::Create {
      id: suggested_id.to_string(),
      properties: Box::new(StatePropertyDraft::new(suggested_id, &properties)),
    });
    self.property_editor_field = 0;
    self.property_editor_replace_field = false;
    self.refresh_state_information();
    alerts.push(Ok(format!(
      "Preparing State {suggested_id} in memory; no state file will be created"
    )));
  }

  pub fn open_remove_state_editor(&mut self, alerts: &mut Alerts) {
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso before removing a state"));
      return;
    }
    let Some(state_id) = self.active_state_id else {
      alerts.push(Err("Select an active state before removing it from the session"));
      return;
    };
    let result = self.state_edit_session
      .as_ref()
      .ok_or_else(|| "State removal is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        edit.validate_removable_state(state_id)
          .map_err(|error| error.to_string())?;
        Ok(edit.state_province_count(state_id))
      });
    let province_count = match result {
      Ok(count) => count,
      Err(error) => {
        alerts.push(Err(error));
        return;
      },
    };
    self.state_lifecycle_draft = Some(StateLifecycleDraft::Remove {
      state_id,
      target_id: String::new(),
      unassign: province_count == 0,
      province_count,
    });
    self.property_editor_field = 0;
    self.property_editor_replace_field = false;
    self.refresh_state_information();
    alerts.push(Ok(format!(
      "Preparing to remove State {state_id} from the in-memory session only"
    )));
  }

  pub fn open_province_data_editor(&mut self, alerts: &mut Alerts) {
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso before editing province data"));
      return;
    }
    let Some(province_id) = self.active_province_id else {
      alerts.push(Err("Select a land province before editing province data"));
      return;
    };
    let result = self.state_edit_session
      .as_ref()
      .ok_or_else(|| "Province editing is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        let state_id = edit.editable_province_state(province_id)
          .map_err(|error| error.to_string())?;
        let data = edit.province_data(province_id)
          .ok_or_else(|| format!("Province {province_id} does not exist in the map"))?;
        Ok((state_id, data))
      });
    let (state_id, data) = match result {
      Ok(result) => result,
      Err(error) => {
        alerts.push(Err(error));
        return;
      },
    };
    self.province_data_draft = Some(ProvinceDataDraft::new(province_id, state_id, &data));
    self.property_editor_field = 0;
    self.property_editor_replace_field = false;
    self.province_editor_page = 0;
    self.refresh_state_information();
    alerts.push(Ok(format!(
      "Editing Province {province_id} data in a temporary draft"
    )));
  }

  pub fn apply_state_property_draft(&mut self, alerts: &mut Alerts) -> bool {
    if self.state_lifecycle_draft.is_some() {
      return self.apply_state_lifecycle_draft(None, alerts);
    }
    if self.province_data_draft.is_some() {
      return self.apply_province_data_draft(alerts);
    }
    let Some(draft) = self.state_property_draft.as_ref() else { return true };
    let state_id = draft.state_id;
    let properties = match draft.validate() {
      Ok(properties) => properties,
      Err(errors) => {
        alerts.push(Err(format!(
          "Draft has {} validation error(s); no values were applied",
          errors.len()
        )));
        return false;
      },
    };
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        edit.update_state_properties(state_id, properties)
          .map_err(|error| error.to_string())
      });
    match result {
      Ok(changed) => {
        self.state_property_draft = None;
        self.property_editor_replace_field = false;
        self.refresh_state_information();
        alerts.push(Ok(if changed {
          format!("Applied State {state_id} properties to the in-memory session")
        } else {
          format!("State {state_id} properties were unchanged")
        }));
        true
      },
      Err(error) => {
        alerts.push(Err(error));
        false
      },
    }
  }

  fn can_apply_state_creation(&self, use_selected: bool) -> bool {
    let Some(StateLifecycleDraft::Create { id, properties }) =
      self.state_lifecycle_draft.as_ref()
    else {
      return false;
    };
    let Ok(state_id) = id.trim().parse::<u32>() else { return false };
    let Some(edit) = self.state_edit_session.as_ref() else { return false };
    properties.validate()
      .is_ok_and(|properties| {
        edit.validate_state_creation(state_id, &properties, use_selected).is_ok()
      })
  }

  fn can_apply_state_removal(&self) -> bool {
    let Some(StateLifecycleDraft::Remove {
      state_id,
      target_id,
      unassign,
      ..
    }) = self.state_lifecycle_draft.as_ref()
    else {
      return false;
    };
    let Some(edit) = self.state_edit_session.as_ref() else { return false };
    let policy = if *unassign {
      StateRemovalPolicy::Unassign
    } else {
      let Ok(target) = target_id.trim().parse::<u32>() else { return false };
      StateRemovalPolicy::MoveToState(target)
    };
    edit.validate_state_removal(*state_id, policy).is_ok()
  }

  fn apply_state_lifecycle_draft(
    &mut self,
    use_selected_override: Option<bool>,
    alerts: &mut Alerts,
  ) -> bool {
    let Some(draft) = self.state_lifecycle_draft.clone() else { return true };
    let result = match draft {
      StateLifecycleDraft::Create { id, properties } => {
        let state_id = match id.trim().parse::<u32>() {
          Ok(state_id) => state_id,
          Err(_) => {
            alerts.push(Err("State ID must be an integer within the supported range"));
            return false;
          },
        };
        let properties = match properties.validate() {
          Ok(properties) => properties,
          Err(errors) => {
            alerts.push(Err(format!(
              "New state draft has {} validation error(s); nothing was created",
              errors.len()
            )));
            return false;
          },
        };
        let use_selected = use_selected_override.unwrap_or_else(|| {
          self.state_edit_session
            .as_ref()
            .is_some_and(|edit| !edit.selected_provinces().is_empty())
        });
        self.state_edit_session
          .as_mut()
          .ok_or_else(|| "State creation is available only for loaded state projects".to_owned())
          .and_then(|edit| {
            edit.create_state(state_id, properties, use_selected)
              .map_err(|error| error.to_string())
          })
          .map(|_| (Some(state_id), format!("Created State {state_id} in memory")))
      },
      StateLifecycleDraft::Remove {
        state_id,
        target_id,
        unassign,
        ..
      } => {
        let policy = if unassign {
          StateRemovalPolicy::Unassign
        } else {
          let target_state_id = match target_id.trim().parse::<u32>() {
            Ok(state_id) => state_id,
            Err(_) => {
              alerts.push(Err("Removal target must be a valid state ID"));
              return false;
            },
          };
          StateRemovalPolicy::MoveToState(target_state_id)
        };
        self.state_edit_session
          .as_mut()
          .ok_or_else(|| "State removal is available only for loaded state projects".to_owned())
          .and_then(|edit| {
            edit.remove_state(state_id, policy)
              .map_err(|error| error.to_string())
          })
          .map(|_| (None, format!("Removed State {state_id} from the in-memory session")))
      },
    };
    match result {
      Ok((created_state_id, message)) => {
        self.state_lifecycle_draft = None;
        self.property_editor_replace_field = false;
        if let Some(state_id) = created_state_id {
          self.active_state_id = Some(state_id);
          if let Some(province_id) = self.state_edit_session
            .as_ref()
            .and_then(|edit| edit.selected_provinces().iter().next().copied())
          {
            self.active_province_id = Some(province_id);
            self.state_selection = Some(StateSelection::State {
              state_id,
              province_id,
            });
          } else {
            self.state_selection = None;
          }
        } else {
          self.deactivate_state_brush();
          self.state_selection = None;
          self.selection_texture = None;
          self.selected_state_boundaries.clear();
          self.active_state_id = self.state_edit_session
            .as_ref()
            .and_then(StateEditSession::target_state_id);
        }
        self.refresh_state_visuals();
        alerts.push(Ok(message));
        true
      },
      Err(error) => {
        alerts.push(Err(error));
        false
      },
    }
  }

  fn apply_province_data_draft(&mut self, alerts: &mut Alerts) -> bool {
    let Some(draft) = self.province_data_draft.as_ref() else { return true };
    let province_id = draft.province_id;
    let state_id = draft.state_id;
    let data = match draft.validate() {
      Ok(data) => data,
      Err(errors) => {
        alerts.push(Err(format!(
          "Province draft has {} validation error(s); no values were applied",
          errors.len()
        )));
        return false;
      },
    };
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "Province editing is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        edit.update_province_data(province_id, state_id, data)
          .map_err(|error| error.to_string())
      });
    match result {
      Ok(changed) => {
        self.province_data_draft = None;
        self.property_editor_replace_field = false;
        self.province_editor_page = 0;
        self.refresh_state_visuals();
        alerts.push(Ok(if changed {
          format!("Applied Province {province_id} data to the in-memory session")
        } else {
          format!("Province {province_id} data was unchanged")
        }));
        true
      },
      Err(error) => {
        alerts.push(Err(error));
        false
      },
    }
  }

  pub fn discard_state_property_draft(&mut self, alerts: &mut Alerts) {
    if self.state_lifecycle_draft.take().is_some() {
      self.property_editor_replace_field = false;
      self.refresh_state_information();
      alerts.push(Ok("Cancelled the in-memory state lifecycle draft"));
      return;
    }
    if let Some(draft) = self.province_data_draft.take() {
      self.property_editor_replace_field = false;
      self.province_editor_page = 0;
      self.refresh_state_information();
      alerts.push(Ok(format!(
        "Discarded unapplied draft for Province {}",
        draft.province_id
      )));
      return;
    }
    if let Some(draft) = self.state_property_draft.take() {
      self.property_editor_replace_field = false;
      self.refresh_state_information();
      alerts.push(Ok(format!("Discarded unapplied draft for State {}", draft.state_id)));
    }
  }

  pub fn discard_unmodified_property_draft(&mut self) -> bool {
    if self.province_data_draft.as_ref().is_some_and(|draft| !draft.is_modified()) {
      self.province_data_draft = None;
      self.property_editor_replace_field = false;
      self.province_editor_page = 0;
      self.refresh_state_information();
      return true;
    }
    if self.state_property_draft.as_ref().is_some_and(|draft| !draft.is_modified()) {
      self.state_property_draft = None;
      self.property_editor_replace_field = false;
      self.refresh_state_information();
      true
    } else {
      false
    }
  }

  pub fn state_click_would_change_property_draft(
    &self,
    interface: &Interface,
    cursor_pos: Vector2<f64>,
  ) -> bool {
    if !self.property_draft_is_modified() {
      return false;
    }
    if self.state_lifecycle_draft.is_some() {
      return true;
    }
    let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) else {
      return true;
    };
    if let Some(draft) = self.province_data_draft.as_ref().filter(|draft| draft.is_modified()) {
      return self.bundle.map.get_province_at(pos).preserved_id != Some(draft.province_id);
    }
    let Some(draft) = self.state_property_draft.as_ref().filter(|draft| draft.is_modified()) else {
      return false;
    };
    let Some(project) = self.project.as_ref() else { return true };
    let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps() else {
      return true;
    };
    !matches!(
      resolve_state_at_for(
        &self.bundle.map,
        state_by_province,
        &project.ambiguous_provinces,
        unassigned_land_provinces,
        pos,
      ),
      Some(StateSelection::State { state_id, .. }) if state_id == draft.state_id
    )
  }

  pub fn state_property_editor_click(
    &mut self,
    interface: &Interface,
    pos: Vector2<f64>,
    alerts: &mut Alerts,
  ) -> bool {
    if self.state_lifecycle_draft.is_some() {
      return self.state_lifecycle_editor_click(interface, pos, alerts);
    }
    if self.province_data_draft.is_some() {
      return self.province_data_editor_click(interface, pos, alerts);
    }
    let Some(draft) = self.state_property_draft.as_ref() else { return false };
    let layout = PropertyEditorLayout::new(interface);
    if !point_in_rect(pos, layout.panel) {
      return false;
    }
    for index in 0..StatePropertyDraft::TEXT_FIELD_COUNT {
      if point_in_rect(pos, layout.field(index)) {
        self.property_editor_field = index;
        self.property_editor_replace_field = false;
        return true;
      }
    }
    if point_in_rect(pos, layout.impassable()) {
      self.property_editor_field = StatePropertyDraft::TEXT_FIELD_COUNT;
      if let Some(draft) = self.state_property_draft.as_mut() {
        draft.impassable = !draft.impassable;
      }
      return true;
    }
    if point_in_rect(pos, layout.apply()) {
      if draft.is_modified() && draft.validate().is_ok() {
        self.apply_state_property_draft(alerts);
      }
      return true;
    }
    if point_in_rect(pos, layout.discard()) {
      self.discard_state_property_draft(alerts);
      return true;
    }
    true
  }

  fn state_lifecycle_editor_click(
    &mut self,
    interface: &Interface,
    pos: Vector2<f64>,
    alerts: &mut Alerts,
  ) -> bool {
    match self.state_lifecycle_draft.as_ref() {
      Some(StateLifecycleDraft::Create { .. }) => {
        let layout = StateCreationEditorLayout::new(interface);
        if !point_in_rect(pos, layout.panel) {
          return false;
        }
        for index in 0..=StatePropertyDraft::TEXT_FIELD_COUNT {
          if point_in_rect(pos, layout.field(index)) {
            self.property_editor_field = index;
            self.property_editor_replace_field = false;
            return true;
          }
        }
        if point_in_rect(pos, layout.use_next_id()) {
          if let (Some(edit), Some(StateLifecycleDraft::Create { id, .. })) = (
            self.state_edit_session.as_ref(),
            self.state_lifecycle_draft.as_mut(),
          ) {
            *id = edit.suggest_next_state_id().to_string();
          }
          self.property_editor_field = 0;
          self.property_editor_replace_field = false;
          return true;
        }
        if point_in_rect(pos, layout.impassable()) {
          if let Some(StateLifecycleDraft::Create { properties, .. }) =
            self.state_lifecycle_draft.as_mut()
          {
            properties.impassable = !properties.impassable;
          }
          self.property_editor_field = StatePropertyDraft::TEXT_FIELD_COUNT + 1;
          return true;
        }
        if point_in_rect(pos, layout.create_selected()) {
          if self.can_apply_state_creation(true) {
            self.apply_state_lifecycle_draft(Some(true), alerts);
          }
          return true;
        }
        if point_in_rect(pos, layout.create_empty()) {
          if self.can_apply_state_creation(false) {
            self.apply_state_lifecycle_draft(Some(false), alerts);
          }
          return true;
        }
        if point_in_rect(pos, layout.cancel()) {
          self.discard_state_property_draft(alerts);
        }
        true
      },
      Some(StateLifecycleDraft::Remove { .. }) => {
        let layout = StateRemovalEditorLayout::new(interface);
        if !point_in_rect(pos, layout.panel) {
          return false;
        }
        if point_in_rect(pos, layout.target_field()) {
          self.property_editor_field = 0;
          self.property_editor_replace_field = false;
          return true;
        }
        if point_in_rect(pos, layout.move_all()) {
          if let Some(StateLifecycleDraft::Remove { unassign, .. }) =
            self.state_lifecycle_draft.as_mut()
          {
            *unassign = false;
          }
          return true;
        }
        if point_in_rect(pos, layout.unassign_all()) {
          if let Some(StateLifecycleDraft::Remove { unassign, .. }) =
            self.state_lifecycle_draft.as_mut()
          {
            *unassign = true;
          }
          return true;
        }
        if point_in_rect(pos, layout.remove()) {
          if self.can_apply_state_removal() {
            self.apply_state_lifecycle_draft(None, alerts);
          }
          return true;
        }
        if point_in_rect(pos, layout.cancel()) {
          self.discard_state_property_draft(alerts);
        }
        true
      },
      None => false,
    }
  }

  fn province_data_editor_click(
    &mut self,
    interface: &Interface,
    pos: Vector2<f64>,
    alerts: &mut Alerts,
  ) -> bool {
    let Some(draft) = self.province_data_draft.as_ref() else { return false };
    let layout = ProvinceEditorLayout::new(
      interface,
      draft.buildings.len(),
      self.province_editor_page,
    );
    if !point_in_rect(pos, layout.panel) {
      return false;
    }
    if draft.victory_point.is_some() && point_in_rect(pos, layout.victory_point_field()) {
      self.property_editor_field = 0;
      self.property_editor_replace_field = false;
      return true;
    }
    if point_in_rect(pos, layout.victory_point_toggle()) {
      if let Some(draft) = self.province_data_draft.as_mut() {
        draft.toggle_victory_point();
      }
      self.property_editor_field = 0;
      self.property_editor_replace_field = false;
      return true;
    }
    for row in layout.visible_range() {
      if point_in_rect(pos, layout.building_name(row)) {
        self.property_editor_field = 1 + row * 2;
        self.property_editor_replace_field = false;
        return true;
      }
      if point_in_rect(pos, layout.building_value(row)) {
        self.property_editor_field = 2 + row * 2;
        self.property_editor_replace_field = false;
        return true;
      }
      if point_in_rect(pos, layout.building_remove(row)) {
        if let Some(draft) = self.province_data_draft.as_mut() {
          draft.remove_building(row);
        }
        self.property_editor_field = 0;
        self.property_editor_replace_field = false;
        self.province_editor_page = layout.clamp_page(self.province_editor_page);
        return true;
      }
    }
    if point_in_rect(pos, layout.add_building()) {
      if let Some(draft) = self.province_data_draft.as_mut() {
        let row = draft.add_building();
        self.property_editor_field = 1 + row * 2;
        self.province_editor_page = row / layout.visible_rows.max(1);
      }
      self.property_editor_replace_field = false;
      return true;
    }
    if point_in_rect(pos, layout.previous_page()) && self.province_editor_page > 0 {
      self.province_editor_page -= 1;
      return true;
    }
    if point_in_rect(pos, layout.next_page())
      && layout.has_next_page(self.province_editor_page)
    {
      self.province_editor_page += 1;
      return true;
    }
    if point_in_rect(pos, layout.apply()) {
      if draft.is_modified() && draft.validate().is_ok() {
        self.apply_province_data_draft(alerts);
      }
      return true;
    }
    if point_in_rect(pos, layout.discard()) {
      self.discard_state_property_draft(alerts);
      return true;
    }
    true
  }

  pub fn input_state_property_text(&mut self, text: &str) {
    if let Some(draft) = self.state_lifecycle_draft.as_mut() {
      let Some(field) = draft.field_mut(self.property_editor_field) else { return };
      if self.property_editor_replace_field {
        field.clear();
        self.property_editor_replace_field = false;
      }
      field.extend(text.chars().filter(|character| !character.is_control()));
      return;
    }
    if let Some(draft) = self.province_data_draft.as_mut() {
      let Some(field) = draft.field_mut(self.property_editor_field) else { return };
      if self.property_editor_replace_field {
        field.clear();
        self.property_editor_replace_field = false;
      }
      field.extend(text.chars().filter(|character| !character.is_control()));
      return;
    }
    let Some(draft) = self.state_property_draft.as_mut() else { return };
    let Some(field) = draft.field_mut(self.property_editor_field) else { return };
    if self.property_editor_replace_field {
      field.clear();
      self.property_editor_replace_field = false;
    }
    field.extend(text.chars().filter(|character| !character.is_control()));
  }

  pub fn state_property_editor_select_all(&mut self) {
    if self.state_lifecycle_draft.as_ref()
      .and_then(|draft| draft.field(self.property_editor_field))
      .is_some()
    {
      self.property_editor_replace_field = true;
      return;
    }
    if self.province_data_draft.as_ref()
      .and_then(|draft| draft.field(self.property_editor_field))
      .is_some()
    {
      self.property_editor_replace_field = true;
      return;
    }
    if self.property_editor_field < StatePropertyDraft::TEXT_FIELD_COUNT {
      self.property_editor_replace_field = true;
    }
  }

  pub fn state_property_editor_backspace(&mut self) {
    if let Some(draft) = self.state_lifecycle_draft.as_mut() {
      let Some(field) = draft.field_mut(self.property_editor_field) else { return };
      if self.property_editor_replace_field {
        field.clear();
        self.property_editor_replace_field = false;
      } else {
        field.pop();
      }
      return;
    }
    if let Some(draft) = self.province_data_draft.as_mut() {
      let Some(field) = draft.field_mut(self.property_editor_field) else { return };
      if self.property_editor_replace_field {
        field.clear();
        self.property_editor_replace_field = false;
      } else {
        field.pop();
      }
      return;
    }
    let Some(draft) = self.state_property_draft.as_mut() else { return };
    let Some(field) = draft.field_mut(self.property_editor_field) else { return };
    if self.property_editor_replace_field {
      field.clear();
      self.property_editor_replace_field = false;
    } else {
      field.pop();
    }
  }

  pub fn state_property_editor_clear_field(&mut self) {
    if let Some(field) = self.state_lifecycle_draft
      .as_mut()
      .and_then(|draft| draft.field_mut(self.property_editor_field))
    {
      field.clear();
      self.property_editor_replace_field = false;
      return;
    }
    if let Some(field) = self.province_data_draft
      .as_mut()
      .and_then(|draft| draft.field_mut(self.property_editor_field))
    {
      field.clear();
      self.property_editor_replace_field = false;
      return;
    }
    if let Some(field) = self.state_property_draft
      .as_mut()
      .and_then(|draft| draft.field_mut(self.property_editor_field))
    {
      field.clear();
      self.property_editor_replace_field = false;
    }
  }

  pub fn state_property_editor_next_field(&mut self, backwards: bool) {
    let count = self.state_lifecycle_draft.as_ref()
      .map(|draft| match draft {
        StateLifecycleDraft::Create { .. } => draft.text_field_count() + 1,
        StateLifecycleDraft::Remove { .. } => draft.text_field_count(),
      })
      .or_else(|| self.province_data_draft.as_ref().map(ProvinceDataDraft::text_field_count))
      .unwrap_or(StatePropertyDraft::TEXT_FIELD_COUNT + 1)
      .max(1);
    self.property_editor_field = if backwards {
      self.property_editor_field.checked_sub(1).unwrap_or(count - 1)
    } else {
      (self.property_editor_field + 1) % count
    };
    self.property_editor_replace_field = false;
  }

  pub fn set_location(&mut self, location: Location) {
    self.location = location;
  }

  pub fn config(&self) -> &Config {
    &self.bundle.config
  }

  pub fn draw(&mut self, ctx: Context, interface: &Interface, glyph_cache: &mut FontGlyphCache, cursor_pos: Option<Vector2<f64>>, gl: &mut GlGraphics) {
    use super::alerts::PADDING;

    let transform = ctx.transform.append_transform(self.camera.display_matrix(interface));
    let showing_states = self.map_view_mode == MapViewMode::States;
    let texture = if showing_states {
      self.state_texture.as_ref().unwrap_or(&self.texture)
    } else {
      &self.texture
    };
    graphics::image(texture, transform, gl);

    if showing_states
      && let Some(selection_texture) = &self.selection_texture
    {
        graphics::rectangle(
          colors::OVERLAY_T,
          [0.0, 0.0, interface.get_window_size()[0], interface.get_window_size()[1]],
          ctx.transform,
          gl
        );
        graphics::image(selection_texture, transform, gl);
        self.draw_selected_state_boundaries(ctx, interface, gl);
    };
    if showing_states {
      self.draw_selected_province_boundaries(ctx, interface, gl);
      self.draw_state_lasso(ctx, interface, cursor_pos, gl);
      self.draw_state_brush(ctx, interface, gl);
    }

    let texture_overlay = self.bundle.map.get_rivers_overlay()
      .filter(|_| self.show_river_overlay)
      .map(|rivers_overlay| self.texture_overlay.get_or_insert_with(|| {
        let texture_settings = TextureSettings::new().mag(Filter::Nearest);
        let texture = Texture::from_image(&rivers_overlay, &texture_settings);
        texture
      }));

    if let Some(texture_overlay) = texture_overlay {
      graphics::image(texture_overlay, transform, gl);
    };

    if self.camera.scale_factor() > 1.0 && self.show_province_boundaries {
      self.draw_boundaries(ctx, interface, gl);
    };

    if self.view_mode == ViewMode::Adjacencies {
      self.draw_adjacencies(ctx, interface, cursor_pos, gl);
    } else if self.camera.scale_factor() > 1.0 && self.show_province_ids {
      self.draw_ids(ctx, interface, glyph_cache, gl);
    };

    self.draw_problems(ctx, interface, gl);

    self.draw_tool(ctx, interface, cursor_pos, gl);
    if self.state_lifecycle_draft.is_some() {
      self.draw_state_lifecycle_editor(ctx, interface, glyph_cache, gl);
    } else if self.province_data_draft.is_some() {
      self.draw_province_data_editor(ctx, interface, glyph_cache, gl);
    } else if self.state_property_draft.is_some() {
      self.draw_state_property_editor(ctx, interface, glyph_cache, gl);
    } else {
      self.draw_project_information(ctx, interface, glyph_cache, gl);
    }

    let camera_info = self.camera_info(interface, cursor_pos);
    let pos = [PADDING[0] + interface.get_sidebar_width() as f64, interface.get_window_size()[1] - PADDING[1] * 1.25];
    let transform = ctx.transform.trans_pos(pos);
    graphics::text(colors::WHITE, FONT_SIZE, &camera_info, glyph_cache, transform, gl)
      .expect("unable to draw text");
  }

  fn draw_ids(&self, ctx: Context, interface: &Interface, glyph_cache: &mut FontGlyphCache, gl: &mut GlGraphics) {
    for (_color, province_data) in self.bundle.map.iter_province_data() {
      let preserved_id = province_data.preserved_id
        .map_or_else(|| "X".to_owned(), |id| id.to_string());
      let color = if self.map_view_mode == MapViewMode::States {
        colors::BLACK
      } else {
        match self.view_mode {
        ViewMode::Color | ViewMode::Adjacencies => match province_data.kind {
          ProvinceKind::Land | ProvinceKind::Lake => colors::BLACK,
          ProvinceKind::Sea | ProvinceKind::Unknown => colors::WHITE
        },
        ViewMode::Kind | ViewMode::Terrain => colors::BLACK,
        ViewMode::Continent => colors::WHITE,
        ViewMode::Coastal => match province_data.coastal {
          Some(true) => colors::BLACK,
          Some(false) | None => colors::WHITE
        }
        }
      };

      let center_of_mass = vecmath::vec2_add([0.5, 0.5], province_data.center_of_mass());
      let center_of_mass = self.camera.compute_position(interface, center_of_mass);
      if self.camera.within_viewport(interface, center_of_mass) {
        let preserved_id = preserved_id.to_string();
        let offset = [
          font::get_width_metric_str(&preserved_id) / -2.0,
          font::get_v_metrics().ascent - font::get_height_metric() / 2.0
        ];
        let transform = ctx.transform.trans_pos(center_of_mass).trans_pos(offset);
        graphics::text(color, FONT_SIZE, &preserved_id, glyph_cache, transform, gl)
          .expect("unable to draw text");
      };
    };
  }

  fn draw_adjacencies(&self, ctx: Context, interface: &Interface, cursor_pos: Option<Vector2<f64>>, gl: &mut GlGraphics) {
    // Draw the adjacency the user is currently creating
    if let (Some(sel), Some(kind), Some(cursor_pos)) = (self.tool.adjacency_selection, self.tool.adjacency_brush, cursor_pos) {
      let color = kind.draw_color();
      let pos = self.bundle.map.get_province(sel).center_of_mass();
      let pos = self.camera.compute_position(interface, pos);

      graphics::line_from_to(color, 2.0, pos, cursor_pos, ctx.transform, gl);
    };

    // Draw all adjacencies as lines between the centers of every province (except for impassable)
    for (rel, connection_data) in self.bundle.map.iter_connection_data() {
      if connection_data.kind != ConnectionKind::Impassable {
        let color = connection_data.kind.draw_color();
        let (center1, center2) = self.bundle.map.get_connection_positions(rel);
        let center1 = self.camera.compute_position(interface, center1);
        let center2 = self.camera.compute_position(interface, center2);

        graphics::line_from_to(color, 2.0, center1, center2, ctx.transform, gl);
      };
    };

    // Draw impassible adjacencies as black boundaries
    for (boundary, is_special) in self.bundle.map.iter_boundaries() {
      if is_special {
        let rel = boundary.map(|pos| self.bundle.map.get_color_at(pos));
        if self.bundle.map.get_connection(rel).kind == ConnectionKind::Impassable {
          let [b1, b2] = boundary_to_line(boundary).into_array();
          let b1 = self.camera.compute_position(interface, [b1[0] as f64, b1[1] as f64]);
          let b2 = self.camera.compute_position(interface, [b2[0] as f64, b2[1] as f64]);
          if self.camera.within_viewport(interface, b1) || self.camera.within_viewport(interface, b2) {
            graphics::line_from_to(colors::ADJ_IMPASSABLE, 2.0, b1, b2, ctx.transform, gl);
          };
        };
      };
    };
  }

  fn draw_boundaries(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
    for (boundary, _is_special) in self.bundle.map.iter_boundaries() {
      let [b1, b2] = boundary_to_line(boundary).into_array();
      let b1 = self.camera.compute_position(interface, [b1[0] as f64, b1[1] as f64]);
      let b2 = self.camera.compute_position(interface, [b2[0] as f64, b2[1] as f64]);
      if self.camera.within_viewport(interface, b1) || self.camera.within_viewport(interface, b2) {
        let color = match self.view_mode {
          ViewMode::Color | ViewMode::Adjacencies => {
            drawable_color(boundary_color(&self.bundle.map, boundary))
          },
          ViewMode::Kind | ViewMode::Terrain => colors::BLACK,
          ViewMode::Continent => colors::WHITE,
          ViewMode::Coastal => colors::NEUTRAL
        };

        graphics::line_from_to(color, 1.0, b1, b2, ctx.transform, gl);
      };
    };
  }

  fn draw_selected_state_boundaries(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
    for &boundary in &self.selected_state_boundaries {
      let [b1, b2] = boundary_to_line(boundary).into_array();
      let b1 = self.camera.compute_position(interface, [b1[0] as f64, b1[1] as f64]);
      let b2 = self.camera.compute_position(interface, [b2[0] as f64, b2[1] as f64]);
      if self.camera.within_viewport(interface, b1) || self.camera.within_viewport(interface, b2) {
        graphics::line_from_to(colors::WARNING, 2.5, b1, b2, ctx.transform, gl);
      };
    };
  }

  fn draw_selected_province_boundaries(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
    for &boundary in &self.selected_province_boundaries {
      let [b1, b2] = boundary_to_line(boundary).into_array();
      let b1 = self.camera.compute_position(interface, [b1[0] as f64, b1[1] as f64]);
      let b2 = self.camera.compute_position(interface, [b2[0] as f64, b2[1] as f64]);
      if self.camera.within_viewport(interface, b1) || self.camera.within_viewport(interface, b2) {
        graphics::line_from_to(colors::WHITE, 3.0, b1, b2, ctx.transform, gl);
      };
    };
  }

  fn draw_state_lasso(
    &self,
    ctx: Context,
    interface: &Interface,
    cursor_pos: Option<Vector2<f64>>,
    gl: &mut GlGraphics
  ) {
    const PREVIEW: DrawColor = [0.0, 0.9, 1.0, 1.0];
    const BLOCKED: DrawColor = [1.0, 0.0, 0.75, 1.0];
    const POLYGON: DrawColor = [1.0, 0.85, 0.1, 1.0];

    self.draw_boundary_set(ctx, interface, &self.lasso_preview_boundaries, PREVIEW, 3.5, gl);
    self.draw_boundary_set(ctx, interface, &self.lasso_blocked_boundaries, BLOCKED, 4.0, gl);

    let Some(points) = self.state_lasso_phase.points() else { return };
    let points = points.iter()
      .copied()
      .map(|point| self.camera.compute_position(interface, point))
      .collect::<Vec<_>>();
    let first_point = points.first().copied();
    let drawing = matches!(self.state_lasso_phase, StateLassoPhase::Drawing { .. });
    let can_finish = drawing && cursor_pos
      .zip(first_point)
      .is_some_and(|(cursor, first)| vecmath::vec2_len(vecmath::vec2_sub(first, cursor)) < 5.0);
    let last_point = if drawing {
      if can_finish { first_point } else { cursor_pos }
    } else {
      first_point
    };

    if let Some(first_point) = first_point {
      let ellipse = Ellipse::new(POLYGON).resolution(8);
      let transform = ctx.transform.trans_pos(first_point);
      ellipse.draw_from_to([5.0, 5.0], [-5.0, -5.0], &Default::default(), transform, gl);
    }
    for (a, b) in points.into_iter()
      .chain(last_point)
      .tuple_windows::<(_, _)>()
    {
      graphics::line_from_to(POLYGON, 2.0, a, b, ctx.transform, gl);
    }
  }

  fn draw_state_brush(
    &self,
    ctx: Context,
    interface: &Interface,
    gl: &mut GlGraphics,
  ) {
    const PREVIEW: DrawColor = [0.1, 1.0, 0.45, 1.0];
    const BLOCKED: DrawColor = [1.0, 0.0, 0.75, 1.0];

    self.draw_boundary_set(
      ctx,
      interface,
      &self.brush_preview_boundaries,
      PREVIEW,
      3.5,
      gl,
    );
    self.draw_boundary_set(
      ctx,
      interface,
      &self.brush_blocked_boundaries,
      BLOCKED,
      4.0,
      gl,
    );
  }

  fn draw_boundary_set(
    &self,
    ctx: Context,
    interface: &Interface,
    boundaries: &[UOrd<Vector2<u32>>],
    color: DrawColor,
    width: f64,
    gl: &mut GlGraphics
  ) {
    for &boundary in boundaries {
      let [b1, b2] = boundary_to_line(boundary).into_array();
      let b1 = self.camera.compute_position(interface, [b1[0] as f64, b1[1] as f64]);
      let b2 = self.camera.compute_position(interface, [b2[0] as f64, b2[1] as f64]);
      if self.camera.within_viewport(interface, b1) || self.camera.within_viewport(interface, b2) {
        graphics::line_from_to(color, width, b1, b2, ctx.transform, gl);
      }
    }
  }

  fn draw_project_information(
    &self,
    ctx: Context,
    interface: &Interface,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics
  ) {
    let Some(project_status) = self.project_status.as_deref() else { return };
    let selection_info = self.selection_info.as_deref();
    let lines = || project_status.lines()
      .chain(selection_info.into_iter().flat_map(str::lines));
    let line_height = font::get_height_metric() * 1.15;
    let width = lines()
      .map(font::get_width_metric_str)
      .fold(0.0, f64::max)
      + 12.0;
    let height = lines().count() as f64 * line_height + 8.0;
    let pos = [
      interface.get_sidebar_width() as f64 + 6.0,
      interface.get_toolbar_height() as f64 + line_height + 10.0
    ];

    graphics::rectangle(
      colors::OVERLAY_T,
      [pos[0], pos[1], width, height],
      ctx.transform,
      gl
    );
    for (index, line) in lines().enumerate() {
      let transform = ctx.transform.trans(
        pos[0] + 6.0,
        pos[1] + 4.0 + font::get_v_metrics().ascent + index as f64 * line_height
      );
      graphics::text(colors::WHITE, FONT_SIZE, line, glyph_cache, transform, gl)
        .expect("unable to draw state information");
    };
  }

  fn draw_state_lifecycle_editor(
    &self,
    ctx: Context,
    interface: &Interface,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics,
  ) {
    match self.state_lifecycle_draft.as_ref() {
      Some(StateLifecycleDraft::Create { id, properties }) => {
        let layout = StateCreationEditorLayout::new(interface);
        let property_errors = properties.validate().err().unwrap_or_default();
        let invalid_fields = property_errors.iter()
          .map(|error| error.field)
          .collect::<BTreeSet<_>>();
        let state_id = id.trim().parse::<u32>().ok();
        let id_valid = state_id.is_some_and(|state_id| {
          self.state_edit_session
            .as_ref()
            .is_some_and(|edit| edit.validate_new_state_id(state_id).is_ok())
        });
        let selected = self.state_edit_session
          .as_ref()
          .map_or(0, |edit| edit.selected_provinces().len());

        graphics::rectangle([0.06, 0.07, 0.09, 0.97], layout.panel, ctx.transform, gl);
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, layout.panel[1] + 24.0],
          "NEW STATE",
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WARNING,
          [layout.panel[0] + 12.0, layout.panel[1] + 44.0],
          "Created in memory — no state file will be created",
        );

        let id_rect = layout.field(0);
        graphics::rectangle(
          editor_field_color(self.property_editor_field == 0, !id_valid),
          id_rect,
          ctx.transform,
          gl,
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, id_rect[1] + 17.0],
          "State ID",
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [id_rect[0] + 6.0, id_rect[1] + 17.0],
          &fit_editor_text(id, id_rect[2] - 12.0),
        );
        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.use_next_id(),
          "Use next available",
          true,
        );

        for (index, label) in STATE_PROPERTY_LABELS.iter().enumerate() {
          let field_index = index + 1;
          let rect = layout.field(field_index);
          graphics::rectangle(
            editor_field_color(
              self.property_editor_field == field_index,
              invalid_fields.contains(STATE_PROPERTY_FIELD_KEYS[index]),
            ),
            rect,
            ctx.transform,
            gl,
          );
          draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [layout.panel[0] + 12.0, rect[1] + 17.0],
            label,
          );
          draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [rect[0] + 6.0, rect[1] + 17.0],
            &fit_editor_text(properties.field(index).unwrap_or_default(), rect[2] - 12.0),
          );
        }

        let impassable = layout.impassable();
        graphics::rectangle(
          if properties.impassable { colors::BUTTON_ACTIVE } else { colors::BUTTON },
          impassable,
          ctx.transform,
          gl,
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, impassable[1] + 17.0],
          "Impassable",
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [impassable[0] + 6.0, impassable[1] + 17.0],
          if properties.impassable { "Yes" } else { "No" },
        );

        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, layout.selection_y],
          &format!("Selected provinces: {selected}"),
        );
        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.create_selected(),
          &format!("Create selected ({selected})"),
          self.can_apply_state_creation(true),
        );
        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.create_empty(),
          "Create empty state",
          self.can_apply_state_creation(false),
        );
        draw_editor_button(ctx, glyph_cache, gl, layout.cancel(), "Cancel", true);

        let status = if !id_valid {
          "State ID is invalid, occupied, or reserved".to_owned()
        } else if !property_errors.is_empty() {
          format!("New state validation errors: {}", property_errors.len())
        } else if selected == 0 {
          "Creating empty will add a session warning".to_owned()
        } else {
          "Ready for one atomic creation command".to_owned()
        };
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          if id_valid && property_errors.is_empty() { colors::WHITE } else { colors::PROBLEM },
          [layout.panel[0] + 12.0, layout.status_y],
          &status,
        );
      },
      Some(StateLifecycleDraft::Remove {
        state_id,
        target_id,
        unassign,
        province_count,
      }) => {
        let layout = StateRemovalEditorLayout::new(interface);
        let can_remove = self.can_apply_state_removal();
        let removal_warning = match self.state_edit_session
          .as_ref()
          .and_then(|edit| edit.state_origin(*state_id))
        {
          Some(WorkingStateOrigin::CreatedInSession) => {
            "Removes the temporary state — no state file exists"
          },
          _ => "In-memory only — the original state file will not be changed",
        };
        graphics::rectangle([0.06, 0.07, 0.09, 0.97], layout.panel, ctx.transform, gl);
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, layout.panel[1] + 24.0],
          &format!("REMOVE STATE {state_id} FROM SESSION"),
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WARNING,
          [layout.panel[0] + 12.0, layout.panel[1] + 46.0],
          removal_warning,
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [layout.panel[0] + 12.0, layout.panel[1] + 76.0],
          &format!("State {state_id} contains {province_count} provinces."),
        );

        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.move_all(),
          "Move all to State",
          !*unassign,
        );
        let target_rect = layout.target_field();
        graphics::rectangle(
          editor_field_color(self.property_editor_field == 0, !*unassign && !can_remove),
          target_rect,
          ctx.transform,
          gl,
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WHITE,
          [target_rect[0] + 6.0, target_rect[1] + 17.0],
          if *unassign { "Target disabled" } else { target_id },
        );
        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.unassign_all(),
          "Unassign all",
          *unassign,
        );
        draw_canvas_text(
          ctx,
          glyph_cache,
          gl,
          colors::WARNING,
          [layout.panel[0] + 12.0, layout.panel[1] + 164.0],
          if *unassign && *province_count > 0 {
            "These land provinces will be temporarily unassigned."
          } else {
            "Province data will move with each province."
          },
        );
        draw_editor_button(
          ctx,
          glyph_cache,
          gl,
          layout.remove(),
          "Remove from session",
          can_remove,
        );
        draw_editor_button(ctx, glyph_cache, gl, layout.cancel(), "Cancel", true);
      },
      None => {},
    }
  }

  fn draw_state_property_editor(
    &self,
    ctx: Context,
    interface: &Interface,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics
  ) {
    let Some(draft) = self.state_property_draft.as_ref() else { return };
    let layout = PropertyEditorLayout::new(interface);
    let errors = draft.validate().err().unwrap_or_default();
    let can_apply = draft.is_modified() && errors.is_empty();
    let invalid_fields = errors.iter().map(|error| error.field).collect::<BTreeSet<_>>();

    graphics::rectangle([0.06, 0.07, 0.09, 0.97], layout.panel, ctx.transform, gl);
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [layout.panel[0] + 12.0, layout.panel[1] + 24.0],
      &format!("STATE {} — EDIT PROPERTIES", draft.state_id),
    );
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WARNING,
      [layout.panel[0] + 12.0, layout.panel[1] + 44.0],
      "In-memory draft — no files will be written",
    );

    for (index, label) in STATE_PROPERTY_LABELS.iter().enumerate() {
      let rect = layout.field(index);
      let active = self.property_editor_field == index;
      let invalid = invalid_fields.contains(STATE_PROPERTY_FIELD_KEYS[index]);
      let background = if invalid {
        [0.38, 0.08, 0.08, 1.0]
      } else if active {
        [0.12, 0.25, 0.42, 1.0]
      } else {
        [0.14, 0.15, 0.18, 1.0]
      };
      graphics::rectangle(background, rect, ctx.transform, gl);
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::WHITE,
        [layout.panel[0] + 12.0, rect[1] + 17.0],
        label,
      );
      let value = draft.field(index).unwrap_or_default();
      let value = fit_editor_text(value, rect[2] - 12.0);
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::WHITE,
        [rect[0] + 6.0, rect[1] + 17.0],
        &value,
      );
    }

    let impassable = layout.impassable();
    graphics::rectangle(
      if draft.impassable { colors::BUTTON_ACTIVE } else { colors::BUTTON },
      impassable,
      ctx.transform,
      gl,
    );
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [layout.panel[0] + 12.0, impassable[1] + 17.0],
      "Impassable",
    );
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [impassable[0] + 6.0, impassable[1] + 17.0],
      if draft.impassable { "Yes" } else { "No" },
    );

    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.apply(),
      "Apply to session",
      can_apply,
    );
    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.discard(),
      "Discard draft",
      true,
    );

    let status = if !draft.is_modified() {
      "No draft changes".to_owned()
    } else if errors.is_empty() {
      "Draft modified — ready to apply".to_owned()
    } else {
      format!("Draft validation errors: {}", errors.len())
    };
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      if errors.is_empty() { colors::WHITE } else { colors::PROBLEM },
      [layout.panel[0] + 12.0, layout.error_y],
      &status,
    );
    for (index, error) in errors.iter().take(layout.visible_error_lines).enumerate() {
      let message = format!("{}: {}", error.field, error.message);
      let message = fit_editor_text(&message, layout.panel[2] - 24.0);
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::PROBLEM,
        [
          layout.panel[0] + 12.0,
          layout.error_y + 19.0 + index as f64 * 18.0,
        ],
        &message,
      );
    }
  }

  fn draw_province_data_editor(
    &self,
    ctx: Context,
    interface: &Interface,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics
  ) {
    let Some(draft) = self.province_data_draft.as_ref() else { return };
    let layout = ProvinceEditorLayout::new(
      interface,
      draft.buildings.len(),
      self.province_editor_page,
    );
    let errors = draft.validate().err().unwrap_or_default();
    let can_apply = draft.is_modified() && errors.is_empty();
    let invalid_fields = errors.iter()
      .filter_map(|error| error.field_index)
      .collect::<BTreeSet<_>>();

    graphics::rectangle([0.06, 0.07, 0.09, 0.97], layout.panel, ctx.transform, gl);
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [layout.panel[0] + 12.0, layout.panel[1] + 24.0],
      &format!(
        "PROVINCE {} â€” STATE {} â€” EDIT DATA",
        draft.province_id,
        draft.state_id
      ),
    );
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WARNING,
      [layout.panel[0] + 12.0, layout.panel[1] + 44.0],
      "In-memory province draft â€” no files will be written",
    );

    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [layout.panel[0] + 12.0, layout.victory_point_field()[1] + 17.0],
      "Victory point value",
    );
    let vp_field = layout.victory_point_field();
    graphics::rectangle(
      if invalid_fields.contains(&0) {
        [0.38, 0.08, 0.08, 1.0]
      } else if self.property_editor_field == 0 {
        [0.12, 0.25, 0.42, 1.0]
      } else {
        [0.14, 0.15, 0.18, 1.0]
      },
      vp_field,
      ctx.transform,
      gl,
    );
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      if draft.victory_point.is_some() { colors::WHITE } else { colors::WHITE_T },
      [vp_field[0] + 6.0, vp_field[1] + 17.0],
      draft.victory_point.as_deref().unwrap_or("No victory point"),
    );
    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.victory_point_toggle(),
      if draft.victory_point.is_some() {
        "Remove victory point"
      } else {
        "Add victory point"
      },
      true,
    );

    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      colors::WHITE,
      [layout.panel[0] + 12.0, layout.buildings_title_y],
      "Province Buildings",
    );
    for row in layout.visible_range() {
      let Some(building) = draft.buildings.get(row) else { continue };
      let name_field = 1 + row * 2;
      let value_field = name_field + 1;
      let name_rect = layout.building_name(row);
      let value_rect = layout.building_value(row);
      graphics::rectangle(
        editor_field_color(
          self.property_editor_field == name_field,
          invalid_fields.contains(&name_field),
        ),
        name_rect,
        ctx.transform,
        gl,
      );
      graphics::rectangle(
        editor_field_color(
          self.property_editor_field == value_field,
          invalid_fields.contains(&value_field),
        ),
        value_rect,
        ctx.transform,
        gl,
      );
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::WHITE,
        [name_rect[0] + 6.0, name_rect[1] + 17.0],
        &fit_editor_text(&building.name, name_rect[2] - 12.0),
      );
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::WHITE,
        [value_rect[0] + 6.0, value_rect[1] + 17.0],
        &fit_editor_text(&building.value, value_rect[2] - 12.0),
      );
      draw_editor_button(
        ctx,
        glyph_cache,
        gl,
        layout.building_remove(row),
        "Remove",
        true,
      );
    }
    if draft.buildings.is_empty() {
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::WHITE_T,
        [layout.panel[0] + 12.0, layout.buildings_y + 18.0],
        "No province buildings",
      );
    }

    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.add_building(),
      "Add provincial building",
      true,
    );
    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.previous_page(),
      "Previous",
      self.province_editor_page > 0,
    );
    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.next_page(),
      "Next",
      layout.has_next_page(self.province_editor_page),
    );
    draw_editor_button(ctx, glyph_cache, gl, layout.apply(), "Apply to session", can_apply);
    draw_editor_button(
      ctx,
      glyph_cache,
      gl,
      layout.discard(),
      "Discard province draft",
      true,
    );

    let status = if !draft.is_modified() {
      "No province draft changes".to_owned()
    } else if errors.is_empty() {
      "Province draft modified â€” ready to apply".to_owned()
    } else {
      format!("Province draft validation errors: {}", errors.len())
    };
    draw_canvas_text(
      ctx,
      glyph_cache,
      gl,
      if errors.is_empty() { colors::WHITE } else { colors::PROBLEM },
      [layout.panel[0] + 12.0, layout.error_y],
      &status,
    );
    for (index, error) in errors.iter().take(layout.visible_error_lines).enumerate() {
      let message = fit_editor_text(
        &format!("{}: {}", error.field, error.message),
        layout.panel[2] - 24.0,
      );
      draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        colors::PROBLEM,
        [
          layout.panel[0] + 12.0,
          layout.error_y + 19.0 + index as f64 * 18.0,
        ],
        &message,
      );
    }
  }

  fn draw_problems(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
    let extras = self.bundle.config.extra_warnings.enabled;
    for problem in self.problems.iter() {
      problem.draw(ctx, extras, CameraCombo { camera: &self.camera, interface }, gl);
    };
  }

  fn draw_tool(&self, ctx: Context, interface: &Interface, cursor_pos: Option<Vector2<f64>>, gl: &mut GlGraphics) {
    if self.map_view_mode == MapViewMode::States {
      return;
    }

    let color = if self.tool.color_brush.is_some() { colors::WHITE } else { colors::WHITE_T };
    match (self.view_mode, &self.tool.mode, cursor_pos) {
      (ViewMode::Color, ToolMode::PaintArea, Some(cursor_pos)) => {
        let ellipse = Ellipse::new_border(color, 0.5).resolution(16);
        let r = self.tool.radius * self.camera.scale_factor();
        let transform = ctx.transform.trans_pos(cursor_pos);
        ellipse.draw_from_to([r, r], [-r, -r], &Default::default(), transform, gl);
      },
      (ViewMode::Color, ToolMode::Lasso(lasso), cursor_pos) => {
        let can_finish = cursor_pos
          .map(|cursor_pos| lasso.can_finish(interface, &self.camera, cursor_pos))
          .unwrap_or(false);
        let points = lasso.iter()
          .map(|pos| self.camera.compute_position(interface, pos))
          .collect::<Vec<Vector2<f64>>>();
        let first_point = points.first().cloned();
        let last_point = if can_finish { first_point } else { cursor_pos };

        if let (true, Some(first_point)) = (can_finish, first_point) {
          let ellipse = Ellipse::new(color).resolution(6);
          let transform = ctx.transform.trans_pos(first_point);
          ellipse.draw_from_to([5.0, 5.0], [-5.0, -5.0], &Default::default(), transform, gl);
        };

        let lines = points.into_iter()
          .chain(last_point.into_iter())
          .tuple_windows::<(_, _)>();
        for (pos1, pos2) in lines {
          graphics::line_from_to(color, 0.5, pos1, pos2, ctx.transform, gl);
        };
      },
      _ => ()
    };
  }

  pub fn toggle_province_ids(&mut self) {
    self.show_province_ids = !self.show_province_ids;
  }

  pub fn toggle_province_boundaries(&mut self) {
    self.show_province_boundaries = !self.show_province_boundaries;
  }

  pub fn toggle_river_overlay(&mut self) -> bool {
    if !self.show_river_overlay && self.bundle.map.get_rivers_overlay().is_none() {
      return true;
    };

    self.show_river_overlay = !self.show_river_overlay;
    self.texture_overlay = None;

    false
  }

  pub fn enabled_options(&self) -> [bool; 3] {
    [
      self.show_province_ids,
      self.show_province_boundaries,
      self.show_river_overlay
    ]
  }

  pub fn toggle_lasso_snap(&mut self) {
    self.tool.lasso_snap = !self.tool.lasso_snap;
  }

  pub fn reload_config(&mut self, alerts: &mut Alerts) {
    match Config::load() {
      Ok(config) => {
        self.bundle.config = config;
        alerts.push(Ok("Reloaded config"));
      },
      Err(err) => alerts.push(Err(format!("Error: {}", err)))
    };
  }

  pub fn export_land_map<P: AsRef<Path>>(&self, path: P, alerts: &mut Alerts) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      alerts.push(Err("Export is unavailable for read-only state projects"));
      return;
    }

    if let Some(image) = self.bundle.image_buffer_mapgen_land() {
      let path = path.as_ref();
      match export_image_buffer(path, image) {
        Ok(()) => alerts.push(Ok(format!("Exported land map to {}", path.display()))),
        Err(err) => alerts.push(Err(format!("Error: {}", err)))
      };
    } else {
      alerts.push(Err("Error: province with unknown type present"));
    };
  }

  pub fn export_terrain_map<P: AsRef<Path>>(&self, path: P, alerts: &mut Alerts) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      alerts.push(Err("Export is unavailable for read-only state projects"));
      return;
    }

    if let Some(unknown_terrains) = self.unknown_terrains() {
      alerts.push(Err(unknown_terrains));
    } else {
      let path = path.as_ref();
      let image = self.bundle.image_buffer_mapgen_terrain().unwrap();
      match export_image_buffer(path, image) {
        Ok(()) => alerts.push(Ok(format!("Exported terrain map to {}", path.display()))),
        Err(err) => alerts.push(Err(format!("Error: {}", err)))
      };
    };
  }

  pub fn undo(&mut self) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      if self.property_draft_is_modified() {
        return;
      }
      self.discard_unmodified_property_draft();
      self.cancel_state_lasso();
      self.cancel_state_brush();
      if let Some(edit) = self.state_edit_session.as_mut()
        && edit.undo()
      {
        self.repair_active_state_after_history();
        self.refresh_state_visuals();
      }
      return;
    };

    if let Some(commit) = self.history.undo(&mut self.bundle.map) {
      self.bundle.map.recalculate_all_boundaries();
      self.problems.clear();
      if self.bundle.config.change_view_mode_on_undo {
        self.view_mode = commit.view_mode;
      };
      self.refresh();
    };
  }

  pub fn redo(&mut self) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      if self.property_draft_is_modified() {
        return;
      }
      self.discard_unmodified_property_draft();
      self.cancel_state_lasso();
      self.cancel_state_brush();
      if let Some(edit) = self.state_edit_session.as_mut()
        && edit.redo()
      {
        self.repair_active_state_after_history();
        self.refresh_state_visuals();
      }
      return;
    };

    if let Some(commit) = self.history.redo(&mut self.bundle.map) {
      self.bundle.map.recalculate_all_boundaries();
      self.problems.clear();
      if self.bundle.config.change_view_mode_on_undo {
        self.view_mode = commit.view_mode;
      };
      self.refresh();
    };
  }

  pub fn calculate_coastal_provinces(&mut self) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      return;
    };

    self.history.calculate_coastal_provinces(&mut self.bundle);
    self.view_mode = ViewMode::Coastal;
    self.refresh();
  }

  pub fn calculate_recolor_map(&mut self) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      return;
    };

    self.history.calculate_recolor_map(&mut self.bundle);
    self.view_mode = ViewMode::Color;
    self.tool.color_brush = None;
    self.refresh();
  }

  pub fn display_problems(&mut self, alerts: &mut Alerts) {
    self.problems = self.bundle.generate_problems();
    if self.problems.is_empty() {
      alerts.push(Ok("No map problems detected"));
    } else {
      for problem in self.problems.iter() {
        alerts.push(Ok(format!("Problem: {}", problem)));
      };
    };
  }

  pub fn set_view_mode(&mut self, alerts: &mut Alerts, view_mode: ViewMode) {
    self.cancel_state_lasso();
    self.map_view_mode = MapViewMode::Provinces;
    if let (ViewMode::Terrain, Some(unknown_terrains)) = (view_mode, self.unknown_terrains()) {
      alerts.push(Err(unknown_terrains));
    } else if view_mode != self.view_mode {
      if let ViewMode::Color | ViewMode::Adjacencies = self.view_mode {
        self.cancel_tool();
      };

      self.view_mode = view_mode;
      self.refresh();
    };
  }

  pub fn set_map_view_mode(&mut self, alerts: &mut Alerts, map_view_mode: MapViewMode) {
    if map_view_mode == MapViewMode::States && self.state_texture.is_none() {
      alerts.push(Err("State view is available only for loaded state projects"));
    } else if map_view_mode != self.map_view_mode {
      self.cancel_state_lasso();
      self.deactivate_state_brush();
      self.map_view_mode = map_view_mode;
      let message = match map_view_mode {
        MapViewMode::Provinces => "Province map view",
        MapViewMode::States => "State map view"
      };
      alerts.push(Ok(message));
    };
  }

  pub fn state_lasso_is_active(&self) -> bool {
    !matches!(self.state_lasso_phase, StateLassoPhase::Inactive)
  }

  pub fn state_brush_is_active(&self) -> bool {
    !matches!(self.state_brush_phase, StateBrushPhase::Inactive)
  }

  pub fn state_brush_is_stroking(&self) -> bool {
    matches!(self.state_brush_phase, StateBrushPhase::Stroking(_))
  }

  pub fn state_action_availability(&self) -> StateActionAvailability {
    let state_view = self.map_view_mode == MapViewMode::States;
    let lasso_active = self.state_lasso_is_active();
    let lasso_preview = matches!(self.state_lasso_phase, StateLassoPhase::Preview { .. });
    let brush_active = self.state_brush_is_active();
    let Some(edit) = self.state_edit_session.as_ref() else {
      return StateActionAvailability::default();
    };
    let draft_modified = self.property_draft_is_modified();
    StateActionAvailability {
      state_view,
      lasso_active,
      lasso_preview,
      brush_active,
      has_selection: !edit.selected_provinces().is_empty(),
      has_target: edit.target_state_id().is_some_and(|state_id| edit.is_state_active(state_id)),
      can_move: !lasso_active && !brush_active && edit.can_move_selection_to_target(),
      can_unassign: !lasso_active && !brush_active && edit.can_unassign_selection(),
      can_edit_properties: !lasso_active && !brush_active
        && self.active_state_id.is_some_and(|state_id| edit.is_state_active(state_id)),
      can_edit_province_data: !lasso_active && !brush_active
        && self.active_province_id
          .is_some_and(|province_id| edit.editable_province_state(province_id).is_ok()),
      can_create_state: !lasso_active && !brush_active,
      can_remove_state: !lasso_active && !brush_active
        && self.active_state_id
          .is_some_and(|state_id| edit.validate_removable_state(state_id).is_ok()),
      property_editor_open: self.property_editor_is_open(),
      property_draft_modified: draft_modified,
      can_undo: !draft_modified && edit.can_undo(),
      can_redo: !draft_modified && edit.can_redo(),
      has_edits: edit.is_dirty() || draft_modified,
    }
  }

  pub fn activate_state_lasso(&mut self, mode_override: Option<LassoSelectionMode>, alerts: &mut Alerts) {
    if self.property_draft_is_modified() {
      alerts.push(Err("Apply or discard the modified draft before starting a lasso"));
      return;
    }
    self.discard_unmodified_property_draft();
    if self.map_view_mode != MapViewMode::States || self.state_edit_session.is_none() {
      alerts.push(Err("State lasso is available only in the state map view"));
      return;
    }
    self.deactivate_state_brush();
    if let Some(mode) = mode_override {
      self.state_lasso_mode = mode;
    }
    self.state_lasso_phase = StateLassoPhase::Drawing {
      points: Vec::new(),
      mode: self.state_lasso_mode,
      inclusion: self.state_lasso_inclusion,
    };
    self.clear_state_lasso_preview_visuals();
    self.refresh_state_information();
    alerts.push(Ok(format!(
      "State lasso started: {} / {}",
      self.state_lasso_mode.label(),
      self.state_lasso_inclusion.label()
    )));
  }

  pub fn set_state_lasso_mode(&mut self, mode: LassoSelectionMode, alerts: &mut Alerts) {
    self.state_lasso_mode = mode;
    match &mut self.state_lasso_phase {
      StateLassoPhase::Drawing { mode: active, .. }
      | StateLassoPhase::Preview { mode: active, .. } => *active = mode,
      StateLassoPhase::Inactive => (),
    }
    self.refresh_state_information();
    alerts.push(Ok(format!("State lasso selection mode: {}", mode.label())));
  }

  pub fn set_state_lasso_inclusion(&mut self, inclusion: ProvinceInclusionMode, alerts: &mut Alerts) {
    self.state_lasso_inclusion = inclusion;
    let preview_points = match &mut self.state_lasso_phase {
      StateLassoPhase::Drawing { inclusion: active, .. } => {
        *active = inclusion;
        None
      },
      StateLassoPhase::Preview { points, .. } => Some(points.clone()),
      StateLassoPhase::Inactive => None,
    };
    if let Some(points) = preview_points {
      self.build_state_lasso_preview(points, alerts);
    } else {
      self.refresh_state_information();
      alerts.push(Ok(format!("State lasso inclusion: {}", inclusion.label())));
    }
  }

  pub fn state_lasso_add_point(
    &mut self,
    interface: &Interface,
    cursor_pos: Vector2<f64>,
    mode_override: Option<LassoSelectionMode>,
    alerts: &mut Alerts
  ) {
    if !matches!(self.state_lasso_phase, StateLassoPhase::Drawing { .. }) {
      self.activate_state_lasso(mode_override, alerts);
    }
    let can_finish = match &self.state_lasso_phase {
      StateLassoPhase::Drawing { points, .. } => points.first()
        .map(|first| self.camera.compute_position(interface, *first))
        .is_some_and(|first| {
          points.len() >= 3
            && vecmath::vec2_len(vecmath::vec2_sub(first, cursor_pos)) < 5.0
        }),
      _ => false,
    };
    if can_finish {
      self.finish_state_lasso_drawing(alerts);
      return;
    }

    let mut point = self.camera.relative_position(interface, cursor_pos);
    if self.tool.lasso_snap {
      point = [point[0].round(), point[1].round()];
    }
    if let StateLassoPhase::Drawing { points, mode, .. } = &mut self.state_lasso_phase {
      if points.is_empty()
        && let Some(mode_override) = mode_override
      {
        *mode = mode_override;
        self.state_lasso_mode = mode_override;
      }
      points.push(point);
    }
    self.refresh_state_information();
  }

  pub fn advance_state_lasso(&mut self, alerts: &mut Alerts) -> bool {
    match self.state_lasso_phase {
      StateLassoPhase::Drawing { .. } => {
        self.finish_state_lasso_drawing(alerts);
        true
      },
      StateLassoPhase::Preview { .. } => {
        self.confirm_state_lasso(alerts);
        true
      },
      StateLassoPhase::Inactive => false,
    }
  }

  pub fn cancel_state_lasso(&mut self) -> bool {
    if matches!(self.state_lasso_phase, StateLassoPhase::Inactive) {
      return false;
    }
    self.state_lasso_phase = StateLassoPhase::Inactive;
    self.clear_state_lasso_preview_visuals();
    self.refresh_state_information();
    true
  }

  pub fn confirm_state_lasso(&mut self, alerts: &mut Alerts) {
    let StateLassoPhase::Preview { candidates, mode, .. } = &self.state_lasso_phase else {
      alerts.push(Err("No state lasso preview to confirm"));
      return;
    };
    let province_ids = candidates.selectable.clone();
    let mode = *mode;
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        edit.apply_lasso_selection(&province_ids, mode)
          .map_err(|error| error.to_string())
      });
    match result {
      Ok(count) => {
        self.state_lasso_phase = StateLassoPhase::Inactive;
        self.clear_state_lasso_preview_visuals();
        self.refresh_selected_province_boundaries();
        self.refresh_state_information();
        alerts.push(Ok(format!(
          "{} state lasso selection confirmed: {count} provinces selected",
          mode.label()
        )));
      },
      Err(error) => alerts.push(Err(error)),
    }
  }

  fn finish_state_lasso_drawing(&mut self, alerts: &mut Alerts) {
    let points = match &self.state_lasso_phase {
      StateLassoPhase::Drawing { points, .. } => points.clone(),
      _ => return,
    };
    self.build_state_lasso_preview(points, alerts);
  }

  fn build_state_lasso_preview(&mut self, points: Vec<Vector2<f64>>, alerts: &mut Alerts) {
    let Some(project) = self.project.as_ref() else { return };
    let Some(edit) = self.state_edit_session.as_ref() else { return };
    let ambiguous = project.ambiguous_provinces.keys().copied().collect::<BTreeSet<_>>();
    let result = classify_state_lasso(
      &self.bundle.map,
      &points,
      self.state_lasso_inclusion,
      edit.selected_provinces(),
      edit.state_by_province(),
      &ambiguous,
      edit.valid_state_ids(),
    );
    match result {
      Ok(candidates) => {
        let preview_ms = candidates.computed_in.as_millis();
        let selectable = candidates.selectable.clone();
        let blocked = candidates.blocked.clone();
        self.lasso_preview_boundaries = self.boundaries_for_provinces(&selectable);
        self.lasso_blocked_boundaries = self.boundaries_for_provinces(&blocked);
        self.state_lasso_phase = StateLassoPhase::Preview {
          points,
          candidates,
          mode: self.state_lasso_mode,
          inclusion: self.state_lasso_inclusion,
        };
        self.refresh_state_information();
        let StateLassoPhase::Preview { candidates, .. } = &self.state_lasso_phase else {
          unreachable!()
        };
        println!(
          "State lasso preview: {} selectable, {} blocked, {} ignored non-land, {} pixels scanned in {} ms.",
          candidates.selectable.len(),
          candidates.blocked.len(),
          candidates.ignored_non_land,
          candidates.scanned_pixels,
          preview_ms
        );
        alerts.push(Ok(format!(
          "Lasso preview: {} selectable, {} blocked, {} ignored",
          candidates.selectable.len(),
          candidates.blocked.len(),
          candidates.ignored_non_land
        )));
      },
      Err(error) => alerts.push(Err(error.to_string())),
    }
  }

  fn clear_state_lasso_preview_visuals(&mut self) {
    self.lasso_preview_boundaries.clear();
    self.lasso_blocked_boundaries.clear();
  }

  pub fn activate_state_brush(&mut self, mode: StateBrushMode, alerts: &mut Alerts) {
    if self.property_draft_is_modified() {
      alerts.push(Err("Apply or discard the modified draft before starting the State Brush"));
      return;
    }
    self.discard_unmodified_property_draft();
    let Some(edit) = self.state_edit_session.as_ref() else {
      alerts.push(Err("State Brush is available only for loaded state projects"));
      return;
    };
    if self.map_view_mode != MapViewMode::States {
      alerts.push(Err("State Brush is available only in the state map view"));
      return;
    }
    if edit.validate_brush_target(mode, edit.target_state_id()).is_err() {
      alerts.push(Err("Select a valid target state before using the State Brush"));
      return;
    }
    self.cancel_state_lasso();
    self.clear_state_brush_preview();
    self.state_brush_mode = mode;
    self.state_brush_phase = StateBrushPhase::Ready;
    self.refresh_state_information();
    alerts.push(Ok(format!("State Brush ready: {}", mode.label())));
  }

  pub fn begin_state_brush(
    &mut self,
    interface: &Interface,
    cursor_pos: Vector2<f64>,
    alerts: &mut Alerts,
  ) -> bool {
    if !matches!(self.state_brush_phase, StateBrushPhase::Ready) {
      return false;
    }
    if self.property_draft_is_modified() || self.state_lasso_is_active() {
      alerts.push(Err("Resolve the active draft or lasso before using the State Brush"));
      return false;
    }
    let Some(edit) = self.state_edit_session.as_ref() else { return false };
    let target_state_id = match edit.validate_brush_target(
      self.state_brush_mode,
      edit.target_state_id(),
    ) {
      Ok(target_state_id) => target_state_id,
      Err(_) => {
        alerts.push(Err("Select a valid target state before using the State Brush"));
        return false;
      },
    };
    let map_position = self.camera.relative_position(interface, cursor_pos);
    if !self.camera.within_dimensions(map_position) {
      return false;
    }
    let mut stroke = StateBrushStroke {
      mode: self.state_brush_mode,
      target_state_id,
      visited_provinces: BTreeSet::new(),
      selectable_provinces: BTreeSet::new(),
      no_op_provinces: BTreeSet::new(),
      blocked_ambiguous: BTreeSet::new(),
      blocked_invalid_state: BTreeSet::new(),
      ignored_non_land: BTreeSet::new(),
      encountered_unknown: false,
      previous_map_position: map_position,
      last_editable_province: None,
      input_events: 1,
      sampled_points: 0,
      started: Instant::now(),
    };
    self.collect_state_brush_segment(&mut stroke, map_position);
    self.state_brush_phase = StateBrushPhase::Stroking(Box::new(stroke));
    self.refresh_state_brush_preview();
    true
  }

  pub fn update_state_brush(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
    let map_position = self.camera.relative_position(interface, cursor_pos);
    if !self.camera.within_dimensions(map_position) {
      return;
    }
    let StateBrushPhase::Stroking(mut stroke) =
      std::mem::take(&mut self.state_brush_phase)
    else {
      return;
    };
    stroke.input_events += 1;
    let changed = self.collect_state_brush_segment(&mut stroke, map_position);
    stroke.previous_map_position = map_position;
    self.state_brush_phase = StateBrushPhase::Stroking(stroke);
    if changed {
      self.refresh_state_brush_preview();
    }
  }

  pub fn finish_state_brush(&mut self, alerts: &mut Alerts) {
    let StateBrushPhase::Stroking(stroke) =
      std::mem::take(&mut self.state_brush_phase)
    else {
      return;
    };
    self.state_brush_phase = StateBrushPhase::Ready;
    self.clear_state_brush_preview();

    let changed = stroke.selectable_provinces.len();
    let blocked = stroke.blocked_ambiguous.len() + stroke.blocked_invalid_state.len();
    let ignored = stroke.ignored_non_land.len() + usize::from(stroke.encountered_unknown);
    let collection_ms = stroke.started.elapsed().as_millis();
    let province_ids = stroke.selectable_provinces.iter().copied().collect::<Vec<_>>();
    if province_ids.is_empty() {
      let message = format!(
        "Stroke contained no editable provinces (no-op {}, blocked {}, ignored {}).",
        stroke.no_op_provinces.len(),
        blocked,
        ignored,
      );
      self.last_state_brush_result = Some(message.clone());
      self.refresh_state_information();
      alerts.push(Err(message));
      return;
    }

    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| {
        edit.reassign_provinces(&province_ids, stroke.target_state_id)
          .map_err(|error| error.to_string())
      });
    match result {
      Ok(()) => {
        self.active_province_id = stroke.last_editable_province;
        self.refresh_state_visuals();
        let action = match stroke.mode {
          StateBrushMode::AssignToTarget => format!(
            "assigned {changed} provinces to State {}",
            stroke.target_state_id.unwrap_or_default(),
          ),
          StateBrushMode::Unassign => format!("unassigned {changed} provinces"),
        };
        let timings = self.state_edit_session
          .as_ref()
          .map(StateEditSession::last_timings)
          .unwrap_or_default();
        let message = format!(
          "State Brush {action}; {} events, {} samples, {} no-op, {blocked} blocked, \
           {ignored} ignored; collection {collection_ms} ms, preflight {} us, apply {} us, \
           visual {} {} ms.",
          stroke.input_events,
          stroke.sampled_points,
          stroke.no_op_provinces.len(),
          timings.command_preflight.as_micros(),
          timings.command_apply.as_micros(),
          self.last_state_visual_update_kind,
          self.last_state_visual_update_ms,
        );
        println!("{message}");
        self.last_state_brush_result = Some(message.clone());
        self.refresh_state_information();
        alerts.push(Ok(message));
      },
      Err(error) => {
        let message = format!("Cannot apply State Brush: {error}");
        self.last_state_brush_result = Some(message.clone());
        self.refresh_state_information();
        alerts.push(Err(message));
      },
    }
  }

  pub fn cancel_state_brush(&mut self) -> bool {
    match std::mem::take(&mut self.state_brush_phase) {
      StateBrushPhase::Inactive => return false,
      StateBrushPhase::Ready => self.state_brush_phase = StateBrushPhase::Inactive,
      StateBrushPhase::Stroking(_) => self.state_brush_phase = StateBrushPhase::Ready,
    }
    self.clear_state_brush_preview();
    self.refresh_state_information();
    true
  }

  fn deactivate_state_brush(&mut self) {
    self.state_brush_phase = StateBrushPhase::Inactive;
    self.clear_state_brush_preview();
  }

  fn collect_state_brush_segment(
    &self,
    stroke: &mut StateBrushStroke,
    current_map_position: Vector2<f64>,
  ) -> bool {
    let Some(edit) = self.state_edit_session.as_ref() else { return false };
    let mut changed = false;
    for position in sample_segment(
      stroke.previous_map_position,
      current_map_position,
      1.0,
      self.bundle.map.dimensions(),
    ) {
      stroke.sampled_points += 1;
      let province = self.bundle.map.get_province_at(position);
      let Some(province_id) = province.preserved_id else {
        stroke.encountered_unknown = true;
        continue;
      };
      if !stroke.visited_provinces.insert(province_id) {
        continue;
      }
      changed = true;
      match edit.classify_brush_province(province_id, stroke.mode, stroke.target_state_id) {
        BrushProvinceClassification::Selectable => {
          stroke.selectable_provinces.insert(province_id);
          stroke.last_editable_province = Some(province_id);
        },
        BrushProvinceClassification::NoOp => {
          stroke.no_op_provinces.insert(province_id);
          stroke.last_editable_province = Some(province_id);
        },
        BrushProvinceClassification::IgnoredNonLand => {
          stroke.ignored_non_land.insert(province_id);
        },
        BrushProvinceClassification::BlockedAmbiguous => {
          stroke.blocked_ambiguous.insert(province_id);
        },
        BrushProvinceClassification::BlockedInvalidState => {
          stroke.blocked_invalid_state.insert(province_id);
        },
        BrushProvinceClassification::Unknown => stroke.encountered_unknown = true,
      }
    }
    changed
  }

  fn refresh_state_brush_preview(&mut self) {
    let StateBrushPhase::Stroking(stroke) = &self.state_brush_phase else { return };
    let selectable = stroke.selectable_provinces.clone();
    let blocked = stroke.blocked_ambiguous
      .union(&stroke.blocked_invalid_state)
      .copied()
      .collect();
    self.brush_preview_boundaries = self.boundaries_for_provinces(&selectable);
    self.brush_blocked_boundaries = self.boundaries_for_provinces(&blocked);
    self.refresh_state_information();
  }

  fn clear_state_brush_preview(&mut self) {
    self.brush_preview_boundaries.clear();
    self.brush_blocked_boundaries.clear();
  }

  pub fn select_state_at(
    &mut self,
    interface: &Interface,
    cursor_pos: Vector2<f64>,
    toggle_province: bool,
    alerts: &mut Alerts
  ) {
    if self.state_click_would_change_property_draft(interface, cursor_pos) {
      alerts.push(Err(if self.province_data_draft.is_some() {
        "This province has unapplied form changes"
      } else {
        "This state has unapplied form changes"
      }));
      return;
    }
    let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) else {
      self.clear_state_selection();
      return;
    };
    self.active_province_id = self.bundle.map.get_province_at(pos).preserved_id;
    let Some(project) = self.project.as_ref() else {
      return;
    };
    if toggle_province {
      self.toggle_edit_province_at(pos, alerts);
      return;
    }
    let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps() else {
      return;
    };
    let selection = resolve_state_at_for(
      &self.bundle.map,
      state_by_province,
      &project.ambiguous_provinces,
      unassigned_land_provinces,
      pos
    );
    let message = selection_message(project, self.state_edit_session.as_ref(), selection.as_ref());
    let selection_image = match selection.as_ref() {
      Some(StateSelection::State { .. }) => {
        Some(selection_overlay_for(
          &self.bundle.map,
          state_by_province,
          &project.ambiguous_provinces.keys().copied().collect(),
          selection.as_ref()
        ))
      },
      _ => None
    };
    let selected_state_boundaries = match selection.as_ref() {
      Some(StateSelection::State { state_id, .. }) => boundaries_for_state(
        &self.bundle.map,
        state_by_province,
        &self.state_boundaries,
        *state_id
      ),
      _ => Vec::new()
    };

    self.selection_texture = selection_image.map(|image| {
      let settings = TextureSettings::new().mag(Filter::Nearest);
      Texture::from_image(&image, &settings)
    });
    self.selected_state_boundaries = selected_state_boundaries;
    self.state_selection = selection;
    self.active_state_id = match self.state_selection.as_ref() {
      Some(StateSelection::State { state_id, .. }) => Some(*state_id),
      _ => None,
    };
    if let Some(edit) = self.state_edit_session.as_mut()
      && edit.set_target_state(self.active_state_id).is_err()
    {
      edit.set_target_state(None).ok();
      self.active_state_id = None;
    }
    self.refresh_state_information();
    if let Some(message) = message {
      println!("{message}");
      alerts.push(Ok(message));
    };
  }

  pub fn clear_state_selection(&mut self) -> bool {
    if self.property_draft_is_modified() {
      return false;
    }
    self.deactivate_state_brush();
    self.discard_unmodified_property_draft();
    if let Some(edit) = self.state_edit_session.as_mut()
      && edit.clear_selected_provinces()
    {
      self.selected_province_boundaries.clear();
      self.refresh_state_information();
      return true;
    }
    let had_selection = self.state_selection.is_some();
    self.state_selection = None;
    self.active_state_id = None;
    self.active_province_id = None;
    self.selection_texture = None;
    self.selected_state_boundaries.clear();
    if let Some(edit) = self.state_edit_session.as_mut() {
      edit.clear_target_state();
    }
    self.refresh_state_information();
    had_selection
  }

  pub fn move_selected_provinces_to_target(&mut self, alerts: &mut Alerts) {
    if self.property_draft_is_modified() {
      return;
    }
    self.discard_unmodified_property_draft();
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso preview before moving provinces"));
      return;
    }
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| edit.move_selection_to_target().map_err(|err| err.to_string()));
    self.after_state_edit_command(result, "Moved selected provinces in memory", alerts);
  }

  pub fn move_confirmation_message(&self) -> Option<String> {
    let edit = self.state_edit_session.as_ref()?;
    let count = edit.selected_provinces().len();
    let target_state_id = edit.target_state_id()?;
    (count > 1).then(|| {
      let target_name = edit.state_data(target_state_id)
        .and_then(|data| data.name)
        .unwrap_or_else(|| "<unnamed>".to_owned());
      format!(
        "Move {count} selected provinces to State {target_state_id} — {target_name}?\n\n{}",
        selection_sources_message(edit)
      )
    })
  }

  pub fn select_target_state_provinces(&mut self, alerts: &mut Alerts) {
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| edit.select_target_state_provinces().map_err(|err| err.to_string()));
    match result {
      Ok(count) => {
        self.refresh_selected_province_boundaries();
        self.refresh_state_information();
        alerts.push(Ok(format!("Selected {count} provinces from the target state")));
      },
      Err(err) => alerts.push(Err(err))
    }
  }

  pub fn unassign_selected_provinces(&mut self, alerts: &mut Alerts) {
    if self.property_draft_is_modified() {
      return;
    }
    self.discard_unmodified_property_draft();
    if self.state_lasso_is_active() {
      alerts.push(Err("Confirm or cancel the state lasso preview before unassigning provinces"));
      return;
    }
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| edit.unassign_selection().map_err(|err| err.to_string()));
    self.after_state_edit_command(result, "Unassigned selected provinces in memory", alerts);
  }

  pub fn unassign_confirmation_message(&self) -> Option<String> {
    let edit = self.state_edit_session.as_ref()?;
    let count = edit.selected_provinces().len();
    (count > 1).then(|| format!(
      "Unassign {count} selected provinces?\n\n{}\n\n\
       This will temporarily create {count} unassigned land provinces.\n\
       No files will be written.",
      selection_sources_message(edit)
    ))
  }

  pub fn discard_state_edit_session(&mut self, alerts: &mut Alerts) {
    self.state_lifecycle_draft = None;
    self.state_property_draft = None;
    self.province_data_draft = None;
    self.property_editor_replace_field = false;
    self.province_editor_page = 0;
    if let Some(edit) = self.state_edit_session.as_mut() {
      edit.discard();
      self.state_lasso_phase = StateLassoPhase::Inactive;
      self.clear_state_lasso_preview_visuals();
      self.deactivate_state_brush();
      self.last_state_brush_result = None;
      self.state_selection = None;
      self.active_state_id = None;
      self.active_province_id = None;
      self.selection_texture = None;
      self.selected_state_boundaries.clear();
      self.selected_province_boundaries.clear();
      self.refresh_state_visuals();
      alerts.push(Ok("Discarded all in-memory state edits"));
    }
  }

  fn toggle_edit_province_at(&mut self, pos: Vector2<u32>, alerts: &mut Alerts) {
    let province = self.bundle.map.get_province_at(pos);
    let Some(province_id) = province.preserved_id else {
      alerts.push(Err("No province ID at cursor"));
      return;
    };
    self.active_province_id = Some(province_id);
    self.active_state_id = self.state_edit_session
      .as_ref()
      .and_then(|edit| edit.province_state_id(province_id));
    let result = self.state_edit_session
      .as_mut()
      .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
      .and_then(|edit| edit.toggle_selected_province(province_id).map_err(|err| err.to_string()));
    match result {
      Ok(selected) => {
        self.refresh_selected_province_boundaries();
        self.refresh_state_information();
        let action = if selected { "Selected" } else { "Deselected" };
        alerts.push(Ok(format!("{action} province {province_id} for state editing")));
      },
      Err(err) => alerts.push(Err(err))
    }
  }

  fn after_state_edit_command(
    &mut self,
    result: Result<(), String>,
    success: &str,
    alerts: &mut Alerts
  ) {
    match result {
      Ok(()) => {
        self.refresh_state_visuals();
        alerts.push(Ok(success));
      },
      Err(err) => alerts.push(Err(err))
    }
  }

  fn refresh_state_visuals(&mut self) {
    let changed = self.state_edit_session
      .as_mut()
      .map(StateEditSession::take_last_changed_provinces)
      .unwrap_or_default();
    if changed.is_empty() {
      self.refresh_state_target_overlay();
      self.refresh_selected_province_boundaries();
      self.refresh_state_information();
      return;
    }

    if let Some(extents) = self.selective_state_update_extents(&changed) {
      let (_, [width, height]) = extents.to_offset_size();
      let area = width as u64 * height as u64;
      let map_area = self.bundle.map.width() as u64 * self.bundle.map.height() as u64;
      if changed.len() <= 128 && area.saturating_mul(4) <= map_area {
        self.refresh_state_visuals_selective(&changed, extents);
        return;
      }
    }
    self.refresh_state_visuals_full(changed.len());
  }

  fn repair_active_state_after_history(&mut self) {
    let Some(edit) = self.state_edit_session.as_ref() else { return };
    let brush_target_is_invalid = self.state_brush_mode == StateBrushMode::AssignToTarget
      && self.state_brush_is_active()
      && !edit.target_state_id().is_some_and(|state_id| edit.is_state_active(state_id));
    if self.active_state_id.is_some_and(|state_id| !edit.is_state_active(state_id)) {
      self.active_state_id = edit.target_state_id();
    }
    if matches!(
      self.state_selection,
      Some(StateSelection::State { state_id, .. }) if !edit.is_state_active(state_id)
    ) {
      self.state_selection = None;
      self.selection_texture = None;
      self.selected_state_boundaries.clear();
    }
    if brush_target_is_invalid {
      self.deactivate_state_brush();
    }
  }

  fn refresh_state_visuals_full(&mut self, changed_count: usize) {
    let Some(project) = self.project.as_ref() else { return };
    let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps() else {
      return;
    };
    let state_view = generate_state_view_for(
      &self.bundle.map,
      state_by_province,
      &project.ambiguous_provinces.keys().copied().collect(),
      unassigned_land_provinces
    );
    let texture_time = state_view.generated_in;
    let boundary_time = state_view.boundary_scan_in;
    self.last_state_visual_update_ms = texture_time.as_millis() + boundary_time.as_millis();
    self.last_state_visual_update_kind = "full";
    let settings = TextureSettings::new().mag(Filter::Nearest);
    self.state_texture = Some(Texture::from_image(&state_view.image, &settings));
    self.state_boundaries = state_view.state_boundaries;
    if let Some(edit) = self.state_edit_session.as_mut() {
      edit.set_visual_timings(texture_time, boundary_time);
    }
    self.refresh_state_target_overlay();
    self.refresh_selected_province_boundaries();
    self.refresh_state_information();
    println!(
      "Rebuilt state texture and boundaries for {changed_count} provinces in {} ms.",
      self.last_state_visual_update_ms
    );
  }

  fn refresh_state_visuals_selective(
    &mut self,
    changed: &BTreeSet<u32>,
    extents: Extents
  ) {
    use opengl_graphics::{Format, UpdateTexture};

    let Some(project) = self.project.as_ref() else { return };
    let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps() else {
      return;
    };
    let ambiguous = project.ambiguous_provinces.keys().copied().collect::<BTreeSet<_>>();
    let region = generate_state_view_region_for(
      &self.bundle.map,
      state_by_province,
      &ambiguous,
      unassigned_land_provinces,
      extents
    );
    let (offset, size) = extents.to_offset_size();
    let Some(texture) = self.state_texture.as_mut() else {
      self.refresh_state_visuals_full(changed.len());
      return;
    };
    UpdateTexture::update(texture, &mut (), Format::Rgba8, &region.image, offset, size)
      .expect("unable to update state texture");

    self.state_boundaries.retain(|boundary| {
      let [a, b] = boundary.into_array();
      !extents.contains(a) && !extents.contains(b)
    });
    self.state_boundaries.extend(region.state_boundaries);

    let texture_time = region.generated_in;
    let boundary_time = region.boundary_scan_in;
    self.last_state_visual_update_ms = texture_time.as_millis() + boundary_time.as_millis();
    self.last_state_visual_update_kind = "selective";
    if let Some(edit) = self.state_edit_session.as_mut() {
      edit.set_visual_timings(texture_time, boundary_time);
    }
    self.refresh_state_target_overlay();
    self.refresh_selected_province_boundaries();
    self.refresh_state_information();
    println!(
      "Updated {} provinces selectively in {} ms.",
      changed.len(),
      self.last_state_visual_update_ms
    );
  }

  fn selective_state_update_extents(
    &mut self,
    changed: &BTreeSet<u32>
  ) -> Option<Extents> {
    if self.state_province_extents.is_none() {
      let started = std::time::Instant::now();
      self.state_province_extents = Some(self.bundle.map.province_extents_by_id());
      println!(
        "Indexed province bounds for selective state updates in {} ms.",
        started.elapsed().as_millis()
      );
    }
    let extents = self.state_province_extents.as_ref()?;
    let mut changed_extents = changed.iter().map(|province_id| extents.get(province_id).copied());
    let mut combined = changed_extents.next()??;
    for extents in changed_extents {
      combined = combined.join(extents?);
    }
    let [width, height] = self.bundle.map.dimensions();
    combined.lower = [
      combined.lower[0].saturating_sub(1),
      combined.lower[1].saturating_sub(1),
    ];
    combined.upper = [
      combined.upper[0].saturating_add(1).min(width - 1),
      combined.upper[1].saturating_add(1).min(height - 1),
    ];
    Some(combined)
  }

  fn refresh_selected_province_boundaries(&mut self) {
    let selected = self.state_edit_session
      .as_ref()
      .map(StateEditSession::selected_provinces)
      .cloned()
      .unwrap_or_default();
    self.selected_province_boundaries = self.boundaries_for_provinces(&selected);
  }

  fn boundaries_for_provinces(
    &self,
    province_ids: &BTreeSet<u32>
  ) -> Vec<UOrd<Vector2<u32>>> {
    province_ids.iter()
      .filter_map(|province_id| self.province_boundaries.get(province_id))
      .flatten()
      .copied()
      .collect::<BTreeSet<_>>()
      .into_iter()
      .filter(|boundary| {
        let [a, b] = boundary.into_array();
        let a_selected = self.bundle.map.get_province_at(a).preserved_id
          .is_some_and(|id| province_ids.contains(&id));
        let b_selected = self.bundle.map.get_province_at(b).preserved_id
          .is_some_and(|id| province_ids.contains(&id));
        a_selected != b_selected
      })
      .collect()
  }

  fn refresh_state_target_overlay(&mut self) {
    let selection = self.state_selection.clone();
    let Some((image, selected_state_boundaries)) = self.project.as_ref().and_then(|project| {
      let (state_by_province, _) = self.effective_state_maps()?;
      let ambiguous = project.ambiguous_provinces.keys().copied().collect::<BTreeSet<_>>();
      let image = matches!(selection, Some(StateSelection::State { .. })).then(|| {
        selection_overlay_for(
          &self.bundle.map,
          state_by_province,
          &ambiguous,
          selection.as_ref()
        )
      });
      let boundaries = match selection {
        Some(StateSelection::State { state_id, .. }) => boundaries_for_state(
          &self.bundle.map,
          state_by_province,
          &self.state_boundaries,
          state_id
        ),
        _ => Vec::new()
      };
      Some((image, boundaries))
    }) else { return };
    self.selection_texture = image.map(|image| {
      let settings = TextureSettings::new().mag(Filter::Nearest);
      Texture::from_image(&image, &settings)
    });
    self.selected_state_boundaries = selected_state_boundaries;
  }

  fn effective_state_maps(
    &self
  ) -> Option<(&std::collections::HashMap<u32, u32>, &BTreeSet<u32>)> {
    if let Some(edit) = self.state_edit_session.as_ref() {
      Some((edit.state_by_province(), edit.unassigned_land_provinces()))
    } else {
      self.project.as_ref()
        .map(|project| (&project.state_by_province, &project.unassigned_land_provinces))
    }
  }

  fn state_edit_status_text(&self) -> Option<String> {
    let edit = self.state_edit_session.as_ref()?;
    Some(state_edit_status_message(edit))
  }

  fn state_lasso_status_text(&self) -> Option<String> {
    self.state_edit_session.as_ref()?;
    let text = match &self.state_lasso_phase {
      StateLassoPhase::Inactive => format!(
        "State lasso: Inactive | Mode: {} | Inclusion: {}\n\
         Controls: L start | click points | Enter close/confirm | Esc cancel | Shift Add | Alt Remove",
        self.state_lasso_mode.label(),
        self.state_lasso_inclusion.label()
      ),
      StateLassoPhase::Drawing { points, mode, inclusion } => format!(
        "State lasso: Drawing ({} points) | Mode: {} | Inclusion: {}\n\
         Click first point or Enter to calculate preview; Esc cancels",
        points.len(),
        mode.label(),
        inclusion.label()
      ),
      StateLassoPhase::Preview { candidates, mode, inclusion, .. } => format!(
        "State lasso: Preview | Mode: {} | Inclusion: {} | {} ms\n\
         Selectable: {} | Already selected: {} | Blocked: {} (ambiguous {}, invalid state {}) | Ignored non-land: {}\n\
         Enter/Edit menu confirms selection only; Esc cancels without changing selection",
        mode.label(),
        inclusion.label(),
        candidates.computed_in.as_millis(),
        candidates.selectable.len(),
        candidates.already_selected.len(),
        candidates.blocked.len(),
        candidates.ambiguous.len(),
        candidates.invalid_state.len(),
        candidates.ignored_non_land
      ),
    };
    Some(text)
  }

  fn state_brush_status_text(&self) -> Option<String> {
    let edit = self.state_edit_session.as_ref()?;
    let target = match self.state_brush_mode {
      StateBrushMode::AssignToTarget => edit.target_state_id()
        .and_then(|state_id| {
          let name = edit.state_data(state_id)?.name
            .unwrap_or_else(|| "<unnamed>".to_owned());
          Some(format!("State {state_id} — {name}"))
        })
        .unwrap_or_else(|| "No valid target".to_owned()),
      StateBrushMode::Unassign => "Unassigned land".to_owned(),
    };
    let text = match &self.state_brush_phase {
      StateBrushPhase::Inactive => format!(
        "State Brush: Inactive | Mode: {} | Target: {target}\n\
         Controls: B activate Assign | State Brush menu selects Unassign",
        self.state_brush_mode.label(),
      ),
      StateBrushPhase::Ready => format!(
        "State Brush: Ready | Mode: {} | Target: {target}\n\
         Left click/drag previews; release applies one command; Esc deactivates",
        self.state_brush_mode.label(),
      ),
      StateBrushPhase::Stroking(stroke) => format!(
        "State Brush: Stroking | Mode: {} | Target: {target}\n\
         Will change: {} | No-op: {} | Blocked: {} | Ignored: {} | Events: {} | Samples: {}\n\
         Release applies once; Esc cancels without changing the session",
        stroke.mode.label(),
        stroke.selectable_provinces.len(),
        stroke.no_op_provinces.len(),
        stroke.blocked_ambiguous.len() + stroke.blocked_invalid_state.len(),
        stroke.ignored_non_land.len() + usize::from(stroke.encountered_unknown),
        stroke.input_events,
        stroke.sampled_points,
      ),
    };
    Some(match self.last_state_brush_result.as_deref() {
      Some(result) => format!("{text}\nLast stroke: {result}"),
      None => text,
    })
  }

  fn refresh_state_information(&mut self) {
    self.project_status = self.project.as_ref().map(|project| {
      project_status_message_with_session(
        project,
        self.state_edit_session.as_ref(),
        self.last_state_visual_update_ms,
        self.last_state_visual_update_kind
      )
    });
    let details = self.project.as_ref().and_then(|project| {
      let edit = self.state_edit_session.as_ref();
      let active_matches_loaded_selection = self.active_state_id.is_some_and(|active_state_id| {
        matches!(
          self.state_selection,
          Some(StateSelection::State { state_id, .. }) if state_id == active_state_id
        ) && edit
          .and_then(|edit| edit.state_origin(active_state_id))
          .is_some_and(|origin| matches!(origin, WorkingStateOrigin::Loaded { .. }))
      });
      if active_matches_loaded_selection {
        selection_information(project, edit, self.state_selection.as_ref())
      } else if let (Some(edit), Some(state_id)) = (edit, self.active_state_id) {
        active_state_information(edit, state_id, self.active_province_id)
      } else {
        selection_information(project, edit, self.state_selection.as_ref())
      }
    });
    let province_details = self.project.as_ref().and_then(|project| {
      active_province_information(
        project,
        self.state_edit_session.as_ref(),
        self.active_province_id,
      )
    });
    let status = self.state_edit_status_text();
    let lasso = self.state_lasso_status_text();
    let brush = self.state_brush_status_text();
    self.selection_info = [province_details, details, status, lasso, brush]
      .into_iter()
      .flatten()
      .join("\n")
      .into();
  }

  pub fn set_tool_mode(&mut self, mode: ToolMode) {
    self.deactivate_state_brush();
    self.tool.mode = mode;
  }

  pub fn cycle_tool_brush(&mut self, interface: &Interface, cursor_pos: Option<Vector2<f64>>, backwards: bool, alerts: &mut Alerts) {
    match self.view_mode {
      ViewMode::Color => {
        let kind = self.tool.kind_brush
          .map(ProvinceKind::from)
          .or_else(|| {
            let pos = cursor_pos.and_then(|cursor_pos| {
              self.camera.relative_position_int(interface, cursor_pos)
            })?;
            Some(self.bundle.map.get_province_at(pos).kind)
          })
          .unwrap_or(ProvinceKind::Land);
        let color = self.bundle.random_color_pure(kind);
        self.tool.color_brush = Some(color);
        alerts.push(Ok(format!("Brush set to color {}", stringify_color(color))))
      },
      ViewMode::Kind => {
        let kind = self.tool.kind_brush;
        let kind = cycle_kinds(kind, backwards);
        self.tool.kind_brush = Some(kind);
        alerts.push(Ok(format!("Brush set to type {}", kind.to_str().to_uppercase())));
      },
      ViewMode::Terrain => {
        let terrain = self.tool.terrain_brush.as_deref();
        let terrain = self.bundle.config.cycle_terrains(terrain, backwards);
        alerts.push(Ok(format!("Brush set to terrain {}", terrain.to_uppercase())));
        self.tool.terrain_brush = Some(terrain);
      },
      ViewMode::Continent => {
        let continent = self.tool.continent_brush;
        let continent = cycle_continents(continent, backwards);
        self.tool.continent_brush = Some(continent);
        alerts.push(Ok(format!("Brush set to continent {}", continent)));
      },
      ViewMode::Coastal => (),
      ViewMode::Adjacencies => {
        let adjacency_kind = self.tool.adjacency_brush;
        let adjacency_kind = cycle_connection(adjacency_kind, backwards);
        self.tool.adjacency_brush = Some(adjacency_kind);
        alerts.push(Ok(format!("Brush set to adjacencies {}", adjacency_kind.to_str().to_uppercase())));
      }
    };
  }

  pub fn pick_tool_brush(&mut self, interface: &Interface, cursor_pos: Vector2<f64>, alerts: &mut Alerts) {
    if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
      let color = self.bundle.map.get_color_at(pos);
      let province_data = self.bundle.map.get_province_at(pos);
      match self.view_mode {
        ViewMode::Color => {
          self.tool_paint_end();
          self.tool.color_brush = Some(color);
          alerts.push(Ok(format!("Picked color {}", stringify_color(color))));
        },
        ViewMode::Kind => if let Some(kind) = province_data.kind.to_definition_kind() {
          self.tool.kind_brush = Some(kind);
          alerts.push(Ok(format!("Picked type {}", kind.to_str().to_uppercase())));
        },
        ViewMode::Terrain => if province_data.terrain != "unknown" {
          let terrain = province_data.terrain.as_str();
          self.tool.terrain_brush = Some(terrain.to_owned());
          alerts.push(Ok(format!("Picked terrain {}", terrain.to_uppercase())));
        },
        ViewMode::Continent => {
          let continent = province_data.continent;
          self.tool.continent_brush = Some(continent);
          alerts.push(Ok(format!("Picked continent {}", continent)));
        },
        ViewMode::Coastal => (),
        ViewMode::Adjacencies => ()
      };
    };
  }

  pub fn change_tool_radius(&mut self, d: f64) {
    const LIMIT: f64 = std::f64::consts::SQRT_2 / 2.0;
    if let (ViewMode::Color, ToolMode::PaintArea) = (self.view_mode, &self.tool.mode) {
      let r = self.tool.radius;
      let d = d * (1.0 + 0.025 * r);
      self.tool.radius = (r + d).max(LIMIT);
    };
  }

  /// Activates the tool, ie, performs a left-click action
  pub fn activate_tool(&mut self, interface: &Interface, cursor_pos: Vector2<f64>, modifier: bool) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      return;
    };

    match self.view_mode {
      ViewMode::Color => match self.tool.mode {
        ToolMode::PaintArea => self.tool_paint_brush(interface, cursor_pos),
        ToolMode::PaintBucket => self.tool_paint_bucket(interface, cursor_pos, modifier),
        ToolMode::Lasso(_) => self.tool_lasso_add_point(interface, cursor_pos)
      },
      ViewMode::Adjacencies => self.tool_connect_activate(interface, cursor_pos),
      _ => self.tool_paint_brush(interface, cursor_pos)
    };
  }

  /// Deactivates the tool, ie, performs a release-left-click action
  pub fn deactivate_tool(&mut self) {
    if let ToolMode::PaintArea = self.tool.mode {
      self.tool_paint_end();
    };
  }

  pub fn cancel_tool(&mut self) {
    self.tool.adjacency_selection = None;
    if let ToolMode::Lasso(lasso) = &mut self.tool.mode {
      lasso.drain();
    };
  }

  pub fn finish_tool(&mut self) {
    if self.map_access_mode == MapAccessMode::ReadOnly {
      return;
    };

    if let ToolMode::Lasso(lasso) = &mut self.tool.mode {
      let lasso = lasso.drain();
      self.tool_lasso_finish(lasso);
    };
  }

  fn tool_lasso_add_point(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
    if let ToolMode::Lasso(lasso) = &mut self.tool.mode {
      if lasso.can_finish(interface, &self.camera, cursor_pos) {
        let lasso = lasso.drain();
        self.tool_lasso_finish(lasso);
      } else {
        let point = self.camera.relative_position(interface, cursor_pos);
        let point = if self.tool.lasso_snap {
          [point[0].round(), point[1].round()]
        } else {
          point
        };

        lasso.push(point);
      };
    };
  }

  fn tool_lasso_finish(&mut self, lasso: Vec<Vector2<f64>>) {
    if let (Some(color), ViewMode::Color) = (self.tool.color_brush, self.view_mode) {
      if lasso.len() > 2 {
        if let Some(extents) = self.history.paint_pixel_lasso(&mut self.bundle, lasso, color, self.tool.brush_mask) {
          self.problems.clear();
          self.modified = true;
          self.refresh_selective(extents);
        };
      };
    };
  }

  fn tool_paint_brush(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
    if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
      if let (Some(color), ViewMode::Color) = (self.tool.color_brush, self.view_mode) {
        let pos = self.camera.relative_position(interface, cursor_pos);
        if let Some(extents) = self.history.paint_pixel_area(&mut self.bundle, pos, self.tool.radius, color, self.tool.brush_mask, self.tool.id) {
          self.problems.clear();
          self.modified = true;
          self.refresh_selective(extents);
        };
      } else if let (Some(kind), ViewMode::Kind) = (self.tool.kind_brush, self.view_mode) {
        if let Some(extents) = self.history.paint_province_kind(&mut self.bundle, pos, kind) {
          self.modified = true;
          self.refresh_selective(extents);
        };
      } else if let (Some(terrain), ViewMode::Terrain) = (&self.tool.terrain_brush, self.view_mode) {
        if let Some(extents) = self.history.paint_province_terrain(&mut self.bundle, pos, terrain.clone()) {
          self.modified = true;
          self.refresh_selective(extents);
        };
      } else if let (Some(continent), ViewMode::Continent) = (self.tool.continent_brush, self.view_mode) {
        if let Some(extents) = self.history.paint_province_continent(&mut self.bundle, pos, continent) {
          self.modified = true;
          self.refresh_selective(extents);
        };
      };
    };
  }

  fn tool_paint_end(&mut self) {
    self.tool.id += 1;
  }

  fn tool_paint_bucket(&mut self, interface: &Interface, cursor_pos: Vector2<f64>, fill_all: bool) {
    if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
      if let (Some(fill_color), ViewMode::Color) = (self.tool.color_brush, self.view_mode) {
        let result = if fill_all {
          self.history.paint_entire_province(&mut self.bundle, pos, fill_color)
        } else {
          self.history.paint_pixel_bucket(&mut self.bundle, pos, fill_color, self.tool.brush_mask)
        };

        if let Some(extents) = result {
          self.problems.clear();
          self.modified = true;
          self.refresh_selective(extents);
        };
      };
    };
  }

  fn tool_connect_activate(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
    if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
      let which = self.bundle.map.get_color_at(pos);
      if let Some(kind) = self.tool.adjacency_brush {
        if let Some(color) = self.tool.adjacency_selection.take() {
          self.history.add_or_remove_connection(&mut self.bundle, UOrd::new([which, color]), kind);
        } else {
          self.tool.adjacency_selection = Some(which);
        };
      };
    };
  }

  pub fn validate_pixel_counts(&self, alerts: &mut Alerts) {
    if self.bundle.map.validate_pixel_counts() {
      alerts.push(Ok("Validation successful"));
    } else {
      alerts.push(Err("Validation failed"));
    };
  }

  fn unknown_terrains(&self) -> Option<String> {
    if let Some(unknown_terrains) = &self.unknown_terrains {
      let unknown_terrains = unknown_terrains.iter().map(|s| s.to_uppercase()).join(", ");
      Some(format!("Terrain mode unavailable, unknown terrains present: {}", unknown_terrains))
    } else {
      None
    }
  }

  fn refresh(&mut self) {
    let buffer = match self.view_mode {
      ViewMode::Color => self.bundle.texture_buffer_color(),
      ViewMode::Kind => self.bundle.texture_buffer_kind(),
      ViewMode::Terrain => self.bundle.texture_buffer_terrain(),
      ViewMode::Continent => self.bundle.texture_buffer_continent(),
      ViewMode::Coastal => self.bundle.texture_buffer_coastal(),
      ViewMode::Adjacencies => self.bundle.texture_buffer_color()
    };

    self.texture.update(&buffer);
  }

  fn refresh_selective(&mut self, extents: Extents) {
    use opengl_graphics::{UpdateTexture, Format};
    let (offset, size) = extents.to_offset_size();
    let buffer = match self.view_mode {
      ViewMode::Color => self.bundle.texture_buffer_selective_color(extents),
      ViewMode::Kind => self.bundle.texture_buffer_selective_kind(extents),
      ViewMode::Terrain => self.bundle.texture_buffer_selective_terrain(extents),
      ViewMode::Continent => self.bundle.texture_buffer_selective_continent(extents),
      ViewMode::Coastal => self.bundle.texture_buffer_selective_coastal(extents),
      ViewMode::Adjacencies => self.bundle.texture_buffer_selective_color(extents)
    };

    UpdateTexture::update(&mut self.texture, &mut (), Format::Rgba8, &buffer, offset, size)
      .expect("unable to update texture");
  }

  fn brush_info(&self) -> String {
    if self.map_view_mode == MapViewMode::States {
      return "State inspection".to_owned();
    }

    match self.view_mode {
      ViewMode::Color => match self.tool.color_brush {
        Some(color) => format!("Color {}", stringify_color(color)),
        None => "Color (No Brush)".to_owned()
      },
      ViewMode::Kind => match self.tool.kind_brush {
        Some(kind) => format!("Type {}", kind.to_str().to_uppercase()),
        None => "Type (No Brush)".to_owned()
      },
      ViewMode::Terrain => match &self.tool.terrain_brush {
        Some(terrain) => format!("Terrain {}", terrain.to_uppercase()),
        None => "Terrain (No Brush)".to_owned()
      },
      ViewMode::Continent => match self.tool.continent_brush {
        Some(continent) => format!("Continent {}", continent),
        None => "Continent (No Brush)".to_owned()
      },
      ViewMode::Coastal => "Coastal".to_owned(),
      ViewMode::Adjacencies => match self.tool.adjacency_brush {
        Some(connection) => format!("Adjacencies {}", connection.to_str().to_uppercase()),
        None => "Adjacencies (No Brush)".to_owned()
      }
    }
  }

  fn brush_mask_info(&self) -> String {
    if self.view_mode == ViewMode::Color {
      match self.tool.brush_mask {
        Some(brush_mask) => format!("Mask {}", brush_mask.to_str().to_uppercase()),
        None => "No Mask".to_owned()
      }
    } else {
      String::new()
    }
  }

  fn camera_info(&self, interface: &Interface, cursor_pos: Option<Vector2<f64>>) -> String {
    let zoom_info = format!("{:.2}%", self.camera.scale_factor() * 100.0);
    let cursor_info = cursor_pos
      .and_then(|cursor_pos| self.camera.relative_position_int(interface, cursor_pos))
      .map_or_else(String::new, |[x, y]| format!("{}, {} px", x, y));
    let brush_info = self.brush_info();
    let brush_mask_info = self.brush_mask_info();
    format!("{:<24}{:<24}{:<24}{}", cursor_info, zoom_info, brush_info, brush_mask_info)
  }
}

#[derive(Debug, Clone, Copy)]
struct ProvinceEditorLayout {
  panel: [f64; 4],
  row_height: f64,
  buildings_title_y: f64,
  buildings_y: f64,
  visible_rows: usize,
  page: usize,
  total_rows: usize,
  actions_y: f64,
  error_y: f64,
  visible_error_lines: usize,
}

impl ProvinceEditorLayout {
  fn new(interface: &Interface, total_rows: usize, requested_page: usize) -> Self {
    let window = interface.get_window_size();
    let x = interface.get_sidebar_width() as f64 + 8.0;
    let y = interface.get_toolbar_height() as f64 + 8.0;
    let width = (window[0] - x - 8.0).clamp(560.0, 760.0);
        let height = (window[1] - y - 8.0).clamp(390.0, 620.0);
    let row_height = 29.0;
    let buildings_title_y = y + 122.0;
    let buildings_y = y + 132.0;
    let visible_rows = ((height - 285.0) / row_height).floor().max(1.0) as usize;
    let max_page = total_rows.saturating_sub(1) / visible_rows;
    let page = requested_page.min(max_page);
    let actions_y = buildings_y + visible_rows as f64 * row_height + 8.0;
    let error_y = actions_y + 78.0;
    let visible_error_lines = ((height - (error_y - y) - 8.0) / 18.0)
      .floor()
      .max(0.0) as usize;
    Self {
      panel: [x, y, width, height],
      row_height,
      buildings_title_y,
      buildings_y,
      visible_rows,
      page,
      total_rows,
      actions_y,
      error_y,
      visible_error_lines,
    }
  }

  fn victory_point_field(self) -> [f64; 4] {
    [self.panel[0] + 190.0, self.panel[1] + 55.0, 170.0, 26.0]
  }

  fn victory_point_toggle(self) -> [f64; 4] {
    [self.panel[0] + 372.0, self.panel[1] + 55.0, 180.0, 26.0]
  }

  fn visible_range(self) -> std::ops::Range<usize> {
    let start = self.page * self.visible_rows;
    start..(start + self.visible_rows).min(self.total_rows)
  }

  fn building_y(self, row: usize) -> f64 {
    self.buildings_y + (row - self.page * self.visible_rows) as f64 * self.row_height
  }

  fn building_name(self, row: usize) -> [f64; 4] {
    [
      self.panel[0] + 12.0,
      self.building_y(row),
      self.panel[2] - 246.0,
      self.row_height - 3.0,
    ]
  }

  fn building_value(self, row: usize) -> [f64; 4] {
    [
      self.panel[0] + self.panel[2] - 224.0,
      self.building_y(row),
      88.0,
      self.row_height - 3.0,
    ]
  }

  fn building_remove(self, row: usize) -> [f64; 4] {
    [
      self.panel[0] + self.panel[2] - 126.0,
      self.building_y(row),
      114.0,
      self.row_height - 3.0,
    ]
  }

  fn add_building(self) -> [f64; 4] {
    [self.panel[0] + 12.0, self.actions_y, 190.0, 28.0]
  }

  fn previous_page(self) -> [f64; 4] {
    [self.panel[0] + 212.0, self.actions_y, 92.0, 28.0]
  }

  fn next_page(self) -> [f64; 4] {
    [self.panel[0] + 314.0, self.actions_y, 72.0, 28.0]
  }

  fn apply(self) -> [f64; 4] {
    [self.panel[0] + 12.0, self.actions_y + 36.0, 180.0, 28.0]
  }

  fn discard(self) -> [f64; 4] {
    [self.panel[0] + 202.0, self.actions_y + 36.0, 210.0, 28.0]
  }

  fn has_next_page(self, page: usize) -> bool {
    (page + 1) * self.visible_rows < self.total_rows
  }

  fn clamp_page(self, page: usize) -> usize {
    page.min(self.total_rows.saturating_sub(1) / self.visible_rows)
  }
}

#[derive(Debug, Clone, Copy)]
struct PropertyEditorLayout {
  panel: [f64; 4],
  label_width: f64,
  row_height: f64,
  fields_y: f64,
  buttons_y: f64,
  error_y: f64,
  visible_error_lines: usize,
}

impl PropertyEditorLayout {
  fn new(interface: &Interface) -> Self {
    let window = interface.get_window_size();
    let x = interface.get_sidebar_width() as f64 + 8.0;
    let y = interface.get_toolbar_height() as f64 + 8.0;
    let width = (window[0] - x - 8.0).clamp(420.0, 760.0);
    let height = (window[1] - y - 34.0).clamp(455.0, 535.0);
    let row_height = ((height - 170.0) / 12.0).clamp(23.0, 29.0);
    let fields_y = y + 55.0;
    let buttons_y = fields_y + row_height * 12.0 + 8.0;
    let error_y = buttons_y + 47.0;
    let visible_error_lines = ((height - (error_y - y) - 8.0) / 18.0)
      .floor()
      .max(0.0) as usize;
    Self {
      panel: [x, y, width, height],
      label_width: 205.0,
      row_height,
      fields_y,
      buttons_y,
      error_y,
      visible_error_lines,
    }
  }

  fn field(self, index: usize) -> [f64; 4] {
    [
      self.panel[0] + self.label_width,
      self.fields_y + index as f64 * self.row_height,
      self.panel[2] - self.label_width - 12.0,
      self.row_height - 3.0,
    ]
  }

  fn impassable(self) -> [f64; 4] {
    self.field(StatePropertyDraft::TEXT_FIELD_COUNT)
  }

  fn apply(self) -> [f64; 4] {
    [self.panel[0] + 12.0, self.buttons_y, 180.0, 28.0]
  }

  fn discard(self) -> [f64; 4] {
    [self.panel[0] + 202.0, self.buttons_y, 160.0, 28.0]
  }
}

#[derive(Debug, Clone, Copy)]
struct StateCreationEditorLayout {
  panel: [f64; 4],
  label_width: f64,
  row_height: f64,
  fields_y: f64,
  selection_y: f64,
  buttons_y: f64,
  status_y: f64,
}

impl StateCreationEditorLayout {
  fn new(interface: &Interface) -> Self {
    let window = interface.get_window_size();
    let x = interface.get_sidebar_width() as f64 + 8.0;
    let y = interface.get_toolbar_height() as f64 + 8.0;
    let width = (window[0] - x - 8.0).clamp(470.0, 800.0);
    let height = (window[1] - y - 8.0).clamp(510.0, 620.0);
    let row_height = ((height - 188.0) / 13.0).clamp(23.0, 29.0);
    let fields_y = y + 55.0;
    let selection_y = fields_y + row_height * 13.0 + 19.0;
    let buttons_y = selection_y + 12.0;
    let status_y = buttons_y + 48.0;
    Self {
      panel: [x, y, width, height],
      label_width: 205.0,
      row_height,
      fields_y,
      selection_y,
      buttons_y,
      status_y,
    }
  }

  fn field(self, index: usize) -> [f64; 4] {
    let button_space = if index == 0 { 176.0 } else { 0.0 };
    [
      self.panel[0] + self.label_width,
      self.fields_y + index as f64 * self.row_height,
      self.panel[2] - self.label_width - 12.0 - button_space,
      self.row_height - 3.0,
    ]
  }

  fn use_next_id(self) -> [f64; 4] {
    let field = self.field(0);
    [
      field[0] + field[2] + 8.0,
      field[1],
      168.0,
      field[3],
    ]
  }

  fn impassable(self) -> [f64; 4] {
    self.field(StatePropertyDraft::TEXT_FIELD_COUNT + 1)
  }

  fn create_selected(self) -> [f64; 4] {
    let width = (self.panel[2] - 48.0) / 3.0;
    [self.panel[0] + 12.0, self.buttons_y, width, 28.0]
  }

  fn create_empty(self) -> [f64; 4] {
    let width = (self.panel[2] - 48.0) / 3.0;
    [self.panel[0] + 20.0 + width, self.buttons_y, width, 28.0]
  }

  fn cancel(self) -> [f64; 4] {
    let width = (self.panel[2] - 48.0) / 3.0;
    [self.panel[0] + 28.0 + width * 2.0, self.buttons_y, width, 28.0]
  }
}

#[derive(Debug, Clone, Copy)]
struct StateRemovalEditorLayout {
  panel: [f64; 4],
}

impl StateRemovalEditorLayout {
  fn new(interface: &Interface) -> Self {
    let window = interface.get_window_size();
    let x = interface.get_sidebar_width() as f64 + 8.0;
    let y = interface.get_toolbar_height() as f64 + 8.0;
    let width = (window[0] - x - 8.0).clamp(470.0, 680.0);
    Self {
      panel: [x, y, width, 255.0],
    }
  }

  fn move_all(self) -> [f64; 4] {
    [self.panel[0] + 12.0, self.panel[1] + 96.0, 164.0, 28.0]
  }

  fn target_field(self) -> [f64; 4] {
    [self.panel[0] + 184.0, self.panel[1] + 96.0, 112.0, 28.0]
  }

  fn unassign_all(self) -> [f64; 4] {
    [self.panel[0] + 306.0, self.panel[1] + 96.0, 150.0, 28.0]
  }

  fn remove(self) -> [f64; 4] {
    [self.panel[0] + 12.0, self.panel[1] + 205.0, 190.0, 30.0]
  }

  fn cancel(self) -> [f64; 4] {
    [self.panel[0] + 212.0, self.panel[1] + 205.0, 92.0, 30.0]
  }
}

fn editor_field_color(active: bool, invalid: bool) -> DrawColor {
  if invalid {
    [0.38, 0.08, 0.08, 1.0]
  } else if active {
    [0.12, 0.25, 0.42, 1.0]
  } else {
    [0.14, 0.15, 0.18, 1.0]
  }
}

fn draw_canvas_text(
  ctx: Context,
  glyph_cache: &mut FontGlyphCache,
  gl: &mut GlGraphics,
  color: DrawColor,
  pos: Vector2<f64>,
  text: &str,
) {
  graphics::text(
    color,
    FONT_SIZE,
    text,
    glyph_cache,
    ctx.transform.trans_pos(pos),
    gl,
  ).expect("unable to draw state property editor text");
}

fn draw_editor_button(
  ctx: Context,
  glyph_cache: &mut FontGlyphCache,
  gl: &mut GlGraphics,
  rect: [f64; 4],
  text: &str,
  enabled: bool,
) {
  graphics::rectangle(
    if enabled { colors::BUTTON_ACTIVE } else { colors::BUTTON_TOOLBAR },
    rect,
    ctx.transform,
    gl,
  );
  draw_canvas_text(
    ctx,
    glyph_cache,
    gl,
    if enabled { colors::WHITE } else { colors::WHITE_T },
    [rect[0] + 8.0, rect[1] + 19.0],
    text,
  );
}

fn fit_editor_text(text: &str, max_width: f64) -> String {
  if font::get_width_metric_str(text) <= max_width {
    return text.to_owned();
  }
  let mut characters = text.chars().collect::<Vec<_>>();
  while characters.len() > 1 {
    characters.remove(0);
    let candidate = format!("…{}", characters.iter().collect::<String>());
    if font::get_width_metric_str(&candidate) <= max_width {
      return candidate;
    }
  }
  "…".to_owned()
}

fn point_in_rect(point: Vector2<f64>, rect: [f64; 4]) -> bool {
  point[0] >= rect[0]
    && point[1] >= rect[1]
    && point[0] <= rect[0] + rect[2]
    && point[1] <= rect[1] + rect[3]
}

fn project_status_message_with_session(
  project: &Hoi4Project,
  edit: Option<&StateEditSession>,
  visual_update_ms: u128,
  visual_update_kind: &str
) -> String {
  let name = project.paths.root
    .file_name()
    .map(|name| name.to_string_lossy())
    .unwrap_or_else(|| project.paths.root.to_string_lossy());
  let summary = &project.load_summary;
  let mut text = format!(
    "Project: {name}\n\
     Original: {} indexed / {} files | Assigned: {} | Unassigned land: {}\n\
     Original diagnostics: {} errors, {} warnings | Ambiguous: {} | Unknown refs: {}",
    summary.indexed_states,
    summary.files_found,
    summary.assigned_provinces,
    summary.land_provinces_without_state,
    summary.errors,
    summary.warnings,
    summary.duplicate_provinces,
    summary.missing_province_references
  );
  if let Some(edit) = edit {
    let edit_summary = edit.summary();
    text.push('\n');
    let timings = edit.last_timings();
    text.push_str(&format!(
      "Current session: Active states: {} | Created: {} | Removed: {} | Reserved IDs: {}\n\
       Assigned: {} | Unassigned land: {} | Commands: {} | Modified states: {}\n\
       Selected provinces: {} | Target: {} | Session diagnostics: {} errors, {} warnings | Visual refresh: {} {} ms\n\
       Last command: preflight {} us, apply {} us, index {} us | Visual timings: texture {} ms, boundaries {} ms",
      edit_summary.active_states,
      edit_summary.created_states,
      edit_summary.removed_states,
      edit_summary.reserved_state_ids,
      edit_summary.assigned_provinces,
      edit_summary.unassigned_land_provinces,
      edit_summary.commands,
      edit_summary.modified_states,
      edit_summary.selected_provinces,
      edit_summary.target_state_id.map_or_else(|| "None".to_owned(), |id| id.to_string()),
      edit_summary.session_errors,
      edit_summary.session_warnings,
      visual_update_kind,
      visual_update_ms,
      timings.command_preflight.as_micros(),
      timings.command_apply.as_micros(),
      timings.index_update.as_micros(),
      timings.state_texture_update.as_millis(),
      timings.state_boundary_update.as_millis()
    ));
  }
  text
}

fn state_edit_status_message(edit: &StateEditSession) -> String {
  let summary = edit.summary();
  let diagnostics = edit.diagnostics().len();
  let dirty_states = edit.dirty_state_ids().iter()
    .map(u32::to_string)
    .join(", ");
  let sources = edit.selection_sources().into_iter()
    .map(|(state_id, count)| match state_id {
      Some(state_id) => format!("State {state_id}: {count}"),
      None => format!("Unassigned: {count}"),
    })
    .join(", ");
  format!(
    "State edit session: active {} | created {} | removed {} | reserved IDs {}\n\
     {} selected [{}] | target {} | undo {} ({}) | redo {} ({}) | dirty states {} [{}] | diagnostics {}\n\
     Last command: {}\n\
     Actions: Ctrl+click select province | normal click active/target | Edit > New/Remove/State properties/Province data/Move/Unassign/Discard",
    summary.active_states,
    summary.created_states,
    summary.removed_states,
    summary.reserved_state_ids,
    summary.selected_provinces,
    if sources.is_empty() { "none".to_owned() } else { sources },
    summary.target_state_id.map_or_else(|| "None".to_owned(), |id| id.to_string()),
    summary.commands,
    if edit.can_undo() { "available" } else { "empty" },
    summary.redo_commands,
    if edit.can_redo() { "available" } else { "empty" },
    summary.modified_states,
    if dirty_states.is_empty() { "none".to_owned() } else { dirty_states },
    diagnostics,
    edit.last_command_description().unwrap_or_else(|| "none".to_owned())
  )
}

fn selection_sources_message(edit: &StateEditSession) -> String {
  edit.selection_sources().into_iter()
    .map(|(state_id, count)| match state_id {
      Some(state_id) => format!("From State {state_id}: {count}"),
      None => format!("Unassigned: {count}"),
    })
    .join("\n")
}

fn selection_message(
  project: &Hoi4Project,
  edit: Option<&StateEditSession>,
  selection: Option<&StateSelection>
) -> Option<String> {
  match selection {
    Some(StateSelection::State { state_id, province_id }) => {
      let working = edit.and_then(|edit| edit.state_data(*state_id));
      let name = working.as_ref()
        .or_else(|| project.state_document(*state_id).and_then(|document| document.data.as_ref()))
        .and_then(|data| data.name.as_deref())
        .unwrap_or("<unnamed>");
      Some(format!(
        "Selected state {state_id} — {name} from province {province_id}."
      ))
    },
    Some(StateSelection::AmbiguousProvince { province_id, state_ids }) => Some(format!(
      "Province {province_id} is assigned to states {}.",
      state_ids.iter().join(", ")
    )),
    Some(StateSelection::UnassignedProvince { province_id }) => {
      Some(format!("Province {province_id} has no state assignment."))
    },
    None => None
  }
}

fn option_display(value: Option<impl fmt::Display>) -> String {
  value.map(|value| value.to_string()).unwrap_or_else(|| "—".to_owned())
}

fn option_text_display(value: &Option<String>) -> String {
  value.clone().unwrap_or_else(|| "—".to_owned())
}

fn format_set(values: &BTreeSet<String>) -> String {
  if values.is_empty() { "—".to_owned() } else { values.iter().join(", ") }
}

fn format_named_values(values: &BTreeMap<String, i64>) -> String {
  if values.is_empty() {
    "—".to_owned()
  } else {
    values.iter().map(|(name, value)| format!("{name}={value}")).join(", ")
  }
}

fn compared_value(working: String, original: String) -> String {
  if working == original {
    working
  } else {
    format!("{working} (original: {original})")
  }
}

fn selection_information(
  project: &Hoi4Project,
  edit: Option<&StateEditSession>,
  selection: Option<&StateSelection>
) -> Option<String> {
  match selection {
    Some(StateSelection::State { state_id, province_id }) => {
      let document = project.state_document(*state_id)?;
      let data = document.data.as_ref()?;
      let working = edit.and_then(|edit| edit.state_data(*state_id));
      let working = working.as_ref().unwrap_or(data);
      let resources = format_named_values(&working.resources);
      let original_resources = format_named_values(&data.resources);
      let victory_points = if working.history.victory_points.is_empty() {
        "—".to_owned()
      } else {
        working.history.victory_points.iter()
          .map(|victory_point| format!(
            "{}={}",
            victory_point.province_id,
            victory_point.value
          ))
          .join(", ")
      };
      let cores = format_set(&working.history.cores);
      let original_cores = format_set(&data.history.cores);
      let claims = format_set(&working.history.claims);
      let original_claims = format_set(&data.history.claims);
      let state_buildings = format_named_values(&working.history.state_buildings);
      let original_state_buildings = format_named_values(&data.history.state_buildings);
      let building_entries = working.history.state_buildings.len()
        + working.history.province_buildings.values().map(BTreeMap::len).sum::<usize>();
      let modified = edit.is_some_and(|edit| edit.is_state_dirty(*state_id));
      let (errors, warnings) = project.diagnostics.iter()
        .filter(|diagnostic| diagnostic.path.as_ref() == Some(&document.path))
        .fold((0, 0), |(errors, warnings), diagnostic| {
          match diagnostic.severity {
            DiagnosticSeverity::Error => (errors + 1, warnings),
            DiagnosticSeverity::Warning => (errors, warnings + 1),
            DiagnosticSeverity::Info => (errors, warnings)
          }
        });
      Some(format!(
        "State {state_id} — {}\n\
         {}\n\
         Source province: {province_id}\n\
         Provinces: {}\n\
         Manpower: {} | Category: {}\n\
         Max level factor: {} | Local supplies: {} | Impassable: {}\n\
         Owner: {} | Controller: {}\n\
         Cores: {}\n\
         Claims: {}\n\
         Resources: {}\n\
         State buildings: {}\n\
         Victory points: {}\n\
         Building entries: {}\n\
         Original diagnostics: {} errors, {} warnings\n\
         File: {}\n\
         Actions: Edit > Edit state properties | Remove state from session",
        working.name.as_deref().unwrap_or("<unnamed>"),
        if modified { "Modified in memory" } else { "Working values match original" },
        working.provinces.len(),
        compared_value(option_display(working.manpower), option_display(data.manpower)),
        compared_value(
          option_text_display(&working.state_category),
          option_text_display(&data.state_category)
        ),
        compared_value(
          option_display(working.buildings_max_level_factor),
          option_display(data.buildings_max_level_factor)
        ),
        compared_value(option_display(working.local_supplies), option_display(data.local_supplies)),
        compared_value(
          working.impassable.unwrap_or(false).to_string(),
          data.impassable.unwrap_or(false).to_string()
        ),
        compared_value(
          option_text_display(&working.history.owner),
          option_text_display(&data.history.owner)
        ),
        compared_value(
          option_text_display(&working.history.controller),
          option_text_display(&data.history.controller)
        ),
        compared_value(cores, original_cores),
        compared_value(claims, original_claims),
        compared_value(resources, original_resources),
        compared_value(state_buildings, original_state_buildings),
        victory_points,
        building_entries,
        errors,
        warnings,
        document.path.display()
      ))
    },
    Some(StateSelection::AmbiguousProvince { province_id, state_ids }) => Some(format!(
      "Ambiguous province {province_id}\nCandidate states: {}",
      state_ids.iter().join(", ")
    )),
    Some(StateSelection::UnassignedProvince { province_id }) => {
      Some(format!("Unassigned land province {province_id}"))
    },
    None => None
  }
}

fn active_state_information(
  edit: &StateEditSession,
  state_id: u32,
  source_province_id: Option<u32>,
) -> Option<String> {
  let data = edit.state_data(state_id)?;
  let origin = match edit.state_origin(state_id)? {
    WorkingStateOrigin::Loaded { document_path } => {
      format!("Loaded from {}", document_path.display())
    },
    WorkingStateOrigin::CreatedInSession => {
      "Created in memory\nNo state file exists yet".to_owned()
    },
  };
  let victory_points = if data.history.victory_points.is_empty() {
    "—".to_owned()
  } else {
    data.history.victory_points.iter()
      .map(|victory_point| format!(
        "{}={}",
        victory_point.province_id,
        victory_point.value
      ))
      .join(", ")
  };
  let province_buildings = data.history.province_buildings.values()
    .map(BTreeMap::len)
    .sum::<usize>();
  Some(format!(
    "State {state_id} — {}\n\
     {}\n\
     {}\n\
     Source province: {}\n\
     Provinces: {}\n\
     Manpower: {} | Category: {}\n\
     Owner: {} | Controller: {}\n\
     Resources: {}\n\
     State buildings: {}\n\
     Victory points: {}\n\
     Province building entries: {}\n\
     Actions: Edit properties | Remove state from session",
    data.name.as_deref().unwrap_or("<unnamed>"),
    if edit.is_state_dirty(state_id) {
      "Modified in memory"
    } else {
      "Working values match original"
    },
    origin,
    source_province_id.map_or_else(|| "None".to_owned(), |id| id.to_string()),
    data.provinces.len(),
    option_display(data.manpower),
    option_text_display(&data.state_category),
    option_text_display(&data.history.owner),
    option_text_display(&data.history.controller),
    format_named_values(&data.resources),
    format_named_values(&data.history.state_buildings),
    victory_points,
    province_buildings,
  ))
}

fn active_province_information(
  project: &Hoi4Project,
  edit: Option<&StateEditSession>,
  province_id: Option<u32>,
) -> Option<String> {
  let province_id = province_id?;
  let edit = edit?;
  let state_id = edit.province_state_id(province_id);
  let data = edit.province_data(province_id).unwrap_or_default();
  let state = state_id.map(|state_id| {
    let name = edit.state_data(state_id)
      .or_else(|| {
        project.state_document(state_id)
          .and_then(|document| document.data.clone())
      })
      .and_then(|data| data.name)
      .unwrap_or_else(|| "<unnamed>".to_owned());
    format!("State {state_id} â€” {name}")
  }).unwrap_or_else(|| "Unassigned".to_owned());
  let edit_status = match edit.editable_province_state(province_id) {
    Ok(state_id) => format!(
      "Editable in State {state_id} | Action: Edit > Edit province data"
    ),
    Err(error) => error.to_string(),
  };
  Some(format!(
    "PROVINCE {province_id}\n\
     {state}\n\
     Overview: active province | Selected provinces for Move: {}\n\
     Victory point: {}\n\
     Province Buildings: {}\n\
     Diagnostics: {}",
    edit.selected_provinces().len(),
    option_display(data.victory_point),
    format_named_values(&data.buildings),
    edit_status,
  ))
}

impl fmt::Debug for Canvas {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.debug_struct("Canvas")
      .field("bundle", &self.bundle)
      .field("history", &self.history)
      .field("texture", &format_args!("..."))
      .field("state_texture", &self.state_texture.as_ref().map(|_| "..."))
      .field("state_boundaries", &self.state_boundaries.len())
      .field("selected_state_boundaries", &self.selected_state_boundaries.len())
      .field("selected_province_boundaries", &self.selected_province_boundaries.len())
      .field("view_mode", &self.view_mode)
      .field("map_view_mode", &self.map_view_mode)
      .field("state_selection", &self.state_selection)
      .field("active_state_id", &self.active_state_id)
      .field("active_province_id", &self.active_province_id)
      .field("tool", &self.tool)
      .field("problems", &self.problems)
      .field("unknown_terrains", &self.unknown_terrains)
      .field("location", &self.location)
      .field("project", &self.project)
      .field("state_edit_session", &self.state_edit_session.as_ref().map(|_| "..."))
      .field("state_lifecycle_draft", &self.state_lifecycle_draft)
      .field("last_state_visual_update_ms", &self.last_state_visual_update_ms)
      .field("map_access_mode", &self.map_access_mode)
      .field("modified", &self.modified)
      .field("camera", &self.camera)
      .finish_non_exhaustive()
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MapAccessMode {
  #[default]
  ReadOnly,
  EditableProvinceMap
}



#[derive(Debug, Clone)]
pub struct ToolSettings {
  pub color_brush: Option<Color>,
  pub kind_brush: Option<DefinitionKind>,
  pub terrain_brush: Option<String>,
  pub continent_brush: Option<u16>,
  pub adjacency_brush: Option<ConnectionKind>,
  pub adjacency_selection: Option<Color>,
  pub brush_mask: Option<BrushMask>,
  pub lasso_snap: bool,
  pub radius: f64,
  pub id: u32,
  pub mode: ToolMode
}

impl ToolSettings {
  pub fn cycle_brush_mask(&mut self) {
    self.brush_mask = match self.brush_mask {
      None => Some(BrushMask::LandLakes),
      Some(BrushMask::LandLakes) => Some(BrushMask::Sea),
      Some(BrushMask::Sea) => None
    }
  }
}

impl Default for ToolSettings {
  fn default() -> ToolSettings {
    ToolSettings {
      color_brush: None,
      kind_brush: None,
      terrain_brush: None,
      continent_brush: None,
      adjacency_brush: None,
      adjacency_selection: None,
      brush_mask: None,
      lasso_snap: false,
      radius: 8.0,
      id: 0,
      mode: ToolMode::default()
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolMode {
  PaintArea,
  PaintBucket,
  Lasso(Lasso)
}

impl ToolMode {
  pub fn new_lasso() -> Self {
    ToolMode::Lasso(Lasso(Vec::new()))
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lasso(pub Vec<Vector2<f64>>);

impl Lasso {
  fn can_finish(&self, interface: &Interface, camera: &Camera, cursor_pos: Vector2<f64>) -> bool {
    if let &[point, _, _, ..] = self.0.as_slice() {
      let point = camera.compute_position(interface, point);
      vecmath::vec2_len(vecmath::vec2_sub(point, cursor_pos)) < 5.0
    } else {
      false
    }
  }

  fn drain(&mut self) -> Vec<Vector2<f64>> {
    std::mem::replace(&mut self.0, Vec::new())
  }

  fn push(&mut self, point: Vector2<f64>) {
    self.0.push(point);
  }

  fn iter(&self) -> std::iter::Copied<std::slice::Iter<'_, Vector2<f64>>> {
    self.0.iter().copied()
  }
}

impl Default for ToolMode {
  fn default() -> ToolMode {
    ToolMode::PaintArea
  }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BrushMask {
  LandLakes,
  Sea
}

impl BrushMask {
  #[inline]
  pub fn includes(&self, kind: impl Into<ProvinceKind>) -> bool {
    match (self, kind.into()) {
      (BrushMask::LandLakes, ProvinceKind::Land) => true,
      (BrushMask::LandLakes, ProvinceKind::Lake) => true,
      (BrushMask::Sea, ProvinceKind::Sea) => true,
      (_, ProvinceKind::Unknown) => true,
      _ => false
    }
  }

  fn to_str(self) -> &'static str {
    match self {
      BrushMask::LandLakes => "land + lakes",
      BrushMask::Sea => "sea"
    }
  }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewMode {
  Color,
  Kind,
  Terrain,
  Continent,
  Coastal,
  Adjacencies
}

impl Default for ViewMode {
  fn default() -> ViewMode {
    ViewMode::Color
  }
}

#[derive(Debug, Clone, Copy)]
pub struct CameraCombo<'a> {
  pub(super) camera: &'a Camera,
  pub(super) interface: &'a Interface
}

#[allow(unused)]
impl<'a> CameraCombo<'a> {
  #[inline]
  pub(super) fn relative_position(&self, pos: Vector2<f64>) -> Vector2<f64> {
    self.camera.relative_position(self.interface, pos)
  }

  #[inline]
  pub(super) fn relative_position_int(&self, pos: Vector2<f64>) -> Option<Vector2<u32>> {
    self.camera.relative_position_int(self.interface, pos)
  }

  #[inline]
  pub(super) fn compute_position(&self, pos: Vector2<f64>) -> Vector2<f64> {
    self.camera.compute_position(self.interface, pos)
  }

  #[inline]
  pub(super) fn within_viewport(&self, pos: Vector2<f64>) -> bool {
    self.camera.within_viewport(self.interface, pos)
  }
}

#[derive(Debug)]
pub struct Camera {
  pub texture_size: Vector2<f64>,
  pub display_matrix: Matrix2x3<f64>,
  pub panning: bool
}

impl Camera {
  fn new(texture: &Texture) -> Self {
    use opengl_graphics::ImageSize;
    let (width, height) = texture.get_size();
    let texture_size = [width as f64, height as f64];
    let display_matrix = vecmath::mat2x3_id()
      .trans_pos(vecmath::vec2_scale(texture_size, -0.5));
    Camera {
      texture_size,
      display_matrix,
      panning: false
    }
  }

  pub fn on_mouse_relative(&mut self, rel: Vector2<f64>) {
    if self.panning {
      let rel = vecmath::vec2_scale(rel, self.scale_factor().recip());
      self.display_matrix = self.display_matrix.trans_pos(rel);
    };
  }

  pub fn on_mouse_zoom(&mut self, interface: &Interface, dz: f64, cursor_pos: Vector2<f64>) {
    let zoom = 2.0f64.powf(dz * ZOOM_SENSITIVITY);
    let window_center = interface.get_window_center();
    let cursor_rel = self.relative_position(interface, cursor_pos);
    self.display_matrix = self.display_matrix
      .trans_pos(cursor_rel)
      .trans_pos(window_center)
      .zoom(zoom)
      .trans_pos(vecmath::vec2_neg(cursor_rel))
      .trans_pos(vecmath::vec2_neg(window_center));
  }

  pub fn reset(&mut self) {
    self.display_matrix = vecmath::mat2x3_id()
      .trans_pos(vecmath::vec2_scale(self.texture_size, -0.5));
  }

  pub fn set_panning(&mut self, panning: bool) {
    self.panning = panning;
  }

  /// Converts a point from camera space to map space
  pub(super) fn relative_position(&self, interface: &Interface, pos: Vector2<f64>) -> Vector2<f64> {
    vecmath::row_mat2x3_transform_pos2(self.display_matrix_inv(interface), pos)
  }

  pub(super) fn relative_position_int(&self, interface: &Interface, pos: Vector2<f64>) -> Option<Vector2<u32>> {
    let pos = self.relative_position(interface, pos);
    self.within_dimensions(pos)
      .then(|| [pos[0] as u32, pos[1] as u32])
  }

  /// Converts from map space to camera space
  pub(super) fn compute_position(&self, interface: &Interface, pos: Vector2<f64>) -> Vector2<f64> {
    vecmath::row_mat2x3_transform_pos2(self.display_matrix(interface), pos)
  }

  fn display_matrix(&self, interface: &Interface) -> Matrix2x3<f64> {
    self.display_matrix.trans_pos(interface.get_window_center())
  }

  #[inline]
  fn display_matrix_inv(&self, interface: &Interface) -> Matrix2x3<f64> {
    vecmath::mat2x3_inv(self.display_matrix(interface))
  }

  #[inline]
  pub fn scale_factor(&self) -> f64 {
    (self.display_matrix[0][0] + self.display_matrix[1][1]) / 2.0
  }

  #[inline]
  pub(super) fn within_dimensions(&self, pos: Vector2<f64>) -> bool {
    0.0 <= pos[0] && pos[0] < self.texture_size[0] &&
    0.0 <= pos[1] && pos[1] < self.texture_size[1]
  }

  #[inline]
  pub(super) fn within_viewport(&self, interface: &Interface, pos: Vector2<f64>) -> bool {
    0.0 <= pos[0] && pos[0] < interface.get_window_size()[0] as f64 &&
    0.0 <= pos[1] && pos[1] < interface.get_window_size()[1] as f64
  }
}

fn export_image_buffer<P: AsRef<Path>>(path: P, image: RgbImage) -> Result<(), Error> {
  let file = crate::util::files::create_file(path.as_ref())?;
  super::map::write_rgb_bmp_image(BufWriter::new(file), &image)
}

#[inline]
fn drawable_color(color: Color) -> DrawColor {
  [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, 1.0]
}

fn cycle_kinds<P>(kind: Option<P>, backwards: bool) -> DefinitionKind
where P: Into<ProvinceKind> {
  match kind.map(P::into) {
    Some(ProvinceKind::Land) => if backwards { DefinitionKind::Lake } else { DefinitionKind::Sea },
    Some(ProvinceKind::Sea) => if backwards { DefinitionKind::Land } else { DefinitionKind::Lake },
    Some(ProvinceKind::Lake) => if backwards { DefinitionKind::Sea } else { DefinitionKind::Land },
    Some(ProvinceKind::Unknown) | None => DefinitionKind::Land
  }
}

fn cycle_continents(continent: Option<u16>, backwards: bool) -> u16 {
  const MAX_CONTINENTS: u16 = 32;
  continent.map_or(0, |continent| {
    if backwards {
      (continent + MAX_CONTINENTS - 1) % MAX_CONTINENTS
    } else {
      (continent + 1) % MAX_CONTINENTS
    }
  })
}

fn cycle_connection(connection_kind: Option<ConnectionKind>, backwards: bool) -> ConnectionKind {
  match connection_kind {
    None => ConnectionKind::Strait,
    Some(ConnectionKind::Strait) => if backwards { ConnectionKind::Impassable } else { ConnectionKind::Canal },
    Some(ConnectionKind::Canal) => if backwards { ConnectionKind::Strait } else { ConnectionKind::Impassable },
    Some(ConnectionKind::Impassable) => if backwards { ConnectionKind::Canal } else { ConnectionKind::Strait },
  }
}

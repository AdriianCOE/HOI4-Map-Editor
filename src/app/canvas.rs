use ahash::AHashSet;
use graphics::Transformed;
use graphics::context::Context;
use graphics::ellipse::Ellipse;
use graphics::types::Color as DrawColor;
#[cfg(test)]
use image::Rgba;
use image::{RgbImage, RgbaImage};
use itertools::Itertools;
use opengl_graphics::{Filter, GlGraphics, Texture, TextureSettings};
use uord::UOrd2 as UOrd;
use vecmath::{Matrix2x3, Vector2};

use super::alerts::Alerts;
use super::format::DefinitionKind;
use super::inspector::{
    ClickedState, DeveloperDiagnosticsMode, DoubleClickOutcome, DoubleClickTracker,
    INSPECTOR_ICONS, ProvinceLabelMode, ProvinceSearchEntry, ProvinceSearchIndex,
    ProvinceSearchResult, StateInspectorState, StateInspectorVisibility, StateOpenSource,
    StateSearchEntry, StateSearchIndex, StateSearchResult,
};
use super::inspector_controls::{
    InspectorControlId, InspectorControlLayout, InspectorControlRect, InspectorPickTarget,
    InspectorValueTarget, MapTagPickTarget, MapTagPicker, SearchablePicker,
};
use super::interface::{Interface, StateActionAvailability};
use super::map::*;
use super::map_layers::{
    ImageOverlaySource, MapBaseView, MapLayerState, WorkspaceMode, WorkspaceViewPreferences,
    political_fallback_color,
};
use super::political::{
    PoliticalCountryCatalog, PoliticalLabel, PoliticalLabelVisibility, PoliticalProvince,
    PoliticalStateOwnership, TerritoryAnchorIndex, political_label_visibility,
    political_labels_visible_in_view, prepare_country_labels_with_index,
};
use super::project::{
    BrushProvinceClassification, BuildingScope, CombinedRoundTripValidationReport,
    DiagnosticSeverity, EditableProvinceData, EditableStateProperties, GameDefinitionCatalog,
    Hoi4Project, LassoSelectionMode, MapViewMode, ProjectPatchPlan, ProjectSavePlan,
    ProjectValidationChange, ProjectValidationDiagnostic, ProjectValidationDomain,
    ProjectValidationReport, ProjectValidationTarget, ProvinceAdjacency, ProvinceDataDraft,
    ProvinceDataValidationError, ProvinceInclusionMode, ProvinceRemovalPolicy, RecoveryInfo,
    RoundTripCancellation, RoundTripStage, RoundTripStatus, RoundTripValidationPolicy,
    RoundTripValidationReport, RoundTripValidator, SaveTransactionState, StateBrushMode,
    StateEditSession, StateFillMode, StateFillPreview, StateFillProvince, StateFillProvinceKind,
    StateLassoPhase, StatePropertyDraft, StateRemovalPolicy, StateSaveCancellation,
    StateSaveConditions, StateSaveFault, StateSaveOutcome, StateSaveReport, StateSelection,
    WorkingStateOrigin, boundaries_for_state, classify_state_lasso, detect_state_save_recovery,
    execute_project_save, execute_state_save, format_integer_pt_br, generate_state_view,
    generate_state_view_for, generate_state_view_region_for, parse_grouped_nonnegative_integer,
    plan_state_fill, plan_state_patches, recover_interrupted_state_save, sample_segment,
    save_confirmation_text, select_state_at_for as resolve_state_at_for, selection_overlay_for,
    state_save_eligibility, validate_project,
};
use super::resources::{
    ResourceIconResolver, ResourceMapLabel, ResourceMapState, prepare_resource_labels_with_index,
    resource_label_visible,
};
use super::{FontGlyphCache, colors};
use crate::config::{Config, ImageOverlayProjectSettings, ProjectConfig};
use crate::error::Error;
use crate::font::{self, FONT_SIZE};
use crate::localization::{tr, tr_args};
use crate::util::files::Location;
use crate::util::stringify_color;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

const ZOOM_SENSITIVITY: f64 = 0.125;
const STATE_PROPERTY_LABELS: [&str; StatePropertyDraft::TEXT_FIELD_COUNT] = [
    "Name",
    "Manpower",
    "Category",
    "Max level factor",
    "Local supplies",
    "Owner",
    "Controller",
    "Cores",
    "Claims",
    "Resources",
    "State Buildings",
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
    // Assigned by App only after a complete replacement succeeds. Every field
    // below is owned by this Canvas, so dropping a Canvas is also the atomic
    // invalidation boundary for project-scoped CPU/GPU/cache state.
    project_generation: ProjectGeneration,
    bundle: Bundle,
    history: History,
    texture: Texture,
    state_texture: Option<Texture>,
    political_texture: Option<Texture>,
    political_country_catalog: Option<PoliticalCountryCatalog>,
    political_cache_generation: Option<ProjectGeneration>,
    political_labels: Vec<PoliticalLabel>,
    political_flag_textures: BTreeMap<String, Texture>,
    political_adjacency_pairs: Vec<(u32, u32)>,
    territory_anchor_index: Option<TerritoryAnchorIndex>,
    territory_anchor_generation: Option<ProjectGeneration>,
    political_language: String,
    resource_labels: Vec<ResourceMapLabel>,
    resource_icon_resolver: Option<ResourceIconResolver>,
    resource_icon_textures: BTreeMap<String, Option<Texture>>,
    resource_cache_generation: Option<ProjectGeneration>,
    image_overlay_texture: Option<Texture>,
    image_overlay_status: String,
    state_boundaries: Vec<UOrd<Vector2<u32>>>,
    province_boundaries: BTreeMap<u32, Vec<UOrd<Vector2<u32>>>>,
    selected_state_boundaries: Vec<UOrd<Vector2<u32>>>,
    selected_province_boundaries: Vec<UOrd<Vector2<u32>>>,
    lasso_preview_boundaries: Vec<UOrd<Vector2<u32>>>,
    lasso_blocked_boundaries: Vec<UOrd<Vector2<u32>>>,
    brush_preview_boundaries: Vec<UOrd<Vector2<u32>>>,
    brush_blocked_boundaries: Vec<UOrd<Vector2<u32>>>,
    fill_preview_boundaries: Vec<UOrd<Vector2<u32>>>,
    fill_blocked_boundaries: Vec<UOrd<Vector2<u32>>>,
    selection_texture: Option<Texture>,
    texture_overlay: Option<Texture>,
    view_mode: ViewMode,
    workspace_mode: WorkspaceMode,
    workspace_views: WorkspaceViewPreferences,
    map_layers: MapLayerState,
    state_selection: Option<StateSelection>,
    active_state_id: Option<u32>,
    active_province_id: Option<u32>,
    project_status: Option<String>,
    selection_info: Option<String>,
    problems: Vec<Problem>,
    unknown_terrains: Option<AHashSet<String>>,
    location: Location,
    project: Option<Hoi4Project>,
    definition_catalog: Option<GameDefinitionCatalog>,
    definition_base_game_root: Option<PathBuf>,
    state_edit_session: Option<StateEditSession>,
    patch_preview: Option<ProjectPatchPlan>,
    patch_preview_file: usize,
    round_trip_report: Option<RoundTripValidationReport>,
    round_trip_failure_snapshot: Option<RoundTripFailureSnapshot>,
    round_trip_task: Option<RoundTripTask>,
    round_trip_status: Option<String>,
    state_save_report: Option<StateSaveReport>,
    state_save_task: Option<StateSaveTask>,
    state_save_status: Option<String>,
    state_save_recovery: Option<RecoveryInfo>,
    project_save_plan: Option<ProjectSavePlan>,
    last_project_save_summary: Option<ProjectSavePresentationSummary>,
    project_save_validation: Option<CombinedRoundTripValidationReport>,
    project_validation_report: Option<ProjectValidationReport>,
    validation_problems_view: ValidationProblemsView,
    last_validation: Option<LastValidationState>,
    province_save_report: Option<ProvinceSaveReport>,
    province_save_task: Option<ProvinceSaveTask>,
    province_save_status: Option<String>,
    state_apply_dialog: Option<StateApplyDialog>,
    state_apply_after_validation: bool,
    state_apply_ready_for_confirmation: bool,
    review_required_apply_approved: bool,
    province_removal_draft: Option<ProvinceRemovalDraft>,
    province_removal_undo: Vec<ProvinceRemovalTransaction>,
    province_removal_redo: Vec<ProvinceRemovalTransaction>,
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
    state_fill_phase: StateFillPhase,
    state_fill_mode: StateFillMode,
    province_adjacency: ProvinceAdjacency,
    state_pan_tool: bool,
    last_state_brush_result: Option<String>,
    state_province_extents: Option<BTreeMap<u32, Extents>>,
    last_state_visual_update_ms: u128,
    last_state_visual_update_kind: &'static str,
    map_access_mode: MapAccessMode,
    inspector: StateInspectorState,
    inspector_search_focused: bool,
    inspector_search_index: usize,
    inspector_picker: Option<InspectorPickerState>,
    map_tag_picker: MapTagPicker,
    state_double_click: DoubleClickTracker,
    session_started: Instant,
    pub tool: ToolSettings,
    pub modified: bool,
    pub camera: Camera,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorExternalRequest {
    OpenSource(PathBuf),
    CopyPath(String),
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

enum RoundTripTaskMessage {
    Stage(RoundTripStage),
    Finished(Box<RoundTripValidationReport>),
}

struct RoundTripTask {
    receiver: Receiver<RoundTripTaskMessage>,
    cancellation: RoundTripCancellation,
}

enum StateSaveTaskMessage {
    Stage(SaveTransactionState, usize, usize),
    Finished(Box<StateSaveReport>),
}

struct StateSaveTask {
    receiver: Receiver<StateSaveTaskMessage>,
    cancellation: StateSaveCancellation,
    state: SaveTransactionState,
    includes_province_map: bool,
}

enum ProvinceSaveTaskMessage {
    Progress(ProvinceSaveProgress),
    Finished(Result<Box<ProvinceSaveReport>, String>),
}

struct ProvinceSaveTask {
    receiver: Receiver<ProvinceSaveTaskMessage>,
    cancellation: ProvinceSaveCancellation,
    stage: ProvinceSaveStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateApplyDialog {
    Review,
    ViewChanges,
    ProjectSaveReview,
    AdditionalValidation,
    Blocked,
    Progress,
    Result,
    ValidationResults,
    IntegrityProblem,
    ImageOverlay,
    ProvinceRemoval,
}

/// Identity assigned to a fully loaded project context. It intentionally does
/// not track edit revisions: it distinguishes one loaded map/mod from another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ProjectGeneration(pub(crate) u64);

#[derive(Debug, Clone)]
struct RoundTripFailureSnapshot {
    generation: ProjectGeneration,
    summary: String,
    details: String,
}

impl RoundTripFailureSnapshot {
    fn from_report(generation: ProjectGeneration, report: &RoundTripValidationReport) -> Self {
        Self {
            generation,
            summary: report
                .failure
                .as_ref()
                .map_or_else(|| report.summary_text(), |failure| failure.summary()),
            details: report.full_text(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateApplyDialogAction {
    None,
    ConfirmSave,
    ConfirmProjectSave,
    OpenSource(PathBuf),
    CopyDetails(String),
    ChooseImageOverlay,
    UseProjectHeightmap,
    DecreaseImageOverlayOpacity,
    IncreaseImageOverlayOpacity,
    ClearImageOverlay,
    ConfirmProvinceTransfer,
    ConfirmProvinceReferenceRemoval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ValidationSeverityFilter {
    #[default]
    All,
    Errors,
    Warnings,
    Information,
}

impl ValidationSeverityFilter {
    fn cycle(self) -> Self {
        match self {
            Self::All => Self::Errors,
            Self::Errors => Self::Warnings,
            Self::Warnings => Self::Information,
            Self::Information => Self::All,
        }
    }

    fn matches(self, severity: DiagnosticSeverity) -> bool {
        match self {
            Self::All => true,
            Self::Errors => severity == DiagnosticSeverity::Error,
            Self::Warnings => severity == DiagnosticSeverity::Warning,
            Self::Information => severity == DiagnosticSeverity::Information,
        }
    }

    fn label(self) -> &'static str {
        tr(match self {
            Self::All => "project_validation.all",
            Self::Errors => "project_validation.errors",
            Self::Warnings => "project_validation.warnings",
            Self::Information => "project_validation.information",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ValidationSourceFilter {
    #[default]
    All,
    New,
    Aggravated,
    Unchanged,
    Resolved,
    Improved,
}

impl ValidationSourceFilter {
    fn cycle(self) -> Self {
        match self {
            Self::All => Self::New,
            Self::New => Self::Aggravated,
            Self::Aggravated => Self::Unchanged,
            Self::Unchanged => Self::Resolved,
            Self::Resolved => Self::Improved,
            Self::Improved => Self::All,
        }
    }

    fn matches(self, source: Self) -> bool {
        self == Self::All || self == source
    }

    fn label(self) -> &'static str {
        tr(match self {
            Self::All => "project_validation.all",
            Self::New => "project_validation.new",
            Self::Aggravated => "project_validation.aggravated",
            Self::Unchanged => "project_validation.unchanged",
            Self::Resolved => "project_validation.resolved",
            Self::Improved => "project_validation.improved",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ValidationDomainFilter {
    #[default]
    All,
    ProvinceMap,
    Definition,
    States,
    CrossDomain,
}

impl ValidationDomainFilter {
    fn cycle(self) -> Self {
        match self {
            Self::All => Self::ProvinceMap,
            Self::ProvinceMap => Self::Definition,
            Self::Definition => Self::States,
            Self::States => Self::CrossDomain,
            Self::CrossDomain => Self::All,
        }
    }

    fn matches(self, domain: ProjectValidationDomain) -> bool {
        match self {
            Self::All => true,
            Self::ProvinceMap => domain == ProjectValidationDomain::Province,
            Self::Definition => domain == ProjectValidationDomain::Definition,
            Self::States => matches!(
                domain,
                ProjectValidationDomain::State
                    | ProjectValidationDomain::Syntax
                    | ProjectValidationDomain::Resource
                    | ProjectValidationDomain::Building
            ),
            Self::CrossDomain => domain == ProjectValidationDomain::CrossDomain,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => tr("project_validation.all"),
            Self::ProvinceMap => "Province Map",
            Self::Definition => "Definition",
            Self::States => "States",
            Self::CrossDomain => "Cross Domain",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ValidationProblemsView {
    severity: ValidationSeverityFilter,
    source: ValidationSourceFilter,
    domain: ValidationDomainFilter,
    selected: usize,
    offset: usize,
    filters_expanded: bool,
    show_technical_details: bool,
    blocking_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct ProjectSavePresentationSummary {
    province_files: usize,
    state_files: usize,
    coastal_flags_recalculated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectSaveReviewPrimaryAction {
    ConfirmSave,
    ViewBlockingProblems,
    ViewIntegrityProblem,
}

fn project_save_review_primary_action(
    validation_blocked: bool,
    round_trip_failed: bool,
) -> ProjectSaveReviewPrimaryAction {
    if validation_blocked {
        ProjectSaveReviewPrimaryAction::ViewBlockingProblems
    } else if round_trip_failed {
        ProjectSaveReviewPrimaryAction::ViewIntegrityProblem
    } else {
        ProjectSaveReviewPrimaryAction::ConfirmSave
    }
}

impl ProjectSavePresentationSummary {
    fn from_plan(plan: &ProjectSavePlan) -> Self {
        let dirty = plan.dirty();
        Self {
            province_files: dirty.province_files,
            state_files: dirty.state_files,
            coastal_flags_recalculated: plan.coastal_flags_recalculated(),
        }
    }
}

#[derive(Debug, Clone)]
struct ProvinceRemovalDraft {
    province_id: u32,
    target_id: String,
}

#[derive(Debug, Clone, Copy)]
struct ProvinceRemovalTransaction {
    map_before: usize,
    map_after: usize,
    state_before: usize,
    state_after: usize,
}

#[derive(Debug, Clone)]
struct LastValidationState {
    target: ProjectValidationTarget,
    result: String,
    duration_ms: u128,
    semantic: Option<bool>,
    indexes: Option<bool>,
    bytes: Option<bool>,
    unexpected_diagnostics: usize,
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

#[derive(Debug, Clone)]
struct InspectorPickerState {
    target: InspectorPickTarget,
    picker: SearchablePicker<String>,
}

#[derive(Debug, Clone)]
enum InspectorSearchResult {
    State(StateSearchResult),
    Province(ProvinceSearchResult),
}

#[derive(Debug, Clone, Default)]
enum StateFillPhase {
    #[default]
    Inactive,
    Ready,
    Preview {
        preview: StateFillPreview,
        target_state_id: u32,
    },
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
    /// Bind this newly built Canvas to the active App project generation.
    /// Canvas loading is deliberately complete before this call; no existing
    /// project is mutated while a replacement can still fail.
    pub(crate) fn bind_project_generation(&mut self, generation: ProjectGeneration) {
        debug_assert!(self.political_country_catalog.is_none());
        debug_assert!(self.resource_icon_resolver.is_none());
        debug_assert!(self.territory_anchor_index.is_none());
        debug_assert!(self.political_cache_generation.is_none());
        debug_assert!(self.resource_cache_generation.is_none());
        self.project_generation = generation;
        self.territory_anchor_generation = None;
        self.round_trip_failure_snapshot = None;
    }

    pub fn load(location: Location) -> Result<Canvas, Error> {
        Self::load_with_access(
            location,
            None,
            MapAccessMode::EditableProvinceMap,
            Config::load()?,
            None,
        )
    }

    pub fn load_project(project: Hoi4Project) -> Result<Canvas, Error> {
        let location = Location::Directory(project.paths.map_directory.clone());
        let overlay_settings = ProjectConfig::load(&project.paths.root)?
            .value
            .image_overlay;
        let config = Config::load_for_project(&project.paths.root)?;
        Self::load_with_access(
            location,
            Some(project),
            MapAccessMode::EditableProvinceMap,
            config,
            Some(overlay_settings),
        )
    }

    fn load_with_access(
        location: Location,
        mut project: Option<Hoi4Project>,
        map_access_mode: MapAccessMode,
        config: Config,
        overlay_settings: Option<ImageOverlayProjectSettings>,
    ) -> Result<Canvas, Error> {
        let profile_open = std::env::var_os("HOI4_MAP_EDITOR_PROFILE_OPEN").is_some();
        let open_started = Instant::now();
        let bundle = Bundle::load(&location, config)?;
        let bundle_loaded_in = open_started.elapsed();
        let state_loading_started = Instant::now();
        if let Some(project) = project.as_mut() {
            let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
            let land_province_ids = bundle
                .map
                .iter_province_data()
                .filter(|(_, province)| province.kind == ProvinceKind::Land)
                .filter_map(|(_, province)| province.preserved_id)
                .collect::<BTreeSet<_>>();
            project.load_states(&province_ids, &land_province_ids);
        }
        let state_loading_in = state_loading_started.elapsed();
        let state_discovery_ms = project
            .as_ref()
            .map_or(0, |project| project.load_summary.state_discovery_ms);
        let state_read_parse_ms = project
            .as_ref()
            .map_or(0, |project| project.load_summary.state_read_parse_ms);
        let state_indexing_ms = project
            .as_ref()
            .map_or(0, |project| project.load_summary.state_indexing_ms);
        let definitions_started = Instant::now();
        let base_game_root = discover_base_game_root();
        let definition_catalog = project
            .as_ref()
            .map(|project| GameDefinitionCatalog::build(project, base_game_root.as_deref()));
        let definitions_in = definitions_started.elapsed();
        let history = History::new(bundle.config.max_undo_states, &bundle.map);
        let texture_settings = TextureSettings::new().mag(Filter::Nearest);
        let map_texture_started = Instant::now();
        let texture = Texture::from_image(&bundle.texture_buffer_color(), &texture_settings);
        let map_texture_in = map_texture_started.elapsed();
        let state_view_started = Instant::now();
        let state_view = project
            .as_ref()
            .map(|project| generate_state_view(&bundle.map, project));
        if let (Some(project), Some(state_view)) = (project.as_mut(), state_view.as_ref()) {
            project.load_summary.state_texture_generation_ms = state_view.generated_in.as_millis();
            project.load_summary.state_boundary_generation_ms =
                state_view.boundary_scan_in.as_millis();
        }
        let state_texture = state_view
            .as_ref()
            .map(|state_view| Texture::from_image(&state_view.image, &texture_settings));
        let state_boundaries = state_view
            .map(|state_view| state_view.state_boundaries)
            .unwrap_or_default();
        let state_view_in = state_view_started.elapsed();
        let adjacency_started = Instant::now();
        let mut province_boundaries = BTreeMap::<u32, Vec<_>>::new();
        let mut adjacency_pairs = Vec::new();
        if project.is_some() {
            for (boundary, _) in bundle.map.iter_boundaries() {
                let [a, b] = boundary.into_array();
                if let (Some(a), Some(b)) = (
                    bundle.map.province_id_for_color(bundle.map.get_color_at(a)),
                    bundle.map.province_id_for_color(bundle.map.get_color_at(b)),
                ) {
                    adjacency_pairs.push((a, b));
                }
                for position in boundary.into_array() {
                    if let Some(province_id) = bundle
                        .map
                        .province_id_for_color(bundle.map.get_color_at(position))
                    {
                        province_boundaries
                            .entry(province_id)
                            .or_default()
                            .push(boundary);
                    }
                }
            }
        }
        let adjacency_in = adjacency_started.elapsed();
        let map_view_mode = if state_texture.is_some() {
            MapViewMode::States
        } else {
            MapViewMode::ProvinceColors
        };
        let overlay_settings = overlay_settings.unwrap_or_default();
        let overlay_started = Instant::now();
        let (
            image_overlay_source,
            image_overlay_texture,
            image_overlay_status,
            image_overlay_dimensions,
        ) = load_configured_image_overlay(
            &location,
            project.as_ref(),
            &overlay_settings,
            bundle.map.dimensions(),
            &texture_settings,
        );
        let overlay_in = overlay_started.elapsed();
        let session_started = Instant::now();
        let state_edit_session = project
            .as_ref()
            .map(|project| StateEditSession::new(project, &bundle.map));
        let workspace_mode = if state_edit_session.is_some() {
            WorkspaceMode::States
        } else {
            WorkspaceMode::Provinces
        };
        let state_save_recovery = project
            .as_ref()
            .and_then(|project| detect_state_save_recovery(&project.paths.root));
        let project_status = project.as_ref().map(|project| {
            project_status_message_with_session(project, state_edit_session.as_ref(), 0, "initial")
        });
        let session_in = session_started.elapsed();
        if let Some(project) = project.as_ref() {
            println!("{}", project.load_summary_message());
            println!(
                "Generated state map texture in {} ms.\nGenerated state boundaries in {} ms.",
                project.load_summary.state_texture_generation_ms,
                project.load_summary.state_boundary_generation_ms
            );
            println!(
                "State project diagnostics:\n{}",
                project.diagnostic_report()
            );
        }
        let problems = bundle.generate_problems();
        let unknown_terrains = bundle.search_unknown_terrains();
        let show_province_ids = bundle.config.preserve_ids;
        let camera = Camera::new(&texture);
        let mut map_layers = MapLayerState::default().with_base_view(map_view_mode);
        map_layers.show_province_ids = show_province_ids;
        map_layers.image_overlay.opacity = f32::from(overlay_settings.opacity_percent) / 100.0;
        map_layers.image_overlay.source = image_overlay_source;
        if image_overlay_texture.is_some() {
            map_layers.image_overlay.dimensions = image_overlay_dimensions;
            map_layers.image_overlay.content_revision = 1;
            map_layers.image_overlay.enabled = overlay_settings.visible;
        }

        let canvas = Canvas {
            project_generation: ProjectGeneration::default(),
            bundle,
            history,
            texture,
            state_texture,
            political_texture: None,
            // Political country files/localization and Resources presentation are view-only.
            // Loading them here used to make ordinary State/Province project open pay for both.
            political_country_catalog: None,
            political_cache_generation: None,
            political_labels: Vec::new(),
            political_flag_textures: BTreeMap::new(),
            political_adjacency_pairs: adjacency_pairs.clone(),
            territory_anchor_index: None,
            territory_anchor_generation: None,
            political_language: crate::localization::language().to_owned(),
            resource_labels: Vec::new(),
            resource_icon_resolver: None,
            resource_icon_textures: BTreeMap::new(),
            resource_cache_generation: None,
            image_overlay_texture,
            image_overlay_status,
            state_boundaries,
            province_boundaries,
            selected_state_boundaries: Vec::new(),
            selected_province_boundaries: Vec::new(),
            lasso_preview_boundaries: Vec::new(),
            lasso_blocked_boundaries: Vec::new(),
            brush_preview_boundaries: Vec::new(),
            brush_blocked_boundaries: Vec::new(),
            fill_preview_boundaries: Vec::new(),
            fill_blocked_boundaries: Vec::new(),
            selection_texture: None,
            texture_overlay: None,
            view_mode: ViewMode::default(),
            workspace_mode,
            workspace_views: WorkspaceViewPreferences::default(),
            map_layers,
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
            definition_catalog,
            definition_base_game_root: base_game_root,
            state_edit_session,
            patch_preview: None,
            patch_preview_file: 0,
            round_trip_report: None,
            round_trip_failure_snapshot: None,
            round_trip_task: None,
            round_trip_status: None,
            state_save_report: None,
            state_save_task: None,
            state_save_status: state_save_recovery
                .as_ref()
                .map(|recovery| recovery.message.clone()),
            state_save_recovery,
            project_save_plan: None,
            last_project_save_summary: None,
            project_save_validation: None,
            project_validation_report: None,
            validation_problems_view: ValidationProblemsView::default(),
            last_validation: None,
            province_save_report: None,
            province_save_task: None,
            province_save_status: None,
            state_apply_dialog: None,
            state_apply_after_validation: false,
            state_apply_ready_for_confirmation: false,
            review_required_apply_approved: false,
            province_removal_draft: None,
            province_removal_undo: Vec::new(),
            province_removal_redo: Vec::new(),
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
            state_fill_phase: StateFillPhase::Inactive,
            state_fill_mode: StateFillMode::ConnectedUnassigned,
            province_adjacency: ProvinceAdjacency::from_pairs(adjacency_pairs),
            state_pan_tool: false,
            last_state_brush_result: None,
            state_province_extents: None,
            last_state_visual_update_ms: 0,
            last_state_visual_update_kind: "initial",
            map_access_mode,
            inspector: StateInspectorState::session_default(),
            inspector_search_focused: false,
            inspector_search_index: 0,
            inspector_picker: None,
            map_tag_picker: MapTagPicker::default(),
            state_double_click: DoubleClickTracker::default(),
            session_started: Instant::now(),
            modified: false,
            camera,
        };
        if profile_open {
            println!(
                "Project open profile (optional Political/Resources assets deferred):\n\
                 - core map bundle: {} ms\n\
                 - State files + indexes: {} ms (discovery {} ms, read + parse {} ms, indexes {} ms)\n\
                 - definition catalog: {} ms\n\
                 - base map GPU texture: {} ms\n\
                 - State texture + boundaries: {} ms\n\
                 - province adjacency/boundaries: {} ms\n\
                 - image overlay: {} ms\n\
                 - State edit session: {} ms\n\
                 - TOTAL usable core: {} ms",
                bundle_loaded_in.as_millis(),
                state_loading_in.as_millis(),
                state_discovery_ms,
                state_read_parse_ms,
                state_indexing_ms,
                definitions_in.as_millis(),
                map_texture_in.as_millis(),
                state_view_in.as_millis(),
                adjacency_in.as_millis(),
                overlay_in.as_millis(),
                session_in.as_millis(),
                open_started.elapsed().as_millis(),
            );
        }
        Ok(canvas)
    }

    pub fn location(&self) -> &Location {
        &self.location
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    pub fn map_view_mode(&self) -> MapViewMode {
        self.map_layers.base_view
    }

    pub fn workspace_mode(&self) -> WorkspaceMode {
        self.workspace_mode
    }

    pub fn is_state_workspace(&self) -> bool {
        self.workspace_mode() == WorkspaceMode::States
    }

    pub fn has_state_workspace(&self) -> bool {
        self.state_edit_session.is_some()
    }

    pub fn set_workspace_mode(&mut self, workspace: WorkspaceMode, alerts: &mut Alerts) -> bool {
        if workspace == WorkspaceMode::States && !self.has_state_workspace() {
            alerts.push(Err("States workspace requires loaded state history"));
            return false;
        }
        if workspace == self.workspace_mode {
            return false;
        }
        self.workspace_views
            .remember(self.workspace_mode, self.map_layers.base_view);
        self.workspace_mode = workspace;
        if workspace == WorkspaceMode::Provinces {
            self.cancel_state_lasso();
            self.cancel_state_brush();
            self.cancel_state_fill();
            self.state_pan_tool = false;
        }
        let map_view = self.workspace_views.get(workspace);
        self.set_map_view_mode(alerts, map_view);
        self.refresh_state_information();
        alerts.push(Ok(format!(
            "Workspace: {} · Map View: {}",
            workspace.label(),
            self.map_layers.base_view.label()
        )));
        true
    }

    pub fn map_access_mode(&self) -> MapAccessMode {
        self.map_access_mode
    }

    pub fn project(&self) -> Option<&Hoi4Project> {
        self.project.as_ref()
    }

    pub fn detected_capabilities_message(&self) -> String {
        let mut capabilities = vec!["Province map detected", "Definition catalog detected"];
        if self.project.is_some() {
            capabilities.push("State history detected");
        }
        if self.image_overlay_texture.is_some() {
            capabilities.push("Heightmap detected");
        }
        capabilities.join("\n")
    }

    pub fn has_unsaved_state_edits(&self) -> bool {
        self.state_edit_session
            .as_ref()
            .is_some_and(StateEditSession::is_dirty)
    }

    pub fn has_unsaved_province_edits(&self) -> bool {
        self.history.is_dirty(&self.bundle.map)
    }

    fn sync_province_dirty(&mut self) {
        self.modified = self.has_unsaved_province_edits();
    }

    pub fn property_editor_is_open(&self) -> bool {
        self.state_lifecycle_draft.is_some()
            || self.state_property_draft.is_some()
            || self.province_data_draft.is_some()
    }

    pub fn blocks_interface_tooltips(&self) -> bool {
        self.property_editor_is_open()
            || self.inspector_picker_is_open()
            || self.map_tag_picker.active_target().is_some()
            || self.save_blocks_editing()
            || self.state_apply_dialog.is_some()
    }

    pub fn inspector_search_is_focused(&self) -> bool {
        self.inspector_search_focused
    }

    pub fn inspector_search_backspace(&mut self) {
        self.inspector.search.pop();
        self.inspector_search_index = 0;
    }

    pub fn inspector_search_cancel(&mut self) {
        self.inspector_search_focused = false;
        self.inspector_search_index = 0;
    }

    pub fn focus_map_search(&mut self) {
        self.inspector.visibility = StateInspectorVisibility::Expanded;
        self.inspector_search_focused = true;
        self.inspector_search_index = 0;
    }

    pub fn inspector_search_move(&mut self, next: bool) {
        let count = self.inspector_search_results().len();
        if count == 0 {
            self.inspector_search_index = 0;
            return;
        }
        self.inspector_search_index = if next {
            (self.inspector_search_index + 1) % count
        } else {
            self.inspector_search_index
                .checked_sub(1)
                .unwrap_or(count - 1)
        };
    }

    pub fn inspector_search_select(
        &mut self,
        interface: &Interface,
        open_editor: bool,
        alerts: &mut Alerts,
    ) {
        let results = self.inspector_search_results();
        let Some(result) = results
            .get(
                self.inspector_search_index
                    .min(results.len().saturating_sub(1)),
            )
            .cloned()
        else {
            return;
        };
        self.inspector_search_focused = false;
        self.inspector_search_index = 0;
        match result {
            InspectorSearchResult::State(result) => {
                self.select_state_by_id(interface, result.state_id, alerts);
                if open_editor {
                    self.open_state_property_editor(alerts);
                }
            }
            InspectorSearchResult::Province(result) => {
                self.select_province_by_id(interface, result.entry.province_id, alerts);
                if open_editor {
                    self.set_workspace_mode(WorkspaceMode::States, alerts);
                    self.open_province_data_editor(alerts);
                }
            }
        }
    }

    pub fn property_draft_is_modified(&self) -> bool {
        self.state_lifecycle_draft.is_some()
            || self
                .state_property_draft
                .as_ref()
                .is_some_and(StatePropertyDraft::is_modified)
            || self
                .province_data_draft
                .as_ref()
                .is_some_and(ProvinceDataDraft::is_modified)
    }

    pub fn province_data_editor_is_open(&self) -> bool {
        self.province_data_draft.is_some()
    }

    pub fn open_state_property_editor(&mut self, alerts: &mut Alerts) {
        if self.state_lasso_is_active() {
            alerts.push(Err(
                "Confirm or cancel the state lasso before editing properties",
            ));
            return;
        }
        let Some(state_id) = self.active_state_id else {
            alerts.push(Err("Select a valid state before editing properties"));
            return;
        };
        let Some(data) = self
            .state_edit_session
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
        alerts.push(Ok(format!(
            "Editing State {state_id} properties in a temporary draft"
        )));
    }

    pub fn open_new_state_editor(&mut self, alerts: &mut Alerts) {
        if self.state_lasso_is_active() {
            alerts.push(Err(
                "Confirm or cancel the state lasso before creating a state",
            ));
            return;
        }
        let Some(edit) = self.state_edit_session.as_ref() else {
            alerts.push(Err(
                "State creation is available only for loaded state projects",
            ));
            return;
        };
        let suggested_id = edit.suggest_next_state_id();
        self.cancel_state_brush();
        self.cancel_state_fill();
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
            alerts.push(Err(
                "Confirm or cancel the state lasso before removing a state",
            ));
            return;
        }
        let Some(state_id) = self.active_state_id else {
            alerts.push(Err(
                "Select an active state before removing it from the session",
            ));
            return;
        };
        let result = self
            .state_edit_session
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
            }
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

    pub fn open_province_removal_dialog(&mut self, alerts: &mut Alerts) {
        let Some(province_id) = self.active_province_id else {
            alerts.push(Err("Select a land province before deleting it"));
            return;
        };
        if !self.bundle.map.iter_province_data().any(|(_, province)| {
            province.preserved_id == Some(province_id) && province.kind == ProvinceKind::Land
        }) {
            alerts.push(Err(
                "Only land provinces can be removed from a State project",
            ));
            return;
        }
        let result = self
            .state_edit_session
            .as_ref()
            .ok_or_else(|| {
                "Province removal is available only for loaded state projects".to_owned()
            })
            .and_then(|edit| {
                edit.province_data(province_id).map(|_| ()).ok_or_else(|| {
                    format!("Province {province_id} is not a selectable land province")
                })
            });
        if let Err(error) = result {
            alerts.push(Err(error));
            return;
        }
        self.province_removal_draft = Some(ProvinceRemovalDraft {
            province_id,
            target_id: String::new(),
        });
        self.state_apply_dialog = Some(StateApplyDialog::ProvinceRemoval);
    }

    pub fn input_province_removal_text(&mut self, text: &str) {
        let Some(draft) = self.province_removal_draft.as_mut() else {
            return;
        };
        draft
            .target_id
            .extend(text.chars().filter(|character| character.is_ascii_digit()));
    }

    pub fn province_removal_backspace(&mut self) {
        if let Some(draft) = self.province_removal_draft.as_mut() {
            draft.target_id.pop();
        }
    }

    pub fn confirm_province_removal(&mut self, transfer: bool, alerts: &mut Alerts) {
        let Some(draft) = self.province_removal_draft.clone() else {
            return;
        };
        let target_id = match draft.target_id.trim().parse::<u32>() {
            Ok(id) if id != 0 && id != draft.province_id => id,
            _ => {
                alerts.push(Err("Enter a different existing target Province ID"));
                return;
            }
        };
        let Some(target_color) = self.bundle.map.color_for_province_id(target_id) else {
            alerts.push(Err(format!(
                "Province {target_id} does not exist in the map"
            )));
            return;
        };
        let Some(source_color) = self.bundle.map.color_for_province_id(draft.province_id) else {
            alerts.push(Err(format!(
                "Province {} does not exist in the map",
                draft.province_id
            )));
            return;
        };
        let Some(source_pos) = (0..self.bundle.map.height()).find_map(|y| {
            (0..self.bundle.map.width()).find_map(|x| {
                (self.bundle.map.get_color_at([x, y]) == source_color).then_some([x, y])
            })
        }) else {
            alerts.push(Err(format!(
                "Province {} no longer has pixels",
                draft.province_id
            )));
            return;
        };
        if transfer && !self.province_is_coastal(target_id) {
            let has_naval_base = self
                .state_edit_session
                .as_ref()
                .and_then(|edit| edit.province_data(draft.province_id))
                .is_some_and(|data| {
                    data.buildings
                        .keys()
                        .any(|name| is_coastal_only_building(name))
                });
            if has_naval_base {
                alerts.push(Err(format!(
                    "Province {target_id} is not coastal, so it cannot receive a naval base"
                )));
                return;
            }
        }
        if self
            .bundle
            .map
            .adjacency_references_province_id(draft.province_id)
        {
            alerts.push(Err(format!(
                "Province {} is referenced by adjacencies.csv and cannot be removed until those references are updated",
                draft.province_id
            )));
            return;
        }
        let policy = if transfer {
            ProvinceRemovalPolicy::TransferToProvince(target_id)
        } else {
            ProvinceRemovalPolicy::RemoveReferences
        };
        let map_before = self.history.position();
        let state_before = self
            .state_edit_session
            .as_ref()
            .map_or(0, |edit| edit.summary().commands);
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "Province removal is unavailable".to_owned())
            .and_then(|edit| {
                edit.remove_province_references(draft.province_id, policy)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            alerts.push(Err(error));
            return;
        }
        if self
            .history
            .merge_entire_province(&mut self.bundle, source_pos, target_color)
            .is_none()
        {
            if let Some(edit) = self.state_edit_session.as_mut() {
                edit.undo();
            }
            alerts.push(Err(
                "Province removal could not update the map; references were restored",
            ));
            return;
        }
        self.bundle.map.recalculate_all_boundaries();
        let state_after = self
            .state_edit_session
            .as_ref()
            .map_or(state_before, |edit| edit.summary().commands);
        self.province_removal_undo.push(ProvinceRemovalTransaction {
            map_before,
            map_after: self.history.position(),
            state_before,
            state_after,
        });
        self.province_removal_redo.clear();
        self.problems.clear();
        self.refresh();
        self.sync_province_dirty();
        self.refresh_state_visuals();
        self.active_province_id = Some(target_id);
        self.state_apply_dialog = None;
        self.province_removal_draft = None;
        alerts.push(Ok(format!(
            "Province {} was removed and its references were {}.",
            draft.province_id,
            if transfer { "transferred" } else { "removed" }
        )));
    }

    pub fn open_province_data_editor(&mut self, alerts: &mut Alerts) {
        if self.state_lasso_is_active() {
            alerts.push(Err(
                "Confirm or cancel the state lasso before editing province data",
            ));
            return;
        }
        let Some(province_id) = self.active_province_id else {
            alerts.push(Err("Select a land province before editing province data"));
            return;
        };
        let result = self
            .state_edit_session
            .as_ref()
            .ok_or_else(|| {
                "Province editing is available only for loaded state projects".to_owned()
            })
            .and_then(|edit| {
                let state_id = edit
                    .editable_province_state(province_id)
                    .map_err(|error| error.to_string())?;
                let data = edit
                    .province_data(province_id)
                    .ok_or_else(|| format!("Province {province_id} does not exist in the map"))?;
                Ok((state_id, data))
            });
        let (state_id, data) = match result {
            Ok(result) => result,
            Err(error) => {
                alerts.push(Err(error));
                return;
            }
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

    fn selected_province_position(&self, province_id: u32) -> Option<(usize, usize)> {
        let selected = self.state_edit_session.as_ref()?.selected_provinces();
        let index = selected.iter().position(|id| *id == province_id)?;
        Some((index, selected.len()))
    }

    fn navigate_selected_province(
        &mut self,
        interface: &Interface,
        next: bool,
        alerts: &mut Alerts,
    ) {
        if let Some(draft) = self
            .province_data_draft
            .as_ref()
            .filter(|draft| draft.is_modified())
        {
            let message = if self.validate_province_data_draft(draft).is_ok() {
                "Apply Changes or Discard Province Changes before navigating."
            } else {
                "Fix the province validation errors or discard the draft before navigating."
            };
            alerts.push(Err(message));
            return;
        }
        let selected = self
            .state_edit_session
            .as_ref()
            .map(|edit| {
                edit.selected_provinces()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if selected.len() < 2 {
            alerts.push(Err(
                "Select at least two provinces to navigate the selection.",
            ));
            return;
        }
        let current = self
            .active_province_id
            .and_then(|id| selected.iter().position(|candidate| *candidate == id))
            .unwrap_or(0);
        let index = if next {
            (current + 1) % selected.len()
        } else {
            current.checked_sub(1).unwrap_or(selected.len() - 1)
        };
        let province_id = selected[index];
        self.province_data_draft = None;
        self.active_province_id = Some(province_id);
        self.active_state_id = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.province_state_id(province_id));
        if let Some((_color, province)) = self
            .bundle
            .map
            .iter_province_data()
            .find(|(_, province)| province.preserved_id == Some(province_id))
        {
            self.camera.center_on(interface, province.center_of_mass());
        }
        self.open_province_data_editor(alerts);
    }

    pub fn apply_state_property_draft(&mut self, alerts: &mut Alerts) -> bool {
        if self.state_lifecycle_draft.is_some() {
            return self.apply_state_lifecycle_draft(None, alerts);
        }
        if self.province_data_draft.is_some() {
            return self.apply_province_data_draft(alerts);
        }
        let Some(draft) = self.state_property_draft.as_ref() else {
            return true;
        };
        let state_id = draft.state_id;
        let properties = match draft.validate() {
            Ok(properties) => properties,
            Err(errors) => {
                alerts.push(Err(format!(
                    "Draft has {} validation error(s); no values were applied",
                    errors.len()
                )));
                return false;
            }
        };
        let result = self
            .state_edit_session
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
            }
            Err(error) => {
                alerts.push(Err(error));
                false
            }
        }
    }

    fn can_apply_state_creation(&self, use_selected: bool) -> bool {
        let Some(StateLifecycleDraft::Create { id, properties }) =
            self.state_lifecycle_draft.as_ref()
        else {
            return false;
        };
        let Ok(state_id) = id.trim().parse::<u32>() else {
            return false;
        };
        let Some(edit) = self.state_edit_session.as_ref() else {
            return false;
        };
        properties.validate().is_ok_and(|properties| {
            edit.validate_state_creation(state_id, &properties, use_selected)
                .is_ok()
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
        let Some(edit) = self.state_edit_session.as_ref() else {
            return false;
        };
        let policy = if *unassign {
            StateRemovalPolicy::Unassign
        } else {
            let Ok(target) = target_id.trim().parse::<u32>() else {
                return false;
            };
            StateRemovalPolicy::MoveToState(target)
        };
        edit.validate_state_removal(*state_id, policy).is_ok()
    }

    fn apply_state_lifecycle_draft(
        &mut self,
        use_selected_override: Option<bool>,
        alerts: &mut Alerts,
    ) -> bool {
        let Some(draft) = self.state_lifecycle_draft.clone() else {
            return true;
        };
        let result = match draft {
            StateLifecycleDraft::Create { id, properties } => {
                let state_id = match id.trim().parse::<u32>() {
                    Ok(state_id) => state_id,
                    Err(_) => {
                        alerts.push(Err(
                            "State ID must be an integer within the supported range",
                        ));
                        return false;
                    }
                };
                let properties = match properties.validate() {
                    Ok(properties) => properties,
                    Err(errors) => {
                        alerts.push(Err(format!(
                            "New state draft has {} validation error(s); nothing was created",
                            errors.len()
                        )));
                        return false;
                    }
                };
                let use_selected = use_selected_override.unwrap_or_else(|| {
                    self.state_edit_session
                        .as_ref()
                        .is_some_and(|edit| !edit.selected_provinces().is_empty())
                });
                self.state_edit_session
                    .as_mut()
                    .ok_or_else(|| {
                        "State creation is available only for loaded state projects".to_owned()
                    })
                    .and_then(|edit| {
                        edit.create_state(state_id, properties, use_selected)
                            .map_err(|error| error.to_string())
                    })
                    .map(|_| {
                        (
                            Some(state_id),
                            format!("Created State {state_id} in memory"),
                        )
                    })
            }
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
                        }
                    };
                    StateRemovalPolicy::MoveToState(target_state_id)
                };
                self.state_edit_session
                    .as_mut()
                    .ok_or_else(|| {
                        "State removal is available only for loaded state projects".to_owned()
                    })
                    .and_then(|edit| {
                        edit.remove_state(state_id, policy)
                            .map_err(|error| error.to_string())
                    })
                    .map(|_| {
                        (
                            None,
                            format!("Removed State {state_id} from the in-memory session"),
                        )
                    })
            }
        };
        match result {
            Ok((created_state_id, message)) => {
                self.state_lifecycle_draft = None;
                self.property_editor_replace_field = false;
                if let Some(state_id) = created_state_id {
                    self.cancel_state_lasso();
                    self.cancel_state_brush();
                    self.cancel_state_fill();
                    self.map_tag_picker.cancel();
                    self.inspector_picker = None;
                    self.state_pan_tool = false;
                    let first_province = self.state_edit_session.as_mut().and_then(|edit| {
                        let first = edit
                            .state_data(state_id)
                            .and_then(|state| state.provinces.first().copied());
                        edit.clear_selected_provinces();
                        edit.set_target_state(Some(state_id)).ok();
                        first
                    });
                    self.active_state_id = Some(state_id);
                    self.active_province_id = first_province;
                    self.selected_province_boundaries.clear();
                    if let Some(province_id) = first_province {
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
                    self.active_state_id = self
                        .state_edit_session
                        .as_ref()
                        .and_then(StateEditSession::target_state_id);
                }
                self.refresh_state_visuals();
                alerts.push(Ok(message));
                true
            }
            Err(error) => {
                alerts.push(Err(error));
                false
            }
        }
    }

    fn apply_province_data_draft(&mut self, alerts: &mut Alerts) -> bool {
        let Some(draft) = self.province_data_draft.as_ref() else {
            return true;
        };
        let province_id = draft.province_id;
        let state_id = draft.state_id;
        let data = match self.validate_province_data_draft(draft) {
            Ok(data) => data,
            Err(errors) => {
                alerts.push(Err(format!(
                    "Province draft has {} validation error(s); no values were applied",
                    errors.len()
                )));
                return false;
            }
        };
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| {
                "Province editing is available only for loaded state projects".to_owned()
            })
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
            }
            Err(error) => {
                alerts.push(Err(error));
                false
            }
        }
    }

    fn validate_province_data_draft(
        &self,
        draft: &ProvinceDataDraft,
    ) -> Result<EditableProvinceData, Vec<ProvinceDataValidationError>> {
        let data = draft.validate()?;
        if self.province_is_coastal(draft.province_id)
            || !data
                .buildings
                .keys()
                .any(|name| is_coastal_only_building(name))
        {
            return Ok(data);
        }
        let row = draft
            .buildings
            .iter()
            .position(|building| is_coastal_only_building(&building.name))
            .unwrap_or(0);
        Err(vec![ProvinceDataValidationError {
            field: "Province Buildings".to_owned(),
            field_index: Some(1 + row * 2),
            message: "Naval Base requires a coastal province.".to_owned(),
        }])
    }

    fn province_is_coastal(&self, province_id: u32) -> bool {
        self.bundle
            .map
            .iter_province_data()
            .find(|(_, province)| province.preserved_id == Some(province_id))
            .is_some_and(|(_, province)| province.coastal == Some(true))
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
            alerts.push(Ok(format!(
                "Discarded unapplied draft for State {}",
                draft.state_id
            )));
        }
    }

    pub fn discard_unmodified_property_draft(&mut self) -> bool {
        if self
            .province_data_draft
            .as_ref()
            .is_some_and(|draft| !draft.is_modified())
        {
            self.province_data_draft = None;
            self.property_editor_replace_field = false;
            self.province_editor_page = 0;
            self.refresh_state_information();
            return true;
        }
        if self
            .state_property_draft
            .as_ref()
            .is_some_and(|draft| !draft.is_modified())
        {
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
        if let Some(draft) = self
            .province_data_draft
            .as_ref()
            .filter(|draft| draft.is_modified())
        {
            return self.bundle.map.get_province_at(pos).preserved_id != Some(draft.province_id);
        }
        let Some(draft) = self
            .state_property_draft
            .as_ref()
            .filter(|draft| draft.is_modified())
        else {
            return false;
        };
        let Some(project) = self.project.as_ref() else {
            return true;
        };
        let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps()
        else {
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

    pub fn inspector_picker_is_open(&self) -> bool {
        self.inspector_picker
            .as_ref()
            .is_some_and(|state| state.picker.is_open())
    }

    pub fn inspector_picker_cancel(&mut self) {
        self.inspector_picker = None;
    }

    pub fn inspector_picker_backspace(&mut self) {
        if let Some(state) = self.inspector_picker.as_mut() {
            let mut query = state.picker.query().to_owned();
            query.pop();
            state.picker.set_query(query);
        }
    }

    pub fn inspector_picker_move(&mut self, next: bool) {
        if let Some(state) = self.inspector_picker.as_mut() {
            if next {
                state.picker.next(String::as_str);
            } else {
                state.picker.previous(String::as_str);
            }
        }
    }

    pub fn inspector_picker_page(&mut self, next: bool) {
        if let Some(state) = self.inspector_picker.as_mut() {
            state.picker.page(next, String::as_str);
        }
    }

    pub fn inspector_picker_home(&mut self) {
        if let Some(state) = self.inspector_picker.as_mut() {
            state.picker.home(String::as_str);
        }
    }

    pub fn inspector_picker_end(&mut self) {
        if let Some(state) = self.inspector_picker.as_mut() {
            state.picker.end(String::as_str);
        }
    }

    pub fn inspector_picker_confirm(&mut self, alerts: &mut Alerts) {
        let chosen = self.inspector_picker.as_mut().and_then(|state| {
            state
                .picker
                .confirm(String::as_str)
                .cloned()
                .map(|value| (state.target, value))
        });
        if let Some((target, value)) = chosen {
            self.apply_inspector_picker_value(target, value, alerts);
        }
        self.inspector_picker = None;
    }

    fn open_inspector_picker(&mut self, target: InspectorPickTarget, alerts: &mut Alerts) {
        let mut options: Vec<String> = match target {
            InspectorPickTarget::StateCategory => self
                .definition_catalog
                .as_ref()
                .map(|catalog| catalog.state_categories.keys().cloned().collect()),
            InspectorPickTarget::Owner
            | InspectorPickTarget::Controller
            | InspectorPickTarget::Core
            | InspectorPickTarget::Claim => self
                .definition_catalog
                .as_ref()
                .map(|catalog| catalog.country_tags.keys().cloned().collect()),
            InspectorPickTarget::Resource => self
                .definition_catalog
                .as_ref()
                .map(|catalog| catalog.resources.keys().cloned().collect()),
            InspectorPickTarget::StateBuilding => self.definition_catalog.as_ref().map(|catalog| {
                catalog
                    .buildings
                    .iter()
                    .filter(|(_, entry)| {
                        building_matches_picker_scope(
                            InspectorPickTarget::StateBuilding,
                            entry.scope,
                        )
                    })
                    .map(|(name, _)| name.clone())
                    .collect()
            }),
            InspectorPickTarget::ProvinceBuilding => {
                self.definition_catalog.as_ref().map(|catalog| {
                    catalog
                        .buildings
                        .iter()
                        .filter(|(_, entry)| {
                            building_matches_picker_scope(
                                InspectorPickTarget::ProvinceBuilding,
                                entry.scope,
                            )
                        })
                        .map(|(name, _)| name.clone())
                        .collect()
                })
            }
        }
        .unwrap_or_default();
        if let Some(draft) = self.active_state_property_draft() {
            match target {
                InspectorPickTarget::StateCategory => {
                    options.push(draft.state_category.trim().to_owned());
                }
                InspectorPickTarget::Owner => options.push(draft.owner.trim().to_owned()),
                InspectorPickTarget::Controller => {
                    options.push(draft.controller.trim().to_owned());
                }
                InspectorPickTarget::Core => options.extend(
                    draft
                        .cores
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                ),
                InspectorPickTarget::Claim => options.extend(
                    draft
                        .claims
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                ),
                _ => {}
            }
        }
        options.retain(|value| !value.is_empty());
        options.sort();
        options.dedup();
        if options.is_empty() {
            alerts.push(Err("No catalog values are available for this selector"));
            return;
        }
        let mut picker = SearchablePicker::new(options);
        picker.open();
        self.inspector_picker = Some(InspectorPickerState { target, picker });
    }

    fn apply_inspector_picker_value(
        &mut self,
        target: InspectorPickTarget,
        value: String,
        alerts: &mut Alerts,
    ) {
        let province_building_blocked = target == InspectorPickTarget::ProvinceBuilding
            && is_coastal_only_building(&value)
            && self
                .province_data_draft
                .as_ref()
                .is_some_and(|draft| !self.province_is_coastal(draft.province_id));
        let Some(draft) = self.active_state_property_draft_mut() else {
            if target == InspectorPickTarget::ProvinceBuilding
                && let Some(draft) = self.province_data_draft.as_mut()
            {
                if province_building_blocked {
                    alerts.push(Err("Naval Base requires a coastal province."));
                    return;
                }
                if !draft
                    .buildings
                    .iter()
                    .any(|building| building.name.eq_ignore_ascii_case(&value))
                {
                    let row = draft.add_building();
                    draft.buildings[row].name = value;
                }
            }
            return;
        };
        let result = match target {
            InspectorPickTarget::StateCategory => {
                draft.state_category = value;
                Ok(())
            }
            InspectorPickTarget::Owner => {
                draft.owner = value;
                Ok(())
            }
            InspectorPickTarget::Controller => {
                draft.controller = value;
                Ok(())
            }
            InspectorPickTarget::Core => {
                append_unique_csv(&mut draft.cores, &value);
                Ok(())
            }
            InspectorPickTarget::Claim => {
                append_unique_csv(&mut draft.claims, &value);
                Ok(())
            }
            InspectorPickTarget::Resource => {
                if draft
                    .resource_values()
                    .is_ok_and(|values| values.contains_key(&value))
                {
                    Ok(())
                } else {
                    draft
                        .set_resource(&value, 1)
                        .map_err(|errors| errors[0].message.clone())
                }
            }
            InspectorPickTarget::StateBuilding => {
                if draft
                    .state_building_values()
                    .is_ok_and(|values| values.contains_key(&value))
                {
                    Ok(())
                } else {
                    draft
                        .set_state_building(&value, 1)
                        .map_err(|errors| errors[0].message.clone())
                }
            }
            InspectorPickTarget::ProvinceBuilding => Ok(()),
        };
        if let Err(error) = result {
            alerts.push(Err(error));
        }
    }

    fn active_state_property_draft(&self) -> Option<&StatePropertyDraft> {
        self.state_property_draft.as_ref().or_else(|| {
            let StateLifecycleDraft::Create { properties, .. } =
                self.state_lifecycle_draft.as_ref()?
            else {
                return None;
            };
            Some(properties)
        })
    }

    fn active_state_property_draft_mut(&mut self) -> Option<&mut StatePropertyDraft> {
        if self.state_property_draft.is_some() {
            return self.state_property_draft.as_mut();
        }
        let StateLifecycleDraft::Create { properties, .. } = self.state_lifecycle_draft.as_mut()?
        else {
            return None;
        };
        Some(properties)
    }

    pub fn map_tag_picker_cancel(&mut self) -> bool {
        if self.map_tag_picker.active_target().is_none() {
            return false;
        }
        self.map_tag_picker.cancel();
        true
    }

    pub fn pick_tag_from_map(
        &mut self,
        interface: &Interface,
        cursor_pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) -> bool {
        let Some(target) = self.map_tag_picker.active_target() else {
            return false;
        };
        let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) else {
            return true;
        };
        let Some(province_id) = self.bundle.map.get_province_at(pos).preserved_id else {
            alerts.push(Err("No province under cursor"));
            return true;
        };
        let tag = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| {
                edit.province_state_id(province_id)
                    .and_then(|id| edit.state_data(id))
            })
            .and_then(|data| data.history.owner);
        let Some((target, tag)) = self.map_tag_picker.pick(tag) else {
            alerts.push(Err(
                "The clicked state has no owner; map picker remains available",
            ));
            self.map_tag_picker.begin(target);
            return true;
        };
        let pick_target = match target {
            MapTagPickTarget::Owner => InspectorPickTarget::Owner,
            MapTagPickTarget::Controller => InspectorPickTarget::Controller,
            MapTagPickTarget::Core => InspectorPickTarget::Core,
            MapTagPickTarget::Claim => InspectorPickTarget::Claim,
        };
        self.apply_inspector_picker_value(pick_target, tag, alerts);
        alerts.push(Ok("Copied the clicked state's owner into the active draft"));
        true
    }

    pub fn state_inspector_click(
        &mut self,
        interface: &Interface,
        pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) -> (bool, Option<InspectorExternalRequest>) {
        if self.project.is_none() || self.inspector.visibility == StateInspectorVisibility::Hidden {
            return (false, None);
        }
        let layout = InspectorCanvasLayout::new(interface);
        if self.inspector_picker_is_open() {
            let popup = inspector_picker_rect(layout);
            if point_in_rect(pos, popup) {
                if point_in_rect(pos, inspector_picker_up_rect(popup)) {
                    if let Some(state) = self.inspector_picker.as_mut() {
                        state.picker.scroll_by(-1, String::as_str);
                    }
                    return (true, None);
                }
                if point_in_rect(pos, inspector_picker_down_rect(popup)) {
                    if let Some(state) = self.inspector_picker.as_mut() {
                        state.picker.scroll_by(1, String::as_str);
                    }
                    return (true, None);
                }
                let row = ((pos[1] - popup[1] - 34.0) / 22.0).floor() as isize;
                if row >= 0 {
                    let chosen = self.inspector_picker.as_mut().and_then(|state| {
                        state
                            .picker
                            .click(row as usize, String::as_str)
                            .cloned()
                            .map(|value| (state.target, value))
                    });
                    if let Some((target, value)) = chosen {
                        self.apply_inspector_picker_value(target, value, alerts);
                        self.inspector_picker = None;
                    }
                }
                return (true, None);
            }
            self.inspector_picker_cancel();
            return (true, None);
        }
        if !point_in_rect(pos, layout.panel) {
            return (false, None);
        }
        if self.inspector.visibility == StateInspectorVisibility::Collapsed {
            self.inspector.visibility = StateInspectorVisibility::Expanded;
            return (true, None);
        }
        if point_in_rect(pos, layout.collapse()) {
            self.inspector.visibility = StateInspectorVisibility::Collapsed;
            self.inspector_search_focused = false;
            return (true, None);
        }
        let source = self.active_state_source();
        if point_in_rect(pos, layout.open_source()) {
            return match source {
                Some(path) => (true, Some(InspectorExternalRequest::OpenSource(path))),
                None => {
                    alerts.push(Err("This state has no source file"));
                    (true, None)
                }
            };
        }
        if point_in_rect(pos, layout.copy_path()) {
            return match source {
                Some(path) => (
                    true,
                    Some(InspectorExternalRequest::CopyPath(
                        path.display().to_string(),
                    )),
                ),
                None => {
                    alerts.push(Err("This state has no source path"));
                    (true, None)
                }
            };
        }
        if point_in_rect(pos, layout.search()) {
            self.focus_map_search();
            return (true, None);
        }
        if pos[1] >= layout.panel[1] + 112.0 && pos[1] < layout.panel[1] + 135.0 {
            self.inspector_search_select(interface, false, alerts);
            return (true, None);
        }
        for (index, icon) in INSPECTOR_ICONS.iter().enumerate() {
            if point_in_rect(pos, layout.section(index)) {
                self.inspector.active_section = icon.section;
                self.inspector.scroll_y = 0.0;
                self.inspector_search_focused = false;
                return (true, None);
            }
        }
        if point_in_rect(pos, layout.footer_left()) {
            if self.state_property_draft.is_some() {
                self.apply_state_property_draft(alerts);
            } else if self.province_data_draft.is_some() {
                self.apply_province_data_draft(alerts);
            } else {
                self.open_state_property_editor(alerts);
            }
            return (true, None);
        }
        if point_in_rect(pos, layout.footer_right()) {
            if self.state_property_draft.is_some() || self.province_data_draft.is_some() {
                self.discard_state_property_draft(alerts);
            } else {
                self.open_province_data_editor(alerts);
            }
            return (true, None);
        }
        if point_in_rect(pos, layout.body) {
            self.inspector_search_focused = false;
            if self.state_property_draft.is_some() {
                let controls = self.inspector_draft_controls(layout);
                if let Some(id) = controls.iter().find_map(|control| control.hit_test(pos)) {
                    self.handle_inspector_control(id, alerts);
                    return (true, None);
                }
                let row = ((pos[1] - layout.body[1] + self.inspector.scroll_y) / 19.0)
                    .floor()
                    .max(0.0) as usize;
                let field = match self.inspector.active_section {
                    super::inspector::InspectorSection::Overview if row < 5 => Some(row),
                    super::inspector::InspectorSection::Overview if row == 5 => {
                        if let Some(draft) = self.state_property_draft.as_mut() {
                            draft.impassable = !draft.impassable;
                        }
                        None
                    }
                    super::inspector::InspectorSection::History if row < 4 => Some(5 + row),
                    super::inspector::InspectorSection::Resources => Some(9),
                    super::inspector::InspectorSection::Buildings => Some(10),
                    _ => None,
                };
                if let Some(field) = field {
                    self.property_editor_field = field;
                    self.property_editor_replace_field = false;
                }
            }
            return (true, None);
        }
        (true, None)
    }

    fn inspector_draft_controls(&self, layout: InspectorCanvasLayout) -> Vec<InspectorControlRect> {
        if self.state_property_draft.is_none() {
            return Vec::new();
        }
        let control_layout = InspectorControlLayout::new(
            [layout.body[0], layout.body[1]],
            layout.body[2],
            0.0,
            self.inspector.scroll_y,
            1.0,
        );
        let mut controls = Vec::new();
        match self.inspector.active_section {
            super::inspector::InspectorSection::Overview => {
                controls.extend(overview_property_controls(layout, control_layout));
            }
            super::inspector::InspectorSection::History => {
                for (row, target, map_target) in [
                    (0, InspectorPickTarget::Owner, MapTagPickTarget::Owner),
                    (
                        1,
                        InspectorPickTarget::Controller,
                        MapTagPickTarget::Controller,
                    ),
                    (2, InspectorPickTarget::Core, MapTagPickTarget::Core),
                    (3, InspectorPickTarget::Claim, MapTagPickTarget::Claim),
                ] {
                    controls.push(control_layout.body_control(
                        InspectorControlId::Select(target),
                        row,
                        layout.body[2] - 190.0,
                        96.0,
                        16.0,
                    ));
                    controls.push(control_layout.body_control(
                        InspectorControlId::MapPick(map_target),
                        row,
                        layout.body[2] - 88.0,
                        83.0,
                        16.0,
                    ));
                }
            }
            super::inspector::InspectorSection::Resources => {
                let count = self
                    .state_property_draft
                    .as_ref()
                    .and_then(|draft| draft.resource_values().ok())
                    .map_or(0, |values| values.len());
                for index in 0..count {
                    controls.extend(
                        control_layout
                            .numeric_stepper(index + 1, InspectorValueTarget::Resource(index)),
                    );
                    controls.push(control_layout.body_control(
                        InspectorControlId::RemoveValue(InspectorValueTarget::Resource(index)),
                        index + 1,
                        layout.body[2] - 173.0,
                        75.0,
                        16.0,
                    ));
                }
                controls.push(control_layout.body_control(
                    InspectorControlId::Add(InspectorPickTarget::Resource),
                    count + 1,
                    layout.body[2] - 150.0,
                    145.0,
                    16.0,
                ));
            }
            super::inspector::InspectorSection::Buildings => {
                let count = self
                    .state_property_draft
                    .as_ref()
                    .and_then(|draft| draft.state_building_values().ok())
                    .map_or(0, |values| values.len());
                for index in 0..count {
                    controls.extend(
                        control_layout
                            .numeric_stepper(index + 1, InspectorValueTarget::StateBuilding(index)),
                    );
                    controls.push(control_layout.body_control(
                        InspectorControlId::RemoveValue(InspectorValueTarget::StateBuilding(index)),
                        index + 1,
                        layout.body[2] - 173.0,
                        75.0,
                        16.0,
                    ));
                }
                controls.push(control_layout.body_control(
                    InspectorControlId::Add(InspectorPickTarget::StateBuilding),
                    count + 1,
                    layout.body[2] - 170.0,
                    165.0,
                    16.0,
                ));
            }
            _ => {}
        }
        controls
    }

    fn handle_inspector_control(&mut self, id: InspectorControlId, alerts: &mut Alerts) {
        match id {
            InspectorControlId::Select(target) | InspectorControlId::Add(target) => {
                if target == InspectorPickTarget::StateCategory {
                    self.property_editor_field = 2;
                    self.property_editor_replace_field = false;
                }
                self.open_inspector_picker(target, alerts);
            }
            InspectorControlId::MapPick(target) => {
                self.cancel_state_lasso();
                self.deactivate_state_brush();
                self.cancel_state_fill();
                self.state_pan_tool = false;
                self.map_tag_picker.begin(target);
                alerts.push(Ok(
                    "Map tag picker active — click a state's owner; Esc cancels",
                ));
            }
            InspectorControlId::Decrement(target) | InspectorControlId::Increment(target) => {
                let increment = matches!(id, InspectorControlId::Increment(_));
                self.adjust_inspector_value(target, increment, alerts);
            }
            InspectorControlId::RemoveValue(target) => {
                let Some(draft) = self.state_property_draft.as_mut() else {
                    return;
                };
                match target {
                    InspectorValueTarget::Resource(index) => {
                        if let Some(name) = draft
                            .resource_values()
                            .ok()
                            .and_then(|values| values.keys().nth(index).cloned())
                        {
                            let _ = draft.remove_resource(&name);
                        }
                    }
                    InspectorValueTarget::StateBuilding(index) => {
                        if let Some(name) = draft
                            .state_building_values()
                            .ok()
                            .and_then(|values| values.keys().nth(index).cloned())
                        {
                            let _ = draft.remove_state_building(&name);
                        }
                    }
                    _ => {}
                }
            }
            InspectorControlId::Field(field) => {
                self.property_editor_field = field;
                self.property_editor_replace_field = false;
            }
            InspectorControlId::Toggle(5) => {
                self.property_editor_field = StatePropertyDraft::TEXT_FIELD_COUNT;
                self.property_editor_replace_field = false;
                if let Some(draft) = self.state_property_draft.as_mut() {
                    draft.impassable = !draft.impassable;
                }
            }
            _ => {}
        }
    }

    fn adjust_inspector_value(
        &mut self,
        target: InspectorValueTarget,
        increment: bool,
        alerts: &mut Alerts,
    ) {
        let Some(draft) = self.state_property_draft.as_mut() else {
            return;
        };
        match target {
            InspectorValueTarget::Manpower => {
                let value = parse_grouped_nonnegative_integer(&draft.manpower).unwrap_or(0);
                draft.manpower = if increment {
                    value.saturating_add(1)
                } else {
                    value.saturating_sub(1)
                }
                .to_string();
            }
            InspectorValueTarget::BuildingsMaxLevelFactor | InspectorValueTarget::LocalSupplies => {
                let field = if target == InspectorValueTarget::BuildingsMaxLevelFactor {
                    &mut draft.buildings_max_level_factor
                } else {
                    &mut draft.local_supplies
                };
                let value = field.trim().parse::<f64>().unwrap_or(0.0);
                *field = format!(
                    "{:.3}",
                    (value + if increment { 0.1 } else { -0.1 }).max(0.0)
                );
            }
            InspectorValueTarget::Resource(index) => {
                let Some((name, value)) = draft
                    .resource_values()
                    .ok()
                    .and_then(|values| values.into_iter().nth(index))
                else {
                    return;
                };
                let value = if increment {
                    value.saturating_add(1)
                } else {
                    value.saturating_sub(1)
                };
                if let Err(errors) = draft.set_resource(&name, value) {
                    alerts.push(Err(errors[0].message.clone()));
                }
            }
            InspectorValueTarget::StateBuilding(index) => {
                let Some((name, value)) = draft
                    .state_building_values()
                    .ok()
                    .and_then(|values| values.into_iter().nth(index))
                else {
                    return;
                };
                let value = if increment {
                    value.saturating_add(1)
                } else {
                    value.saturating_sub(1)
                };
                if let Err(errors) = draft.set_state_building(&name, value) {
                    alerts.push(Err(errors[0].message.clone()));
                }
            }
        }
    }

    fn inspector_search_results(&self) -> Vec<InspectorSearchResult> {
        let Some(edit) = self.state_edit_session.as_ref() else {
            return Vec::new();
        };
        if self.is_state_workspace() {
            let index =
                StateSearchIndex::new(edit.valid_state_ids().iter().filter_map(|state_id| {
                    let data = edit.state_data(*state_id)?;
                    Some(
                        StateSearchEntry::new(
                            *state_id,
                            data.name.unwrap_or_else(|| "<unnamed>".to_owned()),
                        )
                        .with_context(
                            data.history.owner,
                            data.history.controller,
                            data.provinces,
                        ),
                    )
                }));
            return index
                .search(&self.inspector.search)
                .into_iter()
                .map(InspectorSearchResult::State)
                .collect();
        }

        let index = ProvinceSearchIndex::new(self.bundle.map.iter_province_data().filter_map(
            |(rgb, province)| {
                let province_id = province.preserved_id?;
                let state_id = edit.province_state_id(province_id);
                let state_name = state_id
                    .and_then(|state_id| edit.state_data(state_id))
                    .and_then(|data| data.name);
                Some(ProvinceSearchEntry {
                    province_id,
                    rgb,
                    kind: province.kind.to_str().to_owned(),
                    terrain: province.terrain.clone(),
                    coastal: province.coastal == Some(true),
                    continent: province.continent,
                    state_id,
                    state_name,
                })
            },
        ));
        index
            .search(&self.inspector.search)
            .into_iter()
            .map(InspectorSearchResult::Province)
            .collect()
    }

    fn select_state_by_id(&mut self, interface: &Interface, state_id: u32, alerts: &mut Alerts) {
        if self.property_draft_is_modified() {
            alerts.push(Err(
                "Apply or discard the modified draft before changing state",
            ));
            return;
        }
        self.discard_unmodified_property_draft();
        let Some(edit) = self.state_edit_session.as_ref() else {
            return;
        };
        let Some(data) = edit.state_data(state_id) else {
            alerts.push(Err("State is invalid or unavailable"));
            return;
        };
        self.active_state_id = Some(state_id);
        self.active_province_id = data.provinces.first().copied();
        if self.state_province_extents.is_none() {
            self.state_province_extents = Some(self.bundle.map.province_extents_by_id());
        }
        if let Some(extents) = self.state_province_extents.as_ref() {
            let mut state_extents = data
                .provinces
                .iter()
                .filter_map(|province_id| extents.get(province_id).copied());
            if let Some(mut combined) = state_extents.next() {
                for extents in state_extents {
                    combined = combined.join(extents);
                }
                self.camera.focus_extents(interface, combined, 4.0);
            }
        }
        self.state_selection = self
            .active_province_id
            .map(|province_id| StateSelection::State {
                state_id,
                province_id,
            });
        if let Some(edit) = self.state_edit_session.as_mut() {
            edit.set_target_state(Some(state_id)).ok();
        }
        if let (Some(project), Some(edit)) =
            (self.project.as_ref(), self.state_edit_session.as_ref())
        {
            let ambiguous = project.ambiguous_provinces.keys().copied().collect();
            let image = selection_overlay_for(
                &self.bundle.map,
                edit.state_by_province(),
                &ambiguous,
                self.state_selection.as_ref(),
            );
            let settings = TextureSettings::new().mag(Filter::Nearest);
            self.selection_texture = Some(Texture::from_image(&image, &settings));
            self.selected_state_boundaries = boundaries_for_state(
                &self.bundle.map,
                edit.state_by_province(),
                &self.state_boundaries,
                state_id,
            );
        }
        self.refresh_state_information();
        alerts.push(Ok(format!("Selected State {state_id} from map search")));
    }

    fn select_province_by_id(
        &mut self,
        interface: &Interface,
        province_id: u32,
        alerts: &mut Alerts,
    ) {
        if self.property_draft_is_modified() {
            alerts.push(Err(
                "Apply or discard the modified draft before changing province",
            ));
            return;
        }
        self.discard_unmodified_property_draft();
        let Some((_rgb, province)) = self
            .bundle
            .map
            .iter_province_data()
            .find(|(_, province)| province.preserved_id == Some(province_id))
        else {
            alerts.push(Err(format!("Province {province_id} is unavailable")));
            return;
        };
        let kind = province.kind;
        let center = province.center_of_mass();
        let state_id = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.province_state_id(province_id));
        self.active_province_id = Some(province_id);
        self.active_state_id = state_id;
        self.camera.ensure_scale(interface, 1.5);
        self.camera.center_on(interface, center);
        if let Some(edit) = self.state_edit_session.as_mut() {
            let selected = BTreeSet::from([province_id]);
            if edit
                .apply_lasso_selection(&selected, LassoSelectionMode::Replace)
                .is_err()
            {
                edit.clear_selected_provinces();
            }
        }
        self.refresh_selected_province_boundaries();
        self.state_selection = state_id
            .map(|state_id| StateSelection::State {
                state_id,
                province_id,
            })
            .or_else(|| {
                (kind == ProvinceKind::Land)
                    .then_some(StateSelection::UnassignedProvince { province_id })
            });
        self.refresh_state_target_overlay();
        self.refresh_state_information();
        alerts.push(Ok(format!(
            "Focused Province {province_id} from map search"
        )));
    }

    fn active_state_source(&self) -> Option<PathBuf> {
        let state_id = self.active_state_id?;
        match self.state_edit_session.as_ref()?.state_origin(state_id)? {
            WorkingStateOrigin::Loaded { document_path } => Some(document_path.clone()),
            WorkingStateOrigin::CreatedInSession => None,
        }
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
        let Some(draft) = self.state_property_draft.as_ref() else {
            return false;
        };
        if self.inspector.visibility != StateInspectorVisibility::Hidden
            && !interface.inspector_contains(pos)
        {
            if draft.is_modified() && draft.validate().is_ok() {
                self.apply_state_property_draft(alerts);
            }
            return true;
        }
        let layout = PropertyEditorLayout::new(interface);
        if !point_in_rect(pos, layout.panel) {
            return true;
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
                    return true;
                }
                for index in 0..=StatePropertyDraft::TEXT_FIELD_COUNT {
                    if point_in_rect(pos, layout.field(index)) {
                        let picker = state_creation_picker(index);
                        if let Some(target) = picker {
                            self.open_inspector_picker(target, alerts);
                            return true;
                        }
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
            }
            Some(StateLifecycleDraft::Remove { .. }) => {
                let layout = StateRemovalEditorLayout::new(interface);
                if !point_in_rect(pos, layout.panel) {
                    return true;
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
            }
            None => false,
        }
    }

    fn province_data_editor_click(
        &mut self,
        interface: &Interface,
        pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) -> bool {
        let Some(draft) = self.province_data_draft.as_ref() else {
            return false;
        };
        let layout =
            ProvinceEditorLayout::new(interface, draft.buildings.len(), self.province_editor_page);
        if !point_in_rect(pos, layout.panel) {
            return true;
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
        if point_in_rect(pos, layout.previous_selected()) {
            self.navigate_selected_province(interface, false, alerts);
            return true;
        }
        if point_in_rect(pos, layout.next_selected()) {
            self.navigate_selected_province(interface, true, alerts);
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
            self.open_inspector_picker(InspectorPickTarget::ProvinceBuilding, alerts);
            return true;
        }
        if point_in_rect(pos, layout.previous_page()) && self.province_editor_page > 0 {
            self.province_editor_page -= 1;
            return true;
        }
        if point_in_rect(pos, layout.next_page()) && layout.has_next_page(self.province_editor_page)
        {
            self.province_editor_page += 1;
            return true;
        }
        if point_in_rect(pos, layout.apply()) {
            if draft.is_modified() && self.validate_province_data_draft(draft).is_ok() {
                self.apply_province_data_draft(alerts);
            }
            return true;
        }
        if point_in_rect(pos, layout.discard()) {
            self.discard_state_property_draft(alerts);
            return true;
        }
        if point_in_rect(pos, layout.close()) {
            if draft.is_modified() {
                alerts.push(Err(
                    "Apply Changes or Discard Province Changes before closing.",
                ));
            } else {
                self.province_data_draft = None;
                self.refresh_state_information();
            }
            return true;
        }
        true
    }

    pub fn input_state_property_text(&mut self, text: &str) {
        if let Some(state) = self.inspector_picker.as_mut() {
            let mut query = state.picker.query().to_owned();
            query.extend(text.chars().filter(|character| !character.is_control()));
            state.picker.set_query(query);
            return;
        }
        if self.inspector_search_focused {
            self.inspector
                .search
                .extend(text.chars().filter(|character| !character.is_control()));
            self.inspector_search_index = 0;
            return;
        }
        if let Some(draft) = self.state_lifecycle_draft.as_mut() {
            let Some(field) = draft.field_mut(self.property_editor_field) else {
                return;
            };
            if self.property_editor_replace_field {
                field.clear();
                self.property_editor_replace_field = false;
            }
            field.extend(text.chars().filter(|character| !character.is_control()));
            return;
        }
        if let Some(draft) = self.province_data_draft.as_mut() {
            let Some(field) = draft.field_mut(self.property_editor_field) else {
                return;
            };
            if self.property_editor_replace_field {
                field.clear();
                self.property_editor_replace_field = false;
            }
            field.extend(text.chars().filter(|character| !character.is_control()));
            return;
        }
        let Some(draft) = self.state_property_draft.as_mut() else {
            return;
        };
        let Some(field) = draft.field_mut(self.property_editor_field) else {
            return;
        };
        if self.property_editor_replace_field {
            field.clear();
            self.property_editor_replace_field = false;
        }
        field.extend(text.chars().filter(|character| !character.is_control()));
    }

    pub fn state_property_editor_select_all(&mut self) {
        if self
            .state_lifecycle_draft
            .as_ref()
            .and_then(|draft| draft.field(self.property_editor_field))
            .is_some()
        {
            self.property_editor_replace_field = true;
            return;
        }
        if self
            .province_data_draft
            .as_ref()
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

    pub fn cancel_state_property_field_edit(&mut self) -> bool {
        let Some(draft) = self.state_property_draft.as_mut() else {
            return false;
        };
        if !draft.restore_field(self.property_editor_field) {
            return false;
        }
        self.property_editor_replace_field = false;
        self.refresh_state_information();
        true
    }

    pub fn state_property_editor_backspace(&mut self) {
        if let Some(draft) = self.state_lifecycle_draft.as_mut() {
            let Some(field) = draft.field_mut(self.property_editor_field) else {
                return;
            };
            if self.property_editor_replace_field {
                field.clear();
                self.property_editor_replace_field = false;
            } else {
                field.pop();
            }
            return;
        }
        if let Some(draft) = self.province_data_draft.as_mut() {
            let Some(field) = draft.field_mut(self.property_editor_field) else {
                return;
            };
            if self.property_editor_replace_field {
                field.clear();
                self.property_editor_replace_field = false;
            } else {
                field.pop();
            }
            return;
        }
        let Some(draft) = self.state_property_draft.as_mut() else {
            return;
        };
        let Some(field) = draft.field_mut(self.property_editor_field) else {
            return;
        };
        if self.property_editor_replace_field {
            field.clear();
            self.property_editor_replace_field = false;
        } else {
            field.pop();
        }
    }

    pub fn state_property_editor_clear_field(&mut self) {
        if let Some(field) = self
            .state_lifecycle_draft
            .as_mut()
            .and_then(|draft| draft.field_mut(self.property_editor_field))
        {
            field.clear();
            self.property_editor_replace_field = false;
            return;
        }
        if let Some(field) = self
            .province_data_draft
            .as_mut()
            .and_then(|draft| draft.field_mut(self.property_editor_field))
        {
            field.clear();
            self.property_editor_replace_field = false;
            return;
        }
        if let Some(field) = self
            .state_property_draft
            .as_mut()
            .and_then(|draft| draft.field_mut(self.property_editor_field))
        {
            field.clear();
            self.property_editor_replace_field = false;
        }
    }

    pub fn state_property_editor_next_field(&mut self, backwards: bool) {
        if self.state_property_draft.is_some()
            && self.inspector.visibility != StateInspectorVisibility::Hidden
            && self.inspector.active_section == super::inspector::InspectorSection::Overview
        {
            let fields = [0, 1, 2, 3, 4, StatePropertyDraft::TEXT_FIELD_COUNT];
            let current = fields
                .iter()
                .position(|field| *field == self.property_editor_field)
                .unwrap_or(0);
            let next = if backwards {
                current.checked_sub(1).unwrap_or(fields.len() - 1)
            } else {
                (current + 1) % fields.len()
            };
            self.property_editor_field = fields[next];
            self.property_editor_replace_field = false;
            return;
        }
        let count = self
            .state_lifecycle_draft
            .as_ref()
            .map(|draft| match draft {
                StateLifecycleDraft::Create { .. } => draft.text_field_count() + 1,
                StateLifecycleDraft::Remove { .. } => draft.text_field_count(),
            })
            .or_else(|| {
                self.province_data_draft
                    .as_ref()
                    .map(ProvinceDataDraft::text_field_count)
            })
            .unwrap_or(StatePropertyDraft::TEXT_FIELD_COUNT + 1)
            .max(1);
        self.property_editor_field = if backwards {
            self.property_editor_field
                .checked_sub(1)
                .unwrap_or(count - 1)
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

    pub fn apply_config(&mut self, config: Config) {
        self.history.set_limit(config.max_undo_states);
        self.bundle.config = config;
        self.problems = self.bundle.generate_problems();
        self.unknown_terrains = self.bundle.search_unknown_terrains();
        if self.view_mode == ViewMode::Terrain {
            self.refresh();
        }
    }

    pub fn draw(
        &mut self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        use super::alerts::PADDING;

        self.poll_round_trip_validation();
        self.poll_state_save();
        self.poll_province_save();
        if self.political_language != crate::localization::language() {
            self.reload_political_country_catalog();
        }
        debug_assert!(
            self.political_cache_generation
                .is_none_or(|generation| generation == self.project_generation)
        );
        debug_assert!(
            self.resource_cache_generation
                .is_none_or(|generation| generation == self.project_generation)
        );
        debug_assert!(
            self.territory_anchor_generation
                .is_none_or(|generation| generation == self.project_generation)
        );
        let transform = ctx
            .transform
            .append_transform(self.camera.display_matrix(interface));
        let state_workspace = self.is_state_workspace();
        let texture = match self.map_layers.base_view {
            MapBaseView::ProvinceColors
            | MapBaseView::ProvinceTypes
            | MapBaseView::Terrain
            | MapBaseView::Continents
            | MapBaseView::Coastal => &self.texture,
            MapBaseView::States | MapBaseView::Resources => {
                self.state_texture.as_ref().unwrap_or(&self.texture)
            }
            MapBaseView::Political => self.political_texture.as_ref().unwrap_or(&self.texture),
        };
        graphics::image(texture, transform, gl);
        if self.map_layers.image_overlay.enabled
            && let Some(image_overlay) = &self.image_overlay_texture
        {
            graphics::Image::new_color([1.0, 1.0, 1.0, self.map_layers.image_overlay.opacity])
                .draw(
                    image_overlay,
                    &graphics::DrawState::default(),
                    transform,
                    gl,
                );
        }

        let rivers = self
            .bundle
            .map
            .get_rivers_overlay()
            .filter(|_| self.map_layers.show_rivers)
            .map(|rivers_overlay| {
                self.texture_overlay.get_or_insert_with(|| {
                    let texture_settings = TextureSettings::new().mag(Filter::Nearest);
                    Texture::from_image(rivers_overlay, &texture_settings)
                })
            });
        if let Some(rivers) = rivers {
            graphics::image(rivers, transform, gl);
        }
        if political_labels_visible_in_view(self.map_layers.base_view) {
            self.draw_political_country_labels(ctx, interface, glyph_cache, gl);
        }
        if self.map_layers.show_resources || self.map_layers.base_view == MapBaseView::Resources {
            self.draw_resource_labels(ctx, interface, glyph_cache, gl);
        }
        if self.map_layers.show_adjacencies {
            self.draw_adjacencies(ctx, interface, cursor_pos, gl);
        }
        if self.camera.scale_factor() > 1.0 && self.map_layers.show_province_boundaries {
            self.draw_boundaries(ctx, interface, gl);
        }
        if self.map_layers.show_province_ids
            && self.map_layers.province_label_mode != ProvinceLabelMode::Off
            && (self.map_layers.province_label_mode != ProvinceLabelMode::All
                || self.camera.scale_factor() > 1.0)
        {
            self.draw_ids(ctx, interface, glyph_cache, cursor_pos, gl);
        }
        if state_workspace && self.map_layers.show_state_boundaries {
            self.draw_boundary_set(
                ctx,
                interface,
                &self.state_boundaries,
                [0.07, 0.07, 0.07, 1.0],
                1.25,
                gl,
            );
        }

        if state_workspace && let Some(selection_texture) = &self.selection_texture {
            let viewport = interface.get_map_viewport();
            graphics::rectangle(
                colors::OVERLAY_T,
                [viewport.x, viewport.y, viewport.width, viewport.height],
                ctx.transform,
                gl,
            );
            graphics::image(selection_texture, transform, gl);
            self.draw_selected_state_boundaries(ctx, interface, gl);
        };
        if state_workspace {
            self.draw_selected_province_boundaries(ctx, interface, gl);
            self.draw_state_lasso(ctx, interface, cursor_pos, gl);
            self.draw_state_brush(ctx, interface, gl);
            self.draw_state_fill(ctx, interface, gl);
        }

        self.draw_problems(ctx, interface, gl);

        self.draw_tool(ctx, interface, cursor_pos, gl);
        if self.project.is_none() {
            self.draw_project_information(ctx, interface, glyph_cache, gl);
        }
        if self.project.is_some() {
            self.draw_map_tooltip(ctx, interface, glyph_cache, cursor_pos, gl);
            if self.inspector.diagnostics_mode != DeveloperDiagnosticsMode::Off {
                self.draw_project_information(ctx, interface, glyph_cache, gl);
            }
        }

        let camera_info = self.camera_info(interface, cursor_pos);
        let viewport = interface.get_map_viewport();
        let pos = [
            PADDING[0] + viewport.x,
            viewport.bottom() - PADDING[1] * 1.25,
        ];
        let transform = ctx.transform.trans_pos(pos);
        graphics::text(
            colors::WHITE,
            FONT_SIZE,
            &camera_info,
            glyph_cache,
            transform,
            gl,
        )
        .expect("unable to draw text");

        if self.project.is_some() {
            self.draw_state_inspector(ctx, interface, glyph_cache, gl);
        }
        if self.state_lifecycle_draft.is_some() {
            self.draw_state_lifecycle_editor(ctx, interface, glyph_cache, gl);
        } else if self.province_data_draft.is_some() {
            self.draw_province_data_editor(ctx, interface, glyph_cache, gl);
        } else if self.state_property_draft.is_some()
            && self.inspector.visibility == StateInspectorVisibility::Hidden
        {
            self.draw_state_property_editor(ctx, interface, glyph_cache, gl);
        }
    }

    pub fn draw_state_apply_dialog(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let Some(dialog) = self.state_apply_dialog else {
            return;
        };
        let layout = StateApplyDialogLayout::new(interface);
        let window = interface.get_window_size();
        graphics::rectangle(
            [0.0, 0.0, 0.0, 0.66],
            [0.0, 0.0, window[0], window[1]],
            ctx.transform,
            gl,
        );
        graphics::rectangle([0.075, 0.085, 0.105, 1.0], layout.panel, ctx.transform, gl);
        graphics::rectangle(
            [0.16, 0.18, 0.22, 1.0],
            [layout.panel[0], layout.panel[1], layout.panel[2], 42.0],
            ctx.transform,
            gl,
        );

        let plan = self.patch_preview.as_ref();
        let (title, primary, secondary, close, mut lines) = match dialog {
            StateApplyDialog::Review => {
                let summary = plan.map(|plan| &plan.summary);
                let status = if summary.is_some_and(|summary| summary.blocked_files != 0) {
                    "Blocked"
                } else if summary.is_some_and(|summary| summary.review_required_files != 0) {
                    "Review required"
                } else {
                    "Safe"
                };
                let validation_passed = matches!(
                    self.current_round_trip_status(),
                    Some(RoundTripStatus::Passed | RoundTripStatus::PassedWithReview)
                );
                let primary = if plan.is_some_and(|plan| plan.files_len() == 0) {
                    "Done"
                } else if validation_passed {
                    "Apply State Changes"
                } else {
                    "Validate Changes"
                };
                (
                    "REVIEW STATE CHANGES",
                    primary,
                    "View Details",
                    "Close",
                    vec![
                        format!(
                            "{} state files will be modified",
                            summary.map_or(0, |summary| summary.modified_files)
                        ),
                        format!(
                            "{} files will be created",
                            summary.map_or(0, |summary| summary.created_files)
                        ),
                        format!(
                            "{} files will be removed",
                            summary.map_or(0, |summary| summary.removed_files)
                        ),
                        String::new(),
                        format!("Status: {status}"),
                        self.round_trip_status.clone().unwrap_or_else(|| {
                            "Validation has not been run for this preview.".to_owned()
                        }),
                    ],
                )
            }
            StateApplyDialog::AdditionalValidation => (
                "ADDITIONAL VALIDATION REQUIRED",
                "Validate and Continue",
                "Review State Changes",
                "Cancel",
                vec![
                    "These changes require validation in an isolated temporary copy.".to_owned(),
                    "Your original mod will not be modified during validation.".to_owned(),
                    String::new(),
                    "After validation passes, a final Apply State Changes confirmation will be shown."
                        .to_owned(),
                ],
            ),
            StateApplyDialog::ViewChanges => {
                let (province_modified, pending_states) = self.workspace_dirty_summary();
                let geometry_changed = self.history.has_geometry_changes(&self.bundle.map);
                let mut lines = vec![tr("project_validation.pending_changes").to_owned()];
                if !province_modified && pending_states == 0 {
                    lines.push(tr("project_validation.no_changes").to_owned());
                } else {
                    if province_modified {
                        lines.push(tr("project_validation.province_changes_pending").to_owned());
                    }
                    if pending_states != 0 {
                        lines.push(tr_args(
                            "project_validation.states_pending",
                            &[("count", &pending_states.to_string())],
                        ));
                    }
                    if geometry_changed && self.bundle.config.generate_coastal_on_save {
                        lines.push(tr("project_validation.coastal_maintenance_pending").to_owned());
                    }
                }
                (
                    tr("workspace.view_changes"),
                    tr("project_validation.close"),
                    "",
                    "",
                    lines,
                )
            }
            StateApplyDialog::ProjectSaveReview => {
                let report = self.project_validation_report.as_ref();
                let baseline = report.and_then(|report| report.baseline_summary.as_ref());
                let plan = self.project_save_plan.as_ref();
                let dirty = plan.map(ProjectSavePlan::dirty).unwrap_or_default();
                let files = plan.map_or(0, |plan| plan.patch_plan().files_len());
                let coastal = plan.map_or(0, ProjectSavePlan::coastal_flags_recalculated);
                let validation_blocked = report.is_some_and(|report| report.delta.blocks_save());
                let round_trip_failed = !matches!(
                    self.project_save_round_trip_status(),
                    Some(RoundTripStatus::Passed | RoundTripStatus::PassedWithReview)
                );
                let blocked = validation_blocked || round_trip_failed;
                let blocking = project_validation_blockers(report);
                let mut lines = vec![
                    tr("project_validation.changes").to_owned(),
                    tr_args("project_validation.map_provinces_files", &[("count", &dirty.province_files.to_string())]),
                    tr_args("project_validation.states_files", &[("count", &dirty.state_files.to_string())]),
                    if coastal != 0 {
                        tr_args("project_validation.automatic_coastal", &[("count", &coastal.to_string())])
                    } else {
                        String::new()
                    },
                    tr_args("project_validation.files_will_update", &[("count", &files.to_string())]),
                    tr("project_validation.validation").to_owned(),
                ];
                if blocked {
                    if !blocking.is_empty() {
                        lines.push(tr_args(
                            "project_validation.blocking_problems_count",
                            &[("count", &blocking.len().to_string())],
                        ));
                        lines.extend(blocking.iter().take(3).map(|(source, diagnostic)| {
                            validation_problem_summary(source, diagnostic)
                        }));
                    } else if round_trip_failed {
                        lines.push("Round-trip verification failed.".to_owned());
                        lines.push(
                            self.active_round_trip_failure_snapshot().map_or_else(
                                || tr("project_validation.round_trip_save_blocked").to_owned(),
                                |snapshot| snapshot.summary.clone(),
                            ),
                        );
                    }
                } else {
                    lines.push(tr("project_validation.no_new_blockers").to_owned());
                }
                lines.push(if let Some(baseline) = baseline {
                    tr_args("project_validation.existing_project_issues", &[
                        ("errors", &baseline.errors.to_string()),
                        ("warnings", &baseline.warnings.to_string()),
                    ])
                } else {
                    tr("project_validation.current_project_issues_unclassified").to_owned()
                });
                (
                    if blocked {
                        tr("project_validation.save_blocked")
                    } else {
                        tr("project_validation.ready_to_save")
                    },
                    if validation_blocked {
                        tr("project_validation.view_blocking_problems")
                    } else if round_trip_failed {
                        "View Integrity Problem"
                    } else {
                        tr("workspace.save_project")
                    },
                    if blocked { tr("project_validation.view_existing_issues") } else { tr("project_validation.view_problems") },
                    tr("project_validation.close"),
                    lines,
                )
            }
            StateApplyDialog::IntegrityProblem => {
                let detail = self
                    .active_round_trip_failure_snapshot()
                    .map_or_else(
                        || "Round-trip verification failed before a detailed report could be retained. Prepare Save Project again to capture a fresh report.".to_owned(),
                        |snapshot| snapshot.details.clone(),
                    );
                (
                    "ROUND-TRIP INTEGRITY PROBLEM",
                    "Close",
                    "Copy Details",
                    "",
                    detail.lines().map(str::to_owned).collect(),
                )
            }
            StateApplyDialog::Blocked => (
                "CHANGES CANNOT BE APPLIED",
                "View Problems",
                "View Details",
                "Close",
                vec![
                    format!(
                        "{} blocking problems were found.",
                        plan.map_or(0, |plan| plan.summary.blocked_files)
                    ),
                    "Unsafe file operations cannot continue.".to_owned(),
                    "Resolve the reported diagnostics and regenerate the preview.".to_owned(),
                ],
            ),
            StateApplyDialog::Progress => {
                let validation_running = self.round_trip_task.is_some();
                let status = self
                    .state_save_status
                    .as_ref()
                    .filter(|_| self.state_save_task.is_some())
                    .or(self.round_trip_status.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "Preparing...".to_owned());
                (
                    if validation_running {
                        "VALIDATING CHANGES"
                    } else {
                        "APPLYING CHANGES"
                    },
                    "Cancel Safely",
                    "View Details",
                    "",
                    vec![
                        "[x] Preparing patch".to_owned(),
                        "[x] Checking current files".to_owned(),
                        if validation_running {
                            "[>] Validating temporary copy".to_owned()
                        } else {
                            "[x] Validating temporary copy".to_owned()
                        },
                        if self.state_save_task.is_some() {
                            "[>] Backup, apply, reload and verification".to_owned()
                        } else {
                            "[ ] Backup, apply, reload and verification".to_owned()
                        },
                        String::new(),
                        status,
                    ],
                )
            }
            StateApplyDialog::ValidationResults => {
                let report = self.project_validation_report.as_ref();
                let problems = self.filtered_validation_problems();
                let baseline = report.and_then(|report| report.baseline_summary.as_ref());
                let blocked = report.is_some_and(|report| report.delta.blocks_save());
                let round_trip_failed = self.last_validation.as_ref().is_some_and(|validation| {
                    validation.target == ProjectValidationTarget::PendingChanges
                        && validation.result == RoundTripStatus::Failed.label()
                });
                let mut lines = vec![
                    if blocked {
                        tr("project_validation.save_blocked").to_owned()
                    } else if round_trip_failed {
                        tr("project_validation.round_trip_validation_failed").to_owned()
                    } else {
                        tr("project_validation.project_validation").to_owned()
                    },
                    if blocked {
                        format!("{} new errors must be fixed before saving.", report.map_or(0, |report| report.delta.new_errors() + report.delta.aggravated_to_error()))
                    } else if round_trip_failed {
                        tr("project_validation.round_trip_save_blocked").to_owned()
                    } else { "No new blocking problems were found.".to_owned() },
                    baseline.map_or_else(
                        || tr("project_validation.current_project_issues_unclassified").to_owned(),
                        |summary| tr_args("project_validation.existing_project_issues", &[
                            ("errors", &summary.errors.to_string()),
                            ("warnings", &summary.warnings.to_string()),
                        ]),
                    ),
                    if self.validation_problems_view.show_technical_details {
                        report.map_or_else(|| "Technical details unavailable.".to_owned(), |report| format!("Baseline: {} | Candidate: {} | Delta: +{} !{} ={} -{} ↓{}", report.baseline_summary.as_ref().map_or(0, |summary| summary.total), report.total, report.delta.new.len(), report.delta.aggravated.len(), report.delta.unchanged.len(), report.delta.resolved.len(), report.delta.improved.len()))
                    } else { tr_args("project_validation.errors_warnings", &[("errors", &report.map_or(0, |report| report.errors).to_string()), ("warnings", &report.map_or(0, |report| report.warnings).to_string())]) },
                ];
                if problems.is_empty() {
                    lines.push(tr("project_validation.no_matching_problems").to_owned());
                }
                let navigation = self.selected_validation_problem().map(|(_, diagnostic)| diagnostic);
                (
                    tr("workspace.validation_results"),
                    if navigation.and_then(|diagnostic| diagnostic.province_id).is_some() {
                        tr("project_validation.go_to_province")
                    } else if navigation.and_then(|diagnostic| diagnostic.state_id).is_some() {
                        tr("project_validation.go_to_state")
                    } else if navigation
                        .and_then(|diagnostic| validation_source_path(diagnostic, self.project.as_ref()))
                        .is_some()
                    {
                        tr("project_validation.open_file")
                    } else {
                        tr("project_validation.validate_again")
                    },
                    if self.validation_problems_view.show_technical_details { tr("project_validation.hide_technical_details") } else { tr("project_validation.show_technical_details") },
                    tr("project_validation.close"),
                    lines,
                )
            }
            StateApplyDialog::ImageOverlay => (
                "IMAGE OVERLAY",
                if self.map_layers.image_overlay.enabled { "Hide Image Overlay" } else { "Show Image Overlay" },
                "Close",
                "",
                vec![
                    format!("Status: {}", if self.map_layers.image_overlay.enabled { "Visible" } else { "Hidden" }),
                    format!("Source: {}", self.map_layers.image_overlay.source.as_ref().map_or_else(|| "No image selected".to_owned(), |source| match source { ImageOverlaySource::ProjectHeightmap => "Project heightmap".to_owned(), ImageOverlaySource::Custom(path) => path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned()) })),
                    format!("Opacity: {}%", (self.map_layers.image_overlay.opacity * 100.0).round() as u32),
                    self.image_overlay_status.clone(),
                ],
            ),
            StateApplyDialog::ProvinceRemoval => {
                let draft = self.province_removal_draft.as_ref();
                let province_id = draft.map_or(0, |draft| draft.province_id);
                let data = self
                    .state_edit_session
                    .as_ref()
                    .and_then(|edit| edit.province_data(province_id))
                    .unwrap_or_default();
                (
                    "DELETE PROVINCE",
                    "Transfer References",
                    "Remove Dependent References",
                    "Cancel",
                    vec![
                        format!("Delete Province {province_id}?"),
                        format!(
                            "Dependencies: {} victory point(s), {} province building(s).",
                            usize::from(data.victory_point.is_some()),
                            data.buildings.len()
                        ),
                        "Target Province ID (pixels will merge into this province):".to_owned(),
                        draft.map_or_else(String::new, |draft| draft.target_id.clone()),
                        "Transfer keeps compatible data; remove drops State, VP and building references.".to_owned(),
                    ],
                )
            }
            StateApplyDialog::Result => {
                let project_summary = self.last_project_save_summary;
                let (title, lines) = if let Some(report) = self.state_save_report.as_ref() {
                    if report.outcome == StateSaveOutcome::Completed {
                        let mut lines = vec![tr("project_validation.changes").to_owned()];
                        if let Some(summary) = project_summary {
                            if summary.province_files != 0 {
                                lines.push(tr_args(
                                    "project_validation.map_provinces_files",
                                    &[("count", &summary.province_files.to_string())],
                                ));
                            }
                            if summary.state_files != 0 {
                                lines.push(tr_args(
                                    "project_validation.states_files",
                                    &[("count", &summary.state_files.to_string())],
                                ));
                            }
                            if summary.coastal_flags_recalculated != 0 {
                                lines.push(tr_args(
                                    "project_validation.coastal_flags_recalculated",
                                    &[(
                                        "count",
                                        &summary.coastal_flags_recalculated.to_string(),
                                    )],
                                ));
                            }
                        }
                        lines.extend([
                            tr_args(
                                "project_validation.files_updated",
                                &[ (
                                    "count",
                                    &(report.modified_files
                                        + report.created_files
                                        + report.removed_files)
                                        .to_string(),
                                ) ],
                            ),
                            tr("project_validation.safety").to_owned(),
                            tr("project_validation.validation_passed").to_owned(),
                            if report.backup_path.is_some() {
                                tr("project_validation.backup_created").to_owned()
                            } else {
                                tr("project_validation.backup_status_unavailable").to_owned()
                            },
                            tr("project_validation.round_trip_verified").to_owned(),
                        ]);
                        (
                            tr("project_validation.project_saved"),
                            lines,
                        )
                    } else if report.outcome == StateSaveOutcome::RolledBack {
                        (
                            tr("project_validation.save_failed_restored"),
                            vec![
                                report.error.clone().unwrap_or_else(|| {
                                    tr("project_validation.commit_failure").to_owned()
                                }),
                                tr("project_validation.original_files_restored").to_owned(),
                                tr("project_validation.no_partial_changes").to_owned(),
                            ],
                        )
                    } else {
                        (
                            tr("project_validation.save_blocked"),
                            vec![
                                report
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| report.state.label().to_owned()),
                                tr("project_validation.no_changes_committed").to_owned(),
                            ],
                        )
                    }
                } else {
                    (
                        tr("project_validation.validation_result"),
                        vec![self
                            .round_trip_status
                            .clone()
                            .unwrap_or_else(|| tr("project_validation.validation_incomplete").to_owned())],
                    )
                };
                (title, "Done", "View Report", "Close", lines)
            }
        };

        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [layout.panel[0] + 16.0, layout.panel[1] + 27.0],
            title,
        );
        for (index, line) in lines.drain(..).enumerate() {
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                if line.starts_with("Status: Blocked") {
                    [1.0, 0.42, 0.42, 1.0]
                } else {
                    colors::WHITE
                },
                [
                    layout.panel[0] + 18.0,
                    layout.panel[1] + 72.0 + index as f64 * 26.0,
                ],
                &fit_editor_text(&line, layout.panel[2] - 36.0),
            );
        }
        if dialog == StateApplyDialog::ValidationResults {
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                layout.filters_toggle(),
                if self.validation_problems_view.filters_expanded {
                    tr("project_validation.filters_expanded")
                } else {
                    tr("project_validation.filters_collapsed")
                },
                true,
            );
            if self.validation_problems_view.filters_expanded {
                draw_editor_button(
                    ctx,
                    glyph_cache,
                    gl,
                    layout.severity_filter(),
                    &format!(
                        "{}: {}",
                        tr("project_validation.filter"),
                        self.validation_problems_view.severity.label()
                    ),
                    true,
                );
                draw_editor_button(
                    ctx,
                    glyph_cache,
                    gl,
                    layout.source_filter(),
                    &format!(
                        "{}: {}",
                        tr("project_validation.source"),
                        self.validation_problems_view.source.label()
                    ),
                    true,
                );
                draw_editor_button(
                    ctx,
                    glyph_cache,
                    gl,
                    layout.domain_filter(),
                    &format!(
                        "{}: {}",
                        tr("project_validation.domain"),
                        self.validation_problems_view.domain.label()
                    ),
                    true,
                );
            }
            let problems = self.filtered_validation_problems();
            let visible = if self.validation_problems_view.filters_expanded {
                3
            } else {
                4
            };
            for (index, (source, diagnostic)) in problems
                .iter()
                .skip(self.validation_problems_view.offset)
                .take(visible)
                .enumerate()
            {
                let row = layout
                    .validation_problem_row(index, self.validation_problems_view.filters_expanded);
                let selected = self.validation_problems_view.selected
                    == self.validation_problems_view.offset + index;
                graphics::rectangle(
                    if selected {
                        [0.18, 0.29, 0.40, 1.0]
                    } else {
                        [0.11, 0.13, 0.16, 1.0]
                    },
                    row,
                    ctx.transform,
                    gl,
                );
                draw_canvas_text(
                    ctx,
                    glyph_cache,
                    gl,
                    colors::WHITE,
                    [row[0] + 6.0, row[1] + 17.0],
                    &fit_editor_text(
                        &validation_problem_summary(source, diagnostic),
                        row[2] - 12.0,
                    ),
                );
            }
        }
        if dialog == StateApplyDialog::ImageOverlay {
            for (index, label) in [
                "Choose Image...",
                "Use Project Heightmap",
                &format!(
                    "Opacity: {}% (click to set)",
                    (self.map_layers.image_overlay.opacity * 100.0).round() as u32
                ),
                "Opacity -10%",
                "Opacity +10%",
                "Clear Image",
            ]
            .iter()
            .enumerate()
            {
                draw_editor_button(ctx, glyph_cache, gl, layout.problem_row(index), label, true);
            }
        }
        if dialog == StateApplyDialog::ProvinceRemoval {
            let target = layout.problem_row(0);
            graphics::rectangle(editor_field_color(true, false), target, ctx.transform, gl);
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                colors::WHITE,
                [target[0] + 6.0, target[1] + 17.0],
                self.province_removal_draft
                    .as_ref()
                    .map_or("Target Province ID", |draft| draft.target_id.as_str()),
            );
        }
        let primary_enabled = dialog != StateApplyDialog::Progress
            || self.round_trip_task.is_some()
            || self.state_save_can_cancel();
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.primary(),
            primary,
            primary_enabled,
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.secondary(),
            secondary,
            !secondary.is_empty(),
        );
        if !close.is_empty() {
            if dialog == StateApplyDialog::ValidationResults {
                draw_editor_button(
                    ctx,
                    glyph_cache,
                    gl,
                    layout.close(),
                    tr("project_validation.copy_details"),
                    self.selected_validation_problem().is_some(),
                );
                draw_editor_button(ctx, glyph_cache, gl, layout.validation_close(), close, true);
            } else {
                draw_editor_button(ctx, glyph_cache, gl, layout.close(), close, true);
            }
        }
    }

    fn draw_ids(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        let hovered = cursor_pos
            .and_then(|pos| self.camera.relative_position_int(interface, pos))
            .and_then(|pos| self.bundle.map.get_province_at(pos).preserved_id);
        let selected_state_provinces = self
            .active_state_id
            .and_then(|state_id| self.state_edit_session.as_ref()?.state_data(state_id))
            .map(|data| data.provinces);
        for (_color, province_data) in self.bundle.map.iter_province_data() {
            let province_id = province_data.preserved_id;
            let visible = match self.map_layers.province_label_mode {
                ProvinceLabelMode::Off => false,
                ProvinceLabelMode::Hovered => province_id == hovered,
                ProvinceLabelMode::SelectedState => province_id.is_some_and(|id| {
                    selected_state_provinces
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&id))
                }),
                ProvinceLabelMode::All => true,
            };
            if !visible {
                continue;
            }
            let preserved_id = province_data
                .preserved_id
                .map_or_else(|| "X".to_owned(), |id| id.to_string());
            let color = match self.map_layers.base_view {
                MapBaseView::States | MapBaseView::Political | MapBaseView::Resources => {
                    colors::BLACK
                }
                MapBaseView::ProvinceColors => match province_data.kind {
                    ProvinceKind::Land | ProvinceKind::Lake => colors::BLACK,
                    ProvinceKind::Sea | ProvinceKind::Unknown => colors::WHITE,
                },
                MapBaseView::ProvinceTypes | MapBaseView::Terrain => colors::BLACK,
                MapBaseView::Continents => colors::WHITE,
                MapBaseView::Coastal => match province_data.coastal {
                    Some(true) => colors::BLACK,
                    Some(false) | None => colors::WHITE,
                },
            };

            let center_of_mass = vecmath::vec2_add([0.5, 0.5], province_data.center_of_mass());
            let center_of_mass = self.camera.compute_position(interface, center_of_mass);
            if self.camera.within_viewport(interface, center_of_mass) {
                let preserved_id = preserved_id.to_string();
                let offset = [
                    font::get_width_metric_str(&preserved_id) / -2.0,
                    font::get_v_metrics().ascent - font::get_height_metric() / 2.0,
                ];
                let transform = ctx.transform.trans_pos(center_of_mass).trans_pos(offset);
                graphics::text(color, FONT_SIZE, &preserved_id, glyph_cache, transform, gl)
                    .expect("unable to draw text");
            };
        }
    }

    fn draw_adjacencies(
        &self,
        ctx: Context,
        interface: &Interface,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        let hovered = cursor_pos
            .filter(|pos| interface.map_contains(*pos))
            .map(|pos| self.camera.relative_position(interface, pos))
            .and_then(|pos| self.bundle.map.get_rel_nearest(pos))
            .filter(|(_, distance)| *distance <= 8.0 / self.camera.scale_factor())
            .map(|(rel, _)| rel);

        // Draw the adjacency the user is currently creating
        if let (Some(sel), Some(kind), Some(cursor_pos)) = (
            self.tool.adjacency_selection,
            self.tool.adjacency_brush,
            cursor_pos,
        ) {
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
                let width = if hovered == Some(rel) { 4.0 } else { 2.0 };

                graphics::line_from_to(color, width, center1, center2, ctx.transform, gl);
                if let Some(through) = connection_data.through {
                    let through = self.camera.compute_position(
                        interface,
                        self.bundle.map.get_province(through).center_of_mass(),
                    );
                    graphics::Ellipse::new_border(color, width).draw(
                        [through[0] - 4.0, through[1] - 4.0, 8.0, 8.0],
                        &graphics::DrawState::default(),
                        ctx.transform,
                        gl,
                    );
                }
            };
        }

        // Draw impassible adjacencies as black boundaries
        for (boundary, is_special) in self.bundle.map.iter_boundaries() {
            if is_special {
                let rel = boundary.map(|pos| self.bundle.map.get_color_at(pos));
                if self.bundle.map.get_connection(rel).kind == ConnectionKind::Impassable {
                    let [b1, b2] = boundary_to_line(boundary).into_array();
                    let b1 = self
                        .camera
                        .compute_position(interface, [b1[0] as f64, b1[1] as f64]);
                    let b2 = self
                        .camera
                        .compute_position(interface, [b2[0] as f64, b2[1] as f64]);
                    if self.camera.within_viewport(interface, b1)
                        || self.camera.within_viewport(interface, b2)
                    {
                        graphics::line_from_to(
                            colors::ADJ_IMPASSABLE,
                            2.0,
                            b1,
                            b2,
                            ctx.transform,
                            gl,
                        );
                    };
                };
            };
        }
    }

    fn draw_boundaries(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
        for (boundary, _is_special) in self.bundle.map.iter_boundaries() {
            let [b1, b2] = boundary_to_line(boundary).into_array();
            let b1 = self
                .camera
                .compute_position(interface, [b1[0] as f64, b1[1] as f64]);
            let b2 = self
                .camera
                .compute_position(interface, [b2[0] as f64, b2[1] as f64]);
            if self.camera.within_viewport(interface, b1)
                || self.camera.within_viewport(interface, b2)
            {
                let color = match self.map_layers.base_view {
                    MapBaseView::ProvinceColors => {
                        drawable_color(boundary_color(&self.bundle.map, boundary))
                    }
                    MapBaseView::ProvinceTypes
                    | MapBaseView::Terrain
                    | MapBaseView::States
                    | MapBaseView::Political
                    | MapBaseView::Resources => colors::BLACK,
                    MapBaseView::Continents => colors::WHITE,
                    MapBaseView::Coastal => colors::NEUTRAL,
                };

                graphics::line_from_to(color, 1.0, b1, b2, ctx.transform, gl);
            };
        }
    }

    fn draw_selected_state_boundaries(
        &self,
        ctx: Context,
        interface: &Interface,
        gl: &mut GlGraphics,
    ) {
        for &boundary in &self.selected_state_boundaries {
            let [b1, b2] = boundary_to_line(boundary).into_array();
            let b1 = self
                .camera
                .compute_position(interface, [b1[0] as f64, b1[1] as f64]);
            let b2 = self
                .camera
                .compute_position(interface, [b2[0] as f64, b2[1] as f64]);
            if self.camera.within_viewport(interface, b1)
                || self.camera.within_viewport(interface, b2)
            {
                graphics::line_from_to(colors::WARNING, 4.0, b1, b2, ctx.transform, gl);
            };
        }
    }

    fn draw_selected_province_boundaries(
        &self,
        ctx: Context,
        interface: &Interface,
        gl: &mut GlGraphics,
    ) {
        for &boundary in &self.selected_province_boundaries {
            let [b1, b2] = boundary_to_line(boundary).into_array();
            let b1 = self
                .camera
                .compute_position(interface, [b1[0] as f64, b1[1] as f64]);
            let b2 = self
                .camera
                .compute_position(interface, [b2[0] as f64, b2[1] as f64]);
            if self.camera.within_viewport(interface, b1)
                || self.camera.within_viewport(interface, b2)
            {
                graphics::line_from_to(colors::WHITE, 4.0, b1, b2, ctx.transform, gl);
            };
        }
    }

    fn draw_state_lasso(
        &self,
        ctx: Context,
        interface: &Interface,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        const PREVIEW: DrawColor = [0.0, 0.9, 1.0, 1.0];
        const BLOCKED: DrawColor = [1.0, 0.0, 0.75, 1.0];
        const POLYGON: DrawColor = [1.0, 0.85, 0.1, 1.0];

        self.draw_boundary_set(
            ctx,
            interface,
            &self.lasso_preview_boundaries,
            PREVIEW,
            3.5,
            gl,
        );
        self.draw_boundary_set(
            ctx,
            interface,
            &self.lasso_blocked_boundaries,
            BLOCKED,
            4.0,
            gl,
        );

        let Some(points) = self.state_lasso_phase.points() else {
            return;
        };
        let points = points
            .iter()
            .copied()
            .map(|point| self.camera.compute_position(interface, point))
            .collect::<Vec<_>>();
        let first_point = points.first().copied();
        let drawing = matches!(self.state_lasso_phase, StateLassoPhase::Drawing { .. });
        let can_finish = drawing
            && cursor_pos.zip(first_point).is_some_and(|(cursor, first)| {
                vecmath::vec2_len(vecmath::vec2_sub(first, cursor)) < 5.0
            });
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
        for (a, b) in points
            .into_iter()
            .chain(last_point)
            .tuple_windows::<(_, _)>()
        {
            graphics::line_from_to(POLYGON, 2.0, a, b, ctx.transform, gl);
        }
    }

    fn draw_state_brush(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
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

    fn draw_state_fill(&self, ctx: Context, interface: &Interface, gl: &mut GlGraphics) {
        self.draw_boundary_set(
            ctx,
            interface,
            &self.fill_preview_boundaries,
            [0.2, 0.95, 0.85, 1.0],
            3.5,
            gl,
        );
        self.draw_boundary_set(
            ctx,
            interface,
            &self.fill_blocked_boundaries,
            [1.0, 0.0, 0.75, 1.0],
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
        gl: &mut GlGraphics,
    ) {
        for &boundary in boundaries {
            let [b1, b2] = boundary_to_line(boundary).into_array();
            let b1 = self
                .camera
                .compute_position(interface, [b1[0] as f64, b1[1] as f64]);
            let b2 = self
                .camera
                .compute_position(interface, [b2[0] as f64, b2[1] as f64]);
            if self.camera.within_viewport(interface, b1)
                || self.camera.within_viewport(interface, b2)
            {
                graphics::line_from_to(color, width, b1, b2, ctx.transform, gl);
            }
        }
    }

    fn draw_project_information(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let Some(project_status) = self.project_status.as_deref() else {
            return;
        };
        let selection_info = self.selection_info.as_deref();
        let max_lines = match self.inspector.diagnostics_mode {
            DeveloperDiagnosticsMode::Compact => 10,
            DeveloperDiagnosticsMode::Detailed => usize::MAX,
            DeveloperDiagnosticsMode::Off => usize::MAX,
        };
        let lines = project_status
            .lines()
            .chain(selection_info.into_iter().flat_map(str::lines))
            .take(max_lines)
            .collect::<Vec<_>>();
        let line_height = font::get_height_metric() * 1.15;
        let width = lines
            .iter()
            .map(|line| font::get_width_metric_str(line))
            .fold(0.0, f64::max)
            + 12.0;
        let height = lines.len() as f64 * line_height + 8.0;
        let pos = [
            interface.get_sidebar_width() as f64 + 6.0,
            interface.get_toolbar_height() as f64 + line_height + 10.0,
        ];

        graphics::rectangle(
            colors::OVERLAY_T,
            [pos[0], pos[1], width, height],
            ctx.transform,
            gl,
        );
        for (index, line) in lines.into_iter().enumerate() {
            let transform = ctx.transform.trans(
                pos[0] + 6.0,
                pos[1] + 4.0 + font::get_v_metrics().ascent + index as f64 * line_height,
            );
            graphics::text(colors::WHITE, FONT_SIZE, line, glyph_cache, transform, gl)
                .expect("unable to draw state information");
        }
    }

    fn draw_map_tooltip(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        if !self.is_state_workspace() {
            return;
        }
        let Some(cursor) = cursor_pos.filter(|pos| interface.map_contains(*pos)) else {
            return;
        };
        let Some(map_pos) = self.camera.relative_position_int(interface, cursor) else {
            return;
        };
        let Some(province_id) = self.bundle.map.get_province_at(map_pos).preserved_id else {
            return;
        };
        let state_id = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.province_state_id(province_id));
        let state_name = state_id
            .and_then(|id| self.state_edit_session.as_ref()?.state_data(id)?.name)
            .unwrap_or_else(|| {
                if state_id.is_some() {
                    "<unnamed>".into()
                } else {
                    "<unassigned>".into()
                }
            });
        let lines = if self.map_layers.base_view == MapBaseView::Political {
            state_id
                .and_then(|id| {
                    self.state_edit_session
                        .as_ref()?
                        .state_data(id)
                        .map(|data| (id, data))
                })
                .map_or_else(
                    || vec![format!("Province {province_id} | unassigned")],
                    |(id, data)| {
                        vec![
                            format!("State {id} — {state_name}"),
                            format!("Owner: {}", data.history.owner.as_deref().unwrap_or("—")),
                            format!(
                                "Controller: {}",
                                data.history.controller.as_deref().unwrap_or("—")
                            ),
                            format!(
                                "Category: {}",
                                data.state_category.as_deref().unwrap_or("—")
                            ),
                            format!("Provinces: {}", data.provinces.len()),
                        ]
                    },
                )
        } else {
            vec![state_id.map_or_else(
                || format!("Province {province_id} | unassigned"),
                |id| format!("Province {province_id} | State {id} | {state_name}"),
            )]
        };
        let width = lines
            .iter()
            .map(|line| font::get_width_metric_str(line))
            .fold(0.0, f64::max)
            + 12.0;
        let height = lines.len() as f64 * 19.0 + 8.0;
        let viewport = interface.get_map_viewport();
        let x = (cursor[0] + 12.0)
            .min(viewport.right() - width - 4.0)
            .max(viewport.x + 4.0);
        let y = (cursor[1] + 18.0)
            .min(viewport.bottom() - height - 4.0)
            .max(viewport.y + 4.0);
        graphics::rectangle(colors::OVERLAY_T, [x, y, width, height], ctx.transform, gl);
        for (index, line) in lines.iter().enumerate() {
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                colors::WHITE,
                [x + 6.0, y + 18.0 + index as f64 * 19.0],
                line,
            );
        }
    }

    fn draw_state_inspector(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        if self.inspector.visibility == StateInspectorVisibility::Hidden {
            return;
        }
        let layout = InspectorCanvasLayout::new(interface);
        graphics::rectangle(
            [0.0, 0.0, 0.0, 0.45],
            [
                (layout.panel[0] - 5.0).max(0.0),
                layout.panel[1],
                5.0,
                layout.panel[3],
            ],
            ctx.transform,
            gl,
        );
        graphics::rectangle([0.055, 0.06, 0.075, 0.985], layout.panel, ctx.transform, gl);
        graphics::rectangle(
            colors::WHITE_T,
            [layout.panel[0], layout.panel[1], 1.0, layout.panel[3]],
            ctx.transform,
            gl,
        );
        if self.inspector.visibility == StateInspectorVisibility::Collapsed {
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                colors::WHITE,
                [layout.panel[0] + 14.0, layout.panel[1] + 26.0],
                "<",
            );
            if let Some(state_id) = self.active_state_id {
                draw_canvas_text(
                    ctx,
                    glyph_cache,
                    gl,
                    colors::WARNING,
                    [layout.panel[0] + 7.0, layout.panel[1] + 54.0],
                    &state_id.to_string(),
                );
            }
            return;
        }

        draw_editor_button(ctx, glyph_cache, gl, layout.collapse(), "Collapse", true);

        let (title, subtitle, source_path) = self.inspector_header();
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WARNING,
            [layout.panel[0] + 12.0, layout.panel[1] + 22.0],
            &fit_editor_text(&title, layout.panel[2] - 116.0),
        );
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE_T,
            [layout.panel[0] + 12.0, layout.panel[1] + 45.0],
            &fit_editor_text(&subtitle, layout.panel[2] - 24.0),
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.open_source(),
            "Open source",
            source_path.is_some(),
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.copy_path(),
            "Copy path",
            source_path.is_some(),
        );

        graphics::rectangle(
            if self.inspector_search_focused {
                colors::BUTTON_ACTIVE
            } else {
                colors::BUTTON
            },
            layout.search(),
            ctx.transform,
            gl,
        );
        let search_text = if self.inspector.search.is_empty() {
            if self.is_state_workspace() {
                "Find state, owner, controller, or province"
            } else {
                "Find province by ID, RGB, terrain, type, or state"
            }
        } else {
            &self.inspector.search
        };
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            if self.inspector.search.is_empty() {
                colors::WHITE_T
            } else {
                colors::WHITE
            },
            [layout.search()[0] + 7.0, layout.search()[1] + 18.0],
            &fit_editor_text(search_text, layout.search()[2] - 14.0),
        );
        let results = self.inspector_search_results();
        if let Some(result) = results.get(
            self.inspector_search_index
                .min(results.len().saturating_sub(1)),
        ) {
            let summary = match result {
                InspectorSearchResult::State(result) => {
                    format!("{} — {}", result.state_id, result.name)
                }
                InspectorSearchResult::Province(result) => {
                    let entry = &result.entry;
                    let state = entry.state_id.map_or_else(
                        || "Unassigned".to_owned(),
                        |state_id| {
                            format!(
                                "State {state_id} — {}",
                                entry.state_name.as_deref().unwrap_or("<unnamed>")
                            )
                        },
                    );
                    format!(
                        "{} · {} · {} · RGB {},{},{} · {state}",
                        entry.province_id,
                        entry.kind,
                        entry.terrain,
                        entry.rgb[0],
                        entry.rgb[1],
                        entry.rgb[2],
                    )
                }
            };
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                colors::WHITE_T,
                [layout.panel[0] + 12.0, layout.panel[1] + 129.0],
                &fit_editor_text(
                    &format!(
                        "Result {}/{}: {summary}",
                        self.inspector_search_index + 1,
                        results.len()
                    ),
                    layout.panel[2] - 24.0,
                ),
            );
        }

        for (index, icon) in INSPECTOR_ICONS.iter().enumerate() {
            let active = icon.section == self.inspector.active_section;
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                layout.section(index),
                icon.label,
                active,
            );
        }

        let lines = self.inspector_body_lines();
        let row_height = 19.0;
        let first = (self.inspector.scroll_y / row_height).floor() as usize;
        let offset = self.inspector.scroll_y % row_height;
        for (visible, line) in lines.iter().skip(first).enumerate() {
            let y = layout.body[1] + 18.0 + visible as f64 * row_height - offset;
            if y >= layout.body[1] + layout.body[3] {
                break;
            }
            let line_index = first + visible;
            if self.inspector.active_section == super::inspector::InspectorSection::Overview
                && line_index < 6
                && let Some(draft) = self.state_property_draft.as_ref()
            {
                let labels = [
                    "Name",
                    "Manpower",
                    "Category",
                    "Max level factor",
                    "Local supplies",
                    "Impassable",
                ];
                let value = match line_index {
                    0 => draft.name.clone(),
                    1 => format_integer_input(
                        &draft.manpower,
                        self.property_editor_field != line_index,
                    ),
                    2 => draft.state_category.clone(),
                    3 => draft.buildings_max_level_factor.clone(),
                    4 => draft.local_supplies.clone(),
                    5 if draft.impassable => "Yes".to_owned(),
                    5 => "No".to_owned(),
                    _ => unreachable!(),
                };
                let field_key = (line_index < 5).then_some(STATE_PROPERTY_FIELD_KEYS[line_index]);
                let invalid = field_key.is_some_and(|field| {
                    draft
                        .validate()
                        .err()
                        .is_some_and(|errors| errors.iter().any(|error| error.field == field))
                });
                let input = [
                    layout.body[0] + 112.0,
                    y - 15.0,
                    (layout.body[2] - 117.0).max(0.0),
                    17.0,
                ];
                draw_canvas_text(
                    ctx,
                    glyph_cache,
                    gl,
                    colors::WHITE,
                    [layout.body[0] + 8.0, y],
                    labels[line_index],
                );
                graphics::rectangle(
                    if invalid {
                        [0.38, 0.08, 0.08, 1.0]
                    } else if self.property_editor_field
                        == if line_index == 5 {
                            StatePropertyDraft::TEXT_FIELD_COUNT
                        } else {
                            line_index
                        }
                    {
                        [0.12, 0.25, 0.42, 1.0]
                    } else {
                        [0.14, 0.15, 0.18, 1.0]
                    },
                    input,
                    ctx.transform,
                    gl,
                );
                if line_index == 5 {
                    let check = [input[0] + 4.0, input[1] + 3.0, 11.0, 11.0];
                    graphics::rectangle(
                        if draft.impassable {
                            colors::BUTTON_ACTIVE
                        } else {
                            colors::BUTTON
                        },
                        check,
                        ctx.transform,
                        gl,
                    );
                }
                draw_canvas_text(
                    ctx,
                    glyph_cache,
                    gl,
                    if value.is_empty() {
                        colors::WHITE_T
                    } else {
                        colors::WHITE
                    },
                    [input[0] + if line_index == 5 { 21.0 } else { 6.0 }, y],
                    if value.is_empty() { "<none>" } else { &value },
                );
                if line_index == 2 {
                    draw_canvas_text(
                        ctx,
                        glyph_cache,
                        gl,
                        colors::WHITE_T,
                        [input[0] + input[2] - 14.0, y],
                        "v",
                    );
                }
                continue;
            }
            let reserved_for_controls = self.state_property_draft.is_some()
                && match self.inspector.active_section {
                    super::inspector::InspectorSection::Overview => line_index == 2,
                    super::inspector::InspectorSection::History => line_index < 4,
                    super::inspector::InspectorSection::Resources
                    | super::inspector::InspectorSection::Buildings => line_index > 0,
                    _ => false,
                };
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                if line.starts_with('!') {
                    colors::PROBLEM
                } else {
                    colors::WHITE
                },
                [layout.body[0] + 8.0, y],
                &fit_editor_text(
                    line.trim_start_matches('!'),
                    layout.body[2] - if reserved_for_controls { 205.0 } else { 16.0 },
                ),
            );
        }
        for control in self.inspector_draft_controls(layout) {
            if control.draw.bottom() < layout.body[1]
                || control.draw.y > layout.body[1] + layout.body[3]
            {
                continue;
            }
            let label = match control.id {
                InspectorControlId::Decrement(_) => "-",
                InspectorControlId::Increment(_) => "+",
                InspectorControlId::Select(InspectorPickTarget::StateCategory)
                    if self.inspector.active_section
                        == super::inspector::InspectorSection::Overview =>
                {
                    continue;
                }
                InspectorControlId::Select(_) => "Select",
                InspectorControlId::MapPick(_) => "Pick map",
                InspectorControlId::Add(InspectorPickTarget::Resource) => "Add resource",
                InspectorControlId::Add(InspectorPickTarget::StateBuilding) => "Add building",
                InspectorControlId::RemoveValue(_) => "Remove",
                _ => continue,
            };
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                [
                    control.draw.x,
                    control.draw.y,
                    control.draw.width,
                    control.draw.height,
                ],
                label,
                true,
            );
        }
        self.draw_inspector_picker(ctx, layout, glyph_cache, gl);

        self.draw_inspector_footer(ctx, layout, glyph_cache, gl);
    }

    fn draw_inspector_picker(
        &self,
        ctx: Context,
        layout: InspectorCanvasLayout,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let Some(state) = self.inspector_picker.as_ref() else {
            return;
        };
        let rect = inspector_picker_rect(layout);
        graphics::rectangle([0.06, 0.07, 0.09, 0.995], rect, ctx.transform, gl);
        let title = match state.target {
            InspectorPickTarget::StateCategory => "Select state category",
            InspectorPickTarget::Owner => "Select owner",
            InspectorPickTarget::Controller => "Select controller",
            InspectorPickTarget::Core => "Add core",
            InspectorPickTarget::Claim => "Add claim",
            InspectorPickTarget::Resource => "Add resource",
            InspectorPickTarget::StateBuilding => "Add state building",
            InspectorPickTarget::ProvinceBuilding => "Add province building",
        };
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WARNING,
            [rect[0] + 8.0, rect[1] + 18.0],
            title,
        );
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [rect[0] + 8.0, rect[1] + 38.0],
            if state.picker.query().is_empty() {
                "Search..."
            } else {
                state.picker.query()
            },
        );
        let filtered = state.picker.filtered_indices(String::as_str);
        let up = inspector_picker_up_rect(rect);
        let down = inspector_picker_down_rect(rect);
        graphics::rectangle(colors::BUTTON, up, ctx.transform, gl);
        graphics::rectangle(colors::BUTTON, down, ctx.transform, gl);
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [up[0] + 5.0, up[1] + 14.0],
            "^",
        );
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [down[0] + 5.0, down[1] + 14.0],
            "v",
        );
        for (visible, index) in filtered
            .into_iter()
            .skip(state.picker.scroll())
            .take(8)
            .enumerate()
        {
            let y = rect[1] + 54.0 + visible as f64 * 22.0;
            if state.picker.highlighted() == state.picker.scroll() + visible {
                graphics::rectangle(
                    colors::BUTTON_ACTIVE,
                    [rect[0] + 5.0, y - 16.0, rect[2] - 10.0, 21.0],
                    ctx.transform,
                    gl,
                );
            }
            draw_canvas_text(
                ctx,
                glyph_cache,
                gl,
                colors::WHITE,
                [rect[0] + 9.0, y],
                &state.picker.items()[index],
            );
        }
    }

    fn inspector_header(&self) -> (String, String, Option<PathBuf>) {
        let Some(state_id) = self.active_state_id else {
            return (
                "No state selected".to_owned(),
                "Click a land province or use search".to_owned(),
                None,
            );
        };
        let edit = self.state_edit_session.as_ref();
        let data = edit.and_then(|edit| edit.state_data(state_id));
        let draft = self
            .state_property_draft
            .as_ref()
            .filter(|draft| draft.state_id == state_id);
        let name = draft
            .map(|draft| draft.name.as_str())
            .or_else(|| data.as_ref().and_then(|data| data.name.as_deref()))
            .unwrap_or("<unnamed>");
        let provinces = data.as_ref().map_or(0, |data| data.provinces.len());
        let dirty = draft.is_some_and(StatePropertyDraft::is_modified)
            || edit.is_some_and(|edit| edit.is_state_dirty(state_id));
        let (origin, source) = match edit.and_then(|edit| edit.state_origin(state_id)) {
            Some(WorkingStateOrigin::Loaded { document_path }) => {
                ("loaded", Some(document_path.clone()))
            }
            Some(WorkingStateOrigin::CreatedInSession) => ("created in session", None),
            None => ("invalid/read-only", None),
        };
        (
            format!("STATE {state_id} — {name}"),
            format!(
                "{} · {provinces} provinces · {origin}",
                if dirty { "Modified" } else { "Clean" }
            ),
            source,
        )
    }

    fn inspector_body_lines(&self) -> Vec<String> {
        let Some(state_id) = self.active_state_id else {
            return vec![
                "Select a state to inspect its properties.".to_owned(),
                "Ctrl+click keeps province multi-selection.".to_owned(),
            ];
        };
        let Some(data) = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.state_data(state_id))
        else {
            return vec!["!State data is invalid or unavailable (read-only).".to_owned()];
        };
        if let Some(draft) = self
            .state_property_draft
            .as_ref()
            .filter(|draft| draft.state_id == state_id)
        {
            match self.inspector.active_section {
                super::inspector::InspectorSection::Overview => {
                    let resources = draft.resource_values().map_or(0, |values| values.len());
                    let state_buildings = draft
                        .state_building_values()
                        .map_or(0, |values| values.len());
                    return vec![
                        format!("Name: {}", draft.name),
                        format!("Manpower: {}", format_integer_input(&draft.manpower, true)),
                        format!("Category: {}", draft.state_category),
                        format!("Max level factor: {}", draft.buildings_max_level_factor),
                        format!("Local supplies: {}", draft.local_supplies),
                        format!("Impassable: {}", draft.impassable),
                        format!("Provinces: {}", data.provinces.len()),
                        format!("Resources: {resources}"),
                        format!("State buildings: {state_buildings}"),
                        format!("Victory points: {}", data.history.victory_points.len()),
                        format!(
                            "Province building groups: {}",
                            data.history.province_buildings.len()
                        ),
                        format!(
                            "Cores: {}",
                            draft
                                .cores
                                .split(',')
                                .filter(|v| !v.trim().is_empty())
                                .count()
                        ),
                        format!(
                            "Claims: {}",
                            draft
                                .claims
                                .split(',')
                                .filter(|v| !v.trim().is_empty())
                                .count()
                        ),
                        if draft.is_modified() {
                            "Draft modified - Apply or Discard".to_owned()
                        } else {
                            "Draft unchanged".to_owned()
                        },
                    ];
                }
                super::inspector::InspectorSection::History => {
                    return vec![
                        format!("Owner: {}", draft.owner),
                        format!("Controller: {}", draft.controller),
                        format!("Cores: [{}]", draft.cores),
                        format!("Claims: [{}]", draft.claims),
                    ];
                }
                super::inspector::InspectorSection::Resources => {
                    let mut lines = vec!["Resources".to_owned()];
                    match draft.resource_values() {
                        Ok(values) => lines.extend(
                            values
                                .iter()
                                .map(|(name, value)| format!("{name}: {value}")),
                        ),
                        Err(errors) => lines.push(format!("!{}", errors[0].message)),
                    }
                    lines.push("+ Add resource from catalog".to_owned());
                    return lines;
                }
                super::inspector::InspectorSection::Buildings => {
                    let mut lines = vec!["State buildings".to_owned()];
                    match draft.state_building_values() {
                        Ok(values) => lines.extend(
                            values
                                .iter()
                                .map(|(name, value)| format!("{name}: {value}")),
                        ),
                        Err(errors) => lines.push(format!("!{}", errors[0].message)),
                    }
                    lines.push("+ Add state building from catalog".to_owned());
                    return lines;
                }
                _ => {}
            }
        }
        if let Some(draft) = self.province_data_draft.as_ref() {
            match self.inspector.active_section {
                super::inspector::InspectorSection::History => {
                    return vec![
                        format!("Province {} victory point", draft.province_id),
                        format!(
                            "Value: {}",
                            draft.victory_point.as_deref().unwrap_or("<none>")
                        ),
                        if draft.is_modified() {
                            "Province draft modified - Apply or Discard".to_owned()
                        } else {
                            "Province draft unchanged".to_owned()
                        },
                    ];
                }
                super::inspector::InspectorSection::Buildings => {
                    let mut lines = vec![format!("Province {} buildings:", draft.province_id)];
                    lines.extend(
                        draft
                            .buildings
                            .iter()
                            .map(|building| format!("  {} = {}", building.name, building.value)),
                    );
                    return lines;
                }
                _ => {}
            }
        }
        match self.inspector.active_section {
            super::inspector::InspectorSection::Overview => vec![
                format!("Name: {}", data.name.as_deref().unwrap_or("<none>")),
                format!(
                    "Manpower: {}",
                    data.manpower
                        .map_or_else(|| "<none>".into(), |v| v.to_string())
                ),
                format!(
                    "Category: {}",
                    data.state_category.as_deref().unwrap_or("<none>")
                ),
                format!(
                    "Max level factor: {}",
                    data.buildings_max_level_factor
                        .map_or_else(|| "<none>".into(), |v| v.to_string())
                ),
                format!(
                    "Local supplies: {}",
                    data.local_supplies
                        .map_or_else(|| "<none>".into(), |v| v.to_string())
                ),
                format!("Impassable: {}", data.impassable.unwrap_or(false)),
                format!("Provinces: {}", data.provinces.len()),
                format!("Resources: {}", data.resources.len()),
                format!("State buildings: {}", data.history.state_buildings.len()),
                format!("Victory points: {}", data.history.victory_points.len()),
                format!(
                    "Province building groups: {}",
                    data.history.province_buildings.len()
                ),
                format!("Cores: {}", data.history.cores.len()),
                format!("Claims: {}", data.history.claims.len()),
            ],
            super::inspector::InspectorSection::History => vec![
                format!(
                    "Owner: {}",
                    data.history.owner.as_deref().unwrap_or("<none>")
                ),
                format!(
                    "Controller: {}",
                    data.history.controller.as_deref().unwrap_or("<none>")
                ),
                format!("Cores: {}", data.history.cores.iter().join(", ")),
                format!("Claims: {}", data.history.claims.iter().join(", ")),
            ],
            super::inspector::InspectorSection::Resources => {
                let mut lines = vec!["State resources:".to_owned()];
                lines.extend(
                    data.resources
                        .iter()
                        .map(|(name, value)| format!("  {name} = {value}")),
                );
                if data.resources.is_empty() {
                    lines.push("  <none>".to_owned());
                }
                lines
            }
            super::inspector::InspectorSection::Buildings => {
                let mut lines = vec!["State buildings:".to_owned()];
                lines.extend(
                    data.history
                        .state_buildings
                        .iter()
                        .map(|(name, value)| format!("  {name} = {value}")),
                );
                lines.push("Province buildings:".to_owned());
                for (province, buildings) in &data.history.province_buildings {
                    for (name, value) in buildings {
                        lines.push(format!("  {province}: {name} = {value}"));
                    }
                }
                lines
            }
            super::inspector::InspectorSection::Provinces => {
                let mut lines = vec![
                    format!("Provinces ({})", data.provinces.len()),
                    format!("Victory points ({})", data.history.victory_points.len()),
                ];
                lines.extend(
                    data.history
                        .victory_points
                        .iter()
                        .map(|vp| format!("  VP {} = {}", vp.province_id, vp.value)),
                );
                lines.push(String::new());
                lines.extend(data.provinces.iter().map(u32::to_string));
                lines
            }
            super::inspector::InspectorSection::Diagnostics => {
                let mut lines = vec!["DEFINITION CATALOG".to_owned()];
                if let Some(catalog) = self.definition_catalog.as_ref() {
                    lines.extend([
                        format!("Categories: {}", catalog.state_categories.len()),
                        format!("Resources: {}", catalog.resources.len()),
                        format!("Buildings: {}", catalog.buildings.len()),
                        format!("Country tags: {}", catalog.country_tags.len()),
                        format!(
                            "Base game definitions: {}",
                            self.definition_base_game_root
                                .as_ref()
                                .map_or("not configured (fallback only)".to_owned(), |path| path
                                    .display()
                                    .to_string())
                        ),
                        "Mod definitions: loaded".to_owned(),
                        "Observed project values: loaded".to_owned(),
                        String::new(),
                        "STATE DIAGNOSTICS".to_owned(),
                    ]);
                }
                if let Some(catalog) = self.political_country_catalog.as_ref() {
                    lines.extend([String::new(), "POLITICAL COUNTRY RESOLUTION".to_owned()]);
                    lines.extend(
                        catalog
                            .owner_resolution(data.history.owner.as_deref())
                            .diagnostic_lines(),
                    );
                }
                lines.extend(self.project.as_ref().map_or_else(Vec::new, |project| {
                    let path = project
                        .state_document(state_id)
                        .map(|document| &document.path);
                    project
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.path.as_ref() == path)
                        .map(|diagnostic| {
                            let marker = if diagnostic.severity == DiagnosticSeverity::Error {
                                "!"
                            } else {
                                ""
                            };
                            format!("{marker}{:?}: {}", diagnostic.severity, diagnostic.message)
                        })
                        .collect()
                }));
                if let Some(province_id) = self.active_province_id {
                    lines.extend([String::new(), format!("PROVINCE {province_id} CONTEXT")]);
                    let state_id = self
                        .state_edit_session
                        .as_ref()
                        .and_then(|edit| edit.province_state_id(province_id));
                    lines.push(state_id.map_or_else(
                        || "~Unassigned land province; Save Project is allowed.".to_owned(),
                        |state_id| format!("State assignment: {state_id}"),
                    ));
                    lines.extend(self.project.as_ref().into_iter().flat_map(|project| {
                        project
                            .diagnostics_for_province(province_id)
                            .map(|diagnostic| {
                                let marker = match diagnostic.severity {
                                    DiagnosticSeverity::Error => "!",
                                    DiagnosticSeverity::Warning => "~",
                                    DiagnosticSeverity::Info => "i",
                                };
                                format!("{marker}{:?}: {}", diagnostic.kind, diagnostic.message)
                            })
                    }));
                }
                if let Some(catalog) = self.definition_catalog.as_ref() {
                    lines.extend(catalog.diagnostics.iter().take(12).map(|diagnostic| {
                        format!("Catalog {:?}: {}", diagnostic.severity, diagnostic.message)
                    }));
                }
                if lines.len() <= 10 {
                    lines.push("No contextual diagnostics.".to_owned());
                }
                lines
            }
        }
    }

    fn draw_inspector_footer(
        &self,
        ctx: Context,
        layout: InspectorCanvasLayout,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let draft_modified = self.property_draft_is_modified();
        if self.state_property_draft.is_some() || self.province_data_draft.is_some() {
            if let Some(error) = self
                .state_property_draft
                .as_ref()
                .and_then(|draft| draft.validate().err())
                .and_then(|errors| errors.into_iter().next())
            {
                draw_canvas_text(
                    ctx,
                    glyph_cache,
                    gl,
                    [1.0, 0.42, 0.42, 1.0],
                    [layout.panel[0] + 8.0, layout.footer_left()[1] - 7.0],
                    &fit_editor_text(
                        &format!("{}: {}", error.field, error.message),
                        layout.panel[2] - 16.0,
                    ),
                );
            }
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                layout.footer_left(),
                "Apply draft",
                draft_modified,
            );
            draw_editor_button(ctx, glyph_cache, gl, layout.footer_right(), "Discard", true);
        } else {
            let edit_label =
                if self.inspector.active_section == super::inspector::InspectorSection::Overview {
                    "Edit General Properties"
                } else {
                    "Edit State Properties"
                };
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                layout.footer_left(),
                edit_label,
                self.state_action_availability().can_edit_properties,
            );
            draw_editor_button(
                ctx,
                glyph_cache,
                gl,
                layout.footer_right(),
                "Edit province",
                self.state_action_availability().can_edit_province_data,
            );
        }
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
                let invalid_fields = property_errors
                    .iter()
                    .map(|error| error.field)
                    .collect::<BTreeSet<_>>();
                let state_id = id.trim().parse::<u32>().ok();
                let id_error = match (state_id, self.state_edit_session.as_ref()) {
                    (None, _) => Some("State ID must be a positive integer.".to_owned()),
                    (Some(state_id), Some(edit)) => edit
                        .validate_new_state_id(state_id)
                        .err()
                        .map(|error| error.to_string()),
                    (Some(_), None) => Some("State project is unavailable.".to_owned()),
                };
                let id_valid = id_error.is_none();
                let selected = self
                    .state_edit_session
                    .as_ref()
                    .map_or(0, |edit| edit.selected_provinces().len());

                let window = interface.get_window_size();
                graphics::rectangle(
                    [0.0, 0.0, 0.0, 0.62],
                    [0.0, 0.0, window[0], window[1]],
                    ctx.transform,
                    gl,
                );
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
                    let raw = properties.field(index).unwrap_or_default();
                    let value = match index {
                        9 => properties
                            .resource_values()
                            .ok()
                            .map(|values| {
                                values
                                    .iter()
                                    .map(|(name, value)| format!("{name} {value}"))
                                    .join(" · ")
                            })
                            .filter(|value| !value.is_empty())
                            .unwrap_or_else(|| raw.to_owned()),
                        10 => properties
                            .state_building_values()
                            .ok()
                            .map(|values| {
                                values
                                    .iter()
                                    .map(|(name, value)| format!("{name} {value}"))
                                    .join(" · ")
                            })
                            .filter(|value| !value.is_empty())
                            .unwrap_or_else(|| raw.to_owned()),
                        _ => format_integer_input(
                            raw,
                            index == 1 && self.property_editor_field != field_index,
                        ),
                    };
                    draw_canvas_text(
                        ctx,
                        glyph_cache,
                        gl,
                        colors::WHITE,
                        [rect[0] + 6.0, rect[1] + 17.0],
                        &fit_editor_text(&value, rect[2] - 12.0),
                    );
                }

                let impassable = layout.impassable();
                graphics::rectangle(
                    if properties.impassable {
                        colors::BUTTON_ACTIVE
                    } else {
                        colors::BUTTON
                    },
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

                let status = if let Some(error) = id_error {
                    error
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
                    if id_valid && property_errors.is_empty() {
                        colors::WHITE
                    } else {
                        colors::PROBLEM
                    },
                    [layout.panel[0] + 12.0, layout.status_y],
                    &status,
                );
            }
            Some(StateLifecycleDraft::Remove {
                state_id,
                target_id,
                unassign,
                province_count,
            }) => {
                let layout = StateRemovalEditorLayout::new(interface);
                let can_remove = self.can_apply_state_removal();
                let removal_warning = match self
                    .state_edit_session
                    .as_ref()
                    .and_then(|edit| edit.state_origin(*state_id))
                {
                    Some(WorkingStateOrigin::CreatedInSession) => {
                        "Removes the temporary state — no state file exists"
                    }
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
                    if *unassign {
                        "Target disabled"
                    } else {
                        target_id
                    },
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
            }
            None => {}
        }
    }

    fn draw_state_property_editor(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let Some(draft) = self.state_property_draft.as_ref() else {
            return;
        };
        let layout = PropertyEditorLayout::new(interface);
        let errors = draft.validate().err().unwrap_or_default();
        let can_apply = draft.is_modified() && errors.is_empty();
        let invalid_fields = errors
            .iter()
            .map(|error| error.field)
            .collect::<BTreeSet<_>>();

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
            let value = format_integer_input(
                draft.field(index).unwrap_or_default(),
                index == 1 && !active,
            );
            let value = fit_editor_text(&value, rect[2] - 12.0);
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
            if draft.impassable {
                colors::BUTTON_ACTIVE
            } else {
                colors::BUTTON
            },
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
            if errors.is_empty() {
                colors::WHITE
            } else {
                colors::PROBLEM
            },
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
        gl: &mut GlGraphics,
    ) {
        let Some(draft) = self.province_data_draft.as_ref() else {
            return;
        };
        let layout =
            ProvinceEditorLayout::new(interface, draft.buildings.len(), self.province_editor_page);
        let errors = self
            .validate_province_data_draft(draft)
            .err()
            .unwrap_or_default();
        let can_apply = draft.is_modified() && errors.is_empty();
        let invalid_fields = errors
            .iter()
            .filter_map(|error| error.field_index)
            .collect::<BTreeSet<_>>();
        let state_name = self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.state_data(draft.state_id))
            .and_then(|data| data.name)
            .unwrap_or_else(|| "<unnamed>".to_owned());
        let selection_count = self
            .state_edit_session
            .as_ref()
            .map_or(0, |edit| edit.selected_provinces().len());
        let selected_position = self.selected_province_position(draft.province_id);

        let window = interface.get_window_size();
        graphics::rectangle(
            [0.0, 0.0, 0.0, 0.62],
            [0.0, 0.0, window[0], window[1]],
            ctx.transform,
            gl,
        );
        graphics::rectangle([0.06, 0.07, 0.09, 0.97], layout.panel, ctx.transform, gl);
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [layout.panel[0] + 12.0, layout.panel[1] + 24.0],
            &format!("EDIT PROVINCE {}", draft.province_id),
        );
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WARNING,
            [layout.panel[0] + 12.0, layout.panel[1] + 44.0],
            &format!(
                "State {} — {} · {selection_count} provinces selected · changes remain in memory",
                draft.state_id, state_name
            ),
        );

        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            colors::WHITE,
            [
                layout.panel[0] + 12.0,
                layout.victory_point_field()[1] + 17.0,
            ],
            "Victory Point",
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
            if draft.victory_point.is_some() {
                colors::WHITE
            } else {
                colors::WHITE_T
            },
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
            colors::WHITE_T,
            [layout.panel[0] + 12.0, layout.panel[1] + 110.0],
            &selected_position.map_or_else(
                || "Active province is outside the current selection".to_owned(),
                |(index, total)| format!("Province {} of {total}", index + 1),
            ),
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.previous_selected(),
            "Previous Selected",
            selected_position.is_some_and(|(_, total)| total > 1),
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.next_selected(),
            "Next Selected",
            selected_position.is_some_and(|(_, total)| total > 1),
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
            let Some(building) = draft.buildings.get(row) else {
                continue;
            };
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
            "Add Province Building",
            true,
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.previous_page(),
            "Previous Buildings",
            self.province_editor_page > 0,
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.next_page(),
            "Next Buildings",
            layout.has_next_page(self.province_editor_page),
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.apply(),
            "Apply Changes",
            can_apply,
        );
        draw_editor_button(
            ctx,
            glyph_cache,
            gl,
            layout.discard(),
            "Discard Province Changes",
            true,
        );
        draw_editor_button(ctx, glyph_cache, gl, layout.close(), "Close", true);

        let status = if !draft.is_modified() {
            "No province draft changes".to_owned()
        } else if errors.is_empty() {
            "Province draft modified — ready to apply".to_owned()
        } else {
            format!("Province draft validation errors: {}", errors.len())
        };
        draw_canvas_text(
            ctx,
            glyph_cache,
            gl,
            if errors.is_empty() {
                colors::WHITE
            } else {
                colors::PROBLEM
            },
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
            problem.draw(
                ctx,
                extras,
                CameraCombo {
                    camera: &self.camera,
                    interface,
                },
                gl,
            );
        }
    }

    fn draw_tool(
        &self,
        ctx: Context,
        interface: &Interface,
        cursor_pos: Option<Vector2<f64>>,
        gl: &mut GlGraphics,
    ) {
        if self.is_state_workspace() {
            return;
        }

        let color = if self.tool.color_brush.is_some() {
            colors::WHITE
        } else {
            colors::WHITE_T
        };
        match (self.view_mode, &self.tool.mode, cursor_pos) {
            (ViewMode::Color, ToolMode::PaintArea, Some(cursor_pos)) => {
                let ellipse = Ellipse::new_border(color, 0.5).resolution(16);
                let r = self.tool.radius * self.camera.scale_factor();
                let transform = ctx.transform.trans_pos(cursor_pos);
                ellipse.draw_from_to([r, r], [-r, -r], &Default::default(), transform, gl);
            }
            (ViewMode::Color, ToolMode::Lasso(lasso), cursor_pos) => {
                let can_finish = cursor_pos
                    .map(|cursor_pos| lasso.can_finish(interface, &self.camera, cursor_pos))
                    .unwrap_or(false);
                let points = lasso
                    .iter()
                    .map(|pos| self.camera.compute_position(interface, pos))
                    .collect::<Vec<Vector2<f64>>>();
                let first_point = points.first().cloned();
                let last_point = if can_finish { first_point } else { cursor_pos };

                if let (true, Some(first_point)) = (can_finish, first_point) {
                    let ellipse = Ellipse::new(color).resolution(6);
                    let transform = ctx.transform.trans_pos(first_point);
                    ellipse.draw_from_to(
                        [5.0, 5.0],
                        [-5.0, -5.0],
                        &Default::default(),
                        transform,
                        gl,
                    );
                };

                let lines = points
                    .into_iter()
                    .chain(last_point.into_iter())
                    .tuple_windows::<(_, _)>();
                for (pos1, pos2) in lines {
                    graphics::line_from_to(color, 0.5, pos1, pos2, ctx.transform, gl);
                }
            }
            _ => (),
        };
    }

    pub fn toggle_province_ids(&mut self) {
        self.map_layers.show_province_ids = !self.map_layers.show_province_ids;
    }

    pub fn inspector_reserved_width(&self) -> f64 {
        if self.project.is_some() {
            self.inspector.visibility.reserved_width()
        } else {
            0.0
        }
    }

    pub fn cycle_state_inspector_visibility(&mut self, alerts: &mut Alerts) {
        self.inspector.visibility = self.inspector.visibility.cycle();
        alerts.push(Ok(format!(
            "State Inspector: {:?}",
            self.inspector.visibility
        )));
    }

    pub fn cycle_province_label_mode(&mut self, alerts: &mut Alerts) {
        self.map_layers.province_label_mode = self.map_layers.province_label_mode.cycle();
        alerts.push(Ok(format!(
            "Province labels: {:?}",
            self.map_layers.province_label_mode
        )));
    }

    pub fn cycle_developer_diagnostics(&mut self, alerts: &mut Alerts) {
        self.inspector.diagnostics_mode = self.inspector.diagnostics_mode.cycle();
        alerts.push(Ok(format!(
            "Developer diagnostics: {:?}",
            self.inspector.diagnostics_mode
        )));
    }

    pub fn set_base_game_definition_root(&mut self, root: Option<PathBuf>, alerts: &mut Alerts) {
        let Some(project) = self.project.as_ref() else {
            alerts.push(Err("Load a state project before configuring definitions"));
            return;
        };
        self.definition_catalog = Some(GameDefinitionCatalog::build(project, root.as_deref()));
        self.definition_base_game_root = root;
        self.reload_political_country_catalog();
        self.reload_resource_icons();
        let catalog = self.definition_catalog.as_ref().unwrap();
        alerts.push(Ok(format!(
            "Definition catalog: {} categories, {} resources, {} buildings, {} tags",
            catalog.state_categories.len(),
            catalog.resources.len(),
            catalog.buildings.len(),
            catalog.country_tags.len(),
        )));
    }

    pub fn inspector_scroll(
        &mut self,
        interface: &Interface,
        pos: Vector2<f64>,
        amount: f64,
    ) -> bool {
        if self.inspector_picker_is_open() {
            let popup = inspector_picker_rect(InspectorCanvasLayout::new(interface));
            if point_in_rect(pos, popup) {
                if let Some(state) = self.inspector_picker.as_mut() {
                    state
                        .picker
                        .scroll_by((-amount.signum() as isize) * 3, String::as_str);
                }
                return true;
            }
        }
        if self.inspector.visibility != StateInspectorVisibility::Expanded {
            return false;
        }
        if !point_in_rect(pos, InspectorCanvasLayout::new(interface).panel) {
            return false;
        }
        self.inspector.scroll_y = (self.inspector.scroll_y - amount * 28.0).max(0.0);
        true
    }

    pub fn toggle_province_boundaries(&mut self) {
        self.map_layers.show_province_boundaries = !self.map_layers.show_province_boundaries;
    }

    pub fn toggle_river_overlay(&mut self) -> bool {
        if !self.map_layers.show_rivers && self.bundle.map.get_rivers_overlay().is_none() {
            return true;
        };

        self.map_layers.show_rivers = !self.map_layers.show_rivers;
        self.texture_overlay = None;

        false
    }

    pub fn toggle_adjacencies_overlay(&mut self) {
        self.map_layers.show_adjacencies = !self.map_layers.show_adjacencies;
    }

    pub fn toggle_resources_overlay(&mut self, alerts: &mut Alerts) {
        let available = self
            .project
            .as_ref()
            .is_some_and(Hoi4Project::state_load_is_complete)
            && self.state_edit_session.is_some();
        if !available {
            alerts.push(Err(
                "Resources overlay is available only for fully loaded HOI4 state projects",
            ));
            return;
        }
        self.map_layers.show_resources = !self.map_layers.show_resources;
        if self.map_layers.show_resources {
            self.refresh_resource_labels();
        }
        alerts.push(Ok(format!(
            "Resources overlay: {}",
            if self.map_layers.show_resources {
                "shown"
            } else {
                "hidden"
            }
        )));
    }

    pub fn enabled_options(&self) -> [bool; 7] {
        [
            self.map_layers.show_rivers,
            self.map_layers.show_adjacencies,
            self.map_layers.show_province_ids,
            self.map_layers.show_province_boundaries,
            self.map_layers.show_state_boundaries,
            self.map_layers.image_overlay.enabled,
            self.map_layers.show_resources,
        ]
    }

    pub fn available_options(&self) -> [bool; 7] {
        [
            self.bundle.map.get_rivers_overlay().is_some(),
            self.bundle.map.connections_count() > 0,
            self.bundle.config.preserve_ids,
            true,
            self.is_state_workspace(),
            self.image_overlay_texture.is_some(),
            self.project
                .as_ref()
                .is_some_and(Hoi4Project::state_load_is_complete)
                && self.state_edit_session.is_some(),
        ]
    }

    pub fn toggle_lasso_snap(&mut self) {
        self.tool.lasso_snap = !self.tool.lasso_snap;
    }

    pub fn reload_config(&mut self, alerts: &mut Alerts) {
        let loaded = self
            .project
            .as_ref()
            .map(|project| Config::load_for_project(&project.paths.root))
            .unwrap_or_else(Config::load);
        match loaded {
            Ok(config) => {
                self.bundle.config = config;
                alerts.push(Ok("Reloaded config"));
            }
            Err(err) => alerts.push(Err(format!("Error: {}", err))),
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
                Err(err) => alerts.push(Err(format!("Error: {}", err))),
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
                Err(err) => alerts.push(Err(format!("Error: {}", err))),
            };
        };
    }

    pub fn undo(&mut self, alerts: &mut Alerts) {
        if let Some(transaction) = self.province_removal_undo.last().copied()
            && self.history.position() == transaction.map_after
            && self
                .state_edit_session
                .as_ref()
                .is_some_and(|edit| edit.summary().commands == transaction.state_after)
        {
            let map_undone = self.history.undo(&mut self.bundle.map).is_some();
            let state_undone = self
                .state_edit_session
                .as_mut()
                .is_some_and(StateEditSession::undo);
            if map_undone && state_undone {
                self.province_removal_undo.pop();
                self.province_removal_redo.push(transaction);
                self.bundle.map.recalculate_all_boundaries();
                self.problems.clear();
                self.refresh();
                self.sync_province_dirty();
                self.repair_active_state_after_history();
                self.refresh_state_visuals();
                return;
            }
            if map_undone {
                self.history.redo(&mut self.bundle.map);
            }
            if state_undone {
                self.state_edit_session.as_mut().unwrap().redo();
            }
            alerts.push(Err(
                "Province removal undo could not restore both map and State data",
            ));
            return;
        }
        if self.is_state_workspace() {
            if self.property_draft_is_modified() {
                return;
            }
            self.discard_unmodified_property_draft();
            self.cancel_state_lasso();
            self.cancel_state_brush();
            self.cancel_state_fill();
            if let Some(edit) = self.state_edit_session.as_mut()
                && edit.undo()
            {
                self.repair_active_state_after_history();
                self.refresh_state_visuals();
            }
            return;
        };
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        }

        if let Some(commit) = self.history.undo(&mut self.bundle.map) {
            self.synchronize_created_state_provinces();
            self.bundle.map.recalculate_all_boundaries();
            self.problems.clear();
            if self.bundle.config.change_view_mode_on_undo {
                self.view_mode = commit.view_mode;
            };
            self.refresh();
            self.sync_province_dirty();
        };
    }

    pub fn redo(&mut self, alerts: &mut Alerts) {
        if let Some(transaction) = self.province_removal_redo.last().copied()
            && self.history.position() == transaction.map_before
            && self
                .state_edit_session
                .as_ref()
                .is_some_and(|edit| edit.summary().commands == transaction.state_before)
        {
            let map_redone = self.history.redo(&mut self.bundle.map).is_some();
            let state_redone = self
                .state_edit_session
                .as_mut()
                .is_some_and(StateEditSession::redo);
            if map_redone && state_redone {
                self.province_removal_redo.pop();
                self.province_removal_undo.push(transaction);
                self.bundle.map.recalculate_all_boundaries();
                self.problems.clear();
                self.refresh();
                self.sync_province_dirty();
                self.repair_active_state_after_history();
                self.refresh_state_visuals();
                return;
            }
            if map_redone {
                self.history.undo(&mut self.bundle.map);
            }
            if state_redone {
                self.state_edit_session.as_mut().unwrap().undo();
            }
            alerts.push(Err(
                "Province removal redo could not restore both map and State data",
            ));
            return;
        }
        if self.is_state_workspace() {
            if self.property_draft_is_modified() {
                return;
            }
            self.discard_unmodified_property_draft();
            self.cancel_state_lasso();
            self.cancel_state_brush();
            self.cancel_state_fill();
            if let Some(edit) = self.state_edit_session.as_mut()
                && edit.redo()
            {
                self.repair_active_state_after_history();
                self.refresh_state_visuals();
            }
            return;
        };
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        }

        if let Some(commit) = self.history.redo(&mut self.bundle.map) {
            self.synchronize_created_state_provinces();
            self.bundle.map.recalculate_all_boundaries();
            self.problems.clear();
            if self.bundle.config.change_view_mode_on_undo {
                self.view_mode = commit.view_mode;
            };
            self.refresh();
            self.sync_province_dirty();
        };
    }

    pub fn calculate_coastal_provinces(&mut self) {
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        };

        self.history.calculate_coastal_provinces(&mut self.bundle);
        self.sync_province_dirty();
        self.view_mode = ViewMode::Coastal;
        self.map_layers.base_view = MapBaseView::Coastal;
        self.refresh();
    }

    pub fn calculate_recolor_map(&mut self) {
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        };

        self.history.calculate_recolor_map(&mut self.bundle);
        self.sync_province_dirty();
        self.view_mode = ViewMode::Color;
        self.map_layers.base_view = MapBaseView::ProvinceColors;
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
            }
        };
    }

    pub fn set_view_mode(&mut self, alerts: &mut Alerts, view_mode: ViewMode) {
        if view_mode == ViewMode::Adjacencies {
            self.view_mode = view_mode;
            self.map_layers.show_adjacencies = true;
            alerts.push(Ok("Adjacency editing enabled over the current Map View"));
            return;
        }
        let map_view = match view_mode {
            ViewMode::Color => MapViewMode::ProvinceColors,
            ViewMode::Kind => MapViewMode::ProvinceTypes,
            ViewMode::Terrain => MapViewMode::Terrain,
            ViewMode::Continent => MapViewMode::Continents,
            ViewMode::Coastal => MapViewMode::Coastal,
            ViewMode::Adjacencies => unreachable!(),
        };
        self.set_map_view_mode(alerts, map_view);
    }

    pub fn set_map_view_mode(&mut self, alerts: &mut Alerts, map_view_mode: MapViewMode) {
        if map_view_mode.requires_state_history() && self.state_texture.is_none() {
            alerts.push(Err(
                "States and Political views are available only for loaded state projects",
            ));
        } else if map_view_mode == MapViewMode::Resources {
            // Compatibility for saved pre-overlay preferences. Resources no longer
            // replaces the map view; it is displayed over the active base layer.
            if self.map_layers.base_view == MapViewMode::Resources {
                self.map_layers.base_view = MapViewMode::States;
            }
            if !self.map_layers.show_resources {
                self.toggle_resources_overlay(alerts);
            }
        } else if map_view_mode != self.map_layers.base_view {
            // View-specific refreshes are lazy and therefore need to observe the view
            // being activated before they decide whether work is necessary.
            self.map_layers.base_view = map_view_mode;
            match map_view_mode {
                MapViewMode::ProvinceColors => {
                    self.view_mode = ViewMode::Color;
                    self.refresh();
                }
                MapViewMode::ProvinceTypes => {
                    self.view_mode = ViewMode::Kind;
                    self.refresh();
                }
                MapViewMode::Terrain => {
                    self.view_mode = ViewMode::Terrain;
                    if let Some(unknown_terrains) = self.unknown_terrains() {
                        alerts.push(Err(unknown_terrains));
                    }
                    self.refresh();
                }
                MapViewMode::Continents => {
                    self.view_mode = ViewMode::Continent;
                    self.refresh();
                }
                MapViewMode::Coastal => {
                    self.view_mode = ViewMode::Coastal;
                    self.refresh();
                }
                MapViewMode::States => {}
                MapViewMode::Political => self.refresh_political_texture(),
                MapViewMode::Resources => unreachable!("Resources is an overlay"),
            }
            self.workspace_views
                .remember(self.workspace_mode, map_view_mode);
            let label = if map_view_mode == MapViewMode::Resources {
                crate::localization::tr("view.resources").to_owned()
            } else {
                map_view_mode.label().to_owned()
            };
            alerts.push(Ok(format!("Map View: {label}")));
        } else {
            self.workspace_views
                .remember(self.workspace_mode, map_view_mode);
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
        let state_view = self.is_state_workspace();
        let lasso_active = self.state_lasso_is_active();
        let lasso_preview = matches!(self.state_lasso_phase, StateLassoPhase::Preview { .. });
        let brush_active = self.state_brush_is_active();
        let fill_active = self.state_fill_is_active();
        let fill_preview = matches!(self.state_fill_phase, StateFillPhase::Preview { .. });
        let Some(edit) = self.state_edit_session.as_ref() else {
            return StateActionAvailability::default();
        };
        let draft_modified = self.property_draft_is_modified();
        let patch_preview_stale = self
            .patch_preview
            .as_ref()
            .is_some_and(|plan| plan.is_stale(edit.revision()));
        let patch_preview_blocked = self
            .patch_preview
            .as_ref()
            .is_some_and(|plan| plan.summary.blocked_files != 0);
        let patch_preview_review_required = self
            .patch_preview
            .as_ref()
            .is_some_and(|plan| plan.summary.review_required_files != 0);
        let save_running = self.state_save_task.is_some();
        let recovery_required = self.state_save_recovery.is_some();
        let edits_blocked = save_running || recovery_required;
        let validation_current = self.round_trip_report.as_ref().is_some_and(|report| {
            report.status == super::project::RoundTripStatus::Passed
                && report.eligible_for_atomic_save_preparation
                && !report.is_stale(edit.revision(), self.patch_preview.as_ref())
        });
        let save_eligible = !edits_blocked
            && !self.property_editor_is_open()
            && !lasso_active
            && !brush_active
            && !fill_active
            && self
                .patch_preview
                .as_ref()
                .is_some_and(|plan| plan.files_len() != 0)
            && !patch_preview_stale
            && !patch_preview_blocked
            && !patch_preview_review_required
            && validation_current;
        StateActionAvailability {
            state_view,
            lasso_active,
            lasso_preview,
            brush_active,
            fill_active,
            fill_preview,
            has_selection: !edit.selected_provinces().is_empty(),
            has_target: edit
                .target_state_id()
                .is_some_and(|state_id| edit.is_state_active(state_id)),
            can_move: !edits_blocked
                && !lasso_active
                && !brush_active
                && !fill_active
                && edit.can_move_selection_to_target(),
            can_unassign: !edits_blocked
                && !lasso_active
                && !brush_active
                && !fill_active
                && edit.can_unassign_selection(),
            can_edit_properties: !edits_blocked
                && !lasso_active
                && !brush_active
                && !fill_active
                && self
                    .active_state_id
                    .is_some_and(|state_id| edit.is_state_active(state_id)),
            can_edit_province_data: !edits_blocked
                && !lasso_active
                && !brush_active
                && !fill_active
                && self
                    .active_province_id
                    .is_some_and(|province_id| edit.editable_province_state(province_id).is_ok()),
            can_create_state: !edits_blocked && !lasso_active,
            can_remove_state: !edits_blocked
                && !lasso_active
                && !brush_active
                && !fill_active
                && self
                    .active_state_id
                    .is_some_and(|state_id| edit.validate_removable_state(state_id).is_ok()),
            property_editor_open: self.property_editor_is_open(),
            property_draft_modified: draft_modified,
            can_undo: !edits_blocked && !draft_modified && edit.can_undo(),
            can_redo: !edits_blocked && !draft_modified && edit.can_redo(),
            has_edits: edit.is_dirty() || draft_modified,
            has_patch_preview: self.patch_preview.is_some(),
            patch_preview_files: self
                .patch_preview
                .as_ref()
                .map(ProjectPatchPlan::files_len)
                .unwrap_or(0),
            patch_preview_stale,
            patch_preview_blocked,
            patch_preview_review_required,
            validation_running: self.round_trip_task.is_some() || save_running,
            has_validation_report: self.round_trip_report.is_some(),
            save_eligible,
            save_running,
            save_cancellable: self
                .state_save_task
                .as_ref()
                .is_some_and(|task| task.state.cancellable()),
            recovery_required,
            has_save_report: self.state_save_report.is_some(),
            project_loaded: self.project.is_some(),
        }
    }

    pub fn generate_patch_preview(&mut self, alerts: &mut Alerts) {
        let Some((project, edit)) = self.project.as_ref().zip(self.state_edit_session.as_ref())
        else {
            alerts.push(Err(
                "Patch preview is available only for loaded state projects",
            ));
            return;
        };
        let plan = plan_state_patches(project, edit);
        println!("{}", plan.summary_text());
        for index in 0..plan.files_len() {
            if let Some(report) = plan.file_report(index) {
                println!("\n{report}");
            }
        }
        let message = format!(
            "Generated read-only patch preview: {} modified, {} created, {} removed; {} blocked",
            plan.summary.modified_files,
            plan.summary.created_files,
            plan.summary.removed_files,
            plan.summary.blocked_files,
        );
        self.patch_preview = Some(plan);
        self.patch_preview_file = 0;
        self.round_trip_report = None;
        self.round_trip_status = None;
        self.state_save_report = None;
        if self.state_save_recovery.is_none() {
            self.state_save_status = None;
        }
        self.review_required_apply_approved = false;
        self.refresh_state_information();
        alerts.push(Ok(message));
    }

    pub fn open_patch_review(&mut self, alerts: &mut Alerts) {
        if !self.ensure_current_patch_preview(alerts) {
            return;
        }
        self.state_apply_dialog = Some(
            if self.round_trip_task.is_some() || self.state_save_task.is_some() {
                StateApplyDialog::Progress
            } else if self
                .patch_preview
                .as_ref()
                .is_some_and(|plan| plan.summary.blocked_files != 0)
            {
                StateApplyDialog::Blocked
            } else {
                StateApplyDialog::Review
            },
        );
    }

    pub fn prepare_state_apply(&mut self, alerts: &mut Alerts) -> bool {
        if !self.ensure_current_patch_preview(alerts) {
            return false;
        }
        let Some(plan) = self.patch_preview.as_ref() else {
            return false;
        };
        if plan.files_len() == 0 {
            alerts.push(Ok("No state file changes are ready to apply"));
            return false;
        }
        if plan.summary.blocked_files != 0 {
            self.state_apply_dialog = Some(StateApplyDialog::Blocked);
            return false;
        }
        if plan.summary.review_required_files != 0 {
            if self.current_round_trip_status() == Some(RoundTripStatus::PassedWithReview) {
                self.review_required_apply_approved = true;
                self.state_apply_dialog = Some(StateApplyDialog::Review);
                return true;
            }
            self.review_required_apply_approved = false;
            self.state_apply_dialog = Some(StateApplyDialog::AdditionalValidation);
            return false;
        }
        if self.current_round_trip_status() == Some(RoundTripStatus::Passed) {
            self.state_apply_dialog = Some(StateApplyDialog::Review);
            return true;
        }
        if self.round_trip_task.is_some() {
            self.state_apply_after_validation = true;
            self.state_apply_dialog = Some(StateApplyDialog::Progress);
            return false;
        }
        self.state_apply_after_validation = true;
        self.start_round_trip_validation(false, alerts);
        if self.round_trip_task.is_some() {
            self.state_apply_dialog = Some(StateApplyDialog::Progress);
        } else {
            self.state_apply_after_validation = false;
        }
        false
    }

    fn ensure_current_patch_preview(&mut self, alerts: &mut Alerts) -> bool {
        let Some(edit) = self.state_edit_session.as_ref() else {
            alerts.push(Err(
                "Review and Apply are available only for state projects",
            ));
            return false;
        };
        if self.property_editor_is_open()
            || self.state_lasso_is_active()
            || self.state_brush_is_active()
            || self.state_fill_is_active()
        {
            alerts.push(Err(
                "Apply or cancel the active draft/tool before reviewing state files",
            ));
            return false;
        }
        if self
            .patch_preview
            .as_ref()
            .is_none_or(|plan| plan.is_stale(edit.revision()))
        {
            self.generate_patch_preview(alerts);
        }
        self.patch_preview.is_some()
    }

    fn current_round_trip_status(&self) -> Option<RoundTripStatus> {
        let edit = self.state_edit_session.as_ref()?;
        let plan = self.patch_preview.as_ref()?;
        self.round_trip_report
            .as_ref()
            .filter(|report| !report.is_stale(edit.revision(), Some(plan)))
            .map(|report| report.status)
    }

    /// Save Project validates a combined map/state candidate directly, rather
    /// than through the State patch-preview workflow. Its result must therefore
    /// not be queried through `current_round_trip_status`, which deliberately
    /// requires a patch preview.
    fn project_save_round_trip_status(&self) -> Option<RoundTripStatus> {
        self.project_save_validation
            .as_ref()
            .map(|validation| validation.round_trip.status)
    }

    fn active_round_trip_failure_snapshot(&self) -> Option<&RoundTripFailureSnapshot> {
        self.round_trip_failure_snapshot
            .as_ref()
            .filter(|snapshot| snapshot_belongs_to_generation(snapshot, self.project_generation))
    }

    pub fn state_apply_dialog_is_open(&self) -> bool {
        self.state_apply_dialog.is_some()
    }

    pub fn open_image_overlay_panel(&mut self) {
        self.state_apply_dialog = Some(StateApplyDialog::ImageOverlay);
    }

    pub fn validation_results_scroll(&mut self, amount: f64) -> bool {
        if self.state_apply_dialog != Some(StateApplyDialog::ValidationResults) {
            return false;
        }
        let count = self.filtered_validation_problems().len();
        let visible = if self.validation_problems_view.filters_expanded {
            3
        } else {
            4
        };
        let max_offset = count.saturating_sub(visible);
        if amount < 0.0 {
            self.validation_problems_view.offset =
                (self.validation_problems_view.offset + 1).min(max_offset);
        } else if amount > 0.0 {
            self.validation_problems_view.offset =
                self.validation_problems_view.offset.saturating_sub(1);
        }
        self.validation_problems_view.selected = self.validation_problems_view.offset;
        true
    }

    pub fn close_state_apply_dialog(&mut self) {
        if self.state_apply_dialog != Some(StateApplyDialog::Progress) {
            if self.state_apply_dialog == Some(StateApplyDialog::ProvinceRemoval) {
                self.province_removal_draft = None;
            }
            self.state_apply_dialog = None;
        }
    }

    pub fn take_state_apply_ready_for_confirmation(&mut self) -> bool {
        std::mem::take(&mut self.state_apply_ready_for_confirmation)
    }

    pub fn state_apply_dialog_click(
        &mut self,
        interface: &Interface,
        pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) -> StateApplyDialogAction {
        let Some(dialog) = self.state_apply_dialog else {
            return StateApplyDialogAction::None;
        };
        let layout = StateApplyDialogLayout::new(interface);
        if dialog == StateApplyDialog::ImageOverlay {
            if point_in_rect(pos, layout.problem_row(0)) {
                return StateApplyDialogAction::ChooseImageOverlay;
            }
            if point_in_rect(pos, layout.problem_row(1)) {
                return StateApplyDialogAction::UseProjectHeightmap;
            }
            if point_in_rect(pos, layout.problem_row(2)) {
                let track = layout.problem_row(2);
                self.map_layers.image_overlay.opacity =
                    ((pos[0] - track[0]) / track[2]).clamp(0.0, 1.0) as f32;
                self.persist_image_overlay_settings(alerts);
                return StateApplyDialogAction::None;
            }
            if point_in_rect(pos, layout.problem_row(3)) {
                return StateApplyDialogAction::DecreaseImageOverlayOpacity;
            }
            if point_in_rect(pos, layout.problem_row(4)) {
                return StateApplyDialogAction::IncreaseImageOverlayOpacity;
            }
            if point_in_rect(pos, layout.problem_row(5)) {
                return StateApplyDialogAction::ClearImageOverlay;
            }
        }
        if dialog == StateApplyDialog::ProvinceRemoval && point_in_rect(pos, layout.problem_row(0))
        {
            return StateApplyDialogAction::None;
        }
        if dialog == StateApplyDialog::ValidationResults {
            if point_in_rect(pos, layout.filters_toggle()) {
                self.validation_problems_view.filters_expanded =
                    !self.validation_problems_view.filters_expanded;
                return StateApplyDialogAction::None;
            }
            if self.validation_problems_view.filters_expanded
                && point_in_rect(pos, layout.severity_filter())
            {
                self.validation_problems_view.severity =
                    self.validation_problems_view.severity.cycle();
                self.validation_problems_view.selected = 0;
                self.validation_problems_view.offset = 0;
                return StateApplyDialogAction::None;
            }
            if self.validation_problems_view.filters_expanded
                && point_in_rect(pos, layout.source_filter())
            {
                self.validation_problems_view.source = self.validation_problems_view.source.cycle();
                self.validation_problems_view.selected = 0;
                self.validation_problems_view.offset = 0;
                return StateApplyDialogAction::None;
            }
            if self.validation_problems_view.filters_expanded
                && point_in_rect(pos, layout.domain_filter())
            {
                self.validation_problems_view.domain = self.validation_problems_view.domain.cycle();
                self.validation_problems_view.selected = 0;
                self.validation_problems_view.offset = 0;
                return StateApplyDialogAction::None;
            }
            let visible = self
                .filtered_validation_problems()
                .len()
                .saturating_sub(self.validation_problems_view.offset)
                .min(if self.validation_problems_view.filters_expanded {
                    3
                } else {
                    4
                });
            for index in 0..visible {
                if point_in_rect(
                    pos,
                    layout.validation_problem_row(
                        index,
                        self.validation_problems_view.filters_expanded,
                    ),
                ) {
                    self.validation_problems_view.selected =
                        self.validation_problems_view.offset + index;
                    return StateApplyDialogAction::None;
                }
            }
            if point_in_rect(pos, layout.validation_close()) {
                self.state_apply_dialog = None;
                return StateApplyDialogAction::None;
            }
        }
        if point_in_rect(pos, layout.primary()) {
            match dialog {
                StateApplyDialog::Review => {
                    let Some(plan) = self.patch_preview.as_ref() else {
                        return StateApplyDialogAction::None;
                    };
                    if plan.files_len() == 0 {
                        self.state_apply_dialog = None;
                    } else if plan.summary.blocked_files != 0 {
                        self.print_patch_preview_details();
                        alerts.push(Err("Blocked changes cannot be applied"));
                    } else if self.current_round_trip_status().is_some_and(|status| {
                        status == RoundTripStatus::Passed
                            || (plan.summary.review_required_files != 0
                                && status == RoundTripStatus::PassedWithReview)
                    }) {
                        self.review_required_apply_approved =
                            plan.summary.review_required_files != 0;
                        return StateApplyDialogAction::ConfirmSave;
                    } else {
                        let allow_review_required = plan.summary.review_required_files != 0;
                        self.state_apply_after_validation = false;
                        self.start_round_trip_validation(allow_review_required, alerts);
                        if self.round_trip_task.is_some() {
                            self.state_apply_dialog = Some(StateApplyDialog::Progress);
                        }
                    }
                }
                StateApplyDialog::AdditionalValidation => {
                    self.review_required_apply_approved = false;
                    self.state_apply_after_validation = true;
                    self.start_round_trip_validation(true, alerts);
                    if self.round_trip_task.is_some() {
                        self.state_apply_dialog = Some(StateApplyDialog::Progress);
                    } else {
                        self.state_apply_after_validation = false;
                    }
                }
                StateApplyDialog::ViewChanges => self.state_apply_dialog = None,
                StateApplyDialog::ProjectSaveReview => {
                    let validation_blocked = self
                        .project_validation_report
                        .as_ref()
                        .is_some_and(|report| report.delta.blocks_save());
                    let round_trip_failed = !matches!(
                        self.project_save_round_trip_status(),
                        Some(RoundTripStatus::Passed | RoundTripStatus::PassedWithReview)
                    );
                    match project_save_review_primary_action(validation_blocked, round_trip_failed)
                    {
                        ProjectSaveReviewPrimaryAction::ConfirmSave => {
                            return StateApplyDialogAction::ConfirmProjectSave;
                        }
                        ProjectSaveReviewPrimaryAction::ViewBlockingProblems => {
                            self.open_validation_problems(true, ValidationSourceFilter::All);
                        }
                        ProjectSaveReviewPrimaryAction::ViewIntegrityProblem => {
                            self.state_apply_dialog = Some(StateApplyDialog::IntegrityProblem);
                        }
                    }
                }
                StateApplyDialog::Blocked => {
                    self.print_patch_preview_details();
                    alerts.push(Err(
                        "Blocking patch diagnostics were printed to the console",
                    ));
                }
                StateApplyDialog::Progress => {
                    self.state_apply_after_validation = false;
                    if self.round_trip_task.is_some() {
                        self.cancel_round_trip_validation(alerts);
                    } else if self.state_save_can_cancel() {
                        self.cancel_state_save(alerts);
                    } else {
                        alerts.push(Err("The current save stage cannot be cancelled safely"));
                    }
                }
                StateApplyDialog::ValidationResults => {
                    let selected = self
                        .selected_validation_problem()
                        .map(|(_, diagnostic)| diagnostic);
                    let navigation = selected.and_then(|diagnostic| {
                        diagnostic
                            .province_id
                            .map(|province_id| (Some(province_id), None))
                            .or_else(|| diagnostic.state_id.map(|state_id| (None, Some(state_id))))
                    });
                    if let Some((Some(province_id), _)) = navigation {
                        self.state_apply_dialog = None;
                        self.select_province_by_id(interface, province_id, alerts);
                    } else if let Some((_, Some(state_id))) = navigation {
                        self.state_apply_dialog = None;
                        self.select_state_by_id(interface, state_id, alerts);
                    } else if let Some(path) = selected.and_then(|diagnostic| {
                        validation_source_path(diagnostic, self.project.as_ref())
                    }) {
                        return StateApplyDialogAction::OpenSource(path);
                    } else if let Some(diagnostic) = selected
                        && diagnostic.path.is_some()
                    {
                        alerts.push(Ok(format!(
                            "Pending file: {}",
                            validation_display_path(diagnostic, self.project.as_ref())
                                .strip_prefix("File: ")
                                .unwrap_or_default()
                        )));
                    } else {
                        self.validate_project_for_ui(alerts);
                    }
                }
                StateApplyDialog::IntegrityProblem => self.state_apply_dialog = None,
                StateApplyDialog::ImageOverlay => self.toggle_image_overlay(alerts),
                StateApplyDialog::ProvinceRemoval => {
                    return StateApplyDialogAction::ConfirmProvinceTransfer;
                }
                StateApplyDialog::Result => self.state_apply_dialog = None,
            }
        } else if point_in_rect(pos, layout.secondary()) {
            match dialog {
                StateApplyDialog::AdditionalValidation => {
                    self.state_apply_dialog = Some(StateApplyDialog::Review);
                }
                StateApplyDialog::ProjectSaveReview => {
                    self.open_validation_problems(false, ValidationSourceFilter::All);
                }
                StateApplyDialog::ValidationResults => {
                    self.validation_problems_view.show_technical_details =
                        !self.validation_problems_view.show_technical_details
                }
                StateApplyDialog::IntegrityProblem => {
                    if let Some(snapshot) = self.active_round_trip_failure_snapshot() {
                        return StateApplyDialogAction::CopyDetails(snapshot.details.clone());
                    }
                }
                StateApplyDialog::ImageOverlay => self.state_apply_dialog = None,
                StateApplyDialog::ProvinceRemoval => {
                    return StateApplyDialogAction::ConfirmProvinceReferenceRemoval;
                }
                _ if self.state_save_report.is_some() => self.view_state_save_report(alerts),
                _ if self.round_trip_report.is_some() => self.view_round_trip_report(alerts),
                _ => self.print_patch_preview_details(),
            }
        } else if point_in_rect(pos, layout.close()) && dialog != StateApplyDialog::Progress {
            if dialog == StateApplyDialog::ValidationResults {
                if let Some((source, diagnostic)) = self.selected_validation_problem() {
                    return StateApplyDialogAction::CopyDetails(validation_problem_details(
                        source, diagnostic,
                    ));
                }
            } else {
                self.state_apply_dialog = None;
                if dialog == StateApplyDialog::ProvinceRemoval {
                    self.province_removal_draft = None;
                }
            }
        }
        StateApplyDialogAction::None
    }

    fn filtered_validation_problems(
        &self,
    ) -> Vec<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
        let Some(report) = self.project_validation_report.as_ref() else {
            return Vec::new();
        };
        validation_delta_items(report)
            .into_iter()
            .filter(|(source, diagnostic)| {
                validation_problem_matches(&self.validation_problems_view, *source, diagnostic)
            })
            .collect()
    }

    pub fn open_view_changes(&mut self) {
        self.state_apply_dialog = Some(StateApplyDialog::ViewChanges);
    }

    fn open_validation_problems(&mut self, blocking_only: bool, source: ValidationSourceFilter) {
        self.validation_problems_view = ValidationProblemsView {
            blocking_only,
            source,
            ..ValidationProblemsView::default()
        };
        self.state_apply_dialog = Some(StateApplyDialog::ValidationResults);
    }

    fn selected_validation_problem(
        &self,
    ) -> Option<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
        let problems = self.filtered_validation_problems();
        problems
            .get(
                self.validation_problems_view
                    .selected
                    .min(problems.len().saturating_sub(1)),
            )
            .copied()
    }

    fn print_patch_preview_details(&self) {
        let Some(plan) = self.patch_preview.as_ref() else {
            return;
        };
        println!("{}", plan.summary_text());
        for index in 0..plan.files_len() {
            if let Some(report) = plan.file_report(index) {
                println!("\n{report}");
            }
        }
    }

    pub fn select_patch_preview_file(&mut self, offset: isize, alerts: &mut Alerts) {
        let Some(plan) = self.patch_preview.as_ref() else {
            alerts.push(Err("Generate a patch preview first"));
            return;
        };
        let count = plan.files_len();
        if count == 0 {
            alerts.push(Ok("The patch preview contains no file operations"));
            return;
        }
        self.patch_preview_file =
            (self.patch_preview_file as isize + offset).rem_euclid(count as isize) as usize;
        if let Some(report) = plan.file_report(self.patch_preview_file) {
            println!("{report}");
        }
        self.refresh_state_information();
        alerts.push(Ok(format!(
            "Patch preview file {} of {count}",
            self.patch_preview_file + 1,
        )));
    }

    pub fn clear_patch_preview(&mut self, alerts: &mut Alerts) {
        self.patch_preview = None;
        self.patch_preview_file = 0;
        self.review_required_apply_approved = false;
        self.refresh_state_information();
        alerts.push(Ok("Cleared the in-memory patch preview"));
    }

    pub fn start_round_trip_validation(
        &mut self,
        allow_review_required: bool,
        alerts: &mut Alerts,
    ) {
        if self.round_trip_task.is_some() {
            alerts.push(Err("A round-trip validation is already running"));
            return;
        }
        let Some(project) = self.project.as_ref().cloned() else {
            alerts.push(Err(
                "Round-trip validation is available only for state projects",
            ));
            return;
        };
        let Some(edit) = self.state_edit_session.as_ref().cloned() else {
            alerts.push(Err("The state edit session is unavailable"));
            return;
        };
        let Some(plan) = self.patch_preview.as_ref().cloned() else {
            alerts.push(Err("Generate a patch preview before validating"));
            return;
        };
        if plan.is_stale(edit.revision()) {
            alerts.push(Err(
                "Patch preview is outdated. Regenerate it before round-trip validation.",
            ));
            return;
        }
        if plan.summary.blocked_files != 0 {
            alerts.push(Err("Blocked patch plans cannot be validated"));
            return;
        }
        if plan.summary.review_required_files != 0 && !allow_review_required {
            alerts.push(Err(
                "ReviewRequired changes need the explicit isolated validation action",
            ));
            return;
        }

        let (sender, receiver) = mpsc::channel();
        let cancellation = RoundTripCancellation::default();
        let worker_cancellation = cancellation.clone();
        let policy = RoundTripValidationPolicy {
            allow_review_required,
            ..Default::default()
        };
        let spawn = thread::Builder::new()
            .name("hoi4-roundtrip-validation".to_owned())
            .spawn(move || {
                let validator = RoundTripValidator { policy };
                let report =
                    validator.validate(&project, &edit, &plan, &worker_cancellation, |stage| {
                        let _ = sender.send(RoundTripTaskMessage::Stage(stage));
                    });
                let _ = sender.send(RoundTripTaskMessage::Finished(Box::new(report)));
            });
        match spawn {
            Ok(_) => {
                self.round_trip_task = Some(RoundTripTask {
                    receiver,
                    cancellation,
                });
                self.round_trip_status =
                    Some("Round-trip validation: checking patch plan...".to_owned());
                self.refresh_state_information();
                alerts.push(Ok(
                    "Temporary validation started. The original mod will not be changed.",
                ));
            }
            Err(err) => alerts.push(Err(format!(
                "Failed to start temporary validation worker: {err}"
            ))),
        }
    }

    pub fn cancel_round_trip_validation(&mut self, alerts: &mut Alerts) {
        let Some(task) = self.round_trip_task.as_ref() else {
            alerts.push(Err("No round-trip validation is running"));
            return;
        };
        task.cancellation.cancel();
        self.round_trip_status =
            Some("Round-trip validation: cancellation requested...".to_owned());
        self.refresh_state_information();
        alerts.push(Ok(
            "Cancellation requested; the current file operation will finish safely",
        ));
    }

    /// The replacement Canvas will never receive this worker's result. Ask it
    /// to stop as well so a discarded project cannot keep consuming I/O after
    /// a successful project switch.
    pub(crate) fn retire_project_context(&mut self) {
        if let Some(task) = self.round_trip_task.as_ref() {
            task.cancellation.cancel();
        }
    }

    pub fn view_round_trip_report(&mut self, alerts: &mut Alerts) {
        let Some(report) = self.round_trip_report.as_ref() else {
            alerts.push(Err("No round-trip validation report is available"));
            return;
        };
        println!("{}", report.full_text());
        self.round_trip_status = Some(report.summary_text());
        self.refresh_state_information();
        alerts.push(Ok(
            "Printed the full round-trip validation report to the console",
        ));
    }

    pub fn clear_round_trip_report(&mut self, alerts: &mut Alerts) {
        if self.round_trip_task.is_some() {
            alerts.push(Err(
                "Cancel the running validation before clearing its result",
            ));
            return;
        }
        self.round_trip_report = None;
        self.round_trip_failure_snapshot = None;
        self.round_trip_status = None;
        self.review_required_apply_approved = false;
        self.refresh_state_information();
        alerts.push(Ok("Cleared the in-memory round-trip validation result"));
    }

    pub fn state_save_confirmation_message(&self) -> Result<String, String> {
        if self.province_save_task.is_some() {
            return Err("Wait for the running Province Save or export to finish.".to_owned());
        }
        if self.round_trip_task.is_some() {
            return Err("Wait for the running round-trip validation to finish.".to_owned());
        }
        let project = self.project.as_ref().ok_or_else(|| {
            "Apply State Changes is available only for loaded state projects.".to_owned()
        })?;
        let edit = self
            .state_edit_session
            .as_ref()
            .ok_or_else(|| "The state edit session is unavailable.".to_owned())?;
        let eligibility = state_save_eligibility(
            project,
            edit,
            self.patch_preview.as_ref(),
            self.round_trip_report.as_ref(),
            StateSaveConditions {
                draft_pending: self.property_editor_is_open(),
                tool_interaction_active: self.state_lasso_is_active()
                    || self.state_brush_is_active()
                    || self.state_fill_is_active(),
                recovery_required: self.state_save_recovery.is_some(),
                save_running: self.state_save_task.is_some(),
                allow_review_required: self.review_required_apply_approved,
            },
        );
        if !eligibility.eligible {
            return Err(eligibility
                .reasons
                .first()
                .map(|reason| reason.message())
                .unwrap_or("Apply State Changes is not eligible.")
                .to_owned());
        }
        Ok(save_confirmation_text(
            project,
            self.patch_preview
                .as_ref()
                .expect("eligible Save has a patch plan"),
        ))
    }

    pub fn start_state_save(&mut self, alerts: &mut Alerts) {
        if self.province_save_task.is_some() {
            alerts.push(Err(
                "Wait for the running Province Save or export to finish",
            ));
            return;
        }
        if self.round_trip_task.is_some() {
            alerts.push(Err("Wait for the running round-trip validation to finish"));
            return;
        }
        let Some(project) = self.project.as_ref().cloned() else {
            alerts.push(Err(
                "Apply State Changes is available only for loaded state projects",
            ));
            return;
        };
        let Some(edit) = self.state_edit_session.as_ref().cloned() else {
            alerts.push(Err("The state edit session is unavailable"));
            return;
        };
        let plan = self.patch_preview.as_ref().cloned();
        let report = self.round_trip_report.as_ref().cloned();
        let eligibility = state_save_eligibility(
            &project,
            &edit,
            plan.as_ref(),
            report.as_ref(),
            StateSaveConditions {
                draft_pending: self.property_editor_is_open(),
                tool_interaction_active: self.state_lasso_is_active()
                    || self.state_brush_is_active()
                    || self.state_fill_is_active(),
                recovery_required: self.state_save_recovery.is_some(),
                save_running: self.state_save_task.is_some(),
                allow_review_required: self.review_required_apply_approved,
            },
        );
        let Some(authorization) = eligibility.authorization else {
            alerts.push(Err(eligibility
                .reasons
                .first()
                .map(|reason| reason.message())
                .unwrap_or("Apply State Changes is not eligible.")));
            return;
        };
        let plan = plan.expect("eligible Save has a patch plan");
        let report = report.expect("eligible Save has a validation report");
        let (sender, receiver) = mpsc::channel();
        let cancellation = StateSaveCancellation::default();
        let worker_cancellation = cancellation.clone();
        let spawn = thread::Builder::new()
            .name("hoi4-state-save".to_owned())
            .spawn(move || {
                let report = execute_state_save(
                    &project,
                    &edit,
                    &plan,
                    &report,
                    &authorization,
                    &worker_cancellation,
                    StateSaveFault::None,
                    |state, current, total| {
                        let _ = sender.send(StateSaveTaskMessage::Stage(state, current, total));
                    },
                );
                let _ = sender.send(StateSaveTaskMessage::Finished(Box::new(report)));
            });
        match spawn {
            Ok(_) => {
                self.state_save_task = Some(StateSaveTask {
                    receiver,
                    cancellation,
                    state: SaveTransactionState::Preparing,
                    includes_province_map: false,
                });
                self.state_save_status = Some("Apply State Changes: Preparing...".to_owned());
                self.state_apply_dialog = Some(StateApplyDialog::Progress);
                self.refresh_state_information();
                alerts.push(Ok(
                    "Apply State Changes started; editing is locked until it finishes safely",
                ));
            }
            Err(error) => alerts.push(Err(format!(
                "Failed to start Apply State Changes worker: {error}"
            ))),
        }
    }

    pub fn prepare_project_save(&mut self) -> Result<String, String> {
        if self.save_blocks_editing() {
            return Err(
                "Finish or recover the active save before starting Save Project.".to_owned(),
            );
        }
        if self.property_editor_is_open()
            || self.state_lasso_is_active()
            || self.state_brush_is_active()
            || self.state_fill_is_active()
        {
            return Err("Apply or cancel the active draft/tool before Save Project.".to_owned());
        }
        // A new Save Project preparation supersedes any previous blocker. The
        // next failed candidate will install a fresh immutable snapshot.
        self.round_trip_failure_snapshot = None;
        // A map edit can introduce a stable Province ID before the State
        // workspace is entered. Reconcile that identity immediately before
        // deriving both the State patch plan and the map candidate so they
        // describe the exact same candidate project.
        self.synchronize_created_state_provinces();
        let project = self
            .project
            .as_ref()
            .ok_or_else(|| "Save Project requires a loaded HOI4 mod project.".to_owned())?;
        let edit = self
            .state_edit_session
            .as_ref()
            .ok_or_else(|| "The project state edit session is unavailable.".to_owned())?;
        let state_plan = plan_state_patches(project, edit);
        if state_plan.files_len() != 0 && !project.state_load_is_complete() {
            return Err(format!(
                "Save Project blocked: {}",
                project.state_load_failure_message()
            ));
        }
        let geometry_changed = self.history.has_geometry_changes(&self.bundle.map);
        let mut candidate_bundle = self.bundle.clone();
        let coastal_flags_recalculated =
            if geometry_changed && self.bundle.config.generate_coastal_on_save {
                candidate_bundle.map.recalculate_coastal_flags()
            } else {
                0
            };
        let province_candidate = self
            .has_unsaved_province_edits()
            .then(|| build_province_map_candidate(&candidate_bundle))
            .transpose()?;
        if state_plan.files_len() == 0 && province_candidate.is_none() {
            self.project_save_plan = None;
            self.project_save_validation = None;
            return Err("No changes to save.".to_owned());
        }
        if let Some(candidate) = province_candidate.as_ref() {
            validate_province_map_candidate(candidate, |path| {
                candidate
                    .files
                    .get(path)
                    .cloned()
                    .ok_or_else(|| format!("Missing candidate file: {}", path.display()))
            })?;
        }
        let validator = RoundTripValidator {
            policy: RoundTripValidationPolicy {
                allow_review_required: true,
                ..RoundTripValidationPolicy::default()
            },
        };
        let cancellation = RoundTripCancellation::default();
        let map_files = province_candidate
            .as_ref()
            .map(|candidate| &candidate.files);
        let combined = validator.validate_combined(
            project,
            edit,
            &state_plan,
            map_files,
            |map_directory| {
                let Some(candidate) = province_candidate.as_ref() else {
                    return Ok(());
                };
                validate_province_map_candidate(candidate, |path| {
                    std::fs::read(map_directory.join(path)).map_err(|error| {
                        format!("Cannot read combined candidate {}: {error}", path.display())
                    })
                })
            },
            &cancellation,
            |_| {},
        );
        if !matches!(
            combined.round_trip.status,
            RoundTripStatus::Passed | RoundTripStatus::PassedWithReview
        ) {
            // Keep the Save Project review open with its immutable blocker
            // snapshot. Sending this through ValidationResults made a
            // round-trip failure look like an ordinary source diagnostic.
            self.remember_combined_validation(&combined, StateApplyDialog::ProjectSaveReview);
            return Err(combined.round_trip.summary_text());
        }
        let mut plan = ProjectSavePlan::new(
            project,
            edit.revision(),
            province_candidate.as_ref(),
            Some(&state_plan),
        )?;
        plan.set_coastal_flags_recalculated(coastal_flags_recalculated);
        if combined.candidate_digest != *plan.candidate_digest() {
            return Err(
                "The validated candidate no longer matches the Save Project plan.".to_owned(),
            );
        }
        let validation = combined.project_validation.as_ref();
        let text = format!(
            "SAVE PROJECT\n\nProvince Map: {} file(s)\nStates: {} file(s)\nCoastal flags recalculated: {}\n\nValidation: {} error(s), {} warning(s), {} information message(s)\nFiles affected: {}\n\nA combined verified backup and journal will be created. Each file replacement is atomic; the coordinated project save is rollback-capable.",
            plan.dirty().province_files,
            plan.dirty().state_files,
            plan.coastal_flags_recalculated(),
            validation.map_or(0, |report| report.errors),
            validation.map_or(0, |report| report.warnings),
            validation.map_or(0, |report| report.information),
            plan.patch_plan().files_len(),
        );
        self.project_save_plan = Some(plan);
        self.remember_combined_validation(&combined, StateApplyDialog::ProjectSaveReview);
        self.project_save_validation = Some(combined);
        Ok(text)
    }

    fn remember_combined_validation(
        &mut self,
        combined: &CombinedRoundTripValidationReport,
        dialog: StateApplyDialog,
    ) {
        self.project_validation_report = combined.project_validation.clone();
        self.round_trip_report = Some(combined.round_trip.clone());
        self.round_trip_status = Some(combined.round_trip.summary_text());
        self.round_trip_failure_snapshot = (!matches!(
            combined.round_trip.status,
            RoundTripStatus::Passed | RoundTripStatus::PassedWithReview
        ))
        .then(|| {
            RoundTripFailureSnapshot::from_report(self.project_generation, &combined.round_trip)
        });
        let result = if !matches!(
            combined.round_trip.status,
            RoundTripStatus::Passed | RoundTripStatus::PassedWithReview
        ) {
            combined.round_trip.status.label().to_owned()
        } else {
            combined.project_validation.as_ref().map_or_else(
                || combined.round_trip.status.label().to_owned(),
                |report| {
                    if report.delta.has_preexisting_errors()
                        && !report.delta.blocks_save()
                        && !report.delta.requires_warning_review()
                    {
                        "PASSED WITH PRE-EXISTING ISSUES".to_owned()
                    } else {
                        combined.round_trip.status.label().to_owned()
                    }
                },
            )
        };
        self.last_validation = Some(LastValidationState {
            target: ProjectValidationTarget::PendingChanges,
            result,
            duration_ms: combined.round_trip.timings.total_ms,
            semantic: Some(
                combined.round_trip.semantic_comparison.states_match
                    && combined
                        .round_trip
                        .semantic_comparison
                        .province_coverage_match,
            ),
            indexes: Some(combined.round_trip.semantic_comparison.indexes_match),
            bytes: Some(combined.round_trip.byte_comparison.differences.is_empty()),
            unexpected_diagnostics: combined
                .round_trip
                .diagnostics_comparison
                .unexpected_diagnostics,
        });
        self.validation_problems_view = ValidationProblemsView::default();
        self.state_apply_dialog = Some(dialog);
        self.refresh_state_information();
    }

    pub fn validate_project_for_ui(&mut self, alerts: &mut Alerts) {
        if self.has_unsaved_province_edits() || self.has_unsaved_state_edits() {
            match self.prepare_project_save() {
                Ok(review) => {
                    println!("{review}");
                    self.state_apply_dialog = Some(StateApplyDialog::ValidationResults);
                }
                Err(error) => alerts.push(Err(error)),
            }
            return;
        }
        let Some(project) = self.project.as_ref() else {
            alerts.push(Err("Validate Project requires a loaded HOI4 mod project."));
            return;
        };
        let started = Instant::now();
        let report = validate_project(
            &self.bundle,
            project,
            ProjectValidationTarget::CurrentProject,
        );
        let summary = format!(
            "Validation Results: {} error(s), {} warning(s), {} information message(s)",
            report.errors, report.warnings, report.information
        );
        if report.blocks_save {
            alerts.push(Err(summary.clone()));
        } else {
            alerts.push(Ok(summary.clone()));
        }
        self.last_validation = Some(LastValidationState {
            target: ProjectValidationTarget::CurrentProject,
            result: if report.blocks_save {
                "Blocked"
            } else {
                "Passed"
            }
            .to_owned(),
            duration_ms: started.elapsed().as_millis(),
            semantic: None,
            indexes: None,
            bytes: None,
            unexpected_diagnostics: 0,
        });
        self.project_validation_report = Some(report);
        self.validation_problems_view = ValidationProblemsView::default();
        self.state_apply_dialog = Some(StateApplyDialog::ValidationResults);
        self.refresh_state_information();
    }

    pub fn start_project_save(&mut self, allow_review_required: bool, alerts: &mut Alerts) {
        let Some(project) = self.project.as_ref().cloned() else {
            alerts.push(Err("Save Project requires a loaded HOI4 mod project."));
            return;
        };
        let Some(edit) = self.state_edit_session.as_ref().cloned() else {
            alerts.push(Err("The project state edit session is unavailable."));
            return;
        };
        let Some(plan) = self.project_save_plan.as_ref().cloned() else {
            alerts.push(Err("Review Project Changes before saving."));
            return;
        };
        let Some(validation) = self.project_save_validation.as_ref().cloned() else {
            alerts.push(Err("Validate Project before saving."));
            return;
        };
        if edit.revision() != plan.patch_plan().generation {
            alerts.push(Err(
                "The project changed in memory after validation. Validate again.",
            ));
            return;
        }
        let includes_province_map = plan.dirty().province_files != 0;
        let (sender, receiver) = mpsc::channel();
        let cancellation = StateSaveCancellation::default();
        let worker_cancellation = cancellation.clone();
        let spawn = thread::Builder::new()
            .name("hoi4-project-save".to_owned())
            .spawn(move || {
                let report = execute_project_save(
                    &project,
                    &edit,
                    &plan,
                    &validation,
                    allow_review_required,
                    &worker_cancellation,
                    StateSaveFault::None,
                    |state, current, total| {
                        let _ = sender.send(StateSaveTaskMessage::Stage(state, current, total));
                    },
                );
                let _ = sender.send(StateSaveTaskMessage::Finished(Box::new(report)));
            });
        match spawn {
            Ok(_) => {
                self.state_save_task = Some(StateSaveTask {
                    receiver,
                    cancellation,
                    state: SaveTransactionState::Preparing,
                    includes_province_map,
                });
                self.state_save_status = Some("Save Project: Preparing...".to_owned());
                self.state_apply_dialog = Some(StateApplyDialog::Progress);
                self.refresh_state_information();
                alerts.push(Ok(
                    "Save Project started; editing is locked until verification finishes",
                ));
            }
            Err(error) => alerts.push(Err(format!("Failed to start Save Project: {error}"))),
        }
    }

    pub fn cancel_state_save(&mut self, alerts: &mut Alerts) {
        let Some(task) = self.state_save_task.as_ref() else {
            alerts.push(Err("No Apply State Changes transaction is running"));
            return;
        };
        if !task.state.cancellable() {
            alerts.push(Err(
                "Apply State Changes cannot be cancelled after commit begins",
            ));
            return;
        }
        task.cancellation.cancel();
        self.state_save_status = Some("Apply State Changes: cancellation requested...".to_owned());
        self.refresh_state_information();
        alerts.push(Ok("Cancellation requested before commit"));
    }

    pub fn view_state_save_report(&mut self, alerts: &mut Alerts) {
        let Some(report) = self.state_save_report.as_ref() else {
            alerts.push(Err("No Apply State Changes report is available"));
            return;
        };
        println!("{}", report.summary_text());
        self.state_save_status = Some(report.summary_text());
        self.refresh_state_information();
        alerts.push(Ok("Printed the Apply State Changes report to the console"));
    }

    pub fn recover_state_save(&mut self, alerts: &mut Alerts) {
        if self.state_save_task.is_some() {
            alerts.push(Err(
                "An Apply State Changes or recovery task is already running",
            ));
            return;
        }
        let Some(project) = self.project.as_ref() else {
            alerts.push(Err("No state project is loaded"));
            return;
        };
        if self.state_save_recovery.is_none() {
            alerts.push(Err("No interrupted Apply State Changes was detected"));
            return;
        }
        let root = project.paths.root.clone();
        let (sender, receiver) = mpsc::channel();
        let spawn = thread::Builder::new()
            .name("hoi4-state-save-recovery".to_owned())
            .spawn(move || {
                let _ = sender.send(StateSaveTaskMessage::Stage(
                    SaveTransactionState::RollingBack,
                    0,
                    0,
                ));
                let report = recover_interrupted_state_save(&root);
                let _ = sender.send(StateSaveTaskMessage::Finished(Box::new(report)));
            });
        match spawn {
            Ok(_) => {
                self.state_save_task = Some(StateSaveTask {
                    receiver,
                    cancellation: StateSaveCancellation::default(),
                    state: SaveTransactionState::RollingBack,
                    includes_province_map: false,
                });
                self.state_save_status =
                    Some("Apply State Changes recovery: Rolling back...".to_owned());
                self.refresh_state_information();
                alerts.push(Ok("Verified recovery rollback started"));
            }
            Err(error) => alerts.push(Err(format!("Failed to start recovery worker: {error}"))),
        }
    }

    pub fn state_save_is_running(&self) -> bool {
        self.state_save_task.is_some()
    }

    pub fn state_save_blocks_editing(&self) -> bool {
        self.state_save_task.is_some() || self.state_save_recovery.is_some()
    }

    pub fn state_save_blocks_close(&self) -> bool {
        self.state_save_task
            .as_ref()
            .is_some_and(|task| !task.state.cancellable())
            || self.state_save_recovery.is_some()
    }

    pub fn state_save_can_cancel(&self) -> bool {
        self.state_save_task
            .as_ref()
            .is_some_and(|task| task.state.cancellable())
    }

    pub fn start_province_save(
        &mut self,
        location: Location,
        mode: ProvinceSaveMode,
        alerts: &mut Alerts,
    ) {
        if self.state_save_task.is_some()
            || self.round_trip_task.is_some()
            || self.province_save_task.is_some()
        {
            alerts.push(Err(
                "Wait for the active save, export, or validation to finish",
            ));
            return;
        }
        if self.map_access_mode == MapAccessMode::ReadOnly {
            alerts.push(Err("Province map files are read-only in this project"));
            return;
        }
        if mode.clears_dirty() && self.bundle.config.generate_coastal_on_save {
            self.history.calculate_coastal_provinces(&mut self.bundle);
            self.sync_province_dirty();
        }

        let bundle = self.bundle.clone();
        let (sender, receiver) = mpsc::channel();
        let cancellation = ProvinceSaveCancellation::default();
        let worker_cancellation = cancellation.clone();
        let worker_location = location.clone();
        let spawn = thread::Builder::new()
            .name("hoi4-province-save".to_owned())
            .spawn(move || {
                let result = execute_province_save(
                    &worker_location,
                    &bundle,
                    mode,
                    &worker_cancellation,
                    |progress| {
                        let _ = sender.send(ProvinceSaveTaskMessage::Progress(progress));
                    },
                )
                .map(Box::new);
                let _ = sender.send(ProvinceSaveTaskMessage::Finished(result));
            });
        match spawn {
            Ok(_) => {
                self.province_save_report = None;
                self.province_save_task = Some(ProvinceSaveTask {
                    receiver,
                    cancellation,
                    stage: ProvinceSaveStage::Preparing,
                });
                self.province_save_status = Some(
                    "SAVING PROVINCE MAP\nPreparing province data\nThe original mod files have not been changed yet."
                        .to_owned(),
                );
                alerts.push(Ok(match mode {
                    ProvinceSaveMode::Save => {
                        "Province Save started; the original files remain unchanged until commit"
                    }
                    ProvinceSaveMode::Export => {
                        "Province export started; the open project and dirty state will not change"
                    }
                }));
            }
            Err(error) => alerts.push(Err(format!(
                "Failed to start Province Save worker: {error}"
            ))),
        }
    }

    pub fn cancel_province_save(&mut self, alerts: &mut Alerts) {
        let Some(task) = self.province_save_task.as_ref() else {
            alerts.push(Err("No Province Save or export is running"));
            return;
        };
        if !task.stage.cancellable() {
            alerts.push(Err(
                "Province Save cannot be cancelled after validated files start applying",
            ));
            return;
        }
        task.cancellation.cancel();
        self.province_save_status =
            Some("Province Save: cancellation requested before commit...".to_owned());
        alerts.push(Ok(
            "Cancellation requested; destination files remain unchanged",
        ));
    }

    pub fn save_is_running(&self) -> bool {
        self.state_save_task.is_some() || self.province_save_task.is_some()
    }

    pub fn save_blocks_editing(&self) -> bool {
        self.province_save_task.is_some()
            || self.state_save_task.is_some()
            || self.state_save_recovery.is_some()
    }

    pub fn save_blocks_close(&self) -> bool {
        self.state_save_blocks_close()
            || self
                .province_save_task
                .as_ref()
                .is_some_and(|task| !task.stage.cancellable())
    }

    pub fn save_can_cancel(&self) -> bool {
        self.state_save_can_cancel()
            || self
                .province_save_task
                .as_ref()
                .is_some_and(|task| task.stage.cancellable())
    }

    pub fn cancel_active_save(&mut self, alerts: &mut Alerts) {
        if self.province_save_task.is_some() {
            self.cancel_province_save(alerts);
        } else {
            self.cancel_state_save(alerts);
        }
    }

    fn poll_round_trip_validation(&mut self) {
        let mut stages = Vec::new();
        let mut finished = None;
        let mut disconnected = false;
        if let Some(task) = self.round_trip_task.as_ref() {
            loop {
                match task.receiver.try_recv() {
                    Ok(RoundTripTaskMessage::Stage(stage)) => stages.push(stage),
                    Ok(RoundTripTaskMessage::Finished(report)) => {
                        finished = Some(*report);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut changed = false;
        if let Some(stage) = stages.last().copied() {
            self.round_trip_status = Some(format!("Round-trip validation: {}", stage.message()));
            changed = true;
        }
        if let Some(report) = finished {
            println!("{}", report.full_text());
            let passed = matches!(
                report.status,
                RoundTripStatus::Passed | RoundTripStatus::PassedWithReview
            ) && report.eligible_for_atomic_save_preparation;
            if self.state_apply_after_validation && passed {
                self.review_required_apply_approved =
                    report.status == RoundTripStatus::PassedWithReview;
                self.state_apply_ready_for_confirmation = true;
            }
            if self.state_apply_dialog == Some(StateApplyDialog::Progress) {
                self.state_apply_dialog = Some(if passed {
                    StateApplyDialog::Review
                } else {
                    StateApplyDialog::Result
                });
            }
            self.state_apply_after_validation = false;
            self.round_trip_status = Some(report.summary_text());
            self.round_trip_report = Some(report);
            self.round_trip_task = None;
            changed = true;
        } else if disconnected {
            self.round_trip_status =
                Some("Round-trip validation: worker ended without a report.".to_owned());
            self.round_trip_task = None;
            self.state_apply_after_validation = false;
            self.state_apply_dialog = Some(StateApplyDialog::Result);
            changed = true;
        }
        if changed {
            self.refresh_state_information();
        }
    }

    fn poll_state_save(&mut self) {
        let mut stages = Vec::new();
        let mut finished = None;
        let mut disconnected = false;
        if let Some(task) = self.state_save_task.as_ref() {
            loop {
                match task.receiver.try_recv() {
                    Ok(StateSaveTaskMessage::Stage(state, current, total)) => {
                        stages.push((state, current, total));
                    }
                    Ok(StateSaveTaskMessage::Finished(report)) => {
                        finished = Some(*report);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if let Some((state, current, total)) = stages.last().copied() {
            let action = if self
                .state_save_task
                .as_ref()
                .is_some_and(|task| task.includes_province_map)
            {
                "Save Project"
            } else {
                "Apply State Changes"
            };
            if let Some(task) = self.state_save_task.as_mut() {
                task.state = state;
            }
            self.state_save_status = Some(if total == 0 {
                format!("{action}: {}...", state.label())
            } else {
                format!("{action}: {} {current}/{total}...", state.label())
            });
        }
        if let Some(mut report) = finished {
            let includes_province_map = self
                .state_save_task
                .as_ref()
                .is_some_and(|task| task.includes_province_map);
            let coastal_flags_recalculated = self
                .project_save_plan
                .as_ref()
                .map_or(0, ProjectSavePlan::coastal_flags_recalculated);
            self.last_project_save_summary = self
                .project_save_plan
                .as_ref()
                .map(ProjectSavePresentationSummary::from_plan);
            println!("{}", report.summary_text());
            if report.outcome == StateSaveOutcome::Completed {
                if let Some(reloaded) = report.reloaded_project.take() {
                    self.promote_saved_project(reloaded);
                }
                if includes_province_map {
                    if coastal_flags_recalculated != 0 {
                        let applied = self.bundle.map.recalculate_coastal_flags();
                        debug_assert_eq!(applied, coastal_flags_recalculated);
                    }
                    self.history.promote_baseline(&self.bundle.map);
                    self.sync_province_dirty();
                }
                self.state_save_recovery = None;
            } else if report.outcome == StateSaveOutcome::RolledBack {
                if let Some(reloaded) = report.reloaded_project.take() {
                    self.promote_saved_project(reloaded);
                }
                self.state_save_recovery = None;
                self.patch_preview = None;
                self.round_trip_report = None;
                self.round_trip_status = None;
                self.review_required_apply_approved = false;
            } else if report.outcome == StateSaveOutcome::RecoveryRequired
                || report.outcome == StateSaveOutcome::RollbackFailed
            {
                self.state_save_recovery = self
                    .project
                    .as_ref()
                    .and_then(|project| detect_state_save_recovery(&project.paths.root));
            }
            let mut summary = report.summary_text();
            if includes_province_map {
                summary.push_str(&format!(
                    "\nCoastal flags recalculated: {coastal_flags_recalculated}"
                ));
            }
            self.state_save_status = Some(summary);
            self.state_save_report = Some(report);
            self.state_save_task = None;
            self.project_save_plan = None;
            self.project_save_validation = None;
            self.state_apply_dialog = Some(StateApplyDialog::Result);
            self.refresh_state_information();
        } else if disconnected {
            let action = if self
                .state_save_task
                .as_ref()
                .is_some_and(|task| task.includes_province_map)
            {
                "Save Project"
            } else {
                "Apply State Changes"
            };
            self.state_save_status = Some(format!(
                "{action} worker ended without a report; recovery may be required."
            ));
            self.state_save_task = None;
            self.state_save_recovery = self
                .project
                .as_ref()
                .and_then(|project| detect_state_save_recovery(&project.paths.root));
            self.state_apply_dialog = Some(StateApplyDialog::Result);
            self.refresh_state_information();
        } else if !stages.is_empty() {
            self.refresh_state_information();
        }
    }

    fn poll_province_save(&mut self) {
        let mut progress_updates = Vec::new();
        let mut finished = None;
        let mut disconnected = false;
        if let Some(task) = self.province_save_task.as_ref() {
            loop {
                match task.receiver.try_recv() {
                    Ok(ProvinceSaveTaskMessage::Progress(progress)) => {
                        progress_updates.push(progress);
                    }
                    Ok(ProvinceSaveTaskMessage::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if let Some(progress) = progress_updates.last().copied() {
            if let Some(task) = self.province_save_task.as_mut() {
                task.stage = progress.stage;
            }
            let detail = progress
                .percent()
                .map_or_else(String::new, |percent| format!(" {percent}%"));
            let safety = if progress.stage.cancellable() {
                "\nThe original mod files have not been changed yet."
            } else if progress.stage == ProvinceSaveStage::Committing {
                "\nApplying validated files. Do not close the application."
            } else {
                ""
            };
            self.province_save_status = Some(format!(
                "SAVING PROVINCE MAP\n{}{}{}",
                progress.stage.label(),
                detail,
                safety
            ));
        }
        if let Some(result) = finished {
            match result {
                Ok(report) => {
                    println!("{}", report.summary_text());
                    let states_pending =
                        report.mode.clears_dirty() && self.has_unsaved_state_edits();
                    if report.mode.clears_dirty() {
                        self.history.promote_baseline(&self.bundle.map);
                        self.sync_province_dirty();
                    }
                    let mut summary = report.summary_text();
                    if states_pending {
                        summary.push_str("\nProvince Map saved. State changes are still pending.");
                    }
                    self.province_save_status = Some(summary);
                    self.province_save_report = Some(*report);
                }
                Err(error) => {
                    eprintln!("Province Save failed: {error}");
                    self.province_save_status = Some(format!(
                        "Province Save failed\n{error}\nProvince changes remain pending."
                    ));
                }
            }
            self.province_save_task = None;
            self.refresh_state_information();
        } else if disconnected {
            self.province_save_status = Some(
                "Province Save worker ended without a report. Province changes remain pending."
                    .to_owned(),
            );
            self.province_save_task = None;
            self.refresh_state_information();
        } else if !progress_updates.is_empty() {
            self.refresh_state_information();
        }
    }

    fn promote_saved_project(&mut self, reloaded: Hoi4Project) {
        let selected = self
            .state_edit_session
            .as_ref()
            .map(|edit| edit.selected_provinces().clone())
            .unwrap_or_default();
        let target = self
            .state_edit_session
            .as_ref()
            .and_then(StateEditSession::target_state_id);
        let active_state = self.active_state_id;
        let active_province = self.active_province_id;
        let mut edit = StateEditSession::new(&reloaded, &self.bundle.map);
        let valid_selection = selected
            .into_iter()
            .filter(|province_id| edit.editable_province_state(*province_id).is_ok())
            .collect::<BTreeSet<_>>();
        let _ = edit.apply_lasso_selection(&valid_selection, LassoSelectionMode::Replace);
        if let Some(target) = target.filter(|state_id| edit.is_state_active(*state_id)) {
            let _ = edit.set_target_state(Some(target));
        }
        self.active_state_id = active_state.filter(|state_id| edit.is_state_active(*state_id));
        self.active_province_id = active_province
            .filter(|province_id| edit.editable_province_state(*province_id).is_ok());
        self.project = Some(reloaded);
        self.state_edit_session = Some(edit);
        self.patch_preview = None;
        self.patch_preview_file = 0;
        self.round_trip_report = None;
        self.round_trip_failure_snapshot = None;
        self.round_trip_status = None;
        self.review_required_apply_approved = false;
        self.state_apply_dialog = None;
        self.state_lifecycle_draft = None;
        self.state_property_draft = None;
        self.province_data_draft = None;
        self.state_lasso_phase = StateLassoPhase::Inactive;
        self.state_brush_phase = StateBrushPhase::Inactive;
        self.state_selection = None;
        self.selection_texture = None;
        self.selected_state_boundaries.clear();
        self.reload_political_country_catalog();
        self.refresh_state_visuals_full(0);
    }

    pub fn activate_state_lasso(
        &mut self,
        mode_override: Option<LassoSelectionMode>,
        alerts: &mut Alerts,
    ) {
        if self.property_draft_is_modified() {
            alerts.push(Err(
                "Apply or discard the modified draft before starting a lasso",
            ));
            return;
        }
        self.discard_unmodified_property_draft();
        if self.state_edit_session.is_none() {
            alerts.push(Err("State Lasso is available only in a States workspace"));
            return;
        }
        self.deactivate_state_brush();
        self.cancel_state_fill();
        self.state_pan_tool = false;
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

    pub fn set_state_lasso_inclusion(
        &mut self,
        inclusion: ProvinceInclusionMode,
        alerts: &mut Alerts,
    ) {
        self.state_lasso_inclusion = inclusion;
        let preview_points = match &mut self.state_lasso_phase {
            StateLassoPhase::Drawing {
                inclusion: active, ..
            } => {
                *active = inclusion;
                None
            }
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
        alerts: &mut Alerts,
    ) {
        if !matches!(self.state_lasso_phase, StateLassoPhase::Drawing { .. }) {
            self.activate_state_lasso(mode_override, alerts);
        }
        let can_finish = match &self.state_lasso_phase {
            StateLassoPhase::Drawing { points, .. } => points
                .first()
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
            }
            StateLassoPhase::Preview { .. } => {
                self.confirm_state_lasso(alerts);
                true
            }
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
        let StateLassoPhase::Preview {
            candidates, mode, ..
        } = &self.state_lasso_phase
        else {
            alerts.push(Err("No state lasso preview to confirm"));
            return;
        };
        let province_ids = candidates.selectable.clone();
        let mode = *mode;
        let result = self
            .state_edit_session
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
            }
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
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let Some(edit) = self.state_edit_session.as_ref() else {
            return;
        };
        let ambiguous = project
            .ambiguous_provinces
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
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
            }
            Err(error) => alerts.push(Err(error.to_string())),
        }
    }

    fn clear_state_lasso_preview_visuals(&mut self) {
        self.lasso_preview_boundaries.clear();
        self.lasso_blocked_boundaries.clear();
    }

    pub fn activate_state_brush(&mut self, mode: StateBrushMode, alerts: &mut Alerts) {
        if self.property_draft_is_modified() {
            alerts.push(Err(
                "Apply or discard the modified draft before starting the State Brush",
            ));
            return;
        }
        self.discard_unmodified_property_draft();
        let Some(edit) = self.state_edit_session.as_ref() else {
            alerts.push(Err(
                "State Brush is available only for loaded state projects",
            ));
            return;
        };
        if edit
            .validate_brush_target(mode, edit.target_state_id())
            .is_err()
        {
            alerts.push(Err(
                "Select a valid target state before using the State Brush",
            ));
            return;
        }
        self.cancel_state_lasso();
        self.cancel_state_fill();
        self.state_pan_tool = false;
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
            alerts.push(Err(
                "Resolve the active draft or lasso before using the State Brush",
            ));
            return false;
        }
        let Some(edit) = self.state_edit_session.as_ref() else {
            return false;
        };
        let target_state_id =
            match edit.validate_brush_target(self.state_brush_mode, edit.target_state_id()) {
                Ok(target_state_id) => target_state_id,
                Err(_) => {
                    alerts.push(Err(
                        "Select a valid target state before using the State Brush",
                    ));
                    return false;
                }
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
        let StateBrushPhase::Stroking(mut stroke) = std::mem::take(&mut self.state_brush_phase)
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
        let StateBrushPhase::Stroking(stroke) = std::mem::take(&mut self.state_brush_phase) else {
            return;
        };
        self.state_brush_phase = StateBrushPhase::Ready;
        self.clear_state_brush_preview();

        let changed = stroke.selectable_provinces.len();
        let blocked = stroke.blocked_ambiguous.len() + stroke.blocked_invalid_state.len();
        let ignored = stroke.ignored_non_land.len() + usize::from(stroke.encountered_unknown);
        let collection_ms = stroke.started.elapsed().as_millis();
        let province_ids = stroke
            .selectable_provinces
            .iter()
            .copied()
            .collect::<Vec<_>>();
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

        let result = self
            .state_edit_session
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
                let timings = self
                    .state_edit_session
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
            }
            Err(error) => {
                let message = format!("Cannot apply State Brush: {error}");
                self.last_state_brush_result = Some(message.clone());
                self.refresh_state_information();
                alerts.push(Err(message));
            }
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

    pub fn activate_state_fill(&mut self, mode: StateFillMode, alerts: &mut Alerts) {
        if self.property_draft_is_modified() {
            alerts.push(Err(
                "Apply or discard the modified draft before starting State Fill",
            ));
            return;
        }
        self.discard_unmodified_property_draft();
        let Some(edit) = self.state_edit_session.as_ref() else {
            alerts.push(Err(
                "State Fill is available only for loaded state projects",
            ));
            return;
        };
        if edit.target_state_id().is_none() {
            alerts.push(Err("Select a target state before using State Fill"));
            return;
        }
        self.cancel_state_lasso();
        self.deactivate_state_brush();
        self.map_tag_picker.cancel();
        self.state_pan_tool = false;
        self.state_fill_mode = mode;
        self.state_fill_phase = StateFillPhase::Ready;
        self.clear_state_fill_preview();
        alerts.push(Ok(format!("State Fill ready: {mode:?}")));
    }

    pub fn state_fill_is_active(&self) -> bool {
        !matches!(self.state_fill_phase, StateFillPhase::Inactive)
    }

    pub fn activate_state_select(&mut self) {
        self.cancel_state_lasso();
        self.deactivate_state_brush();
        self.cancel_state_fill();
        self.state_pan_tool = false;
        self.camera.set_panning(false);
    }

    pub fn activate_state_pan(&mut self) {
        self.activate_state_select();
        self.state_pan_tool = true;
    }

    pub fn state_pan_is_active(&self) -> bool {
        self.state_pan_tool
    }

    pub fn state_toolbar_tool(&self) -> usize {
        if self.state_pan_tool {
            1
        } else if self.state_lasso_is_active() {
            2
        } else if self.state_brush_is_active() {
            3
        } else if self.state_fill_is_active() {
            4
        } else {
            0
        }
    }

    pub fn preview_state_fill(
        &mut self,
        interface: &Interface,
        cursor_pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) -> bool {
        if !matches!(self.state_fill_phase, StateFillPhase::Ready) {
            return false;
        }
        let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) else {
            return true;
        };
        let Some(start_province_id) = self.bundle.map.get_province_at(pos).preserved_id else {
            alerts.push(Err("State Fill requires a known province ID"));
            return true;
        };
        let Some(edit) = self.state_edit_session.as_ref() else {
            return true;
        };
        let Some(target_state_id) = edit.target_state_id() else {
            alerts.push(Err("State Fill has no valid target state"));
            return true;
        };
        let ambiguous = self
            .project
            .as_ref()
            .map(|project| &project.ambiguous_provinces);
        let provinces = self
            .bundle
            .map
            .iter_province_data()
            .filter_map(|(_, province)| {
                let province_id = province.preserved_id?;
                Some(StateFillProvince {
                    province_id,
                    kind: match province.kind {
                        ProvinceKind::Land => StateFillProvinceKind::Land,
                        ProvinceKind::Sea => StateFillProvinceKind::Sea,
                        ProvinceKind::Lake => StateFillProvinceKind::Lake,
                        ProvinceKind::Unknown => StateFillProvinceKind::Unknown,
                    },
                    state_id: edit.province_state_id(province_id),
                    ambiguous: ambiguous.is_some_and(|map| map.contains_key(&province_id)),
                })
            })
            .collect::<Vec<_>>();
        let preview = plan_state_fill(
            provinces,
            &self.province_adjacency,
            edit.valid_state_ids(),
            self.state_fill_mode,
            start_province_id,
            Some(target_state_id),
        );
        let applicable = preview.applicable.iter().copied().collect::<BTreeSet<_>>();
        let blocked = preview
            .blocked
            .iter()
            .map(|blocked| blocked.province_id)
            .collect::<BTreeSet<_>>();
        self.fill_preview_boundaries = self.boundaries_for_provinces(&applicable);
        self.fill_blocked_boundaries = self.boundaries_for_provinces(&blocked);
        alerts.push(Ok(format!(
            "State Fill preview: found {}, applicable {}, blocked {}. Enter applies; Esc cancels.",
            preview.found.len(),
            preview.applicable.len(),
            preview.blocked.len()
        )));
        self.state_fill_phase = StateFillPhase::Preview {
            preview,
            target_state_id,
        };
        self.refresh_state_information();
        true
    }

    pub fn confirm_state_fill(&mut self, alerts: &mut Alerts) -> bool {
        let StateFillPhase::Preview {
            preview,
            target_state_id,
            ..
        } = std::mem::replace(&mut self.state_fill_phase, StateFillPhase::Ready)
        else {
            return false;
        };
        self.clear_state_fill_preview();
        if preview.applicable.is_empty() {
            alerts.push(Err("State Fill preview has no applicable provinces"));
            return true;
        }
        let count = preview.applicable.len();
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "State editing session is unavailable".to_owned())
            .and_then(|edit| {
                edit.reassign_provinces(&preview.applicable, Some(target_state_id))
                    .map_err(|error| error.to_string())
            });
        self.after_state_edit_command(
            result,
            &format!("State Fill moved {count} provinces in one transaction"),
            alerts,
        );
        true
    }

    pub fn cancel_state_fill(&mut self) -> bool {
        match std::mem::take(&mut self.state_fill_phase) {
            StateFillPhase::Inactive => return false,
            StateFillPhase::Ready | StateFillPhase::Preview { .. } => {}
        }
        self.clear_state_fill_preview();
        self.refresh_state_information();
        true
    }

    fn clear_state_fill_preview(&mut self) {
        self.fill_preview_boundaries.clear();
        self.fill_blocked_boundaries.clear();
    }

    fn collect_state_brush_segment(
        &self,
        stroke: &mut StateBrushStroke,
        current_map_position: Vector2<f64>,
    ) -> bool {
        let Some(edit) = self.state_edit_session.as_ref() else {
            return false;
        };
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
                }
                BrushProvinceClassification::NoOp => {
                    stroke.no_op_provinces.insert(province_id);
                    stroke.last_editable_province = Some(province_id);
                }
                BrushProvinceClassification::IgnoredNonLand => {
                    stroke.ignored_non_land.insert(province_id);
                }
                BrushProvinceClassification::BlockedAmbiguous => {
                    stroke.blocked_ambiguous.insert(province_id);
                }
                BrushProvinceClassification::BlockedInvalidState => {
                    stroke.blocked_invalid_state.insert(province_id);
                }
                BrushProvinceClassification::Unknown => stroke.encountered_unknown = true,
            }
        }
        changed
    }

    fn refresh_state_brush_preview(&mut self) {
        let StateBrushPhase::Stroking(stroke) = &self.state_brush_phase else {
            return;
        };
        let selectable = stroke.selectable_provinces.clone();
        let blocked = stroke
            .blocked_ambiguous
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
        alerts: &mut Alerts,
    ) -> Option<InspectorExternalRequest> {
        if self.state_click_would_change_property_draft(interface, cursor_pos) {
            alerts.push(Err(if self.province_data_draft.is_some() {
                "This province has unapplied form changes"
            } else {
                "This state has unapplied form changes"
            }));
            return None;
        }
        let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) else {
            self.clear_state_selection();
            return None;
        };
        let Some(project) = self.project.as_ref() else {
            return None;
        };
        if toggle_province {
            self.toggle_edit_province_at(pos, alerts);
            return None;
        }
        self.active_province_id = self.bundle.map.get_province_at(pos).preserved_id;
        let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps()
        else {
            return None;
        };
        let selection = resolve_state_at_for(
            &self.bundle.map,
            state_by_province,
            &project.ambiguous_provinces,
            unassigned_land_provinces,
            pos,
        );
        let message = selection_message(
            project,
            self.state_edit_session.as_ref(),
            selection.as_ref(),
        );
        let selection_image = match selection.as_ref() {
            Some(StateSelection::State { .. }) => Some(selection_overlay_for(
                &self.bundle.map,
                state_by_province,
                &project.ambiguous_provinces.keys().copied().collect(),
                selection.as_ref(),
            )),
            _ => None,
        };
        let selected_state_boundaries = match selection.as_ref() {
            Some(StateSelection::State { state_id, .. }) => boundaries_for_state(
                &self.bundle.map,
                state_by_province,
                &self.state_boundaries,
                *state_id,
            ),
            _ => Vec::new(),
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
        let state_id = self.active_state_id?;
        let source = match self
            .state_edit_session
            .as_ref()
            .and_then(|edit| edit.state_origin(state_id))
        {
            Some(WorkingStateOrigin::Loaded { document_path }) => StateOpenSource::Loaded {
                path: document_path.display().to_string(),
            },
            Some(WorkingStateOrigin::CreatedInSession) => StateOpenSource::CreatedInSession,
            None => StateOpenSource::NoSource,
        };
        let clicked = ClickedState { state_id, source };
        let at_ms = self
            .session_started
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        match self.state_double_click.click(clicked, at_ms, cursor_pos) {
            DoubleClickOutcome::OpenLoadedSource { path, .. } => {
                Some(InspectorExternalRequest::OpenSource(PathBuf::from(path)))
            }
            DoubleClickOutcome::Blocked { reason, .. } => {
                alerts.push(Err(match reason {
                    super::inspector::StateOpenBlock::CreatedInSession => {
                        "Created states do not have a source file yet"
                    }
                    super::inspector::StateOpenBlock::NoSource => "This state has no source file",
                }));
                None
            }
            DoubleClickOutcome::Armed => None,
        }
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
            alerts.push(Err(
                "Confirm or cancel the state lasso preview before moving provinces",
            ));
            return;
        }
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
            .and_then(|edit| {
                edit.move_selection_to_target()
                    .map_err(|err| err.to_string())
            });
        self.after_state_edit_command(result, "Moved selected provinces in memory", alerts);
    }

    pub fn move_confirmation_message(&self) -> Option<String> {
        let edit = self.state_edit_session.as_ref()?;
        let count = edit.selected_provinces().len();
        let target_state_id = edit.target_state_id()?;
        (count > 1).then(|| {
            let target_name = edit
                .state_data(target_state_id)
                .and_then(|data| data.name)
                .unwrap_or_else(|| "<unnamed>".to_owned());
            format!(
                "Move {count} selected provinces to State {target_state_id} — {target_name}?\n\n{}",
                selection_sources_message(edit)
            )
        })
    }

    pub fn select_target_state_provinces(&mut self, alerts: &mut Alerts) {
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
            .and_then(|edit| {
                edit.select_target_state_provinces()
                    .map_err(|err| err.to_string())
            });
        match result {
            Ok(count) => {
                self.refresh_selected_province_boundaries();
                self.refresh_state_information();
                alerts.push(Ok(format!(
                    "Selected {count} provinces from the target state"
                )));
            }
            Err(err) => alerts.push(Err(err)),
        }
    }

    pub fn unassign_selected_provinces(&mut self, alerts: &mut Alerts) {
        if self.property_draft_is_modified() {
            return;
        }
        self.discard_unmodified_property_draft();
        if self.state_lasso_is_active() {
            alerts.push(Err(
                "Confirm or cancel the state lasso preview before unassigning provinces",
            ));
            return;
        }
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
            .and_then(|edit| edit.unassign_selection().map_err(|err| err.to_string()));
        self.after_state_edit_command(result, "Unassigned selected provinces in memory", alerts);
    }

    pub fn unassign_confirmation_message(&self) -> Option<String> {
        let edit = self.state_edit_session.as_ref()?;
        let count = edit.selected_provinces().len();
        (count > 1).then(|| {
            format!(
                "Unassign {count} selected provinces?\n\n{}\n\n\
       This will temporarily create {count} unassigned land provinces.\n\
       No files will be written.",
                selection_sources_message(edit)
            )
        })
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
        let result = self
            .state_edit_session
            .as_mut()
            .ok_or_else(|| "State editing is available only for loaded state projects".to_owned())
            .and_then(|edit| {
                edit.toggle_selected_province(province_id)
                    .map_err(|err| err.to_string())
            });
        match result {
            Ok(selected) => {
                self.refresh_selected_province_boundaries();
                self.refresh_state_information();
                let action = if selected { "Selected" } else { "Deselected" };
                alerts.push(Ok(format!(
                    "{action} province {province_id} for state editing"
                )));
            }
            Err(err) => alerts.push(Err(err)),
        }
    }

    fn after_state_edit_command(
        &mut self,
        result: Result<(), String>,
        success: &str,
        alerts: &mut Alerts,
    ) {
        match result {
            Ok(()) => {
                self.refresh_state_visuals();
                alerts.push(Ok(success));
            }
            Err(err) => alerts.push(Err(err)),
        }
    }

    fn refresh_state_visuals(&mut self) {
        let changed = self
            .state_edit_session
            .as_mut()
            .map(StateEditSession::take_last_changed_provinces)
            .unwrap_or_default();
        if changed.is_empty() {
            self.refresh_political_texture();
            self.refresh_resource_labels();
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
        let Some(edit) = self.state_edit_session.as_ref() else {
            return;
        };
        let brush_target_is_invalid = self.state_brush_mode == StateBrushMode::AssignToTarget
            && self.state_brush_is_active()
            && !edit
                .target_state_id()
                .is_some_and(|state_id| edit.is_state_active(state_id));
        if self
            .active_state_id
            .is_some_and(|state_id| !edit.is_state_active(state_id))
        {
            self.active_state_id = edit.target_state_id();
            if self.active_state_id.is_none() {
                self.active_province_id = None;
            }
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
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps()
        else {
            return;
        };
        let state_view = generate_state_view_for(
            &self.bundle.map,
            state_by_province,
            &project.ambiguous_provinces.keys().copied().collect(),
            unassigned_land_provinces,
        );
        let texture_time = state_view.generated_in;
        let boundary_time = state_view.boundary_scan_in;
        self.last_state_visual_update_ms = texture_time.as_millis() + boundary_time.as_millis();
        self.last_state_visual_update_kind = "full";
        let settings = TextureSettings::new().mag(Filter::Nearest);
        self.state_texture = Some(Texture::from_image(&state_view.image, &settings));
        self.state_boundaries = state_view.state_boundaries;
        self.refresh_political_texture();
        self.refresh_resource_labels();
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

    fn refresh_state_visuals_selective(&mut self, changed: &BTreeSet<u32>, extents: Extents) {
        use opengl_graphics::{Format, UpdateTexture};

        let Some(project) = self.project.as_ref() else {
            return;
        };
        let Some((state_by_province, unassigned_land_provinces)) = self.effective_state_maps()
        else {
            return;
        };
        let ambiguous = project
            .ambiguous_provinces
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let region = generate_state_view_region_for(
            &self.bundle.map,
            state_by_province,
            &ambiguous,
            unassigned_land_provinces,
            extents,
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
        self.refresh_political_texture();
        self.refresh_resource_labels();

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

    fn refresh_political_texture(&mut self) {
        if self.map_layers.base_view != MapViewMode::Political {
            return;
        }
        self.ensure_political_country_catalog();
        if self.project.is_none() {
            self.political_texture = None;
            self.political_labels.clear();
            return;
        }
        let (owners, state_ownership, state_by_province, unassigned) = {
            let Some(edit) = self.state_edit_session.as_ref() else {
                self.political_texture = None;
                self.political_labels.clear();
                return;
            };
            let owners = edit
                .valid_state_ids()
                .iter()
                .filter_map(|state_id| {
                    let state = edit.state_data(*state_id)?;
                    Some((*state_id, state.history.owner.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            let state_ownership = edit
                .valid_state_ids()
                .iter()
                .filter_map(|state_id| {
                    let state = edit.state_data(*state_id)?;
                    Some(PoliticalStateOwnership {
                        owner: state.history.owner.clone(),
                        provinces: state.provinces.iter().copied().collect(),
                    })
                })
                .collect::<Vec<_>>();
            (
                owners,
                state_ownership,
                edit.state_by_province().clone(),
                edit.unassigned_land_provinces().clone(),
            )
        };
        let owner_tags = owners
            .values()
            .filter_map(|owner| owner.clone())
            .collect::<BTreeSet<_>>();
        let ambiguous = self
            .project
            .as_ref()
            .expect("project presence was checked")
            .ambiguous_provinces
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if let Some(catalog) = self.political_country_catalog.as_mut() {
            catalog.resolve_tags(owner_tags);
        }
        self.ensure_territory_anchor_index();
        let anchors = self
            .territory_anchor_index
            .as_ref()
            .expect("territory anchor index was initialized");
        let political_labels = self
            .political_country_catalog
            .as_ref()
            .map_or_else(Vec::new, |catalog| {
                prepare_country_labels_with_index(catalog, &state_ownership, anchors)
            });
        self.political_labels = political_labels;
        if let Some(catalog) = self.political_country_catalog.as_ref() {
            let settings = TextureSettings::new().mag(Filter::Nearest);
            for label in &self.political_labels {
                if self.political_flag_textures.contains_key(&label.tag) {
                    continue;
                }
                if let Some(flag) = catalog
                    .metadata(&label.tag)
                    .and_then(|country| country.flag.as_ref())
                {
                    self.political_flag_textures
                        .insert(label.tag.clone(), Texture::from_image(flag, &settings));
                }
            }
        }
        let image = self.bundle.map.gen_texture_buffer(|province_color| {
            let province = self.bundle.map.get_province(province_color);
            if province.kind == ProvinceKind::Unknown {
                return super::project::UNKNOWN_PROVINCE_COLOR;
            }
            let Some(province_id) = province.preserved_id else {
                return province.kind.color();
            };
            if ambiguous.contains(&province_id) {
                return super::project::AMBIGUOUS_PROVINCE_COLOR;
            }
            if let Some(state_id) = state_by_province.get(&province_id) {
                let owner = owners.get(state_id).and_then(Option::as_deref);
                return self.political_country_catalog.as_ref().map_or_else(
                    || owner.map_or([0x68, 0x68, 0x68], political_fallback_color),
                    |catalog| catalog.owner_resolution(owner).color_for_map(),
                );
            }
            if province.kind == ProvinceKind::Land && unassigned.contains(&province_id) {
                return super::project::UNASSIGNED_LAND_COLOR;
            }
            province.kind.color()
        });
        let settings = TextureSettings::new().mag(Filter::Nearest);
        self.political_texture = Some(Texture::from_image(&image, &settings));
    }

    fn reload_political_country_catalog(&mut self) {
        self.invalidate_political_cache();
        self.political_language = crate::localization::language().to_owned();
        self.refresh_political_texture();
    }

    fn invalidate_political_cache(&mut self) {
        self.political_country_catalog = None;
        self.political_flag_textures.clear();
        self.political_labels.clear();
        self.political_texture = None;
        self.political_cache_generation = None;
    }

    fn ensure_political_country_catalog(&mut self) {
        if self.political_cache_generation != Some(self.project_generation) {
            self.invalidate_political_cache();
        }
        if self.political_country_catalog.is_some() {
            return;
        }
        self.political_country_catalog = self.project.as_ref().map(|project| {
            PoliticalCountryCatalog::load(
                &project.paths.root,
                self.definition_base_game_root.as_deref(),
            )
        });
        if self.political_country_catalog.is_some() {
            self.political_cache_generation = Some(self.project_generation);
        }
    }

    fn draw_political_country_labels(
        &self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let zoom = self.camera.scale_factor();
        for label in &self.political_labels {
            let visibility =
                political_label_visibility(label.territory_pixels, zoom, label.flag_available);
            if visibility == PoliticalLabelVisibility::Hidden {
                continue;
            }
            let anchor = self.camera.compute_position(interface, label.anchor);
            if !self.camera.within_viewport(interface, anchor) {
                continue;
            }
            let font_size = (FONT_SIZE as f64 * zoom.clamp(0.72, 1.35)).round() as u32;
            let label_width = label.display_name.chars().count() as f64 * font_size as f64 * 0.55;
            let name_pos = [anchor[0] - label_width / 2.0, anchor[1] - font_size as f64];
            let show_name = matches!(
                visibility,
                PoliticalLabelVisibility::NameOnly | PoliticalLabelVisibility::NameAndFlag
            );
            let show_flag = matches!(
                visibility,
                PoliticalLabelVisibility::FlagOnly | PoliticalLabelVisibility::NameAndFlag
            );
            if show_name {
                let shadow = ctx.transform.trans(name_pos[0] + 1.0, name_pos[1] + 1.0);
                graphics::text(
                    colors::BLACK,
                    font_size,
                    &label.display_name,
                    glyph_cache,
                    shadow,
                    gl,
                )
                .expect("unable to draw political label shadow");
                let text = ctx.transform.trans_pos(name_pos);
                graphics::text(
                    colors::WHITE,
                    font_size,
                    &label.display_name,
                    glyph_cache,
                    text,
                    gl,
                )
                .expect("unable to draw political label");
            }
            if show_flag && let Some(flag) = self.political_flag_textures.get(&label.tag) {
                let scale = zoom.clamp(0.75, 1.5);
                let width = 26.0 * scale;
                let height = 17.0 * scale;
                let top = anchor[1] + if show_name { 3.0 } else { -height / 2.0 };
                graphics::Image::new()
                    .rect([anchor[0] - width / 2.0, top, width, height])
                    .draw(flag, &graphics::DrawState::default(), ctx.transform, gl);
            }
        }
    }

    fn refresh_resource_labels(&mut self) {
        self.ensure_resource_cache_generation();
        if !self.map_layers.show_resources && self.map_layers.base_view != MapViewMode::Resources {
            return;
        }
        let Some(project) = self.project.as_ref() else {
            self.resource_labels.clear();
            return;
        };
        if !project.state_load_is_complete() {
            self.resource_labels.clear();
            return;
        }
        let Some(edit) = self.state_edit_session.as_ref() else {
            self.resource_labels.clear();
            return;
        };
        let states = edit
            .valid_state_ids()
            .iter()
            .filter_map(|state_id| {
                let state = edit.state_data(*state_id)?;
                Some(ResourceMapState {
                    state_id: *state_id,
                    provinces: state.provinces.iter().copied().collect(),
                    resources: state.resources.clone(),
                })
            })
            .collect::<Vec<_>>();
        self.ensure_territory_anchor_index();
        let anchors = self
            .territory_anchor_index
            .as_ref()
            .expect("territory anchor index was initialized");
        let resource_labels = prepare_resource_labels_with_index(&states, anchors);
        self.resource_labels = resource_labels;
    }

    fn ensure_territory_anchor_index(&mut self) -> &TerritoryAnchorIndex {
        if self.territory_anchor_generation != Some(self.project_generation) {
            self.territory_anchor_index = None;
        }
        if self.territory_anchor_index.is_none() {
            let provinces = self
                .bundle
                .map
                .iter_province_data()
                .filter_map(|(_, province)| {
                    Some(PoliticalProvince {
                        id: province.preserved_id?,
                        is_land: province.kind == ProvinceKind::Land,
                        center: province.center_of_mass(),
                        pixel_count: province.pixel_count,
                    })
                })
                .collect::<Vec<_>>();
            self.territory_anchor_index = Some(TerritoryAnchorIndex::new(
                &provinces,
                &self.political_adjacency_pairs,
            ));
            self.territory_anchor_generation = Some(self.project_generation);
        }
        self.territory_anchor_index
            .as_ref()
            .expect("territory anchor index was initialized")
    }

    fn reload_resource_icons(&mut self) {
        self.resource_icon_resolver = None;
        self.resource_icon_textures.clear();
        self.resource_labels.clear();
        self.resource_cache_generation = None;
    }

    fn ensure_resource_cache_generation(&mut self) {
        if self.resource_cache_generation != Some(self.project_generation) {
            self.resource_icon_resolver = None;
            self.resource_icon_textures.clear();
            self.resource_labels.clear();
            self.resource_cache_generation = Some(self.project_generation);
        }
    }

    fn ensure_resource_icon_resolver(&mut self) {
        self.ensure_resource_cache_generation();
        if self.resource_icon_resolver.is_some() {
            return;
        }
        self.resource_icon_resolver = self.project.as_ref().map(|project| {
            ResourceIconResolver::load(
                &project.paths.root,
                self.definition_base_game_root.as_deref(),
            )
        });
    }

    fn resource_icon_texture(&mut self, key: &str) -> Option<&Texture> {
        self.ensure_resource_icon_resolver();
        if !self.resource_icon_textures.contains_key(key) {
            let texture = self
                .resource_icon_resolver
                .as_mut()
                .and_then(|resolver| resolver.icon(key))
                .map(|image| {
                    Texture::from_image(image, &TextureSettings::new().mag(Filter::Nearest))
                });
            self.resource_icon_textures.insert(key.to_owned(), texture);
        }
        self.resource_icon_textures
            .get(key)
            .and_then(Option::as_ref)
    }

    fn draw_resource_labels(
        &mut self,
        ctx: Context,
        interface: &Interface,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let zoom = self.camera.scale_factor();
        let labels = self.resource_labels.clone();
        for label in labels {
            if !resource_label_visible(label.territory_pixels, zoom) {
                continue;
            }
            let anchor = self.camera.compute_position(interface, label.anchor);
            if !self.camera.within_viewport(interface, anchor) {
                continue;
            }
            let scale = zoom.clamp(0.75, 1.25);
            let icon_size = 16.0 * scale;
            let row_height = icon_size + 3.0;
            let top = anchor[1] - label.rows.len() as f64 * row_height / 2.0;
            for (index, row) in label.rows.iter().enumerate() {
                let y = top + index as f64 * row_height;
                let text = format!("{} {}", row.key, row.amount);
                if let Some(icon) = self.resource_icon_texture(&row.key) {
                    graphics::Image::new()
                        .rect([anchor[0] - icon_size / 2.0, y, icon_size, icon_size])
                        .draw(icon, &graphics::DrawState::default(), ctx.transform, gl);
                    let caption = format!("{}", row.amount);
                    draw_resource_caption(
                        ctx,
                        glyph_cache,
                        gl,
                        &caption,
                        [anchor[0] + icon_size / 2.0 + 2.0, y + icon_size - 2.0],
                        scale,
                    );
                } else {
                    draw_resource_caption(
                        ctx,
                        glyph_cache,
                        gl,
                        &text,
                        [anchor[0] - 18.0 * scale, y + icon_size - 2.0],
                        scale,
                    );
                }
            }
        }
    }

    pub fn toggle_image_overlay(&mut self, alerts: &mut Alerts) {
        if self.image_overlay_texture.is_none() {
            alerts.push(Err(self.image_overlay_status.clone()));
            return;
        }
        self.map_layers.image_overlay.enabled = !self.map_layers.image_overlay.enabled;
        self.persist_image_overlay_settings(alerts);
        alerts.push(Ok(format!(
            "Image Overlay: {} at {}%",
            if self.map_layers.image_overlay.enabled {
                "on"
            } else {
                "off"
            },
            (self.map_layers.image_overlay.opacity * 100.0).round() as u32
        )));
    }

    pub fn adjust_image_overlay_opacity(&mut self, delta: f32, alerts: &mut Alerts) {
        self.map_layers.image_overlay.opacity =
            (self.map_layers.image_overlay.opacity + delta).clamp(0.0, 1.0);
        self.persist_image_overlay_settings(alerts);
        alerts.push(Ok(format!(
            "Image Overlay opacity: {}%",
            (self.map_layers.image_overlay.opacity * 100.0).round() as u32
        )));
    }

    pub fn use_project_heightmap(&mut self, alerts: &mut Alerts) {
        let settings = TextureSettings::new().mag(Filter::Nearest);
        let (texture, status, dimensions) =
            load_project_image_overlay(&self.location, self.bundle.map.dimensions(), &settings);
        let Some(texture) = texture else {
            alerts.push(Err(status));
            return;
        };
        self.image_overlay_texture = Some(texture);
        self.image_overlay_status = status.clone();
        self.map_layers.image_overlay.source = Some(ImageOverlaySource::ProjectHeightmap);
        self.map_layers.image_overlay.dimensions = dimensions;
        self.map_layers.image_overlay.content_revision = self
            .map_layers
            .image_overlay
            .content_revision
            .wrapping_add(1);
        self.map_layers.image_overlay.enabled = true;
        self.persist_image_overlay_settings(alerts);
        alerts.push(Ok(status));
    }

    pub fn load_custom_image_overlay(&mut self, path: PathBuf, alerts: &mut Alerts) {
        let bytes = match fs_err::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                alerts.push(Err(format!(
                    "Image Overlay unavailable: failed to read {}: {error}",
                    path.display()
                )));
                return;
            }
        };
        let settings = TextureSettings::new().mag(Filter::Nearest);
        match decode_image_overlay(
            &bytes,
            self.bundle.map.dimensions(),
            &settings,
            &path.display().to_string(),
        ) {
            Ok((texture, dimensions, status)) => {
                self.image_overlay_texture = Some(texture);
                self.image_overlay_status = status.clone();
                self.map_layers.image_overlay.source = Some(ImageOverlaySource::Custom(path));
                self.map_layers.image_overlay.dimensions = Some(dimensions);
                self.map_layers.image_overlay.content_revision = self
                    .map_layers
                    .image_overlay
                    .content_revision
                    .wrapping_add(1);
                self.map_layers.image_overlay.enabled = true;
                self.persist_image_overlay_settings(alerts);
                alerts.push(Ok(status));
            }
            Err(error) => alerts.push(Err(error)),
        }
    }

    pub fn clear_image_overlay(&mut self, alerts: &mut Alerts) {
        self.image_overlay_texture = None;
        self.image_overlay_status = "Image Overlay unavailable: no image selected.".to_owned();
        self.map_layers.image_overlay.enabled = false;
        self.map_layers.image_overlay.source = None;
        self.map_layers.image_overlay.dimensions = None;
        self.map_layers.image_overlay.content_revision = self
            .map_layers
            .image_overlay
            .content_revision
            .wrapping_add(1);
        self.persist_image_overlay_settings(alerts);
        alerts.push(Ok("Image Overlay cleared"));
    }

    fn persist_image_overlay_settings(&self, alerts: &mut Alerts) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let loaded = match ProjectConfig::load(&project.paths.root) {
            Ok(loaded) if loaded.issue.is_none() => loaded,
            Ok(loaded) => {
                alerts.push(Err(format!(
                    "Image Overlay settings were not saved: {}",
                    loaded.issue.expect("checked above")
                )));
                return;
            }
            Err(error) => {
                alerts.push(Err(format!(
                    "Image Overlay settings were not saved: {error}"
                )));
                return;
            }
        };
        let mut settings = loaded.value;
        settings.image_overlay.visible = self.map_layers.image_overlay.enabled;
        settings.image_overlay.opacity_percent = (self.map_layers.image_overlay.opacity * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        let (use_project_heightmap, source_path) =
            match self.map_layers.image_overlay.source.as_ref() {
                Some(ImageOverlaySource::ProjectHeightmap) => (true, None),
                Some(ImageOverlaySource::Custom(path)) => (
                    false,
                    Some(
                        path.strip_prefix(&project.paths.root)
                            .map_or_else(|_| path.clone(), PathBuf::from),
                    ),
                ),
                None => (false, None),
            };
        settings.image_overlay.use_project_heightmap = use_project_heightmap;
        settings.image_overlay.source_path = source_path;
        if let Err(error) = settings.save(
            &project.paths.root,
            loaded.fingerprint.as_ref(),
            false,
            false,
        ) {
            alerts.push(Err(format!(
                "Image Overlay settings were not saved: {error}"
            )));
        }
    }

    pub fn toggle_state_boundaries(&mut self, alerts: &mut Alerts) {
        self.map_layers.show_state_boundaries = !self.map_layers.show_state_boundaries;
        alerts.push(Ok(format!(
            "State boundaries: {}",
            if self.map_layers.show_state_boundaries {
                "on"
            } else {
                "off"
            }
        )));
    }

    fn selective_state_update_extents(&mut self, changed: &BTreeSet<u32>) -> Option<Extents> {
        if self.state_province_extents.is_none() {
            let started = std::time::Instant::now();
            self.state_province_extents = Some(self.bundle.map.province_extents_by_id());
            println!(
                "Indexed province bounds for selective state updates in {} ms.",
                started.elapsed().as_millis()
            );
        }
        let extents = self.state_province_extents.as_ref()?;
        let mut changed_extents = changed
            .iter()
            .map(|province_id| extents.get(province_id).copied());
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
        let selected = self
            .state_edit_session
            .as_ref()
            .map(StateEditSession::selected_provinces)
            .cloned()
            .unwrap_or_default();
        self.selected_province_boundaries = self.boundaries_for_provinces(&selected);
    }

    fn boundaries_for_provinces(&self, province_ids: &BTreeSet<u32>) -> Vec<UOrd<Vector2<u32>>> {
        province_ids
            .iter()
            .filter_map(|province_id| self.province_boundaries.get(province_id))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|boundary| {
                let [a, b] = boundary.into_array();
                let a_selected = self
                    .bundle
                    .map
                    .get_province_at(a)
                    .preserved_id
                    .is_some_and(|id| province_ids.contains(&id));
                let b_selected = self
                    .bundle
                    .map
                    .get_province_at(b)
                    .preserved_id
                    .is_some_and(|id| province_ids.contains(&id));
                a_selected != b_selected
            })
            .collect()
    }

    fn refresh_state_target_overlay(&mut self) {
        let selection = self.state_selection.clone();
        let Some((image, selected_state_boundaries)) = self.project.as_ref().and_then(|project| {
            let (state_by_province, _) = self.effective_state_maps()?;
            let ambiguous = project
                .ambiguous_provinces
                .keys()
                .copied()
                .collect::<BTreeSet<_>>();
            let image = matches!(selection, Some(StateSelection::State { .. })).then(|| {
                selection_overlay_for(
                    &self.bundle.map,
                    state_by_province,
                    &ambiguous,
                    selection.as_ref(),
                )
            });
            let boundaries = match selection {
                Some(StateSelection::State { state_id, .. }) => boundaries_for_state(
                    &self.bundle.map,
                    state_by_province,
                    &self.state_boundaries,
                    state_id,
                ),
                _ => Vec::new(),
            };
            Some((image, boundaries))
        }) else {
            return;
        };
        self.selection_texture = image.map(|image| {
            let settings = TextureSettings::new().mag(Filter::Nearest);
            Texture::from_image(&image, &settings)
        });
        self.selected_state_boundaries = selected_state_boundaries;
    }

    fn effective_state_maps(
        &self,
    ) -> Option<(&std::collections::HashMap<u32, u32>, &BTreeSet<u32>)> {
        if let Some(edit) = self.state_edit_session.as_ref() {
            Some((edit.state_by_province(), edit.unassigned_land_provinces()))
        } else {
            self.project.as_ref().map(|project| {
                (
                    &project.state_by_province,
                    &project.unassigned_land_provinces,
                )
            })
        }
    }

    fn state_edit_status_text(&self) -> Option<String> {
        let edit = self.state_edit_session.as_ref()?;
        let tool = ["Select", "Pan", "Lasso", "Brush", "Fill"][self.state_toolbar_tool()];
        Some(state_edit_status_message(
            edit,
            self.active_province_id,
            self.active_state_id,
            tool,
        ))
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
            StateLassoPhase::Drawing {
                points,
                mode,
                inclusion,
            } => format!(
                "State lasso: Drawing ({} points) | Mode: {} | Inclusion: {}\n\
         Click first point or Enter to calculate preview; Esc cancels",
                points.len(),
                mode.label(),
                inclusion.label()
            ),
            StateLassoPhase::Preview {
                candidates,
                mode,
                inclusion,
                ..
            } => format!(
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
            StateBrushMode::AssignToTarget => edit
                .target_state_id()
                .and_then(|state_id| {
                    let name = edit
                        .state_data(state_id)?
                        .name
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
                self.last_state_visual_update_kind,
            )
        });
        let details = self.project.as_ref().and_then(|project| {
            let edit = self.state_edit_session.as_ref();
            let active_matches_loaded_selection =
                self.active_state_id.is_some_and(|active_state_id| {
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
        let fill = match &self.state_fill_phase {
            StateFillPhase::Inactive => None,
            StateFillPhase::Ready => Some(format!(
                "State Fill: Ready | Mode: {:?} | click a province to preview",
                self.state_fill_mode
            )),
            StateFillPhase::Preview { preview, .. } => Some(format!(
                "State Fill: Preview | found {} | applicable {} | blocked {} | Enter applies | Esc cancels",
                preview.found.len(),
                preview.applicable.len(),
                preview.blocked.len()
            )),
        };
        let patch = self.patch_preview_status_text();
        let validation = self.round_trip_status_text();
        let save = self.state_save_status.clone();
        let province_save = self.province_save_status.clone();
        let validation_diagnostics = self.project_validation_report.as_ref().map(|report| {
            let baseline = report.baseline_summary.as_ref();
            format!(
                "Baseline Diagnostics: {} error(s), {} warning(s)\nCandidate Diagnostics: {} error(s), {} warning(s)\nDiagnostic Delta: new {} | aggravated {} | unchanged {} | resolved {} | improved {}",
                baseline.map_or(0, |summary| summary.errors),
                baseline.map_or(0, |summary| summary.warnings),
                report.errors,
                report.warnings,
                report.delta.new.len(),
                report.delta.aggravated.len(),
                report.delta.unchanged.len(),
                report.delta.resolved.len(),
                report.delta.improved.len(),
            )
        });
        let last_validation = self.last_validation.as_ref().map(|last| {
            let gate = |value: Option<bool>| match value {
                Some(true) => "Pass",
                Some(false) => "Fail",
                None => "N/A",
            };
            format!(
                "Last Validation: {:?} | {} | {} ms\nSemantic: {} | Indexes: {} | Bytes: {} | Unexpected diagnostics: {}",
                last.target,
                last.result,
                last.duration_ms,
                gate(last.semantic),
                gate(last.indexes),
                gate(last.bytes),
                last.unexpected_diagnostics,
            )
        });
        let last_save = self.state_save_report.as_ref().map(|report| {
            format!(
                "Last Save: {} | Transaction: {} | Files: {}",
                report.state.label(),
                report.transaction_id.as_deref().unwrap_or("N/A"),
                report.modified_files + report.created_files + report.removed_files,
            )
        });
        let dirty = Some(self.workspace_dirty_status_text());
        self.selection_info = [
            province_details,
            details,
            dirty,
            status,
            lasso,
            brush,
            fill,
            patch,
            validation,
            save,
            province_save,
            validation_diagnostics,
            last_validation,
            last_save,
        ]
        .into_iter()
        .flatten()
        .join("\n")
        .into();
    }

    fn workspace_dirty_status_text(&self) -> String {
        let (province_modified, states) = self.workspace_dirty_summary();
        workspace_dirty_status(province_modified, states)
    }

    pub fn workspace_dirty_summary(&self) -> (bool, usize) {
        (
            self.has_unsaved_province_edits(),
            self.state_edit_session
                .as_ref()
                .map(|edit| edit.summary().modified_states)
                .unwrap_or_default(),
        )
    }

    fn patch_preview_status_text(&self) -> Option<String> {
        let plan = self.patch_preview.as_ref()?;
        let stale = self
            .state_edit_session
            .as_ref()
            .is_some_and(|edit| plan.is_stale(edit.revision()));
        let mut text = plan.summary_text();
        if stale {
            text.push_str(
                "\nSTALE: the working state changed; regenerate before relying on this preview.",
            );
        }
        if let Some(report) = plan.file_report(self.patch_preview_file) {
            let lines = report.lines().collect::<Vec<_>>();
            text.push_str(&format!(
                "\nFILE {} OF {}\n{}",
                self.patch_preview_file + 1,
                plan.files_len(),
                lines.iter().take(42).copied().join("\n"),
            ));
            if lines.len() > 42 {
                text.push_str("\n... full diff retained in memory and printed to the console.");
            }
        }
        Some(text)
    }

    fn round_trip_status_text(&self) -> Option<String> {
        let mut text = self.round_trip_status.clone()?;
        let stale = self.round_trip_report.as_ref().is_some_and(|report| {
            let revision = self
                .state_edit_session
                .as_ref()
                .map(StateEditSession::revision)
                .unwrap_or_default();
            report.is_stale(revision, self.patch_preview.as_ref())
        });
        if stale {
            text.push_str("\nSTALE: validation result is outdated.");
        }
        Some(text)
    }

    pub fn set_tool_mode(&mut self, mode: ToolMode) {
        self.deactivate_state_brush();
        self.cancel_state_fill();
        self.tool.mode = mode;
    }

    pub fn cycle_tool_brush(
        &mut self,
        interface: &Interface,
        cursor_pos: Option<Vector2<f64>>,
        backwards: bool,
        alerts: &mut Alerts,
    ) {
        match self.view_mode {
            ViewMode::Color => {
                let kind = self
                    .tool
                    .kind_brush
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
            }
            ViewMode::Kind => {
                let kind = self.tool.kind_brush;
                let kind = cycle_kinds(kind, backwards);
                self.tool.kind_brush = Some(kind);
                alerts.push(Ok(format!(
                    "Brush set to type {}",
                    kind.to_str().to_uppercase()
                )));
            }
            ViewMode::Terrain => {
                let terrain = self.tool.terrain_brush.as_deref();
                let terrain = self.bundle.config.cycle_terrains(terrain, backwards);
                alerts.push(Ok(format!(
                    "Brush set to terrain {}",
                    terrain.to_uppercase()
                )));
                self.tool.terrain_brush = Some(terrain);
            }
            ViewMode::Continent => {
                let continent = self.tool.continent_brush;
                let continent = cycle_continents(continent, backwards);
                self.tool.continent_brush = Some(continent);
                alerts.push(Ok(format!("Brush set to continent {}", continent)));
            }
            ViewMode::Coastal => (),
            ViewMode::Adjacencies => {
                let adjacency_kind = self.tool.adjacency_brush;
                let adjacency_kind = cycle_connection(adjacency_kind, backwards);
                self.tool.adjacency_brush = Some(adjacency_kind);
                alerts.push(Ok(format!(
                    "Brush set to adjacencies {}",
                    adjacency_kind.to_str().to_uppercase()
                )));
            }
        };
    }

    pub fn pick_tool_brush(
        &mut self,
        interface: &Interface,
        cursor_pos: Vector2<f64>,
        alerts: &mut Alerts,
    ) {
        if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
            let color = self.bundle.map.get_color_at(pos);
            let province_data = self.bundle.map.get_province_at(pos);
            match self.view_mode {
                ViewMode::Color => {
                    self.tool_paint_end();
                    self.tool.color_brush = Some(color);
                    alerts.push(Ok(format!("Picked color {}", stringify_color(color))));
                }
                ViewMode::Kind => {
                    if let Some(kind) = province_data.kind.to_definition_kind() {
                        self.tool.kind_brush = Some(kind);
                        alerts.push(Ok(format!("Picked type {}", kind.to_str().to_uppercase())));
                    }
                }
                ViewMode::Terrain => {
                    if province_data.terrain != "unknown" {
                        let terrain = province_data.terrain.as_str();
                        self.tool.terrain_brush = Some(terrain.to_owned());
                        alerts.push(Ok(format!("Picked terrain {}", terrain.to_uppercase())));
                    }
                }
                ViewMode::Continent => {
                    let continent = province_data.continent;
                    self.tool.continent_brush = Some(continent);
                    alerts.push(Ok(format!("Picked continent {}", continent)));
                }
                ViewMode::Coastal => (),
                ViewMode::Adjacencies => (),
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
    pub fn activate_tool(
        &mut self,
        interface: &Interface,
        cursor_pos: Vector2<f64>,
        modifier: bool,
        alerts: &mut Alerts,
    ) {
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        };

        let province_ids = self.bundle.map.province_ids().collect::<BTreeSet<_>>();
        match self.view_mode {
            ViewMode::Color => match self.tool.mode {
                ToolMode::PaintArea => self.tool_paint_brush(interface, cursor_pos),
                ToolMode::PaintBucket => self.tool_paint_bucket(interface, cursor_pos, modifier),
                ToolMode::Lasso(_) => self.tool_lasso_add_point(interface, cursor_pos),
            },
            ViewMode::Adjacencies => self.tool_connect_activate(interface, cursor_pos),
            _ => self.tool_paint_brush(interface, cursor_pos),
        };
        self.restore_removed_referenced_provinces(&province_ids, alerts);
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

    pub fn finish_tool(&mut self, alerts: &mut Alerts) {
        if self.map_access_mode == MapAccessMode::ReadOnly {
            return;
        };

        if let ToolMode::Lasso(lasso) = &mut self.tool.mode {
            let lasso = lasso.drain();
            let province_ids = self.bundle.map.province_ids().collect::<BTreeSet<_>>();
            self.tool_lasso_finish(lasso);
            self.restore_removed_referenced_provinces(&province_ids, alerts);
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
                if let Some(result) = self.history.paint_pixel_lasso(
                    &mut self.bundle,
                    lasso,
                    color,
                    self.tool.brush_mask,
                ) {
                    self.register_new_provinces_for_state_edit(&result.replaced_province_ids);
                    self.problems.clear();
                    self.sync_province_dirty();
                    self.refresh_selective(result.extents);
                };
            };
        };
    }

    fn tool_paint_brush(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
        if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
            if let (Some(color), ViewMode::Color) = (self.tool.color_brush, self.view_mode) {
                let pos = self.camera.relative_position(interface, cursor_pos);
                if let Some(result) = self.history.paint_pixel_area(
                    &mut self.bundle,
                    pos,
                    self.tool.radius,
                    color,
                    self.tool.brush_mask,
                    self.tool.id,
                ) {
                    self.register_new_provinces_for_state_edit(&result.replaced_province_ids);
                    self.problems.clear();
                    self.sync_province_dirty();
                    self.refresh_selective(result.extents);
                };
            } else if let (Some(kind), ViewMode::Kind) = (self.tool.kind_brush, self.view_mode) {
                if let Some(extents) = self
                    .history
                    .paint_province_kind(&mut self.bundle, pos, kind)
                {
                    self.sync_province_dirty();
                    self.refresh_selective(extents);
                };
            } else if let (Some(terrain), ViewMode::Terrain) =
                (&self.tool.terrain_brush, self.view_mode)
            {
                if let Some(extents) =
                    self.history
                        .paint_province_terrain(&mut self.bundle, pos, terrain.clone())
                {
                    self.sync_province_dirty();
                    self.refresh_selective(extents);
                };
            } else if let (Some(continent), ViewMode::Continent) =
                (self.tool.continent_brush, self.view_mode)
            {
                if let Some(extents) =
                    self.history
                        .paint_province_continent(&mut self.bundle, pos, continent)
                {
                    self.sync_province_dirty();
                    self.refresh_selective(extents);
                };
            };
        };
    }

    fn tool_paint_end(&mut self) {
        self.tool.id += 1;
    }

    fn tool_paint_bucket(
        &mut self,
        interface: &Interface,
        cursor_pos: Vector2<f64>,
        fill_all: bool,
    ) {
        if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
            if let (Some(fill_color), ViewMode::Color) = (self.tool.color_brush, self.view_mode) {
                let result = if fill_all {
                    self.history
                        .paint_entire_province(&mut self.bundle, pos, fill_color)
                } else {
                    self.history.paint_pixel_bucket(
                        &mut self.bundle,
                        pos,
                        fill_color,
                        self.tool.brush_mask,
                    )
                };

                if let Some(result) = result {
                    self.register_new_provinces_for_state_edit(&result.replaced_province_ids);
                    self.problems.clear();
                    self.sync_province_dirty();
                    self.refresh_selective(result.extents);
                };
            };
        };
    }

    fn restore_removed_referenced_provinces(
        &mut self,
        previous_ids: &BTreeSet<u32>,
        alerts: &mut Alerts,
    ) {
        let current_ids = self.bundle.map.province_ids().collect::<BTreeSet<_>>();
        let referenced = self
            .state_edit_session
            .as_ref()
            .map(|edit| {
                removed_provinces_with_state_references(
                    previous_ids,
                    &current_ids,
                    edit.state_by_province(),
                )
            })
            .unwrap_or_default();
        let Some((province_id, state_id)) = referenced.first().copied() else {
            return;
        };
        if self.history.undo(&mut self.bundle.map).is_some() {
            self.bundle.map.recalculate_all_boundaries();
            self.problems.clear();
            self.refresh();
            self.sync_province_dirty();
        }
        alerts.push(Err(format!(
            "Province {province_id} no longer has pixels and is still referenced by State {state_id}. The paint was restored; reassign or remove its references before deleting it."
        )));
    }

    fn synchronize_created_state_provinces(&mut self) {
        if let Some(edit) = self.state_edit_session.as_mut() {
            edit.synchronize_created_provinces(&self.bundle.map);
        }
    }

    fn register_new_provinces_for_state_edit(&mut self, replaced_province_ids: &BTreeSet<u32>) {
        let Some(edit) = self.state_edit_session.as_ref() else {
            return;
        };
        let source_states = replaced_province_ids
            .iter()
            .filter_map(|id| edit.province_state_id(*id))
            .collect::<BTreeSet<_>>();
        let inherited_state = (source_states.len() == 1)
            .then(|| *source_states.first().expect("one inherited State"));
        let additions = self
            .bundle
            .map
            .province_id_index()
            .iter()
            .filter(|(id, _)| !edit.knows_province(*id))
            .filter_map(|(id, color)| {
                let province = self.bundle.map.get_province(color);
                (province.kind == ProvinceKind::Land).then_some((id, province.kind))
            })
            .collect::<Vec<_>>();
        if let Some(edit) = self.state_edit_session.as_mut() {
            for (province_id, kind) in additions {
                edit.register_created_province(province_id, kind, inherited_state);
            }
        }
    }

    fn tool_connect_activate(&mut self, interface: &Interface, cursor_pos: Vector2<f64>) {
        if let Some(pos) = self.camera.relative_position_int(interface, cursor_pos) {
            let which = self.bundle.map.get_color_at(pos);
            if let Some(kind) = self.tool.adjacency_brush {
                if let Some(color) = self.tool.adjacency_selection.take() {
                    if self.history.add_or_remove_connection(
                        &mut self.bundle,
                        UOrd::new([which, color]),
                        kind,
                    ) {
                        self.sync_province_dirty();
                    }
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
            Some(format!(
                "Terrain mode unavailable, unknown terrains present: {}",
                unknown_terrains
            ))
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
            ViewMode::Adjacencies => self.bundle.texture_buffer_color(),
        };

        self.texture.update(&buffer);
    }

    fn refresh_selective(&mut self, extents: Extents) {
        use opengl_graphics::{Format, UpdateTexture};
        let (offset, size) = extents.to_offset_size();
        let buffer = match self.view_mode {
            ViewMode::Color => self.bundle.texture_buffer_selective_color(extents),
            ViewMode::Kind => self.bundle.texture_buffer_selective_kind(extents),
            ViewMode::Terrain => self.bundle.texture_buffer_selective_terrain(extents),
            ViewMode::Continent => self.bundle.texture_buffer_selective_continent(extents),
            ViewMode::Coastal => self.bundle.texture_buffer_selective_coastal(extents),
            ViewMode::Adjacencies => self.bundle.texture_buffer_selective_color(extents),
        };

        UpdateTexture::update(
            &mut self.texture,
            &mut (),
            Format::Rgba8,
            &buffer,
            offset,
            size,
        )
        .expect("unable to update texture");
    }

    fn brush_info(&self) -> String {
        if self.is_state_workspace() {
            return format!(
                "Tool: {}",
                ["Select", "Pan", "Lasso", "Brush", "Fill"]
                    .get(self.state_toolbar_tool())
                    .copied()
                    .unwrap_or("Select")
            );
        }

        match self.view_mode {
            ViewMode::Color => match self.tool.color_brush {
                Some(color) => format!("Color {}", stringify_color(color)),
                None => "Color (No Brush)".to_owned(),
            },
            ViewMode::Kind => match self.tool.kind_brush {
                Some(kind) => format!("Type {}", kind.to_str().to_uppercase()),
                None => "Type (No Brush)".to_owned(),
            },
            ViewMode::Terrain => match &self.tool.terrain_brush {
                Some(terrain) => format!("Terrain {}", terrain.to_uppercase()),
                None => "Terrain (No Brush)".to_owned(),
            },
            ViewMode::Continent => match self.tool.continent_brush {
                Some(continent) => format!("Continent {}", continent),
                None => "Continent (No Brush)".to_owned(),
            },
            ViewMode::Coastal => "Coastal".to_owned(),
            ViewMode::Adjacencies => match self.tool.adjacency_brush {
                Some(connection) => format!("Adjacencies {}", connection.to_str().to_uppercase()),
                None => "Adjacencies (No Brush)".to_owned(),
            },
        }
    }

    fn brush_mask_info(&self) -> String {
        if self.view_mode == ViewMode::Color {
            match self.tool.brush_mask {
                Some(brush_mask) => format!("Mask {}", brush_mask.to_str().to_uppercase()),
                None => "No Mask".to_owned(),
            }
        } else {
            String::new()
        }
    }

    fn camera_info(&self, interface: &Interface, cursor_pos: Option<Vector2<f64>>) -> String {
        let zoom_info = format!("{:.2}%", self.camera.scale_factor() * 100.0);
        let mut overlays = Vec::new();
        if self.map_layers.show_rivers {
            overlays.push("Rivers".to_owned());
        }
        if self.map_layers.show_adjacencies {
            overlays.push("Adjacencies".to_owned());
        }
        if self.map_layers.show_province_ids {
            overlays.push("Province IDs".to_owned());
        }
        if self.map_layers.show_province_boundaries {
            overlays.push("Province Borders".to_owned());
        }
        if self.map_layers.show_state_boundaries && self.is_state_workspace() {
            overlays.push("State Borders".to_owned());
        }
        if self.map_layers.image_overlay.enabled {
            overlays.push(format!(
                "Image {}%",
                (self.map_layers.image_overlay.opacity * 100.0).round() as u32
            ));
        }
        if self.map_layers.show_resources {
            overlays.push("Resources".to_owned());
        }
        let mut map_info = format!(
            "Workspace: {} · Map View: {}",
            self.workspace_mode().label(),
            self.map_layers.base_view.label()
        );
        if !overlays.is_empty() {
            map_info.push_str(" · Overlays: ");
            map_info.push_str(&overlays.join(", "));
        }
        let compact_status = interface.get_map_viewport().width < 900.0;
        let cursor_info = cursor_pos
            .and_then(|cursor_pos| self.camera.relative_position_int(interface, cursor_pos))
            .map_or_else(String::new, |[x, y]| format!("{}, {} px", x, y));
        if self.is_state_workspace()
            && let Some(edit) = self.state_edit_session.as_ref()
        {
            let summary = edit.summary();
            if compact_status {
                let tool = ["Select", "Pan", "Lasso", "Brush", "Fill"]
                    .get(self.state_toolbar_tool())
                    .copied()
                    .unwrap_or("Select");
                let overlay_text = if overlays.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", overlays.join(" + "))
                };
                return format!(
                    "{} · {tool} · {}{overlay_text} · {} changes",
                    self.workspace_mode().label(),
                    self.map_layers.base_view.label(),
                    summary.commands,
                );
            }
            let picker = self
                .map_tag_picker
                .active_target()
                .map_or_else(String::new, |target| {
                    let field = match target {
                        MapTagPickTarget::Owner => "Owner",
                        MapTagPickTarget::Controller => "Controller",
                        MapTagPickTarget::Core => "Core",
                        MapTagPickTarget::Claim => "Claim",
                    };
                    format!(
                        " | Pick {field} for State {}: click a state's owner; Esc cancels",
                        self.active_state_id
                            .map_or_else(|| "-".to_owned(), |id| id.to_string())
                    )
                });
            return format!(
                "{map_info} · {} · Cursor: {cursor_info:<14} · Zoom: {zoom_info:<8} · Target: {:<8} · Selected: {:<7} · Commands: {:<5} · Modified states: {}{picker}",
                self.brush_info(),
                summary
                    .target_state_id
                    .map_or_else(|| "-".to_owned(), |id| id.to_string()),
                summary.selected_provinces,
                summary.commands,
                summary.modified_states,
            );
        }
        if compact_status {
            let overlay_text = if overlays.is_empty() {
                String::new()
            } else {
                format!(" · {}", overlays.join(" + "))
            };
            return format!(
                "{} · {} · {}{overlay_text}",
                self.workspace_mode().label(),
                self.brush_info(),
                self.map_layers.base_view.label(),
            );
        }
        let brush_info = self.brush_info();
        let brush_mask_info = self.brush_mask_info();
        format!(
            "{map_info} · Cursor: {cursor_info:<14} · Zoom: {zoom_info:<8} · {brush_info}{}",
            if brush_mask_info.is_empty() {
                String::new()
            } else {
                format!(" · {brush_mask_info}")
            }
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct InspectorCanvasLayout {
    panel: [f64; 4],
    body: [f64; 4],
}

impl InspectorCanvasLayout {
    fn new(interface: &Interface) -> Self {
        let panel = interface
            .get_inspector_viewport()
            .map(|viewport| [viewport.x, viewport.y, viewport.width, viewport.height])
            .unwrap_or([0.0; 4]);
        Self::from_panel(panel)
    }

    fn from_panel(panel: [f64; 4]) -> Self {
        let [x, y, width, height] = panel;
        Self {
            panel,
            body: [
                x + 8.0,
                y + 196.0,
                (width - 16.0).max(0.0),
                (height - 246.0).max(0.0),
            ],
        }
    }

    fn collapse(self) -> [f64; 4] {
        [
            self.panel[0] + self.panel[2] - 92.0,
            self.panel[1] + 6.0,
            82.0,
            24.0,
        ]
    }

    fn open_source(self) -> [f64; 4] {
        [self.panel[0] + 12.0, self.panel[1] + 55.0, 126.0, 25.0]
    }

    fn copy_path(self) -> [f64; 4] {
        [self.panel[0] + 146.0, self.panel[1] + 55.0, 116.0, 25.0]
    }

    fn search(self) -> [f64; 4] {
        [
            self.panel[0] + 12.0,
            self.panel[1] + 86.0,
            self.panel[2] - 24.0,
            25.0,
        ]
    }

    fn section(self, index: usize) -> [f64; 4] {
        const COLUMNS: usize = 3;
        let width = (self.panel[2] - 16.0) / COLUMNS as f64;
        let column = index % COLUMNS;
        let row = index / COLUMNS;
        [
            self.panel[0] + 8.0 + column as f64 * width,
            self.panel[1] + 137.0 + row as f64 * 29.0,
            width - 2.0,
            25.0,
        ]
    }

    fn footer_left(self) -> [f64; 4] {
        let width = (self.panel[2] - 24.0) / 2.0;
        [
            self.panel[0] + 8.0,
            self.panel[1] + self.panel[3] - 42.0,
            width,
            30.0,
        ]
    }

    fn footer_right(self) -> [f64; 4] {
        let width = (self.panel[2] - 24.0) / 2.0;
        [
            self.panel[0] + 16.0 + width,
            self.panel[1] + self.panel[3] - 42.0,
            width,
            30.0,
        ]
    }
}

fn overview_property_controls(
    layout: InspectorCanvasLayout,
    control_layout: InspectorControlLayout,
) -> Vec<InspectorControlRect> {
    let input_x = 112.0;
    let input_width = (layout.body[2] - input_x - 5.0).max(0.0);
    let mut controls = [0, 1, 3, 4]
        .into_iter()
        .map(|field| {
            control_layout.body_control(
                InspectorControlId::Field(field),
                field,
                input_x,
                input_width,
                17.0,
            )
        })
        .collect::<Vec<_>>();
    controls.push(control_layout.body_control(
        InspectorControlId::Select(InspectorPickTarget::StateCategory),
        2,
        input_x,
        input_width,
        17.0,
    ));
    controls.push(control_layout.body_control(
        InspectorControlId::Toggle(5),
        5,
        input_x,
        input_width,
        17.0,
    ));
    controls
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
        let buildings_title_y = y + 132.0;
        let buildings_y = y + 142.0;
        let visible_rows = ((height - 295.0) / row_height).floor().max(1.0) as usize;
        let max_page = total_rows.saturating_sub(1) / visible_rows;
        let page = requested_page.min(max_page);
        let actions_y = buildings_y + visible_rows as f64 * row_height + 8.0;
        let error_y = actions_y + 78.0;
        let visible_error_lines = ((height - (error_y - y) - 8.0) / 18.0).floor().max(0.0) as usize;
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

    fn previous_selected(self) -> [f64; 4] {
        [self.panel[0] + 286.0, self.panel[1] + 92.0, 150.0, 26.0]
    }

    fn next_selected(self) -> [f64; 4] {
        [self.panel[0] + 446.0, self.panel[1] + 92.0, 130.0, 26.0]
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
        [self.panel[0] + 12.0, self.actions_y + 36.0, 160.0, 28.0]
    }

    fn discard(self) -> [f64; 4] {
        [self.panel[0] + 182.0, self.actions_y + 36.0, 210.0, 28.0]
    }

    fn close(self) -> [f64; 4] {
        [self.panel[0] + 402.0, self.actions_y + 36.0, 90.0, 28.0]
    }

    fn has_next_page(self, page: usize) -> bool {
        (page + 1) * self.visible_rows < self.total_rows
    }

    fn clamp_page(self, page: usize) -> usize {
        page.min(self.total_rows.saturating_sub(1) / self.visible_rows)
    }
}

#[derive(Debug, Clone, Copy)]
struct StateApplyDialogLayout {
    panel: [f64; 4],
}

fn validation_problem_summary(
    source: &ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> String {
    let severity = match diagnostic.severity {
        DiagnosticSeverity::Information => tr("project_validation.severity_info"),
        DiagnosticSeverity::Warning => tr("project_validation.severity_warning"),
        DiagnosticSeverity::Error => tr("project_validation.severity_error"),
    };
    let mut context = vec![severity.to_owned(), source.label().to_owned()];
    if let Some(id) = diagnostic.province_id {
        context.push(format!("Province {id}"));
    }
    if let Some(id) = diagnostic.state_id {
        context.push(format!("State {id}"));
    }
    format!("{} — {}", context.join(" · "), diagnostic.message)
}

fn validation_delta_items(
    report: &ProjectValidationReport,
) -> Vec<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
    fn append<'a>(
        output: &mut Vec<(ValidationSourceFilter, &'a ProjectValidationDiagnostic)>,
        source: ValidationSourceFilter,
        changes: &'a [ProjectValidationChange],
        use_before: bool,
    ) {
        output.extend(changes.iter().filter_map(|change| {
            let diagnostic = if use_before {
                change.before.as_ref()
            } else {
                change.after.as_ref().or(change.before.as_ref())
            }?;
            Some((source, diagnostic))
        }));
    }

    let mut output = Vec::with_capacity(report.diagnostics.len());
    append(
        &mut output,
        ValidationSourceFilter::New,
        &report.delta.new,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Aggravated,
        &report.delta.aggravated,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Unchanged,
        &report.delta.unchanged,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Resolved,
        &report.delta.resolved,
        true,
    );
    append(
        &mut output,
        ValidationSourceFilter::Improved,
        &report.delta.improved,
        false,
    );
    output.sort_by_key(|(_, diagnostic)| match diagnostic.severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Information => 2,
    });
    output
}

fn project_validation_blockers(
    report: Option<&ProjectValidationReport>,
) -> Vec<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
    report
        .map(validation_delta_items)
        .unwrap_or_default()
        .into_iter()
        .filter(|(source, diagnostic)| {
            diagnostic.severity == DiagnosticSeverity::Error
                && matches!(
                    *source,
                    ValidationSourceFilter::New | ValidationSourceFilter::Aggravated
                )
        })
        .collect()
}

fn validation_problem_matches(
    view: &ValidationProblemsView,
    source: ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> bool {
    source != ValidationSourceFilter::Resolved
        && view.source.matches(source)
        && view.severity.matches(diagnostic.severity)
        && view.domain.matches(diagnostic.domain)
        && (!view.blocking_only
            || diagnostic.severity == DiagnosticSeverity::Error
                && matches!(
                    source,
                    ValidationSourceFilter::New | ValidationSourceFilter::Aggravated
                ))
}

fn removed_provinces_with_state_references(
    before: &BTreeSet<u32>,
    after: &BTreeSet<u32>,
    state_by_province: &HashMap<u32, u32>,
) -> Vec<(u32, u32)> {
    before
        .difference(after)
        .filter_map(|province_id| {
            state_by_province
                .get(province_id)
                .map(|state_id| (*province_id, *state_id))
        })
        .collect()
}

fn validation_problem_details(
    source: ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> String {
    format!(
        "Source: {}\nSeverity: {:?}\nDomain: {:?}\nCode: {}\nMessage: {}\nPath: {}\nProvince: {}\nState: {}",
        source.label(),
        diagnostic.severity,
        diagnostic.domain,
        diagnostic.code,
        diagnostic.message,
        diagnostic
            .path
            .as_ref()
            .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
        diagnostic
            .province_id
            .map_or_else(|| "-".to_owned(), |id| id.to_string()),
        diagnostic
            .state_id
            .map_or_else(|| "-".to_owned(), |id| id.to_string()),
    )
}

fn validation_source_path(
    diagnostic: &ProjectValidationDiagnostic,
    project: Option<&Hoi4Project>,
) -> Option<PathBuf> {
    let project = project?;
    let path = diagnostic.path.as_ref()?;
    let logical_path = if let Ok(relative) = path.strip_prefix(&project.paths.root) {
        PathBuf::from(relative)
    } else {
        let parts = path.components().collect::<Vec<_>>();
        let index = parts
            .iter()
            .position(|part| part.as_os_str().eq_ignore_ascii_case("candidate"))?;
        parts[index + 1..].iter().collect::<PathBuf>()
    };
    let source = project.paths.root.join(logical_path);
    source.is_file().then_some(source)
}

fn validation_display_path(
    diagnostic: &ProjectValidationDiagnostic,
    project: Option<&Hoi4Project>,
) -> String {
    let Some(path) = diagnostic.path.as_ref() else {
        return String::new();
    };
    if let Some(project) = project
        && let Ok(relative) = path.strip_prefix(&project.paths.root)
    {
        return format!("File: {}", relative.display());
    }
    let parts = path.components().collect::<Vec<_>>();
    if let Some(index) = parts
        .iter()
        .position(|part| part.as_os_str().eq_ignore_ascii_case("candidate"))
    {
        let relative = parts[index + 1..].iter().collect::<PathBuf>();
        return format!("File: {}", relative.display());
    }
    format!(
        "File: {}",
        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned()
        )
    )
}

impl StateApplyDialogLayout {
    fn new(interface: &Interface) -> Self {
        let window = interface.get_window_size();
        let width = (window[0] - 32.0).clamp(440.0, 640.0);
        let height = (window[1] - 32.0).clamp(330.0, 390.0);
        Self {
            panel: [
                (window[0] - width) / 2.0,
                (window[1] - height) / 2.0,
                width,
                height,
            ],
        }
    }

    fn primary(self) -> [f64; 4] {
        let width = (self.panel[2] - 48.0) / 3.0;
        [
            self.panel[0] + 12.0,
            self.panel[1] + self.panel[3] - 42.0,
            width,
            30.0,
        ]
    }

    fn secondary(self) -> [f64; 4] {
        let primary = self.primary();
        [primary[0] + primary[2] + 8.0, primary[1], primary[2], 30.0]
    }

    fn close(self) -> [f64; 4] {
        let secondary = self.secondary();
        [
            secondary[0] + secondary[2] + 8.0,
            secondary[1],
            secondary[2],
            30.0,
        ]
    }

    fn filters_toggle(self) -> [f64; 4] {
        [
            self.panel[0] + 12.0,
            self.panel[1] + 180.0,
            self.panel[2] - 24.0,
            28.0,
        ]
    }

    fn severity_filter(self) -> [f64; 4] {
        let width = (self.panel[2] - 40.0) / 3.0;
        [self.panel[0] + 12.0, self.panel[1] + 216.0, width, 28.0]
    }

    fn source_filter(self) -> [f64; 4] {
        let first = self.severity_filter();
        [first[0] + first[2] + 8.0, first[1], first[2], first[3]]
    }

    fn domain_filter(self) -> [f64; 4] {
        let second = self.source_filter();
        [second[0] + second[2] + 8.0, second[1], second[2], second[3]]
    }

    fn validation_problem_row(self, index: usize, filters_expanded: bool) -> [f64; 4] {
        [
            self.panel[0] + 12.0,
            self.panel[1] + if filters_expanded { 252.0 } else { 216.0 } + index as f64 * 26.0,
            self.panel[2] - 24.0,
            24.0,
        ]
    }

    fn problem_row(self, index: usize) -> [f64; 4] {
        [
            self.panel[0] + 12.0,
            self.panel[1] + 216.0 + index as f64 * 26.0,
            self.panel[2] - 24.0,
            24.0,
        ]
    }

    fn validation_close(self) -> [f64; 4] {
        [
            self.panel[0] + self.panel[2] - 78.0,
            self.panel[1] + 7.0,
            66.0,
            28.0,
        ]
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
        let visible_error_lines = ((height - (error_y - y) - 8.0) / 18.0).floor().max(0.0) as usize;
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
        [field[0] + field[2] + 8.0, field[1], 168.0, field[3]]
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
        [
            self.panel[0] + 28.0 + width * 2.0,
            self.buttons_y,
            width,
            28.0,
        ]
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

fn state_creation_picker(field: usize) -> Option<InspectorPickTarget> {
    match field {
        3 => Some(InspectorPickTarget::StateCategory),
        6 => Some(InspectorPickTarget::Owner),
        7 => Some(InspectorPickTarget::Controller),
        8 => Some(InspectorPickTarget::Core),
        9 => Some(InspectorPickTarget::Claim),
        10 => Some(InspectorPickTarget::Resource),
        11 => Some(InspectorPickTarget::StateBuilding),
        _ => None,
    }
}

fn is_coastal_only_building(name: &str) -> bool {
    name.eq_ignore_ascii_case("naval_base")
}

fn building_matches_picker_scope(target: InspectorPickTarget, scope: BuildingScope) -> bool {
    matches!(
        (target, scope),
        (InspectorPickTarget::StateBuilding, BuildingScope::State)
            | (
                InspectorPickTarget::ProvinceBuilding,
                BuildingScope::Province
            )
    )
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
    )
    .expect("unable to draw state property editor text");
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
        if enabled {
            colors::BUTTON_ACTIVE
        } else {
            colors::BUTTON_TOOLBAR
        },
        rect,
        ctx.transform,
        gl,
    );
    draw_canvas_text(
        ctx,
        glyph_cache,
        gl,
        if enabled {
            colors::WHITE
        } else {
            colors::WHITE_T
        },
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

fn format_integer_input(value: &str, grouped: bool) -> String {
    if grouped {
        parse_grouped_nonnegative_integer(value)
            .map(format_integer_pt_br)
            .unwrap_or_else(|_| value.to_owned())
    } else {
        value.to_owned()
    }
}

fn point_in_rect(point: Vector2<f64>, rect: [f64; 4]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] <= rect[0] + rect[2]
        && point[1] <= rect[1] + rect[3]
}

fn inspector_picker_rect(layout: InspectorCanvasLayout) -> [f64; 4] {
    [
        layout.body[0] + 6.0,
        layout.body[1] + 6.0,
        (layout.body[2] - 12.0).max(160.0),
        234.0,
    ]
}

fn inspector_picker_up_rect(popup: [f64; 4]) -> [f64; 4] {
    [popup[0] + popup[2] - 22.0, popup[1] + 48.0, 18.0, 18.0]
}

fn inspector_picker_down_rect(popup: [f64; 4]) -> [f64; 4] {
    [
        popup[0] + popup[2] - 22.0,
        popup[1] + popup[3] - 22.0,
        18.0,
        18.0,
    ]
}

fn append_unique_csv(field: &mut String, value: &str) {
    if field
        .split(',')
        .any(|entry| entry.trim().eq_ignore_ascii_case(value))
    {
        return;
    }
    if !field.trim().is_empty() {
        field.push_str(", ");
    }
    field.push_str(value);
}

fn load_configured_image_overlay(
    location: &Location,
    project: Option<&Hoi4Project>,
    settings: &ImageOverlayProjectSettings,
    map_dimensions: Vector2<u32>,
    texture_settings: &TextureSettings,
) -> (
    Option<ImageOverlaySource>,
    Option<Texture>,
    String,
    Option<[u32; 2]>,
) {
    if settings.use_project_heightmap {
        let (texture, status, dimensions) =
            load_project_image_overlay(location, map_dimensions, texture_settings);
        return (
            Some(ImageOverlaySource::ProjectHeightmap),
            texture,
            status,
            dimensions,
        );
    }
    let Some(source_path) = settings.source_path.as_ref() else {
        return (
            None,
            None,
            "Image Overlay unavailable: no image selected.".to_owned(),
            None,
        );
    };
    let path = project
        .map(|project| project.paths.root.join(source_path))
        .filter(|_| !source_path.is_absolute())
        .unwrap_or_else(|| source_path.clone());
    let source = ImageOverlaySource::Custom(path.clone());
    match fs_err::read(&path) {
        Ok(bytes) => match decode_image_overlay(
            &bytes,
            map_dimensions,
            texture_settings,
            &path.display().to_string(),
        ) {
            Ok((texture, dimensions, status)) => {
                (Some(source), Some(texture), status, Some(dimensions))
            }
            Err(error) => (Some(source), None, error, None),
        },
        Err(error) => (
            Some(source),
            None,
            format!(
                "Image Overlay unavailable: failed to read {}: {error}",
                path.display()
            ),
            None,
        ),
    }
}

fn load_project_image_overlay(
    location: &Location,
    map_dimensions: [u32; 2],
    texture_settings: &TextureSettings,
) -> (Option<Texture>, String, Option<[u32; 2]>) {
    let bytes = location.clone().manipulate_files(|files| {
        let Some(mut file) = files.open_file_maybe_not_found("heightmap.bmp")? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read heightmap.bmp: {error}"))?;
        Ok(Some(bytes))
    });
    let bytes = match bytes {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return (
                None,
                "Image Overlay unavailable: map/heightmap.bmp was not found.".into(),
                None,
            );
        }
        Err(error) => {
            return (None, format!("Image Overlay unavailable: {error}"), None);
        }
    };
    match decode_image_overlay(
        &bytes,
        map_dimensions,
        texture_settings,
        "map/heightmap.bmp",
    ) {
        Ok((texture, dimensions, status)) => (Some(texture), status, Some(dimensions)),
        Err(error) => (None, error, None),
    }
}

fn decode_image_overlay(
    bytes: &[u8],
    map_dimensions: [u32; 2],
    texture_settings: &TextureSettings,
    source: &str,
) -> Result<(Texture, [u32; 2], String), String> {
    let image = decode_image_overlay_pixels(bytes, map_dimensions, source)?;
    let dimensions = [image.width(), image.height()];
    Ok((
        Texture::from_image(&image, texture_settings),
        dimensions,
        format!(
            "Image Overlay loaded read-only from {source}: {}x{}",
            dimensions[0], dimensions[1]
        ),
    ))
}

fn decode_image_overlay_pixels(
    bytes: &[u8],
    map_dimensions: [u32; 2],
    source: &str,
) -> Result<RgbaImage, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("Image Overlay unavailable: failed to decode {source}: {error}"))?
        .into_rgba8();
    let dimensions = [image.width(), image.height()];
    if dimensions != map_dimensions {
        return Err(format!(
            "Image Overlay blocked: {source} is {}x{}, but provinces.bmp is {}x{}.",
            dimensions[0], dimensions[1], map_dimensions[0], map_dimensions[1]
        ));
    }
    Ok(image)
}

fn project_status_message_with_session(
    project: &Hoi4Project,
    edit: Option<&StateEditSession>,
    visual_update_ms: u128,
    visual_update_kind: &str,
) -> String {
    let name = project
        .paths
        .root
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| project.paths.root.to_string_lossy());
    let summary = &project.load_summary;
    let mut text = format!(
        "Project: {name}\n\
     Baseline: {} indexed / {} files | Assigned: {} | Unassigned land: {}\n\
     Baseline Diagnostics: {} errors, {} warnings | Ambiguous: {} | Unknown refs: {}",
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
       Selected provinces: {} | Target: {} | In-memory edit diagnostics: {} errors, {} warnings | Visual refresh: {} {} ms\n\
       Last edit command: preflight {} us, apply {} us, index {} us | Visual timings: texture {} ms, boundaries {} ms",
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

fn workspace_dirty_status(province_modified: bool, pending_states: usize) -> String {
    let province = if province_modified {
        "Modified"
    } else {
        "Saved"
    };
    format!("Province Map: {province} | States: {pending_states} pending changes")
}

fn state_edit_status_message(
    edit: &StateEditSession,
    active_province_id: Option<u32>,
    active_state_id: Option<u32>,
    active_tool: &str,
) -> String {
    let summary = edit.summary();
    let diagnostics = edit.diagnostics().len();
    let dirty_states = edit.dirty_state_ids().iter().map(u32::to_string).join(", ");
    let sources = edit
        .selection_sources()
        .into_iter()
        .map(|(state_id, count)| match state_id {
            Some(state_id) => format!("State {state_id}: {count}"),
            None => format!("Unassigned: {count}"),
        })
        .join(", ");
    format!(
        "Workspace: States | Tool: {active_tool} | Active province: {} | Active state: {}\n\
     State edit session: active {} | created {} | removed {} | reserved IDs {}\n\
     {} selected [{}] | target {} | undo {} ({}) | redo {} ({}) | dirty states {} [{}] | diagnostics {}\n\
     Last command: {}\n\
     Actions: Ctrl+click select province | normal click active/target | Edit > New/Remove/State properties/Province data/Move/Unassign/Discard",
        active_province_id.map_or_else(|| "None".to_owned(), |id| id.to_string()),
        active_state_id.map_or_else(|| "None".to_owned(), |id| id.to_string()),
        summary.active_states,
        summary.created_states,
        summary.removed_states,
        summary.reserved_state_ids,
        summary.selected_provinces,
        if sources.is_empty() {
            "none".to_owned()
        } else {
            sources
        },
        summary
            .target_state_id
            .map_or_else(|| "None".to_owned(), |id| id.to_string()),
        summary.commands,
        if edit.can_undo() {
            "available"
        } else {
            "empty"
        },
        summary.redo_commands,
        if edit.can_redo() {
            "available"
        } else {
            "empty"
        },
        summary.modified_states,
        if dirty_states.is_empty() {
            "none".to_owned()
        } else {
            dirty_states
        },
        diagnostics,
        edit.last_command_description()
            .unwrap_or_else(|| "none".to_owned())
    )
}

fn selection_sources_message(edit: &StateEditSession) -> String {
    edit.selection_sources()
        .into_iter()
        .map(|(state_id, count)| match state_id {
            Some(state_id) => format!("From State {state_id}: {count}"),
            None => format!("Unassigned: {count}"),
        })
        .join("\n")
}

fn selection_message(
    project: &Hoi4Project,
    edit: Option<&StateEditSession>,
    selection: Option<&StateSelection>,
) -> Option<String> {
    match selection {
        Some(StateSelection::State {
            state_id,
            province_id,
        }) => {
            let working = edit.and_then(|edit| edit.state_data(*state_id));
            let name = working
                .as_ref()
                .or_else(|| {
                    project
                        .state_document(*state_id)
                        .and_then(|document| document.data.as_ref())
                })
                .and_then(|data| data.name.as_deref())
                .unwrap_or("<unnamed>");
            Some(format!(
                "Selected state {state_id} — {name} from province {province_id}."
            ))
        }
        Some(StateSelection::AmbiguousProvince {
            province_id,
            state_ids,
        }) => Some(format!(
            "Province {province_id} is assigned to states {}.",
            state_ids.iter().join(", ")
        )),
        Some(StateSelection::UnassignedProvince { province_id }) => {
            Some(format!("Province {province_id} has no state assignment."))
        }
        None => None,
    }
}

fn option_display(value: Option<impl fmt::Display>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_owned())
}

fn option_text_display(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "—".to_owned())
}

fn format_set(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        "—".to_owned()
    } else {
        values.iter().join(", ")
    }
}

fn format_named_values(values: &BTreeMap<String, i64>) -> String {
    if values.is_empty() {
        "—".to_owned()
    } else {
        values
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .join(", ")
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
    selection: Option<&StateSelection>,
) -> Option<String> {
    match selection {
        Some(StateSelection::State {
            state_id,
            province_id,
        }) => {
            let document = project.state_document(*state_id)?;
            let data = document.data.as_ref()?;
            let working = edit.and_then(|edit| edit.state_data(*state_id));
            let working = working.as_ref().unwrap_or(data);
            let resources = format_named_values(&working.resources);
            let original_resources = format_named_values(&data.resources);
            let victory_points = if working.history.victory_points.is_empty() {
                "—".to_owned()
            } else {
                working
                    .history
                    .victory_points
                    .iter()
                    .map(|victory_point| {
                        format!("{}={}", victory_point.province_id, victory_point.value)
                    })
                    .join(", ")
            };
            let cores = format_set(&working.history.cores);
            let original_cores = format_set(&data.history.cores);
            let claims = format_set(&working.history.claims);
            let original_claims = format_set(&data.history.claims);
            let state_buildings = format_named_values(&working.history.state_buildings);
            let original_state_buildings = format_named_values(&data.history.state_buildings);
            let building_entries = working.history.state_buildings.len()
                + working
                    .history
                    .province_buildings
                    .values()
                    .map(BTreeMap::len)
                    .sum::<usize>();
            let modified = edit.is_some_and(|edit| edit.is_state_dirty(*state_id));
            let (errors, warnings) = project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.path.as_ref() == Some(&document.path))
                .fold((0, 0), |(errors, warnings), diagnostic| {
                    match diagnostic.severity {
                        DiagnosticSeverity::Error => (errors + 1, warnings),
                        DiagnosticSeverity::Warning => (errors, warnings + 1),
                        DiagnosticSeverity::Info => (errors, warnings),
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
         Baseline diagnostics: {} errors, {} warnings\n\
         File: {}\n\
         Actions: Edit > Edit state properties | Remove state from session",
                working.name.as_deref().unwrap_or("<unnamed>"),
                if modified {
                    "Modified in memory"
                } else {
                    "Working values match original"
                },
                working.provinces.len(),
                compared_value(
                    option_display(working.manpower),
                    option_display(data.manpower)
                ),
                compared_value(
                    option_text_display(&working.state_category),
                    option_text_display(&data.state_category)
                ),
                compared_value(
                    option_display(working.buildings_max_level_factor),
                    option_display(data.buildings_max_level_factor)
                ),
                compared_value(
                    option_display(working.local_supplies),
                    option_display(data.local_supplies)
                ),
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
        }
        Some(StateSelection::AmbiguousProvince {
            province_id,
            state_ids,
        }) => Some(format!(
            "Ambiguous province {province_id}\nCandidate states: {}",
            state_ids.iter().join(", ")
        )),
        Some(StateSelection::UnassignedProvince { province_id }) => {
            Some(format!("Unassigned land province {province_id}"))
        }
        None => None,
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
        }
        WorkingStateOrigin::CreatedInSession => {
            "Created in memory\nNo state file exists yet".to_owned()
        }
    };
    let victory_points = if data.history.victory_points.is_empty() {
        "—".to_owned()
    } else {
        data.history
            .victory_points
            .iter()
            .map(|victory_point| format!("{}={}", victory_point.province_id, victory_point.value))
            .join(", ")
    };
    let province_buildings = data
        .history
        .province_buildings
        .values()
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
    let state = state_id
        .map(|state_id| {
            let name = edit
                .state_data(state_id)
                .or_else(|| {
                    project
                        .state_document(state_id)
                        .and_then(|document| document.data.clone())
                })
                .and_then(|data| data.name)
                .unwrap_or_else(|| "<unnamed>".to_owned());
            format!("State {state_id} — {name}")
        })
        .unwrap_or_else(|| "Unassigned".to_owned());
    let edit_status = match edit.editable_province_state(province_id) {
        Ok(state_id) => format!("Editable in State {state_id} | Action: Edit > Edit province data"),
        Err(error) => error.to_string(),
    };
    let mut diagnostics = project
        .diagnostics_for_province(province_id)
        .map(|diagnostic| {
            let marker = match diagnostic.severity {
                DiagnosticSeverity::Error => "!",
                DiagnosticSeverity::Warning => "~",
                DiagnosticSeverity::Info => "i",
            };
            format!("{marker}{}", diagnostic.message)
        })
        .collect::<Vec<_>>();
    if state_id.is_none() {
        diagnostics.push(
            "~Land province has no State assignment (Save Project is still allowed).".to_owned(),
        );
    }
    if diagnostics.is_empty() {
        diagnostics.push("No contextual problems.".to_owned());
    }
    Some(format!(
        "PROVINCE {province_id}\n\
     {state}\n\
     Overview: active province | Selected provinces for Move: {}\n\
     Victory point: {}\n\
     Province Buildings: {}\n\
     State editing: {}\n\
     Problems: {}",
        edit.selected_provinces().len(),
        option_display(data.victory_point),
        format_named_values(&data.buildings),
        edit_status,
        diagnostics.join(" | "),
    ))
}

fn draw_resource_caption(
    ctx: Context,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics,
    text: &str,
    position: [f64; 2],
    scale: f64,
) {
    let font_size = (FONT_SIZE as f64 * scale).round() as u32;
    graphics::text(
        colors::BLACK,
        font_size,
        text,
        glyph_cache,
        ctx.transform.trans(position[0] + 1.0, position[1] + 1.0),
        gl,
    )
    .expect("unable to draw resource label shadow");
    graphics::text(
        colors::WHITE,
        font_size,
        text,
        glyph_cache,
        ctx.transform.trans_pos(position),
        gl,
    )
    .expect("unable to draw resource label");
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("bundle", &self.bundle)
            .field("history", &self.history)
            .field("texture", &format_args!("..."))
            .field("state_texture", &self.state_texture.as_ref().map(|_| "..."))
            .field("state_boundaries", &self.state_boundaries.len())
            .field(
                "selected_state_boundaries",
                &self.selected_state_boundaries.len(),
            )
            .field(
                "selected_province_boundaries",
                &self.selected_province_boundaries.len(),
            )
            .field("view_mode", &self.view_mode)
            .field("map_layers", &self.map_layers)
            .field("state_selection", &self.state_selection)
            .field("active_state_id", &self.active_state_id)
            .field("active_province_id", &self.active_province_id)
            .field("tool", &self.tool)
            .field("problems", &self.problems)
            .field("unknown_terrains", &self.unknown_terrains)
            .field("location", &self.location)
            .field("project", &self.project)
            .field(
                "state_edit_session",
                &self.state_edit_session.as_ref().map(|_| "..."),
            )
            .field("state_lifecycle_draft", &self.state_lifecycle_draft)
            .field(
                "last_state_visual_update_ms",
                &self.last_state_visual_update_ms,
            )
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
    EditableProvinceMap,
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
    pub mode: ToolMode,
}

impl ToolSettings {
    pub fn cycle_brush_mask(&mut self) {
        self.brush_mask = match self.brush_mask {
            None => Some(BrushMask::LandLakes),
            Some(BrushMask::LandLakes) => Some(BrushMask::Sea),
            Some(BrushMask::Sea) => None,
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
            mode: ToolMode::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolMode {
    PaintArea,
    PaintBucket,
    Lasso(Lasso),
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
    Sea,
}

impl BrushMask {
    #[inline]
    pub fn includes(&self, kind: impl Into<ProvinceKind>) -> bool {
        match (self, kind.into()) {
            (BrushMask::LandLakes, ProvinceKind::Land) => true,
            (BrushMask::LandLakes, ProvinceKind::Lake) => true,
            (BrushMask::Sea, ProvinceKind::Sea) => true,
            (_, ProvinceKind::Unknown) => true,
            _ => false,
        }
    }

    fn to_str(self) -> &'static str {
        match self {
            BrushMask::LandLakes => "land + lakes",
            BrushMask::Sea => "sea",
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
    Adjacencies,
}

impl Default for ViewMode {
    fn default() -> ViewMode {
        ViewMode::Color
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CameraCombo<'a> {
    pub(super) camera: &'a Camera,
    pub(super) interface: &'a Interface,
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
    pub panning: bool,
}

impl Camera {
    fn new(texture: &Texture) -> Self {
        use opengl_graphics::ImageSize;
        let (width, height) = texture.get_size();
        let texture_size = [width as f64, height as f64];
        let display_matrix =
            vecmath::mat2x3_id().trans_pos(vecmath::vec2_scale(texture_size, -0.5));
        Camera {
            texture_size,
            display_matrix,
            panning: false,
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
        self.display_matrix = self
            .display_matrix
            .trans_pos(cursor_rel)
            .trans_pos(window_center)
            .zoom(zoom)
            .trans_pos(vecmath::vec2_neg(cursor_rel))
            .trans_pos(vecmath::vec2_neg(window_center));
    }

    pub fn reset(&mut self) {
        self.display_matrix =
            vecmath::mat2x3_id().trans_pos(vecmath::vec2_scale(self.texture_size, -0.5));
    }

    pub fn center_on(&mut self, interface: &Interface, map_pos: Vector2<f64>) {
        let current = self.relative_position(interface, interface.get_window_center());
        self.display_matrix = self
            .display_matrix
            .trans_pos(vecmath::vec2_sub(current, map_pos));
    }

    pub fn ensure_scale(&mut self, interface: &Interface, minimum: f64) {
        let current = self.scale_factor();
        if current < minimum {
            let dz = (minimum / current).log2() / ZOOM_SENSITIVITY;
            self.on_mouse_zoom(interface, dz, interface.get_window_center());
        }
    }

    pub fn focus_extents(&mut self, interface: &Interface, extents: Extents, maximum: f64) {
        let width = (extents.upper[0] - extents.lower[0] + 1) as f64;
        let height = (extents.upper[1] - extents.lower[1] + 1) as f64;
        let viewport = interface.get_map_viewport();
        let target = (viewport.width / (width * 1.2))
            .min(viewport.height / (height * 1.2))
            .clamp(0.2, maximum);
        let dz = (target / self.scale_factor()).log2() / ZOOM_SENSITIVITY;
        self.on_mouse_zoom(interface, dz, interface.get_window_center());
        self.center_on(
            interface,
            [
                (extents.lower[0] + extents.upper[0]) as f64 / 2.0,
                (extents.lower[1] + extents.upper[1]) as f64 / 2.0,
            ],
        );
    }

    pub fn set_panning(&mut self, panning: bool) {
        self.panning = panning;
    }

    /// Converts a point from camera space to map space
    pub(super) fn relative_position(
        &self,
        interface: &Interface,
        pos: Vector2<f64>,
    ) -> Vector2<f64> {
        vecmath::row_mat2x3_transform_pos2(self.display_matrix_inv(interface), pos)
    }

    pub(super) fn relative_position_int(
        &self,
        interface: &Interface,
        pos: Vector2<f64>,
    ) -> Option<Vector2<u32>> {
        if !interface.map_contains(pos) {
            return None;
        }
        let pos = self.relative_position(interface, pos);
        self.within_dimensions(pos)
            .then(|| [pos[0] as u32, pos[1] as u32])
    }

    /// Converts from map space to camera space
    pub(super) fn compute_position(
        &self,
        interface: &Interface,
        pos: Vector2<f64>,
    ) -> Vector2<f64> {
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
        0.0 <= pos[0]
            && pos[0] < self.texture_size[0]
            && 0.0 <= pos[1]
            && pos[1] < self.texture_size[1]
    }

    #[inline]
    pub(super) fn within_viewport(&self, interface: &Interface, pos: Vector2<f64>) -> bool {
        interface.map_contains(pos)
    }
}

fn export_image_buffer<P: AsRef<Path>>(path: P, image: RgbImage) -> Result<(), Error> {
    let file = crate::util::files::create_file(path.as_ref())?;
    super::map::write_rgb_bmp_image(BufWriter::new(file), &image)
}

fn discover_base_game_root() -> Option<PathBuf> {
    std::env::var_os("HOI4_BASE_GAME_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            [
                r"C:\Program Files (x86)\Steam\steamapps\common\Hearts of Iron IV",
                r"C:\Program Files\Steam\steamapps\common\Hearts of Iron IV",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_dir())
        })
}

#[inline]
fn drawable_color(color: Color) -> DrawColor {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        1.0,
    ]
}

fn cycle_kinds<P>(kind: Option<P>, backwards: bool) -> DefinitionKind
where
    P: Into<ProvinceKind>,
{
    match kind.map(P::into) {
        Some(ProvinceKind::Land) => {
            if backwards {
                DefinitionKind::Lake
            } else {
                DefinitionKind::Sea
            }
        }
        Some(ProvinceKind::Sea) => {
            if backwards {
                DefinitionKind::Land
            } else {
                DefinitionKind::Lake
            }
        }
        Some(ProvinceKind::Lake) => {
            if backwards {
                DefinitionKind::Sea
            } else {
                DefinitionKind::Land
            }
        }
        Some(ProvinceKind::Unknown) | None => DefinitionKind::Land,
    }
}

fn snapshot_belongs_to_generation(
    snapshot: &RoundTripFailureSnapshot,
    generation: ProjectGeneration,
) -> bool {
    snapshot.generation == generation
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
        Some(ConnectionKind::Strait) => {
            if backwards {
                ConnectionKind::Impassable
            } else {
                ConnectionKind::Canal
            }
        }
        Some(ConnectionKind::Canal) => {
            if backwards {
                ConnectionKind::Strait
            } else {
                ConnectionKind::Impassable
            }
        }
        Some(ConnectionKind::Impassable) => {
            if backwards {
                ConnectionKind::Canal
            } else {
                ConnectionKind::Strait
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::io::Cursor;

    use image::{DynamicImage, ImageOutputFormat};

    use super::*;

    #[test]
    fn round_trip_failure_snapshot_cannot_cross_project_generations() {
        let snapshot = RoundTripFailureSnapshot {
            generation: ProjectGeneration(4),
            summary: "Province 521 changed state membership".to_owned(),
            details: "semantic mismatch".to_owned(),
        };
        assert!(snapshot_belongs_to_generation(
            &snapshot,
            ProjectGeneration(4)
        ));
        assert!(!snapshot_belongs_to_generation(
            &snapshot,
            ProjectGeneration(5)
        ));
    }

    #[test]
    fn only_newly_removed_provinces_with_state_references_are_blocked() {
        let before = BTreeSet::from([10, 20, 30]);
        let after = BTreeSet::from([10, 30]);
        let state_by_province = HashMap::from([(20, 89), (30, 90)]);

        assert_eq!(
            removed_provinces_with_state_references(&before, &after, &state_by_province),
            vec![(20, 89)]
        );
    }

    #[test]
    fn image_overlay_accepts_matching_image_and_blocks_dimension_mismatch() {
        let image = RgbaImage::from_pixel(2, 3, Rgba([12, 34, 56, 255]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageOutputFormat::Png)
            .unwrap();

        let decoded = decode_image_overlay_pixels(bytes.get_ref(), [2, 3], "test.png").unwrap();
        assert_eq!([decoded.width(), decoded.height()], [2, 3]);

        let error = decode_image_overlay_pixels(bytes.get_ref(), [4, 5], "test.png").unwrap_err();
        assert!(error.contains("test.png is 2x3"));
        assert!(error.contains("provinces.bmp is 4x5"));
    }

    #[test]
    fn inspector_tabs_use_two_non_overlapping_rows_inside_the_panel() {
        let layout = InspectorCanvasLayout::from_panel([100.0, 20.0, 420.0, 600.0]);
        let tabs = (0..INSPECTOR_ICONS.len())
            .map(|index| layout.section(index))
            .collect::<Vec<_>>();

        for tab in &tabs {
            assert!(tab[0] >= layout.panel[0]);
            assert!(tab[1] >= layout.panel[1]);
            assert!(tab[0] + tab[2] <= layout.panel[0] + layout.panel[2]);
            assert!(tab[1] + tab[3] <= layout.body[1]);
        }
        for (index, left) in tabs.iter().enumerate() {
            for right in tabs.iter().skip(index + 1) {
                assert!(
                    left[0] + left[2] <= right[0]
                        || right[0] + right[2] <= left[0]
                        || left[1] + left[3] <= right[1]
                        || right[1] + right[3] <= left[1]
                );
            }
        }
    }

    #[test]
    fn general_editor_controls_are_full_width_and_stay_inside_drawer() {
        let layout = InspectorCanvasLayout::from_panel([100.0, 20.0, 420.0, 600.0]);
        let control_layout = InspectorControlLayout::new(
            [layout.body[0], layout.body[1]],
            layout.body[2],
            0.0,
            0.0,
            1.0,
        );
        let controls = overview_property_controls(layout, control_layout);
        assert_eq!(controls.len(), 6);
        for expected in [
            InspectorControlId::Field(0),
            InspectorControlId::Field(1),
            InspectorControlId::Select(InspectorPickTarget::StateCategory),
            InspectorControlId::Field(3),
            InspectorControlId::Field(4),
            InspectorControlId::Toggle(5),
        ] {
            assert!(controls.iter().any(|control| control.id == expected));
        }
        for control in controls {
            assert!(control.draw.x >= layout.body[0]);
            assert!(control.draw.right() <= layout.body[0] + layout.body[2]);
            assert!(control.draw.y >= layout.body[1]);
            assert!(control.draw.bottom() <= layout.body[1] + layout.body[3]);
            assert!(control.draw.width > layout.body[2] / 2.0);
        }
    }

    #[test]
    fn new_state_uses_catalog_pickers_for_structured_values() {
        assert_eq!(
            state_creation_picker(3),
            Some(InspectorPickTarget::StateCategory)
        );
        assert_eq!(
            state_creation_picker(10),
            Some(InspectorPickTarget::Resource)
        );
        assert_eq!(
            state_creation_picker(11),
            Some(InspectorPickTarget::StateBuilding)
        );
        assert_eq!(state_creation_picker(1), None);
    }

    #[test]
    fn province_editor_distinguishes_selection_navigation_and_building_pages() {
        let layout = ProvinceEditorLayout {
            panel: [40.0, 40.0, 700.0, 600.0],
            row_height: 29.0,
            buildings_title_y: 172.0,
            buildings_y: 182.0,
            visible_rows: 8,
            page: 0,
            total_rows: 12,
            actions_y: 422.0,
            error_y: 500.0,
            visible_error_lines: 4,
        };

        assert_ne!(layout.previous_selected(), layout.previous_page());
        assert_ne!(layout.next_selected(), layout.next_page());
        assert!(is_coastal_only_building("naval_base"));
        assert!(!is_coastal_only_building("bunker"));
        assert!(building_matches_picker_scope(
            InspectorPickTarget::ProvinceBuilding,
            BuildingScope::Province
        ));
        assert!(!building_matches_picker_scope(
            InspectorPickTarget::ProvinceBuilding,
            BuildingScope::State
        ));
    }

    #[test]
    fn workspace_dirty_labels_keep_province_and_state_domains_independent() {
        assert_eq!(
            workspace_dirty_status(true, 0),
            "Province Map: Modified | States: 0 pending changes"
        );
        assert_eq!(
            workspace_dirty_status(false, 4),
            "Province Map: Saved | States: 4 pending changes"
        );
    }

    #[test]
    fn validation_problem_rows_make_space_only_for_expanded_filters() {
        let layout = StateApplyDialogLayout {
            panel: [20.0, 30.0, 560.0, 390.0],
        };
        assert_eq!(layout.validation_problem_row(0, false)[1], 246.0);
        assert_eq!(layout.validation_problem_row(0, true)[1], 282.0);
        assert!(layout.validation_problem_row(2, true)[1] < layout.primary()[1]);
    }

    #[test]
    fn validation_problem_summary_keeps_severity_source_and_navigation_context() {
        let diagnostic = ProjectValidationDiagnostic {
            kind: crate::app::project::ProjectDiagnosticKind::UnknownProvince,
            severity: DiagnosticSeverity::Error,
            domain: ProjectValidationDomain::State,
            code: "unknown-province".to_owned(),
            message_key: "unknown-province".to_owned(),
            path: None,
            related_path: None,
            span: None,
            province_id: Some(501),
            state_id: Some(123),
            blocks_save: true,
            message: "Province reference is invalid".to_owned(),
        };

        let summary = validation_problem_summary(&ValidationSourceFilter::New, &diagnostic);
        assert!(summary.contains("Province 501"));
        assert!(summary.contains("State 123"));
        assert!(summary.contains("Province reference is invalid"));
    }

    #[test]
    fn blocked_project_save_review_cannot_authorize_save() {
        assert_eq!(
            project_save_review_primary_action(true, true),
            ProjectSaveReviewPrimaryAction::ViewBlockingProblems
        );
        assert_ne!(
            project_save_review_primary_action(true, false),
            ProjectSaveReviewPrimaryAction::ConfirmSave
        );
        assert_eq!(
            project_save_review_primary_action(false, false),
            ProjectSaveReviewPrimaryAction::ConfirmSave
        );
        assert_eq!(
            project_save_review_primary_action(false, true),
            ProjectSaveReviewPrimaryAction::ViewIntegrityProblem
        );
    }

    #[test]
    fn normal_problem_list_keeps_current_issues_and_hides_resolved_history() {
        let diagnostic = ProjectValidationDiagnostic {
            kind: crate::app::project::ProjectDiagnosticKind::UnknownProvince,
            severity: DiagnosticSeverity::Error,
            domain: ProjectValidationDomain::State,
            code: "unknown-province".to_owned(),
            message_key: "unknown-province".to_owned(),
            path: None,
            related_path: None,
            span: None,
            province_id: Some(501),
            state_id: Some(123),
            blocks_save: true,
            message: "Province reference is invalid".to_owned(),
        };
        let view = ValidationProblemsView::default();
        assert!(validation_problem_matches(
            &view,
            ValidationSourceFilter::Unchanged,
            &diagnostic,
        ));
        assert!(!validation_problem_matches(
            &view,
            ValidationSourceFilter::Resolved,
            &diagnostic,
        ));

        let blocking = ValidationProblemsView {
            blocking_only: true,
            ..ValidationProblemsView::default()
        };
        assert!(validation_problem_matches(
            &blocking,
            ValidationSourceFilter::New,
            &diagnostic,
        ));
        assert!(!validation_problem_matches(
            &blocking,
            ValidationSourceFilter::Unchanged,
            &diagnostic,
        ));
    }
}

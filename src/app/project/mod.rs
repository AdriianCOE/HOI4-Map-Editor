mod brush;
mod catalog;
mod diagnostics;
mod edit;
mod indexes;
mod lasso;
mod patch;
mod paths;
mod properties;
mod save;
mod save_plan;
mod state_fill;
mod validation;
mod validation_core;
mod view;

pub use brush::{BrushProvinceClassification, StateBrushMode, sample_segment};
pub use catalog::{
    BuildingCatalogEntry, BuildingScope, CatalogDiagnostic, CatalogDiagnosticSeverity,
    CatalogEntry, DefinitionSource, GameDefinitionCatalog,
};
pub use diagnostics::{DiagnosticSeverity, ProjectDiagnostic, ProjectDiagnosticKind};
pub use edit::{
    ProvinceRemovalPolicy, StateEditError, StateEditSession, StateEditSummary, StateEditTimings,
    StateRemovalPolicy, WorkingStateLifecycle, WorkingStateOrigin,
};
pub use indexes::{StateIndexes, index_state_documents};
pub use lasso::{
    LassoCandidateSet, LassoSelectionMode, ProvinceInclusionMode, StateLassoError, StateLassoPhase,
    classify_state_lasso,
};
pub use patch::{
    PatchDiagnostic, PatchDiagnosticKind, PatchPlanSummary, PatchPlanTimings, PatchSafety,
    PlannedFileCreation, PlannedFileModification, PlannedFileRemoval, ProjectPatchPlan,
    SourceFingerprint, TextPatchOperation, plan_state_patches,
};
pub use paths::{ProjectPathError, ProjectPaths};
pub use properties::{
    EditableProvinceData, EditableStateProperties, NamedIntegerValue, PropertyValidationError,
    ProvinceDataDraft, ProvinceDataValidationError, StatePropertyDraft, format_integer_pt_br,
    parse_grouped_nonnegative_integer,
};
pub use save::{
    BackupManifest, BackupManifestEntry, FileOperationProgress, PersistedFingerprint, RecoveryInfo,
    SaveFailure, SaveFileKind, SaveFileOperationJournal, SaveTransactionJournal,
    SaveTransactionState, StateSaveAuthorization, StateSaveBlockReason, StateSaveCancellation,
    StateSaveConditions, StateSaveEligibility, StateSaveFault, StateSaveOutcome, StateSaveReport,
    authorize_project_save_plan, detect_state_save_recovery, execute_project_save,
    execute_state_save, recover_interrupted_state_save, save_confirmation_text,
    state_save_eligibility,
};
pub use save_plan::{ProjectDirtyState, ProjectSavePlan, SaveDomain};
pub use state_fill::{
    ProvinceAdjacency, StateFillBlockedProvince, StateFillBlockedReason, StateFillMode,
    StateFillPreview, StateFillProvince, StateFillProvinceKind, plan_state_fill,
};
pub use validation::{
    ByteComparisonResult, ByteDifference, CandidateApplicationResult,
    CombinedRoundTripValidationReport, DiagnosticComparison, FileFingerprint, ProjectReloadResult,
    ProjectSemanticComparison, RoundTripCancellation, RoundTripDiagnostic, RoundTripStage,
    RoundTripStatus, RoundTripTimings, RoundTripValidationPolicy, RoundTripValidationReport,
    RoundTripValidator, SemanticDifference, SourceVerificationResult, TemporaryProjectManifest,
    TemporaryWorkspaceSummary, resolve_candidate_path,
};
pub use validation_core::{
    ProjectValidationChange, ProjectValidationDelta, ProjectValidationDiagnostic,
    ProjectValidationDomain, ProjectValidationReport, ProjectValidationSummary,
    ProjectValidationTarget, validate_project, validate_project_against_baseline,
};
pub use view::{
    AMBIGUOUS_PROVINCE_COLOR, MapViewMode, SELECTED_STATE_COLOR, STATE_BOUNDARY_COLOR,
    StateMapRegionData, StateMapViewData, StateSelection, UNASSIGNED_LAND_COLOR,
    UNKNOWN_PROVINCE_COLOR, boundaries_for_state, generate_state_view, generate_state_view_for,
    generate_state_view_region_for, select_state_at, select_state_at_for, selection_overlay,
    selection_overlay_for, state_color,
};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use crate::app::state::{StateDocument, StateLoadBatch, load_state_documents};

#[derive(Debug, Clone)]
pub struct Hoi4Project {
    pub paths: ProjectPaths,
    pub states: Vec<StateDocument>,
    pub states_by_id: BTreeMap<u32, usize>,
    pub state_by_province: HashMap<u32, u32>,
    pub ambiguous_provinces: BTreeMap<u32, Vec<u32>>,
    pub unassigned_land_provinces: BTreeSet<u32>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub load_summary: StateLoadSummary,
}

#[derive(Debug, Clone, Default)]
pub struct StateLoadSummary {
    pub files_found: usize,
    pub files_read: usize,
    pub documents_parsed: usize,
    pub valid_states: usize,
    pub invalid_states: usize,
    pub indexed_states: usize,
    pub assigned_provinces: usize,
    pub duplicate_state_ids: usize,
    pub duplicate_provinces: usize,
    pub missing_province_references: usize,
    pub land_provinces_without_state: usize,
    pub errors: usize,
    pub warnings: usize,
    pub state_loading_ms: u128,
    pub state_texture_generation_ms: u128,
    pub state_boundary_generation_ms: u128,
}

impl Hoi4Project {
    pub fn new(paths: ProjectPaths) -> Self {
        Self {
            paths,
            states: Vec::new(),
            states_by_id: BTreeMap::new(),
            state_by_province: HashMap::new(),
            ambiguous_provinces: BTreeMap::new(),
            unassigned_land_provinces: BTreeSet::new(),
            diagnostics: Vec::new(),
            load_summary: StateLoadSummary::default(),
        }
    }

    pub fn load_states(
        &mut self,
        valid_province_ids: &BTreeSet<u32>,
        land_province_ids: &BTreeSet<u32>,
    ) {
        let started = Instant::now();
        let batch = load_state_documents(&self.paths.states_directory);
        let indexes =
            index_state_documents(&batch.documents, valid_province_ids, land_province_ids);
        let mut diagnostics = batch.diagnostics.clone();
        diagnostics.extend(indexes.diagnostics.iter().cloned());
        let load_summary = StateLoadSummary::from_load(
            &batch,
            &indexes,
            &diagnostics,
            started.elapsed().as_millis(),
        );

        self.states = batch.documents;
        self.states_by_id = indexes.states_by_id;
        self.state_by_province = indexes.state_by_province;
        self.ambiguous_provinces = indexes.ambiguous_provinces;
        self.unassigned_land_provinces = indexes.unassigned_land_provinces;
        self.load_summary = load_summary;
        self.diagnostics = diagnostics;
    }

    pub fn load_summary_message(&self) -> String {
        let max_state_id = self.states_by_id.keys().next_back().copied().unwrap_or(0);
        format!(
            "State project loaded:\n\
       - files found: {}\n\
       - files read: {}\n\
       - documents parsed: {}\n\
       - valid states: {}\n\
       - invalid states: {}\n\
       - indexed states: {}\n\
       - maximum state ID: {}\n\
       - assigned provinces: {}\n\
       - duplicate state IDs: {}\n\
       - ambiguous provinces: {}\n\
       - unassigned land provinces: {}\n\
       - unknown province references: {}\n\
       - errors: {}\n\
       - warnings: {}\n\
       - state loading: {} ms\n\
       - state texture generation: {} ms\n\
       - state boundary generation: {} ms",
            self.load_summary.files_found,
            self.load_summary.files_read,
            self.load_summary.documents_parsed,
            self.load_summary.valid_states,
            self.load_summary.invalid_states,
            self.load_summary.indexed_states,
            max_state_id,
            self.load_summary.assigned_provinces,
            self.load_summary.duplicate_state_ids,
            self.load_summary.duplicate_provinces,
            self.load_summary.land_provinces_without_state,
            self.load_summary.missing_province_references,
            self.load_summary.errors,
            self.load_summary.warnings,
            self.load_summary.state_loading_ms,
            self.load_summary.state_texture_generation_ms,
            self.load_summary.state_boundary_generation_ms
        )
    }

    pub fn state_document(&self, state_id: u32) -> Option<&StateDocument> {
        self.states_by_id
            .get(&state_id)
            .and_then(|&index| self.states.get(index))
    }

    pub fn diagnostic_report(&self) -> String {
        self.diagnostics
            .iter()
            .map(|diagnostic| {
                let severity = match diagnostic.severity {
                    DiagnosticSeverity::Info => "INFO",
                    DiagnosticSeverity::Warning => "WARNING",
                    DiagnosticSeverity::Error => "ERROR",
                };
                let path = diagnostic
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<project>".to_owned());
                format!("{severity} [{path}] {}", diagnostic.message)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl StateLoadSummary {
    fn from_load(
        batch: &StateLoadBatch,
        indexes: &StateIndexes,
        diagnostics: &[ProjectDiagnostic],
        state_loading_ms: u128,
    ) -> Self {
        let count = |kind| {
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == kind)
                .count()
        };

        Self {
            files_found: batch.files_found,
            files_read: batch.files_read,
            documents_parsed: batch.documents.len(),
            valid_states: indexes.indexed_state_count,
            invalid_states: batch
                .documents
                .len()
                .saturating_sub(indexes.indexed_state_count),
            indexed_states: indexes.indexed_state_count,
            assigned_provinces: indexes.indexed_province_count,
            duplicate_state_ids: count(ProjectDiagnosticKind::DuplicateStateId),
            duplicate_provinces: indexes.ambiguous_province_count,
            missing_province_references: count(ProjectDiagnosticKind::UnknownProvince),
            land_provinces_without_state: indexes.land_without_state_count,
            errors: diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
                .count(),
            warnings: diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
                .count(),
            state_loading_ms,
            state_texture_generation_ms: 0,
            state_boundary_generation_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Hoi4Project, ProjectPaths};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4-state-project-load-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("map")).unwrap();
            fs::create_dir_all(root.join("history/states")).unwrap();
            fs::write(root.join("map/provinces.bmp"), []).unwrap();
            fs::write(root.join("map/definition.csv"), []).unwrap();
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_documents_indexes_and_summary_without_writing_state_files() {
        let temp = TempProject::new("summary");
        let state_path = temp.path().join("history/states/1-Test.txt");
        let original =
            "state={id=1 name=Test state_category=city provinces={1 2} history={owner=TAG}}";
        fs::write(&state_path, original).unwrap();
        fs::write(temp.path().join("history/states/invalid.txt"), "state={id=").unwrap();

        let mut project = Hoi4Project::new(ProjectPaths::discover(temp.path()).unwrap());
        project.load_states(&BTreeSet::from([1, 2, 3]), &BTreeSet::from([1, 2, 3]));

        assert_eq!(project.load_summary.files_found, 2);
        assert_eq!(project.load_summary.valid_states, 1);
        assert_eq!(project.load_summary.assigned_provinces, 2);
        assert_eq!(project.load_summary.land_provinces_without_state, 1);
        assert_eq!(project.unassigned_land_provinces, BTreeSet::from([3]));
        assert!(
            project
                .load_summary_message()
                .contains("- indexed states: 1")
        );
        assert_eq!(fs::read_to_string(state_path).unwrap(), original);
    }
}

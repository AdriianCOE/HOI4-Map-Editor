mod brush;
mod catalog;
mod compatibility;
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
pub use compatibility::{
    BitmapCompatibilityMetadata, CompatibilityCode, CompatibilityContext, CompatibilityFinding,
    CompatibilityReport, CompatibilitySeverity, DefinitionCompatibilityMetadata, ProvinceColor,
    RelatedBitmapCompatibilityMetadata, StateCompatibilityMetadata, scan_project,
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
    pub report: StateLoadReport,
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
    pub state_discovery_ms: u128,
    pub state_read_parse_ms: u128,
    pub state_indexing_ms: u128,
    pub state_texture_generation_ms: u128,
    pub state_boundary_generation_ms: u128,
}

#[derive(Debug, Clone, Default)]
pub struct StateLoadReport {
    pub files_seen: usize,
    pub files_read: usize,
    pub states_loaded: usize,
    pub files_failed: Vec<StateLoadFailure>,
    pub duplicate_ids: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLoadFailure {
    pub path: std::path::PathBuf,
    pub stage: StateLoadFailureStage,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLoadFailureStage {
    Discovery,
    Read,
    Parse,
    StateBlock,
    StateId,
    Index,
}

impl StateLoadFailureStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Read => "read",
            Self::Parse => "parse",
            Self::StateBlock => "state block",
            Self::StateId => "state id",
            Self::Index => "index",
        }
    }
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
        let indexing_started = Instant::now();
        let indexes =
            index_state_documents(&batch.documents, valid_province_ids, land_province_ids);
        let indexing_in = indexing_started.elapsed().as_millis();
        let mut diagnostics = batch.diagnostics.clone();
        diagnostics.extend(indexes.diagnostics.iter().cloned());
        let load_summary = StateLoadSummary::from_load(
            &batch,
            &indexes,
            &diagnostics,
            started.elapsed().as_millis(),
            indexing_in,
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
       - State discovery: {} ms\n\
       - State read + parse: {} ms\n\
       - State indexes: {} ms\n\
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
            self.load_summary.state_discovery_ms,
            self.load_summary.state_read_parse_ms,
            self.load_summary.state_indexing_ms,
            self.load_summary.state_texture_generation_ms,
            self.load_summary.state_boundary_generation_ms
        )
    }

    pub fn state_document(&self, state_id: u32) -> Option<&StateDocument> {
        self.states_by_id
            .get(&state_id)
            .and_then(|&index| self.states.get(index))
    }

    pub fn state_load_is_complete(&self) -> bool {
        self.load_summary.report.files_failed.is_empty()
    }

    pub fn state_load_failure_message(&self) -> String {
        let failures = &self.load_summary.report.files_failed;
        let mut message = format!(
            "{} State file(s) could not be loaded safely. Open Validate Project for details.",
            failures.len()
        );
        for failure in failures {
            message.push_str(&format!(
                "\n{}: {}: {}",
                failure.path.display(),
                failure.stage.label(),
                failure.reason
            ));
        }
        message
    }

    pub fn diagnostics_for_province(
        &self,
        province_id: u32,
    ) -> impl Iterator<Item = &ProjectDiagnostic> {
        ProjectDiagnostic::for_province(&self.diagnostics, province_id)
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
        state_indexing_ms: u128,
    ) -> Self {
        let count = |kind| {
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == kind)
                .count()
        };

        Self {
            report: StateLoadReport::from_load(batch, indexes),
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
            state_discovery_ms: batch.discovery_in.as_millis(),
            state_read_parse_ms: batch.read_parse_in.as_millis(),
            state_indexing_ms,
            state_texture_generation_ms: 0,
            state_boundary_generation_ms: 0,
        }
    }
}

impl StateLoadReport {
    fn from_load(batch: &StateLoadBatch, indexes: &StateIndexes) -> Self {
        let indexed_documents = indexes
            .states_by_id
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut files_failed = batch
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .filter_map(|diagnostic| {
                diagnostic.path.clone().map(|path| StateLoadFailure {
                    path,
                    stage: StateLoadFailureStage::Discovery,
                    reason: diagnostic.message.clone(),
                })
            })
            .collect::<Vec<_>>();

        for (document_index, document) in batch.documents.iter().enumerate() {
            let document_diagnostic = document
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
            if indexed_documents.contains(&document_index) && document_diagnostic.is_none() {
                continue;
            }
            let diagnostic = document_diagnostic.or_else(|| {
                indexes.diagnostics.iter().find(|diagnostic| {
                    diagnostic.severity == DiagnosticSeverity::Error
                        && diagnostic.path.as_ref() == Some(&document.path)
                })
            });
            files_failed.push(StateLoadFailure {
                path: document.path.clone(),
                stage: failure_stage(document, diagnostic.map(|diagnostic| diagnostic.kind)),
                reason: diagnostic.map_or_else(
                    || "state was not inserted into the State index".to_owned(),
                    |diagnostic| diagnostic.message.clone(),
                ),
            });
        }
        files_failed.sort_by(|left, right| left.path.cmp(&right.path));
        files_failed.dedup_by(|left, right| left.path == right.path);
        Self {
            files_seen: batch.files_found,
            files_read: batch.files_read,
            states_loaded: indexes.indexed_state_count,
            duplicate_ids: indexes
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == ProjectDiagnosticKind::DuplicateStateId)
                .count(),
            files_failed,
        }
    }
}

fn failure_stage(
    document: &StateDocument,
    kind: Option<ProjectDiagnosticKind>,
) -> StateLoadFailureStage {
    if !document.exact_utf8 && document.original_bytes().is_empty() {
        return StateLoadFailureStage::Read;
    }
    match kind {
        Some(ProjectDiagnosticKind::InvalidStateFile) => StateLoadFailureStage::Read,
        Some(ProjectDiagnosticKind::SyntaxError | ProjectDiagnosticKind::EmptyStateFile) => {
            StateLoadFailureStage::Parse
        }
        Some(
            ProjectDiagnosticKind::MissingStateBlock | ProjectDiagnosticKind::MultipleStateBlocks,
        ) => StateLoadFailureStage::StateBlock,
        Some(
            ProjectDiagnosticKind::MissingStateId
            | ProjectDiagnosticKind::InvalidStateId
            | ProjectDiagnosticKind::ZeroStateId,
        ) => StateLoadFailureStage::StateId,
        _ => StateLoadFailureStage::Index,
    }
}

#[cfg(test)]
mod tests {
    use super::{Hoi4Project, ProjectPaths, StateLoadFailureStage};
    use crate::app::map::{Bundle, ProvinceKind};
    use crate::app::project::StateEditSession;
    use crate::config::Config;
    use crate::util::files::Location;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Cursor;
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

        fn write_map_and_states(
            &self,
            dimensions: [u32; 2],
            definitions: &[(u32, [u8; 3])],
            state_id: u32,
            owner: &str,
        ) {
            let colors = definitions
                .iter()
                .map(|(_, color)| *color)
                .collect::<Vec<_>>();
            let image = RgbImage::from_fn(dimensions[0], dimensions[1], |x, y| {
                Rgb(colors[((y * dimensions[0] + x) as usize) % colors.len()])
            });
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::ImageRgb8(image)
                .write_to(&mut bytes, ImageFormat::Bmp)
                .unwrap();
            fs::write(self.path().join("map/provinces.bmp"), bytes.into_inner()).unwrap();

            let mut csv = String::from("0;0;0;0;land;false;unknown;0\r\n");
            for (id, [red, green, blue]) in definitions {
                csv.push_str(&format!(
                    "{id};{red};{green};{blue};land;false;plains;1\r\n"
                ));
            }
            fs::write(self.path().join("map/definition.csv"), csv).unwrap();
            let provinces = definitions
                .iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            fs::write(
                self.path()
                    .join(format!("history/states/{state_id}-Fixture.txt")),
                format!(
                    "state={{id={state_id} provinces={{ {provinces} }} history={{owner={owner}}}}}"
                ),
            )
            .unwrap();
        }

        fn load(&self) -> (Bundle, Hoi4Project) {
            let paths = ProjectPaths::discover(self.path()).unwrap();
            let bundle = Bundle::load(
                &Location::Directory(paths.map_directory.clone()),
                Config {
                    preserve_ids: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
            let mut project = Hoi4Project::new(paths);
            project.load_states(&province_ids, &province_ids);
            (bundle, project)
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
        assert_eq!(project.load_summary.report.files_seen, 2);
        assert_eq!(project.load_summary.report.states_loaded, 1);
        assert_eq!(project.load_summary.report.files_failed.len(), 1);
        assert_eq!(
            project.load_summary.report.files_failed[0].stage,
            StateLoadFailureStage::Parse
        );
        assert!(
            project.load_summary.report.files_failed[0]
                .path
                .ends_with("invalid.txt")
        );
        assert!(!project.state_load_is_complete());
        assert!(project.state_load_failure_message().contains("invalid.txt"));
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

    #[test]
    fn reports_duplicate_state_file_without_losing_the_first_state() {
        let temp = TempProject::new("duplicate");
        fs::write(
            temp.path().join("history/states/first.txt"),
            "state={id=143 provinces={1}}",
        )
        .unwrap();
        fs::write(
            temp.path().join("history/states/second.txt"),
            "state={id=143 provinces={2}}",
        )
        .unwrap();

        let mut project = Hoi4Project::new(ProjectPaths::discover(temp.path()).unwrap());
        project.load_states(&BTreeSet::from([1, 2]), &BTreeSet::from([1, 2]));

        assert_eq!(project.states_by_id.len(), 1);
        assert_eq!(project.state_by_province.get(&1), Some(&143));
        assert_eq!(project.state_by_province.get(&2), None);
        assert_eq!(project.load_summary.report.duplicate_ids, 1);
        assert_eq!(project.load_summary.report.files_failed.len(), 1);
        assert_eq!(
            project.load_summary.report.files_failed[0].stage,
            StateLoadFailureStage::Index
        );
        assert!(
            project.load_summary.report.files_failed[0]
                .path
                .ends_with("second.txt")
        );
    }

    #[test]
    fn independent_project_loads_replace_dimensions_ids_and_state_sessions() {
        let large = TempProject::new("switch-large");
        large.write_map_and_states(
            [130, 70],
            &[(1, [1, 2, 3]), (50_000, [4, 5, 6])],
            900,
            "AAA",
        );
        let small = TempProject::new("switch-small");
        small.write_map_and_states([3, 2], &[(7, [7, 8, 9]), (9, [10, 11, 12])], 2, "BBB");

        let (large_map, large_project) = large.load();
        let mut large_session = StateEditSession::new(&large_project, &large_map.map);
        let _ = large_session.apply_lasso_selection(&BTreeSet::from([50_000]), Default::default());
        assert_eq!(large_map.map.dimensions(), [130, 70]);
        assert_eq!(large_project.state_by_province.get(&50_000), Some(&900));
        assert!(large_session.selected_provinces().contains(&50_000));

        let (small_map, small_project) = small.load();
        let small_session = StateEditSession::new(&small_project, &small_map.map);
        assert_eq!(small_map.map.dimensions(), [3, 2]);
        assert_eq!(small_map.map.province_ids().collect::<Vec<_>>(), vec![7, 9]);
        assert_eq!(small_project.state_by_province.get(&7), Some(&2));
        assert!(!small_project.state_by_province.contains_key(&50_000));
        assert!(!small_session.selected_provinces().contains(&50_000));
        assert!(!small_session.valid_state_ids().contains(&900));

        // A fresh load back to the first source restores that source only;
        // nothing from the small project is retained in its indexes/session.
        let (reloaded_large_map, reloaded_large_project) = large.load();
        assert_eq!(reloaded_large_map.map.dimensions(), [130, 70]);
        assert_eq!(
            reloaded_large_map.map.province_ids().collect::<Vec<_>>(),
            vec![1, 50_000]
        );
        assert_eq!(
            reloaded_large_project.state_by_province.get(&50_000),
            Some(&900)
        );
        assert!(!reloaded_large_project.state_by_province.contains_key(&7));
    }

    #[test]
    fn failed_candidate_discovery_leaves_the_active_project_data_unchanged() {
        let active = TempProject::new("active-after-failed-open");
        active.write_map_and_states([2, 1], &[(1, [1, 2, 3])], 77, "AAA");
        let (active_map, active_project) = active.load();

        let invalid =
            std::env::temp_dir().join(format!("hoi4-invalid-project-open-{}", std::process::id()));
        let _ = fs::remove_dir_all(&invalid);
        fs::create_dir_all(&invalid).unwrap();
        assert!(ProjectPaths::discover(&invalid).is_err());
        let _ = fs::remove_dir_all(&invalid);

        assert_eq!(active_map.map.dimensions(), [2, 1]);
        assert_eq!(active_project.state_by_province.get(&1), Some(&77));
        assert_eq!(active_project.paths.root, active.path());
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_AZARYA_ROOT"]
    fn azarya_state_loader_smoke() {
        let root = std::env::var_os("HOI4_STATE_EDITOR_AZARYA_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_AZARYA_ROOT to the Azarya mod root");
        let paths = ProjectPaths::discover(&root).expect("discover Azarya project paths");
        let bundle = Bundle::load(
            &Location::Directory(paths.map_directory.clone()),
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .expect("load Azarya province map");
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);

        println!(
            "Azarya States: files={}, loaded={}, failed={}, discovery={} ms, read + parse={} ms, indexes={} ms",
            project.load_summary.report.files_seen,
            project.load_summary.report.states_loaded,
            project.load_summary.report.files_failed.len(),
            project.load_summary.state_discovery_ms,
            project.load_summary.state_read_parse_ms,
            project.load_summary.state_indexing_ms,
        );
        for failure in &project.load_summary.report.files_failed {
            println!(
                "FAILED {} [{}]: {}",
                failure.path.display(),
                failure.stage.label(),
                failure.reason
            );
        }
        for state_id in [143, 200] {
            let state = project
                .state_document(state_id)
                .expect("Azarya State must load");
            let data = state.data.as_ref().expect("loaded State data");
            assert!(data.provinces.iter().all(|province_id| {
                project.state_by_province.get(province_id) == Some(&state_id)
            }));
        }
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_AZARYA_ROOT; prints reusable real-project profile"]
    fn azarya_project_open_component_profile() {
        use std::time::Instant;

        use crate::app::political::PoliticalCountryCatalog;
        use crate::app::project::{GameDefinitionCatalog, generate_state_view};
        use crate::app::resources::ResourceIconResolver;

        let root = std::env::var_os("HOI4_STATE_EDITOR_AZARYA_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_AZARYA_ROOT to the Azarya mod root");
        let base_game_root =
            std::env::var_os("HOI4_STATE_EDITOR_BASE_GAME_ROOT").map(PathBuf::from);
        let total_started = Instant::now();
        let paths_started = Instant::now();
        let paths = ProjectPaths::discover(&root).expect("discover Azarya project paths");
        let paths_in = paths_started.elapsed();

        let bundle_started = Instant::now();
        let bundle = Bundle::load(
            &Location::Directory(paths.map_directory.clone()),
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .expect("load Azarya province map");
        let bundle_in = bundle_started.elapsed();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();

        let states_started = Instant::now();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);
        let states_in = states_started.elapsed();

        let definitions_started = Instant::now();
        let _definitions = GameDefinitionCatalog::build(&project, base_game_root.as_deref());
        let definitions_in = definitions_started.elapsed();

        let state_view_started = Instant::now();
        let _state_view = generate_state_view(&bundle.map, &project);
        let state_view_in = state_view_started.elapsed();

        let political_started = Instant::now();
        let _political = PoliticalCountryCatalog::load(&root, base_game_root.as_deref());
        let political_in = political_started.elapsed();

        let resources_started = Instant::now();
        let _resources = ResourceIconResolver::load(&root, base_game_root.as_deref());
        let resources_in = resources_started.elapsed();

        println!(
            "Azarya release-open component profile:\n\
             paths={} ms; map bundle={} ms; States={} ms; definitions={} ms; State view={} ms; \\
             Political eager work={} ms; Resources eager work={} ms; core total={} ms; with eager presentation={} ms",
            paths_in.as_millis(),
            bundle_in.as_millis(),
            states_in.as_millis(),
            definitions_in.as_millis(),
            state_view_in.as_millis(),
            political_in.as_millis(),
            resources_in.as_millis(),
            total_started
                .elapsed()
                .as_millis()
                .saturating_sub(political_in.as_millis() + resources_in.as_millis()),
            total_started.elapsed().as_millis(),
        );
        assert_eq!(project.load_summary.report.files_seen, 519);
        assert_eq!(project.load_summary.report.states_loaded, 519);
        assert!(project.load_summary.report.files_failed.is_empty());
    }

    #[test]
    #[ignore = "requires local HOI4_STATE_EDITOR_AZARYA_ROOT; prints Resources activation profile"]
    fn azarya_resources_activation_profile() {
        use std::time::Instant;

        use crate::app::political::{PoliticalProvince, TerritoryAnchorIndex};
        use crate::app::resources::{
            ResourceIconResolver, ResourceMapState, prepare_resource_labels,
            prepare_resource_labels_with_index,
        };

        let root = std::env::var_os("HOI4_STATE_EDITOR_AZARYA_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_AZARYA_ROOT to the Azarya mod root");
        let base_game_root =
            std::env::var_os("HOI4_STATE_EDITOR_BASE_GAME_ROOT").map(PathBuf::from);
        let paths = ProjectPaths::discover(&root).expect("discover Azarya project paths");
        let bundle = Bundle::load(
            &Location::Directory(paths.map_directory.clone()),
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .expect("load Azarya province map");
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);

        let states = project
            .states
            .iter()
            .filter_map(|document| document.data.as_ref())
            .filter_map(|state| {
                Some(ResourceMapState {
                    state_id: state.id?,
                    provinces: state.provinces.iter().copied().collect(),
                    resources: state.resources.clone(),
                })
            })
            .collect::<Vec<_>>();
        let provinces = bundle
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
        let mut adjacency_pairs = Vec::new();
        for (boundary, _) in bundle.map.iter_boundaries() {
            let [left, right] = boundary.into_array();
            if let (Some(left), Some(right)) = (
                bundle
                    .map
                    .province_id_for_color(bundle.map.get_color_at(left)),
                bundle
                    .map
                    .province_id_for_color(bundle.map.get_color_at(right)),
            ) {
                adjacency_pairs.push((left, right));
            }
        }

        let index_started = Instant::now();
        let anchors = TerritoryAnchorIndex::new(&provinces, &adjacency_pairs);
        let anchor_index_in = index_started.elapsed();
        let labels_started = Instant::now();
        let labels = prepare_resource_labels_with_index(&states, &anchors);
        let labels_in = labels_started.elapsed();
        let legacy_in = std::env::var_os("HOI4_MAP_EDITOR_PROFILE_LEGACY_RESOURCES").map(|_| {
            let legacy_started = Instant::now();
            let legacy_labels = states
                .iter()
                .flat_map(|state| {
                    prepare_resource_labels(
                        std::slice::from_ref(state),
                        &provinces,
                        &adjacency_pairs,
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(labels, legacy_labels);
            legacy_started.elapsed()
        });

        let resolver_started = Instant::now();
        let mut resolver = ResourceIconResolver::load(&root, base_game_root.as_deref());
        let resolver_in = resolver_started.elapsed();
        let unique_keys = labels
            .iter()
            .flat_map(|label| label.rows.iter().map(|row| row.key.clone()))
            .collect::<BTreeSet<_>>();
        let icon_started = Instant::now();
        for key in &unique_keys {
            let _ = resolver.icon(key);
        }
        let icons_in = icon_started.elapsed();
        let resource_entries = labels.iter().map(|label| label.rows.len()).sum::<usize>();
        println!(
            "Azarya Resources activation profile: states={}, states with resources={}, entries={}, unique keys={}; legacy anchors={}; shared anchor index={} ms; labels={} ms; resolver={} ms; unique icon decode={} ms",
            states.len(),
            labels.len(),
            resource_entries,
            unique_keys.len(),
            legacy_in.map_or_else(
                || "not requested".to_owned(),
                |elapsed| format!("{} ms", elapsed.as_millis())
            ),
            anchor_index_in.as_millis(),
            labels_in.as_millis(),
            resolver_in.as_millis(),
            icons_in.as_millis(),
        );
        assert_eq!(project.load_summary.report.states_loaded, 519);
        assert!(project.load_summary.report.files_failed.is_empty());
    }
}

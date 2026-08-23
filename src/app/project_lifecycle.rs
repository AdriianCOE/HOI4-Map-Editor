//! Project-open lifecycle requests, isolated candidate discovery, and UI state.
//!
//! This module deliberately does not load `Canvas` or mutate `App`. It owns the
//! synchronous native-dialog flow and identifies the kind of candidate that the
//! App must load. `Canvas` remains the fully loaded candidate and the owner of
//! live map/session activation.

use std::path::{Path, PathBuf};

use crate::app::project::{
    CompatibilityCode, CompatibilityFinding, Hoi4Project, ProjectGeneration, ProjectPathError,
    ProjectPaths, scan_project,
};
use crate::config::{ConfigIssue, ProjectConfig};
use crate::error::Error;
use crate::util::files::Location;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ProjectLifecycleState {
    #[default]
    Idle,
    Active {
        generation: ProjectGeneration,
    },
    ChoosingProject {
        archive: bool,
        active_generation: Option<ProjectGeneration>,
    },
    ChoosingBaseGame {
        active_generation: Option<ProjectGeneration>,
    },
    PreparingCandidate {
        active_generation: Option<ProjectGeneration>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum ProjectLifecycleEffect {
    RequestProjectDialog { archive: bool },
    RequestBaseGameDialog,
    PrepareCandidate(Location),
    ApplyBaseGameRoot(PathBuf),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectStartupEffect {
    OpenPath(PathBuf),
    ShowError(&'static str),
    ShowWelcome,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectLifecycleController {
    state: ProjectLifecycleState,
}

impl ProjectLifecycleController {
    pub(crate) fn startup_effect(
        command_line_path: Option<PathBuf>,
        open_last_project: bool,
        last_project: Option<PathBuf>,
    ) -> ProjectStartupEffect {
        if let Some(path) = command_line_path {
            ProjectStartupEffect::OpenPath(path)
        } else if open_last_project {
            match last_project {
                Some(path) if path.exists() => ProjectStartupEffect::OpenPath(path),
                Some(_) => ProjectStartupEffect::ShowError(
                    "The last project no longer exists. Open another HOI4 mod to replace it.",
                ),
                None => ProjectStartupEffect::ShowWelcome,
            }
        } else {
            ProjectStartupEffect::ShowWelcome
        }
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> ProjectLifecycleState {
        self.state
    }

    pub(crate) fn request_project_dialog(&mut self, archive: bool) -> ProjectLifecycleEffect {
        self.state = ProjectLifecycleState::ChoosingProject {
            archive,
            active_generation: self.active_generation(),
        };
        ProjectLifecycleEffect::RequestProjectDialog { archive }
    }

    pub(crate) fn project_dialog_result(
        &mut self,
        location: Option<Location>,
    ) -> ProjectLifecycleEffect {
        let active_generation = match self.state {
            ProjectLifecycleState::ChoosingProject {
                active_generation, ..
            } => active_generation,
            _ => self.active_generation(),
        };
        match location {
            Some(location) => {
                self.state = ProjectLifecycleState::PreparingCandidate { active_generation };
                ProjectLifecycleEffect::PrepareCandidate(location)
            }
            None => {
                self.restore_active(active_generation);
                ProjectLifecycleEffect::None
            }
        }
    }

    pub(crate) fn request_open_location(&mut self, location: Location) -> ProjectLifecycleEffect {
        self.state = ProjectLifecycleState::PreparingCandidate {
            active_generation: self.active_generation(),
        };
        ProjectLifecycleEffect::PrepareCandidate(location)
    }

    pub(crate) fn request_base_game_dialog(&mut self) -> ProjectLifecycleEffect {
        self.state = ProjectLifecycleState::ChoosingBaseGame {
            active_generation: self.active_generation(),
        };
        ProjectLifecycleEffect::RequestBaseGameDialog
    }

    pub(crate) fn base_game_dialog_result(
        &mut self,
        root: Option<PathBuf>,
    ) -> ProjectLifecycleEffect {
        let active_generation = match self.state {
            ProjectLifecycleState::ChoosingBaseGame { active_generation } => active_generation,
            _ => self.active_generation(),
        };
        self.restore_active(active_generation);
        root.map_or(
            ProjectLifecycleEffect::None,
            ProjectLifecycleEffect::ApplyBaseGameRoot,
        )
    }

    pub(crate) fn candidate_open_failed(&mut self) {
        let active_generation = match self.state {
            ProjectLifecycleState::PreparingCandidate { active_generation } => active_generation,
            _ => self.active_generation(),
        };
        self.restore_active(active_generation);
    }

    pub(crate) fn candidate_activated(&mut self, generation: ProjectGeneration) {
        self.state = ProjectLifecycleState::Active { generation };
    }

    fn active_generation(&self) -> Option<ProjectGeneration> {
        match self.state {
            ProjectLifecycleState::Active { generation } => Some(generation),
            ProjectLifecycleState::ChoosingProject {
                active_generation, ..
            }
            | ProjectLifecycleState::ChoosingBaseGame { active_generation }
            | ProjectLifecycleState::PreparingCandidate { active_generation } => active_generation,
            ProjectLifecycleState::Idle => None,
        }
    }

    fn restore_active(&mut self, active_generation: Option<ProjectGeneration>) {
        self.state = active_generation.map_or(ProjectLifecycleState::Idle, |generation| {
            ProjectLifecycleState::Active { generation }
        });
    }
}

/// A discovered project root, kept separate from the fully loaded Canvas
/// candidate. App selects the corresponding existing Canvas loader; it does
/// not activate any live state until that loader succeeds.
#[derive(Debug)]
pub(crate) enum ProjectOpenCandidate {
    Project {
        root: PathBuf,
        project: Box<Hoi4Project>,
        project_config_issue: Option<ConfigIssue>,
    },
    ProvinceOnly {
        root: PathBuf,
        location: Location,
    },
    Legacy {
        location: Location,
    },
}

impl ProjectOpenCandidate {
    pub(crate) fn discover(location: Location) -> Result<Self, Error> {
        let Location::Directory(root) = location else {
            return Ok(Self::Legacy { location });
        };
        match ProjectPaths::discover(&root) {
            Ok(paths) => {
                let root = paths.root.clone();
                let project_config_issue = ProjectConfig::load(&root)
                    .ok()
                    .and_then(|loaded| loaded.issue);
                Ok(Self::Project {
                    root,
                    project: Box::new(Hoi4Project::new(paths)),
                    project_config_issue,
                })
            }
            Err(
                ProjectPathError::MissingHistoryDirectory(_)
                | ProjectPathError::MissingStatesDirectory(_),
            ) if root.join("map/provinces.bmp").is_file()
                && root.join("map/definition.csv").is_file() =>
            {
                Ok(Self::ProvinceOnly {
                    location: Location::Directory(root.join("map")),
                    root,
                })
            }
            Err(error) if ProjectPaths::is_project_root_candidate(&root) => {
                Err(compatibility_open_error(&root, error.into()))
            }
            Err(_) => Ok(Self::Legacy {
                location: Location::Directory(root),
            }),
        }
    }

    pub(crate) fn project_success_message(
        root: &Path,
        capabilities: &str,
        project_config_issue: Option<ConfigIssue>,
    ) -> String {
        let mut message = format!("Loaded HOI4 mod from {}\n{capabilities}", root.display());
        if let Some(issue) = project_config_issue {
            message.push_str(&format!(
                "\n{}\n{issue}",
                crate::localization::tr("config.invalid_project")
            ));
        }
        message
    }

    pub(crate) fn province_only_success_message(root: &Path) -> String {
        format!(
            "Loaded Province-only project from {}. State editing is unavailable because history/states is missing.",
            root.display()
        )
    }

    pub(crate) fn legacy_success_message(location: &Location) -> String {
        format!("Loaded legacy editable map from {location}")
    }

    pub(crate) fn load_error(root: &Path, error: Error) -> Error {
        compatibility_open_error(root, error)
    }
}

fn compatibility_open_error(root: &Path, error: Error) -> Error {
    let report = scan_project(root.to_owned());
    let Some(finding) = report.primary_blocker() else {
        return error;
    };
    let dimensions = report
        .bitmap
        .as_ref()
        .map(|bitmap| {
            format!(
                "; map dimensions: {}x{}",
                bitmap.dimensions[0], bitmap.dimensions[1]
            )
        })
        .unwrap_or_default();
    Error::from(format!(
        "{error}\nCompatibility scan: {}{dimensions} [code: {}]",
        compatibility_finding_summary(finding),
        finding.code.identifier(),
    ))
}

fn compatibility_finding_summary(finding: &CompatibilityFinding) -> &'static str {
    match finding.code {
        CompatibilityCode::SparseProvinceIds => {
            "sparse Province IDs are supported and will be preserved"
        }
        CompatibilityCode::ProvinceBitmapUnreadable => "provinces.bmp could not be read as a BMP",
        CompatibilityCode::DefinitionMalformed => "definition.csv could not be interpreted",
        CompatibilityCode::DefinitionEmpty => "definition.csv contains no province records",
        CompatibilityCode::DuplicateProvinceId => "definition.csv contains duplicate Province IDs",
        CompatibilityCode::DuplicateProvinceColor => {
            "definition.csv contains duplicate Province colors"
        }
        CompatibilityCode::BitmapColorMissingDefinition => {
            "provinces.bmp uses colors missing from definition.csv"
        }
        CompatibilityCode::StateReferencesMissingProvince => {
            "State files reference Province IDs missing from definition.csv"
        }
        CompatibilityCode::MapDirectoryMissing => "the map directory is missing",
        CompatibilityCode::ProvinceBitmapMissing => "map/provinces.bmp is missing",
        CompatibilityCode::DefinitionMissing => "map/definition.csv is missing",
        CompatibilityCode::InvalidProvinceIdRange => {
            "definition.csv contains Province ID zero, which the editor cannot interpret"
        }
        CompatibilityCode::DefinitionColorUnused
        | CompatibilityCode::StatesDirectoryMissing
        | CompatibilityCode::RelatedBitmapUnreadable
        | CompatibilityCode::RelatedBitmapDimensionsMismatch => "a compatibility issue was found",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "hoi4-state-editor-project-lifecycle-{}-{name}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("map")).unwrap();
            Self(root)
        }

        fn write_core_map(&self) {
            fs::write(self.0.join("map/provinces.bmp"), []).unwrap();
            fs::write(self.0.join("map/definition.csv"), []).unwrap();
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn canceling_a_project_dialog_preserves_the_active_generation() {
        let mut controller = ProjectLifecycleController::default();
        controller.candidate_activated(ProjectGeneration(4));
        assert!(matches!(
            controller.request_project_dialog(false),
            ProjectLifecycleEffect::RequestProjectDialog { archive: false }
        ));
        assert!(matches!(
            controller.project_dialog_result(None),
            ProjectLifecycleEffect::None
        ));
        assert_eq!(
            controller.state(),
            ProjectLifecycleState::Active {
                generation: ProjectGeneration(4)
            }
        );
    }

    #[test]
    fn failed_candidate_restores_the_previous_active_generation() {
        let mut controller = ProjectLifecycleController::default();
        controller.candidate_activated(ProjectGeneration(6));
        assert!(matches!(
            controller.request_open_location(Location::Directory(PathBuf::from("project-b"))),
            ProjectLifecycleEffect::PrepareCandidate(_)
        ));
        controller.candidate_open_failed();
        assert_eq!(
            controller.state(),
            ProjectLifecycleState::Active {
                generation: ProjectGeneration(6)
            }
        );
    }

    #[test]
    fn accepted_candidate_activates_its_new_generation() {
        let mut controller = ProjectLifecycleController::default();
        let _ = controller.request_open_location(Location::Directory(PathBuf::from("project-b")));
        controller.candidate_activated(ProjectGeneration(7));
        assert_eq!(
            controller.state(),
            ProjectLifecycleState::Active {
                generation: ProjectGeneration(7)
            }
        );
    }

    #[test]
    fn canceling_a_base_game_dialog_preserves_the_active_generation() {
        let mut controller = ProjectLifecycleController::default();
        controller.candidate_activated(ProjectGeneration(8));
        assert!(matches!(
            controller.request_base_game_dialog(),
            ProjectLifecycleEffect::RequestBaseGameDialog
        ));
        assert!(matches!(
            controller.base_game_dialog_result(None),
            ProjectLifecycleEffect::None
        ));
        assert_eq!(
            controller.state(),
            ProjectLifecycleState::Active {
                generation: ProjectGeneration(8)
            }
        );
    }

    #[test]
    fn selected_project_dialog_requests_candidate_preparation() {
        let mut controller = ProjectLifecycleController::default();
        let _ = controller.request_project_dialog(false);
        assert!(matches!(
            controller.project_dialog_result(Some(Location::Directory(PathBuf::from("project")))),
            ProjectLifecycleEffect::PrepareCandidate(Location::Directory(_))
        ));
    }

    #[test]
    fn disabled_last_project_policy_keeps_the_initial_session_empty() {
        assert_eq!(
            ProjectLifecycleController::startup_effect(
                None,
                false,
                Some(PathBuf::from("stale-project")),
            ),
            ProjectStartupEffect::ShowWelcome
        );
    }

    #[test]
    fn discovery_keeps_full_projects_and_province_only_projects_distinct() {
        let full = TempProject::new("full");
        full.write_core_map();
        fs::create_dir_all(full.0.join("history/states")).unwrap();
        assert!(matches!(
            ProjectOpenCandidate::discover(Location::Directory(full.0.clone())),
            Ok(ProjectOpenCandidate::Project { .. })
        ));

        let province_only = TempProject::new("province-only");
        province_only.write_core_map();
        assert!(matches!(
            ProjectOpenCandidate::discover(Location::Directory(province_only.0.clone())),
            Ok(ProjectOpenCandidate::ProvinceOnly { .. })
        ));
    }

    #[test]
    fn invalid_project_roots_do_not_fall_back_to_legacy_loading() {
        let invalid = TempProject::new("invalid");
        fs::write(invalid.0.join("map/provinces.bmp"), []).unwrap();
        assert!(ProjectOpenCandidate::discover(Location::Directory(invalid.0.clone())).is_err());
    }
}

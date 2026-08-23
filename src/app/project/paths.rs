use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{ProjectSources, SourceGeneration, SourceResolutionError};

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub root: PathBuf,
    pub map_directory: PathBuf,
    pub provinces_bmp: PathBuf,
    pub definition_csv: PathBuf,
    pub adjacencies_csv: Option<PathBuf>,
    pub rivers_bmp: Option<PathBuf>,
    pub continent_txt: Option<PathBuf>,
    pub history_directory: PathBuf,
    pub states_directory: PathBuf,
    pub sources: ProjectSources,
}

impl ProjectPaths {
    pub fn discover(root: impl Into<PathBuf>) -> Result<Self, ProjectPathError> {
        let root = root.into();
        require_directory(
            &root,
            ProjectPathError::RootNotFound,
            ProjectPathError::RootIsNotDirectory,
        )?;

        let sources = ProjectSources::discover(&root, None, SourceGeneration::default()).map_err(
            |error| match &error {
                SourceResolutionError::MissingCurrentProjectDirectory {
                    logical_path,
                    physical_path,
                } if logical_path == Path::new("map") => {
                    ProjectPathError::MissingMapDirectory(physical_path.clone())
                }
                SourceResolutionError::MissingCurrentProjectFile {
                    logical_path,
                    physical_path,
                } if logical_path == Path::new("map/provinces.bmp") => {
                    ProjectPathError::MissingProvincesBitmap(physical_path.clone())
                }
                SourceResolutionError::MissingCurrentProjectFile {
                    logical_path,
                    physical_path,
                } if logical_path == Path::new("map/definition.csv") => {
                    ProjectPathError::MissingDefinitionTable(physical_path.clone())
                }
                _ => ProjectPathError::SourceResolution(error),
            },
        )?;
        let manifest = sources.manifest();
        let map_directory = manifest.map_directory.clone();
        let provinces_bmp = manifest
            .map_files
            .provinces_bmp
            .filesystem_path()
            .expect("core project map source is a filesystem path")
            .to_owned();
        let definition_csv = manifest
            .map_files
            .definition_csv
            .filesystem_path()
            .expect("core project map source is a filesystem path")
            .to_owned();
        let adjacencies_csv = manifest.map_files.adjacencies_csv.as_ref().map(|source| {
            source
                .filesystem_path()
                .expect("core project map source is a filesystem path")
                .to_owned()
        });
        let rivers_bmp = manifest.map_files.rivers_bmp.as_ref().map(|source| {
            source
                .filesystem_path()
                .expect("core project map source is a filesystem path")
                .to_owned()
        });
        let continent_txt = manifest.map_files.continent_txt.as_ref().map(|source| {
            source
                .filesystem_path()
                .expect("core project map source is a filesystem path")
                .to_owned()
        });

        let history_directory = root.join("history");
        require_directory(
            &history_directory,
            ProjectPathError::MissingHistoryDirectory,
            ProjectPathError::MissingHistoryDirectory,
        )?;

        let states_directory = history_directory.join("states");
        require_directory(
            &states_directory,
            ProjectPathError::MissingStatesDirectory,
            ProjectPathError::MissingStatesDirectory,
        )?;

        Ok(Self {
            adjacencies_csv,
            rivers_bmp,
            continent_txt,
            root,
            map_directory,
            provinces_bmp,
            definition_csv,
            history_directory,
            states_directory,
            sources,
        })
    }

    pub fn uses_conventional_editable_map_layout(&self) -> bool {
        self.provinces_bmp == self.map_directory.join("provinces.bmp")
            && self.definition_csv == self.map_directory.join("definition.csv")
            && self
                .adjacencies_csv
                .as_ref()
                .is_none_or(|path| path == &self.map_directory.join("adjacencies.csv"))
    }

    pub fn bind_project_generation(&mut self, generation: u64) {
        self.sources
            .rebind_generation(SourceGeneration::new(generation));
    }

    pub fn set_validated_base_game_root(&mut self, root: Option<PathBuf>) {
        self.sources.set_validated_base_game_root(root);
    }

    pub fn is_project_root_candidate(root: &Path) -> bool {
        root.join("map").exists() || root.join("history").exists()
    }
}

fn require_directory(
    path: &Path,
    missing: fn(PathBuf) -> ProjectPathError,
    not_directory: fn(PathBuf) -> ProjectPathError,
) -> Result<(), ProjectPathError> {
    match path.metadata() {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(not_directory(path.to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(missing(path.to_owned())),
        Err(source) => Err(ProjectPathError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

#[derive(Debug, Error)]
pub enum ProjectPathError {
    #[error("mod root was not found: {}", .0.display())]
    RootNotFound(PathBuf),
    #[error("mod root is not a directory: {}", .0.display())]
    RootIsNotDirectory(PathBuf),
    #[error("map directory is missing: {}", .0.display())]
    MissingMapDirectory(PathBuf),
    #[error("province bitmap is missing: {}", .0.display())]
    MissingProvincesBitmap(PathBuf),
    #[error("definition table is missing: {}", .0.display())]
    MissingDefinitionTable(PathBuf),
    #[error("project source resolution failed: {0}")]
    SourceResolution(SourceResolutionError),
    #[error("history directory is missing: {}", .0.display())]
    MissingHistoryDirectory(PathBuf),
    #[error("states directory is missing: {}", .0.display())]
    MissingStatesDirectory(PathBuf),
    #[error("failed to inspect {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{ProjectPathError, ProjectPaths};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("hoi4-state-editor-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn valid(name: &str) -> Self {
            let project = Self::new(name);
            fs::create_dir(project.0.join("map")).unwrap();
            fs::create_dir_all(project.0.join("history/states")).unwrap();
            fs::write(project.0.join("map/provinces.bmp"), []).unwrap();
            fs::write(project.0.join("map/definition.csv"), []).unwrap();
            project
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
    fn discovers_valid_project_root() {
        let project = TempProject::valid("valid");
        let paths = ProjectPaths::discover(project.path()).unwrap();
        assert_eq!(paths.map_directory, project.path().join("map"));
        assert_eq!(
            paths.states_directory,
            project.path().join("history/states")
        );
    }

    #[test]
    fn reports_missing_map_directory_without_panicking() {
        let project = TempProject::new("missing-map");
        assert!(matches!(
            ProjectPaths::discover(project.path()),
            Err(ProjectPathError::MissingMapDirectory(_))
        ));
    }

    #[test]
    fn reports_missing_provinces_bitmap() {
        let project = TempProject::new("missing-provinces");
        fs::create_dir(project.path().join("map")).unwrap();
        assert!(matches!(
            ProjectPaths::discover(project.path()),
            Err(ProjectPathError::MissingProvincesBitmap(_))
        ));
    }

    #[test]
    fn reports_missing_definition_table() {
        let project = TempProject::new("missing-definition");
        fs::create_dir(project.path().join("map")).unwrap();
        fs::write(project.path().join("map/provinces.bmp"), []).unwrap();
        assert!(matches!(
            ProjectPaths::discover(project.path()),
            Err(ProjectPathError::MissingDefinitionTable(_))
        ));
    }

    #[test]
    fn reports_missing_states_directory() {
        let project = TempProject::new("missing-states");
        fs::create_dir(project.path().join("map")).unwrap();
        fs::write(project.path().join("map/provinces.bmp"), []).unwrap();
        fs::write(project.path().join("map/definition.csv"), []).unwrap();
        fs::create_dir(project.path().join("history")).unwrap();
        assert!(matches!(
            ProjectPaths::discover(project.path()),
            Err(ProjectPathError::MissingStatesDirectory(_))
        ));
    }

    #[test]
    fn supports_spaces_and_unicode() {
        let project = TempProject::valid("mod com espaços-Ázarya");
        assert!(ProjectPaths::discover(project.path()).is_ok());
    }

    #[test]
    fn failed_default_map_candidate_does_not_mutate_active_project_paths() {
        let active = TempProject::valid("active-default-map-isolation");
        let active_paths = ProjectPaths::discover(active.path()).unwrap();
        let invalid = TempProject::valid("invalid-default-map-isolation");
        fs::write(
            invalid.path().join("map/default.map"),
            "definitions = \"definition.csv\"\nprovinces = \"missing-custom.bmp\"\n",
        )
        .unwrap();

        assert!(matches!(
            ProjectPaths::discover(invalid.path()),
            Err(ProjectPathError::SourceResolution(_))
        ));
        assert_eq!(active_paths.root, active.path());
        assert_eq!(
            active_paths.provinces_bmp,
            active.path().join("map/provinces.bmp")
        );
    }
}

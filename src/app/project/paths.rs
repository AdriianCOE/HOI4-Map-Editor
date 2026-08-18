use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub root: PathBuf,
    pub map_directory: PathBuf,
    pub provinces_bmp: PathBuf,
    pub definition_csv: PathBuf,
    pub adjacencies_csv: Option<PathBuf>,
    pub rivers_bmp: Option<PathBuf>,
    pub history_directory: PathBuf,
    pub states_directory: PathBuf,
}

impl ProjectPaths {
    pub fn discover(root: impl Into<PathBuf>) -> Result<Self, ProjectPathError> {
        let root = root.into();
        require_directory(
            &root,
            ProjectPathError::RootNotFound,
            ProjectPathError::RootIsNotDirectory,
        )?;

        let map_directory = root.join("map");
        require_directory(
            &map_directory,
            ProjectPathError::MissingMapDirectory,
            ProjectPathError::MissingMapDirectory,
        )?;

        let provinces_bmp = map_directory.join("provinces.bmp");
        require_file(&provinces_bmp, ProjectPathError::MissingProvincesBitmap)?;

        let definition_csv = map_directory.join("definition.csv");
        require_file(&definition_csv, ProjectPathError::MissingDefinitionTable)?;

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

        let optional_file = |path: PathBuf| path.is_file().then_some(path);

        Ok(Self {
            adjacencies_csv: optional_file(map_directory.join("adjacencies.csv")),
            rivers_bmp: optional_file(map_directory.join("rivers.bmp")),
            root,
            map_directory,
            provinces_bmp,
            definition_csv,
            history_directory,
            states_directory,
        })
    }

    pub fn is_project_root_candidate(root: &Path) -> bool {
        root.join("map").exists() || root.join("history").exists()
    }
}

fn require_file(
    path: &Path,
    missing: fn(PathBuf) -> ProjectPathError,
) -> Result<(), ProjectPathError> {
    match path.metadata() {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(missing(path.to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(missing(path.to_owned())),
        Err(source) => Err(ProjectPathError::Io {
            path: path.to_owned(),
            source,
        }),
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
}

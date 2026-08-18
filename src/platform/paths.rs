use std::env;
use std::path::PathBuf;

use thiserror::Error;

/// The platform whose application directories should be resolved.
///
/// Keeping this explicit makes the resolution rules testable without changing
/// the environment of the process running the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
    Other,
}

impl Platform {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOS
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// The environment inputs relevant to application-owned filesystem paths.
///
/// Production code obtains this from the process once. Tests can construct it
/// directly, avoiding dependencies on a developer machine's configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathEnvironment {
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub temp_dir: Option<PathBuf>,
}

impl PathEnvironment {
    pub fn from_process() -> Self {
        Self {
            app_data: environment_path("APPDATA"),
            local_app_data: environment_path("LOCALAPPDATA"),
            xdg_config_home: environment_path("XDG_CONFIG_HOME"),
            xdg_state_home: environment_path("XDG_STATE_HOME"),
            home: environment_path("HOME"),
            temp_dir: Some(env::temp_dir()),
        }
    }
}

fn environment_path(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PathError {
    #[error("application path is unavailable: {variable} is not set")]
    MissingEnvironment { variable: &'static str },
}

/// Resolves paths owned by HOI4 Map Editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    platform: Platform,
    environment: PathEnvironment,
}

impl AppPaths {
    pub const APPLICATION_DIRECTORY: &'static str = "HOI4MapEditor";

    pub fn from_process() -> Self {
        Self::new(Platform::current(), PathEnvironment::from_process())
    }

    pub fn new(platform: Platform, environment: PathEnvironment) -> Self {
        Self {
            platform,
            environment,
        }
    }

    /// Directory for persistent global configuration.
    pub fn config_dir(&self) -> Result<PathBuf, PathError> {
        match self.platform {
            Platform::Windows => {
                self.environment
                    .app_data
                    .clone()
                    .ok_or(PathError::MissingEnvironment {
                        variable: "APPDATA",
                    })
            }
            Platform::Linux => match &self.environment.xdg_config_home {
                Some(path) => Ok(path.clone()),
                None => self
                    .environment
                    .home
                    .clone()
                    .map(|home| home.join(".config"))
                    .ok_or(PathError::MissingEnvironment {
                        variable: "XDG_CONFIG_HOME or HOME",
                    }),
            },
            Platform::MacOS | Platform::Other => Err(PathError::MissingEnvironment {
                variable: "a supported platform configuration directory",
            }),
        }
        .map(|root| root.join(Self::APPLICATION_DIRECTORY))
    }

    /// Directory for crash and diagnostic logs.
    pub fn log_dir(&self) -> Result<PathBuf, PathError> {
        let root = match self.platform {
            Platform::Windows => self
                .environment
                .local_app_data
                .clone()
                // Preserve the historical fallback when LOCALAPPDATA is absent.
                .or_else(|| self.environment.temp_dir.clone())
                .ok_or(PathError::MissingEnvironment {
                    variable: "LOCALAPPDATA or a temporary directory",
                })?,
            Platform::Linux => match &self.environment.xdg_state_home {
                Some(path) => path.clone(),
                None => self
                    .environment
                    .home
                    .clone()
                    .map(|home| home.join(".local").join("state"))
                    .ok_or(PathError::MissingEnvironment {
                        variable: "XDG_STATE_HOME or HOME",
                    })?,
            },
            Platform::MacOS | Platform::Other => {
                self.environment
                    .temp_dir
                    .clone()
                    .ok_or(PathError::MissingEnvironment {
                        variable: "a temporary directory",
                    })?
            }
        };
        Ok(root.join(Self::APPLICATION_DIRECTORY).join("logs"))
    }

    /// Directory for disposable files created by the application.
    pub fn temporary_dir(&self) -> Result<PathBuf, PathError> {
        self.environment
            .temp_dir
            .clone()
            .map(|root| root.join(Self::APPLICATION_DIRECTORY))
            .ok_or(PathError::MissingEnvironment {
                variable: "a temporary directory",
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppPaths, PathEnvironment, PathError, Platform};
    use std::path::PathBuf;

    fn environment() -> PathEnvironment {
        PathEnvironment {
            temp_dir: Some(PathBuf::from("C:/Temp")),
            ..PathEnvironment::default()
        }
    }

    #[test]
    fn windows_configuration_uses_appdata_and_preserves_directory_name() {
        let paths = AppPaths::new(
            Platform::Windows,
            PathEnvironment {
                app_data: Some(PathBuf::from("C:/Users/Editor/AppData/Roaming")),
                ..environment()
            },
        );

        assert_eq!(
            paths.config_dir().unwrap(),
            PathBuf::from("C:/Users/Editor/AppData/Roaming/HOI4MapEditor")
        );
    }

    #[test]
    fn windows_logs_use_localappdata() {
        let paths = AppPaths::new(
            Platform::Windows,
            PathEnvironment {
                local_app_data: Some(PathBuf::from("C:/Users/Editor/AppData/Local")),
                ..environment()
            },
        );

        assert_eq!(
            paths.log_dir().unwrap(),
            PathBuf::from("C:/Users/Editor/AppData/Local/HOI4MapEditor/logs")
        );
    }

    #[test]
    fn linux_configuration_uses_xdg_config_home() {
        let paths = AppPaths::new(
            Platform::Linux,
            PathEnvironment {
                xdg_config_home: Some(PathBuf::from("/var/config")),
                ..environment()
            },
        );

        assert_eq!(
            paths.config_dir().unwrap(),
            PathBuf::from("/var/config/HOI4MapEditor")
        );
    }

    #[test]
    fn linux_configuration_falls_back_to_home() {
        let paths = AppPaths::new(
            Platform::Linux,
            PathEnvironment {
                home: Some(PathBuf::from("/home/editor")),
                ..environment()
            },
        );

        assert_eq!(
            paths.config_dir().unwrap(),
            PathBuf::from("/home/editor/.config/HOI4MapEditor")
        );
    }

    #[test]
    fn linux_logs_use_xdg_state_or_home_state_fallback() {
        let xdg_paths = AppPaths::new(
            Platform::Linux,
            PathEnvironment {
                xdg_state_home: Some(PathBuf::from("/var/state")),
                ..environment()
            },
        );
        assert_eq!(
            xdg_paths.log_dir().unwrap(),
            PathBuf::from("/var/state/HOI4MapEditor/logs")
        );

        let fallback_paths = AppPaths::new(
            Platform::Linux,
            PathEnvironment {
                home: Some(PathBuf::from("/home/editor")),
                ..environment()
            },
        );
        assert_eq!(
            fallback_paths.log_dir().unwrap(),
            PathBuf::from("/home/editor/.local/state/HOI4MapEditor/logs")
        );
    }

    #[test]
    fn missing_required_environment_is_a_typed_error() {
        let paths = AppPaths::new(Platform::Linux, environment());
        assert_eq!(
            paths.config_dir(),
            Err(PathError::MissingEnvironment {
                variable: "XDG_CONFIG_HOME or HOME",
            })
        );
        assert_eq!(
            paths.log_dir(),
            Err(PathError::MissingEnvironment {
                variable: "XDG_STATE_HOME or HOME",
            })
        );
    }

    #[test]
    fn windows_logs_keep_the_historical_temporary_directory_fallback() {
        let paths = AppPaths::new(Platform::Windows, environment());
        assert_eq!(
            paths.log_dir().unwrap(),
            PathBuf::from("C:/Temp/HOI4MapEditor/logs")
        );
    }

    #[test]
    fn temporary_files_use_an_application_owned_directory() {
        let paths = AppPaths::new(Platform::Linux, environment());
        assert_eq!(
            paths.temporary_dir().unwrap(),
            PathBuf::from("C:/Temp/HOI4MapEditor")
        );
    }
}

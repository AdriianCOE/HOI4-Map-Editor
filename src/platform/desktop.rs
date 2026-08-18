use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

use thiserror::Error;

use super::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopCommand {
    pub program: &'static str,
    pub arguments: Vec<OsString>,
}

impl DesktopCommand {
    fn for_path(program: &'static str, path: &Path) -> Self {
        Self {
            program,
            arguments: vec![path.as_os_str().to_owned()],
        }
    }
}

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("opening files is unsupported on this platform")]
    Unsupported,
    #[error("failed to start desktop opener '{program}': {source}")]
    Start {
        program: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("desktop opener '{program}' exited unsuccessfully: {status}")]
    Failed {
        program: &'static str,
        status: ExitStatus,
    },
}

pub fn open_file(path: &Path) -> Result<(), DesktopError> {
    execute(open_file_command(Platform::current(), path)?)
}

pub fn open_folder(path: &Path) -> Result<(), DesktopError> {
    execute(open_folder_command(Platform::current(), path)?)
}

pub fn open_font_license(path: &Path) -> Result<(), DesktopError> {
    execute(font_license_command(Platform::current(), path)?)
}

pub fn open_file_command(platform: Platform, path: &Path) -> Result<DesktopCommand, DesktopError> {
    command_for_path(platform, path)
}

pub fn open_folder_command(
    platform: Platform,
    path: &Path,
) -> Result<DesktopCommand, DesktopError> {
    command_for_path(platform, path)
}

pub fn font_license_command(
    platform: Platform,
    path: &Path,
) -> Result<DesktopCommand, DesktopError> {
    match platform {
        // Keep the existing Windows license viewer behavior.
        Platform::Windows => Ok(DesktopCommand::for_path("notepad", path)),
        Platform::MacOS | Platform::Linux => command_for_path(platform, path),
        Platform::Other => Err(DesktopError::Unsupported),
    }
}

fn command_for_path(platform: Platform, path: &Path) -> Result<DesktopCommand, DesktopError> {
    let program = match platform {
        Platform::Windows => "explorer",
        Platform::MacOS => "open",
        Platform::Linux => "xdg-open",
        Platform::Other => return Err(DesktopError::Unsupported),
    };
    Ok(DesktopCommand::for_path(program, path))
}

fn execute(command: DesktopCommand) -> Result<(), DesktopError> {
    execute_with(command, |command| {
        Command::new(command.program)
            .args(&command.arguments)
            .status()
    })
}

fn execute_with(
    command: DesktopCommand,
    run: impl FnOnce(&DesktopCommand) -> io::Result<ExitStatus>,
) -> Result<(), DesktopError> {
    let status = run(&command).map_err(|source| DesktopError::Start {
        program: command.program,
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(DesktopError::Failed {
            program: command.program,
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopError, execute_with, font_license_command, open_file_command, open_folder_command,
    };
    use crate::platform::Platform;
    use std::io;
    use std::path::Path;

    #[test]
    fn linux_file_and_folder_openers_use_xdg_open() {
        let path = Path::new("/tmp/map");
        let file = open_file_command(Platform::Linux, path).unwrap();
        let folder = open_folder_command(Platform::Linux, path).unwrap();

        assert_eq!(file.program, "xdg-open");
        assert_eq!(folder.program, "xdg-open");
        assert_eq!(file.arguments, folder.arguments);
    }

    #[test]
    fn windows_file_opener_remains_explorer() {
        let command = open_file_command(Platform::Windows, Path::new("C:/map")).unwrap();
        assert_eq!(command.program, "explorer");
        assert_eq!(
            font_license_command(Platform::Windows, Path::new("C:/license.txt"))
                .unwrap()
                .program,
            "notepad"
        );
    }

    #[test]
    fn linux_font_license_uses_the_regular_file_opener() {
        let path = Path::new("/tmp/Inconsolata-OFL.txt");
        assert_eq!(
            font_license_command(Platform::Linux, path).unwrap(),
            open_file_command(Platform::Linux, path).unwrap()
        );
    }

    #[test]
    fn missing_desktop_opener_is_reported() {
        let command = open_file_command(Platform::Linux, Path::new("/tmp/map")).unwrap();
        let error = execute_with(command, |_| {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "xdg-open is unavailable",
            ))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            DesktopError::Start {
                program: "xdg-open",
                ..
            }
        ));
    }
}

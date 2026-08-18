use std::env;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use thiserror::Error;

use super::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardEnvironment {
    pub wayland_display: bool,
}

impl ClipboardEnvironment {
    fn from_process() -> Self {
        Self {
            wayland_display: env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCommand {
    pub program: &'static str,
    pub arguments: &'static [&'static str],
}

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard access is unsupported on this platform")]
    Unsupported,
    #[error("no supported clipboard program was found (tried: {programs})")]
    NoBackend { programs: String },
    #[error("failed to start clipboard command '{program}': {source}")]
    Start {
        program: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("clipboard command '{program}' has no standard input")]
    MissingStandardInput { program: &'static str },
    #[error("failed to write to clipboard command '{program}': {source}")]
    Write {
        program: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("clipboard command '{program}' exited unsuccessfully")]
    Failed { program: &'static str },
}

pub fn copy_to_clipboard(text: &str) -> Result<(), ClipboardError> {
    let commands = clipboard_commands(Platform::current(), ClipboardEnvironment::from_process())?;
    let mut missing = Vec::new();

    for command in commands {
        let mut child = match Command::new(command.program)
            .args(command.arguments)
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                missing.push(command.program);
                continue;
            }
            Err(source) => {
                return Err(ClipboardError::Start {
                    program: command.program,
                    source,
                });
            }
        };
        let stdin = child
            .stdin
            .as_mut()
            .ok_or(ClipboardError::MissingStandardInput {
                program: command.program,
            })?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|source| ClipboardError::Write {
                program: command.program,
                source,
            })?;
        let status = child.wait().map_err(|source| ClipboardError::Start {
            program: command.program,
            source,
        })?;
        if status.success() {
            return Ok(());
        }
        return Err(ClipboardError::Failed {
            program: command.program,
        });
    }

    Err(ClipboardError::NoBackend {
        programs: missing.join(", "),
    })
}

pub fn clipboard_commands(
    platform: Platform,
    environment: ClipboardEnvironment,
) -> Result<Vec<ClipboardCommand>, ClipboardError> {
    match platform {
        Platform::Windows => Ok(vec![ClipboardCommand {
            program: "clip",
            arguments: &[],
        }]),
        Platform::MacOS => Ok(vec![ClipboardCommand {
            program: "pbcopy",
            arguments: &[],
        }]),
        Platform::Linux => {
            let mut commands = Vec::new();
            if environment.wayland_display {
                commands.push(ClipboardCommand {
                    program: "wl-copy",
                    arguments: &[],
                });
            }
            commands.extend([
                ClipboardCommand {
                    program: "xclip",
                    arguments: &["-selection", "clipboard"],
                },
                ClipboardCommand {
                    program: "xsel",
                    arguments: &["--clipboard", "--input"],
                },
            ]);
            Ok(commands)
        }
        Platform::Other => Err(ClipboardError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardEnvironment, clipboard_commands};
    use crate::platform::Platform;

    #[test]
    fn linux_wayland_prefers_wl_copy_then_x11_fallbacks() {
        let commands = clipboard_commands(
            Platform::Linux,
            ClipboardEnvironment {
                wayland_display: true,
            },
        )
        .unwrap();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.program)
                .collect::<Vec<_>>(),
            vec!["wl-copy", "xclip", "xsel"]
        );
    }

    #[test]
    fn linux_x11_uses_xclip_then_xsel() {
        let commands = clipboard_commands(
            Platform::Linux,
            ClipboardEnvironment {
                wayland_display: false,
            },
        )
        .unwrap();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.program)
                .collect::<Vec<_>>(),
            vec!["xclip", "xsel"]
        );
        assert_eq!(commands[0].arguments, ["-selection", "clipboard"]);
    }
}

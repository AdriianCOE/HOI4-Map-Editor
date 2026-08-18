# Privacy

HOI4 Map Editor runs locally. The current code has no telemetry, analytics,
automatic update check, account system, cloud storage, or file-upload path.

- Mod files are read and changed only through actions initiated by the user.
- Logs and crash diagnostics remain local under
  `%LOCALAPPDATA%\HOI4MapEditor\logs` on Windows, or
  `$XDG_STATE_HOME/HOI4MapEditor/logs` (with the standard `~/.local/state`
  fallback) on Linux.
- Optional application preferences remain local under
  `%APPDATA%\HOI4MapEditor\config.toml` on Windows, or
  `$XDG_CONFIG_HOME/HOI4MapEditor/config.toml` (with the standard `~/.config`
  fallback) on Linux.
- Optional project preferences remain in
  `<mod>/.hoi4-map-editor/project.toml`.
- State transaction data and backups remain under
  `<mod>/.hoi4-state-editor/`.
- The application does not sell or share user data.
- Nothing is submitted automatically with a bug report. The user chooses
  which logs or minimal reproducer files to share.

Review this document whenever networking, update checks, telemetry, or external
services are introduced.

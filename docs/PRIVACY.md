# Privacy

HOI4 Map Editor runs locally. The current code has no telemetry, analytics,
automatic update check, account system, cloud storage, or file-upload path.

- Mod files are read and changed only through actions initiated by the user.
- Logs and crash diagnostics remain local under
  `%LOCALAPPDATA%\HOI4MapEditor\logs`.
- State transaction data and backups remain under
  `<mod>/.hoi4-state-editor/`.
- The application does not sell or share user data.
- Nothing is submitted automatically with a bug report. The user chooses
  which logs or minimal reproducer files to share.

Review this document whenever networking, update checks, telemetry, or external
services are introduced.

# Troubleshooting

## The application does not open

Extract the portable ZIP into a writable folder before running it. Check local
logs under `%LOCALAPPDATA%\HOI4MapEditor\logs`. A source build can be checked
with `cargo run`.

## The mod is not detected

Open the mod root. Province tools require `map/provinces.bmp` and
`map/definition.csv`; State tools additionally require
`history/states/*.txt`.

## A map or Image Overlay is rejected

The map inputs must use compatible dimensions. Image Overlay must exactly
match `provinces.bmp`; the error reports both dimensions.

## A state is invalid

Open diagnostics and Patch Preview. Invalid or ambiguous state documents stay
visible for inspection but unsafe edits and Apply remain blocked.

## Apply is disabled

Open Review State Changes. The editor regenerates a stale plan and offers temporary
validation directly. `ReviewRequired` needs **Validate and Continue**;
`Blocked` diagnostics must be resolved. Editing after validation makes the
report stale and requires another review.

## Save Province Map failed

The editor keeps province changes pending when encoding, staging, validation,
backup, or replacement fails. The original `provinces.bmp` and
`definition.csv` remain unchanged before commit; a failure during commit
triggers verified rollback. Do not delete the reported backup if recovery is
required.

## State validation or Apply State Changes failed

Do not remove `<mod>/.hoi4-state-editor/`. Reload the same mod and let recovery
finish. If recovery remains blocked, preserve that directory and the local log
before changing files manually.

## A configuration file is invalid

The editor continues with safe defaults and leaves the original file unchanged.
The error identifies the field or TOML position when available. Open the file
for manual repair, or use Restore Defaults and confirm replacement; the old
file is retained as `config.toml.bak` or `project.toml.bak`.

Global settings are under `%APPDATA%\HOI4MapEditor\config.toml`. Project
settings are under `<mod>/.hoi4-map-editor/project.toml`.

## project.toml was not created

Opening a mod does not create configuration files. Use **File → Project
Settings... → Save** to create one explicitly.

## A custom terrain is rejected

Use a non-empty name containing letters, numbers, `_`, or `-`; exactly three
RGB integers from 0 through 255; and type `land`, `sea`, or `lake`.

## The interface language did not update

Open **Edit → Settings...**, select `en-US` or `pt-BR`, and save. The visible UI
changes immediately. Technical paths, file names, HOI4 identifiers, and mod
content intentionally remain unchanged.

## Share a useful diagnostic

Share the application version, the exact error, relevant local log lines, and
minimal artificial reproducer files when possible. Do not send the whole mod,
backup directory, usernames, credentials, or proprietary game assets.

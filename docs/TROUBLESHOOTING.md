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

Generate a fresh Review Changes plan, resolve `ReviewRequired` or `Blocked`
diagnostics, and run temporary round-trip validation. Editing after validation
makes the report stale and requires another review.

## Validation or Save failed

Do not remove `<mod>/.hoi4-state-editor/`. Reload the same mod and let recovery
finish. If recovery remains blocked, preserve that directory and the local log
before changing files manually.

## Share a useful diagnostic

Share the application version, the exact error, relevant local log lines, and
minimal artificial reproducer files when possible. Do not send the whole mod,
backup directory, usernames, credentials, or proprietary game assets.

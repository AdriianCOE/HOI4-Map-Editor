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

Open **Edit → Settings...**, select the **Language** row, cycle to the
language you want (shown by its native name), and **Save**. The visible UI
changes immediately when you cycle it, even before saving; **Cancel** reverts
to the previously saved language. Technical paths, file names, HOI4
identifiers, and mod content intentionally remain unchanged.

## A saved language does not persist between sessions

The language is written to `config.toml` only when you press **Save** in
Settings, not when you merely preview it by cycling the row. If the editor
starts in the wrong language, confirm you pressed **Save**, and that
`%APPDATA%\HOI4MapEditor\config.toml` is writable. If the file's `language`
value is unrecognized (for example, edited by hand or written by a newer
version), the editor falls back to English for that session and logs a
warning; it does not silently overwrite your file.

## Boxes or missing characters instead of text ("tofu")

The editor bundles a single Latin font and, for Cyrillic (Русский) and
Simplified Chinese (简体中文) text it cannot cover, falls back to fonts already
installed on Windows (Segoe UI, Microsoft YaHei UI, or SimSun). If those
characters still render as empty boxes, your Windows installation is likely
missing the relevant language/font pack. Install the Windows optional
"Chinese (Simplified)" or "Russian" language/font support from **Settings →
Time & Language → Language & region**, then restart the editor. No font is
ever silently swapped for different characters — only the glyphs drawn to
render the text you already see.

## The interface looks cut off or overlapping

Some panels use a fixed minimum width and are most comfortable at a normal
desktop window size. If labels look crowded in a very narrow or snapped
window, resize the window wider. If text is still clipped or overlapping
after resizing, please report it (see below) with your language, the panel
name, and a screenshot.

## Windows SmartScreen warned about the executable

The preview build is not code-signed, so Windows SmartScreen may show an
"unrecognized app" warning on first run. This is expected for an
independently distributed preview tool, not a sign of tampering. Select
**More info → Run anyway** only after verifying the ZIP's SHA-256 checksum
(see below) against the one published with the release you downloaded.

## My antivirus flagged the executable

Unsigned, freshly built executables are sometimes flagged by antivirus
heuristics as a false positive, especially right after a new release goes
up. Verify the SHA-256 checksum first; if it matches the published release,
you can submit the file to your antivirus vendor to have the false positive
reviewed. Do not disable your antivirus to work around this.

## Verifying the download (SHA-256)

Each release publishes a `.sha256` file alongside the ZIP. On Windows,
compare it with:

```powershell
Get-FileHash ".\HOI4-Map-Editor-v<version>-windows-x64.zip" -Algorithm SHA256
```

The `Hash` value must match the hash inside the `.sha256` file exactly. If it
does not match, delete the download and get it again from the official
GitHub Releases page; do not run a file whose hash does not match.

## Share a useful diagnostic

Share the application version, the exact error, relevant local log lines, and
minimal artificial reproducer files when possible. Do not send the whole mod,
backup directory, usernames, credentials, or proprietary game assets.

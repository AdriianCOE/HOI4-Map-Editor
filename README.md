# HOI4 Map Editor

An unofficial visual editor for Hearts of Iron IV province maps and state
history files.

HOI4 Map Editor is based on
[ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
and extends it with state editing, political views, validation, backups, and
transactional file updates.

> Preview software: keep an independent backup of every mod you edit.

## Features

- Province brush, fill, lasso, recolor, terrain, coastal, and diagnostic tools.
- State selection, Lasso, Brush, Fill, properties, buildings, victory points,
  creation, removal, Undo, Redo, and Discard.
- Province Colors, Province Types, Terrain, Continents, Coastal, States, and
  Political Map Views with independent visual overlays.
- Lossless State Patch Preview and validation in a temporary workspace.
- Atomic Province Save and transactional State Apply with backup and rollback.
- Application and project settings with English and Brazilian Portuguese UI.

## Install and open a mod

1. Download and extract the Windows x64 ZIP into a writable folder.
2. Run `HOI4 Map Editor.exe`.
3. Choose **File → Open HOI4 Mod...** and select the mod root.

A province project needs `map/provinces.bmp` and `map/definition.csv`. The
States workspace additionally uses `history/states/*.txt`.

## Safe workflow

```text
Edit in memory
→ Review State Changes
→ Validate in Temporary Workspace
→ Apply State Changes with Backup
→ Reload and Verify
```

**Save Province Map** writes only `provinces.bmp` and `definition.csv`.
**Apply State Changes** writes only `history/states/*.txt`.
**Export Province Map** creates a copy and never changes the open project.

Existing state files retain unrelated comments, formatting, unknown fields,
and unsupported blocks. State transactions use `<mod>/.hoi4-state-editor/`.
Optional project preferences use `<mod>/.hoi4-map-editor/project.toml`.

## Essential shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open a mod |
| `Ctrl+S` | Save the current workspace only |
| `Ctrl+Shift+S` | Export Province Map As |
| `Ctrl+1` / `Ctrl+2` | Provinces / States workspace |
| `1`–`7` | Change Map View |
| `B` / `F` / `L` | Brush / Fill / Lasso |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo |
| `Esc` | Cancel the current action |

## Build

Install a current Rust toolchain and run:

```sh
cargo build --release
```

Create the portable Windows package with:

```powershell
.\scripts\package-windows.ps1
```

## Current limitations

- Preview packages currently target Windows x64.
- Province Save and State Apply are separate transactions; there is no Save All.
- Game-data localization, flags, icons, `.gfx`, and `.dds` loading are not yet
  implemented.
- Adjacencies, Strategic Regions, and Continents are not complete dedicated
  editing workspaces.
- No proprietary Hearts of Iron IV assets are distributed.

See the [User Guide](docs/USER_GUIDE.md),
[Troubleshooting](docs/TROUBLESHOOTING.md), and
[Privacy](docs/PRIVACY.md).

## Credits

Based on HOI4 Province Editor by ScottyThePilot. Developed and extended by
Adrian Costa.

HOI4 Map Editor is an unofficial community project and is not affiliated with
or endorsed by Paradox Interactive. The project retains the original MIT
license and third-party notices.

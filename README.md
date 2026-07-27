# HOI4 Map Editor

An unofficial map editing tool for Hearts of Iron IV mods.

HOI4 Map Editor is based on
[ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
and extends it with state editing, political map views, overlays, validation,
backups and safer state file updates.

> **Preview software:** keep a separate backup of your mod before editing it.

## Features

### Province workspace

- Edit `provinces.bmp` and `definition.csv`
- Brush, fill, lasso and recolor tools
- Terrain, coastal province and map diagnostics
- Adjacency visualization and legacy editing tools
- Support for map folders and the original Province Editor ZIP workflow
- Atomic, validated province-map saves and non-destructive exports

### State workspace

- State and Political map views
- Select, assign and unassign provinces
- State Lasso, Brush and Fill tools
- Create and remove states
- Edit:
  - name and category
  - manpower and local supplies
  - owner and controller
  - cores and claims
  - resources
  - state and province buildings
  - victory points
- Undo, Redo and Discard
- Review and validate changes before applying them
- Automatic backup, rollback and interrupted-save recovery

## Map views

The current Map View changes how the map is displayed without changing what
you are editing.

1. Province Colors
2. Province Types
3. Terrain / Biome
4. Continents
5. Coastal Provinces
6. States
7. Political

Available overlays include:

- Rivers
- Adjacencies
- Province IDs
- Province Borders
- State Borders
- Custom Image Overlay

The Image Overlay supports BMP, PNG and JPG images with the same dimensions as
`provinces.bmp`. The project heightmap can also be loaded as an overlay.

## Installation

1. Download the Windows x64 ZIP from
   [GitHub Releases](../../releases).
2. Extract it into a writable folder.
3. Run `HOI4 Map Editor.exe`.
4. Open the root folder of your mod.

Do not run the application directly from inside the ZIP.

A compatible mod normally contains:

```text
mod-root/
├─ map/
│  ├─ provinces.bmp
│  ├─ definition.csv
│  ├─ heightmap.bmp
│  ├─ adjacencies.csv
│  └─ rivers.bmp
└─ history/
   └─ states/
      └─ *.txt
```

Only `provinces.bmp`, `definition.csv` and `history/states/` are required for
the main province and state workflows. Other files enable additional views
and overlays.

## Basic workflow

1. Open the mod root.
2. Choose the **Provinces** or **States** workspace.
3. Select a tool and make your changes.
4. In Provinces, use **Save Province Map** through **Save Current Workspace**.
5. In States, use **Review State Changes** and then **Apply State Changes**.

State changes remain in memory until they are applied. Existing state files are
updated without rewriting unrelated comments, formatting or unsupported
content.

The save commands are intentionally separate:

```text
Save Province Map    → writes geographic province files
Apply State Changes  → writes state history files
Export Province Map  → creates a copy without modifying the current mod
```

Province saves encode and validate complete staged files before atomically
replacing `provinces.bmp` and `definition.csv`. Province exports do not change
the open project or clear its pending changes.

## Safety

Before state changes are written, the editor can:

- generate a change preview;
- validate the result in a temporary copy;
- create a backup;
- reload and verify the saved files;
- roll back a failed transaction.

Province-map saves likewise stage and validate complete BMP/CSV candidates,
verify a backup, then replace the destination files. The original geographic
files remain unchanged during encoding and validation.

State backups and recovery data are stored under:

```text
<mod>/.hoi4-state-editor/
```

The editor does not include or distribute Hearts of Iron IV game assets.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open a mod |
| `Ctrl+S` | Save the current workspace only |
| `Ctrl+Shift+S` | Export Province Map As |
| `Ctrl+Shift+Alt+S` | Export Province Map Archive |
| `Ctrl+1` | Provinces workspace |
| `Ctrl+2` | States workspace |
| `Ctrl+Tab` | Switch workspace |
| `1`–`7` | Change Map View |
| `B` | Brush |
| `F` | State Fill |
| `L` | Lasso |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo |
| `Esc` | Cancel the current action |
| `H` | Reset zoom |

Map shortcuts are disabled while typing in a field, search box or dialog.

## Building from source

Install a current Rust toolchain and run:

```sh
cargo build --release
```

To create the portable Windows package:

```powershell
.\scripts\package-windows.ps1
```

The package script creates the ZIP, SHA-256 checksum and release manifest in
`dist/`.

## Current limitations

- Preview builds currently support Windows x64.
- Transactional state saving currently covers direct files under
  `history/states/`.
- Province-map save and State Apply are independent transactions; there is no
  combined Save All command.
- Country localization, flags and game icons are not loaded yet.
- Adjacencies can be viewed, but the complete Adjacencies workspace is still
  planned.
- Strategic Regions and Continents are not yet editable as dedicated
  workspaces.
- There is no telemetry or automatic file upload.

More information:

- [User Guide](docs/USER_GUIDE.md)
- [Known Limitations](docs/KNOWN_LIMITATIONS.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Privacy](docs/PRIVACY.md)

## Planned work

- Localized names, country colors, flags and icons
- Adjacencies Editor
- Project Validator
- Strategic Regions
- Continent editing
- Additional release and usability improvements

## Credits

Based on
[HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
by ScottyThePilot.

Developed and extended by Adrian Costa.

HOI4 Map Editor is an unofficial community tool and is not affiliated with or
endorsed by Paradox Interactive.

The original MIT license and third-party notices are included with the project.

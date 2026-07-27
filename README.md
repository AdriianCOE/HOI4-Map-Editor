# HOI4 Map Editor

A visual province and state editor for Hearts of Iron IV mods.

Paint provinces, edit terrain, organize states and review changes without
manually editing large bitmaps or PDXScript files.

HOI4 Map Editor is based on
[ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
and extends it with state editing, map views, project validation, backups and
safer file updates.

[Download the latest preview](https://github.com/AdriianCOE/hoi4_state_editor/releases)

> **Preview software:** keep an independent backup of every mod you edit.

![Province Map View](images/hoi4me_color.png)
![State Map View](images/hoi4me_state.png)

## Features

### Province editing

- Paint, fill, recolor and lasso-select provinces directly on the map.
- Edit terrain, province type, coastal status and continent data.
- Inspect province IDs, colors and map diagnostics.
- Undo and redo geographic changes.
- Save `provinces.bmp` and `definition.csv` through an atomic, validated
  workflow.

### State editing

- Select provinces with Select, Brush, Lasso or Fill.
- Assign, move and unassign provinces between states.
- Create and remove states.
- Edit:
  - name and category;
  - manpower and local supplies;
  - owner and controller;
  - cores and claims;
  - resources;
  - state buildings;
  - victory points;
  - province buildings.
- Keep all changes in memory until they are reviewed and applied.
- Undo, redo or discard state changes.

### Map views

Switch between:

- Province Colors
- Province Types
- Terrain
- Continents
- Coastal Provinces
- States
- Political

Optional overlays include:

- Rivers
- Adjacencies
- Province IDs
- Province Borders
- State Borders
- Custom Image Overlay

### Search and navigation

Use `Ctrl+F` to find provinces and states by:

- ID;
- state name;
- province color;
- terrain;
- type;
- owner or controller;
- continent;
- coastal status.

Selecting a result focuses and zooms the map automatically.

### Safe file updates

Province and state changes use separate save workflows.

**Save Province Map**

- writes only `map/provinces.bmp` and `map/definition.csv`;
- prepares and validates complete replacement files before touching the mod;
- creates a backup;
- rolls back if the operation fails.

**Apply State Changes**

- writes only direct files under `history/states/`;
- generates a lossless patch preview;
- validates the result in a temporary project copy;
- creates a backup;
- applies the changes transactionally;
- reloads and verifies the saved project;
- supports rollback and interrupted-save recovery.

Existing state files retain unrelated comments, formatting, unknown fields and
unsupported blocks.

### Settings and languages

The interface is available in:

- English
- Português do Brasil
- Español
- Français
- Русский
- 简体中文

Application settings are stored under the user's Windows profile.

Optional project settings are stored in:

```text
<mod>/.hoi4-map-editor/project.toml
```

State backups and recovery data are stored in:

```text
<mod>/.hoi4-state-editor/
```

## Installation

1. Download the Windows x64 ZIP from
   [GitHub Releases](https://github.com/AdriianCOE/hoi4_state_editor/releases).
2. Extract the ZIP into a writable folder.
3. Run `HOI4 Map Editor.exe`.
4. Select **File → Open HOI4 Mod...**
5. Choose the root folder of your mod.

Do not run the application directly from inside the ZIP.

A compatible project normally contains:

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

Only `provinces.bmp` and `definition.csv` are required for province editing.

The States workspace additionally requires `history/states/*.txt`.

## Basic workflow

### Province map

```text
Open Mod
→ Edit Provinces
→ Save Province Map
→ Validate
→ Backup
→ Replace Files
→ Reload
```

### States

```text
Open Mod
→ Edit States
→ Review State Changes
→ Validate Temporary Copy
→ Apply State Changes
→ Reload and Verify
```

There is no combined **Save All** operation. Province and state changes remain
independent so one workflow cannot silently save or clear the other.

## Essential shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open a mod |
| `Ctrl+S` | Save the current workspace |
| `Ctrl+Shift+S` | Export Province Map As |
| `Ctrl+1` | Provinces workspace |
| `Ctrl+2` | States workspace |
| `Ctrl+Tab` | Switch workspace |
| `Ctrl+F` | Search |
| `1`–`7` | Change Map View |
| `B` | Brush |
| `F` | Fill |
| `L` | Lasso |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo |
| `Esc` | Cancel the current action |
| `H` | Reset zoom |

Map shortcuts are suspended while typing in a field, picker or search box.

## Current limitations

- Preview builds currently support Windows x64 only.
- Province Save and State Apply are separate transactions.
- Country names, state localizations and other game-data localizations are not
  loaded from the game yet.
- Country flags and game icons are not displayed yet.
- Adjacencies can be inspected, but the full editing workspace is still
  planned.
- Strategic Regions and Continents do not yet have complete editing
  workspaces.
- Rivers, supply networks, heightmaps and trees are not directly editable.
- The executable is not currently digitally signed and Windows SmartScreen may
  display a warning for early preview builds.

The project does not distribute proprietary Hearts of Iron IV assets.

## Planned development

### Game data integration

- Load localized country, state, building, resource and terrain names.
- Resolve data from both the mod and the local Hearts of Iron IV installation.
- Use mod files as overrides over vanilla data.
- Display real country colors in the Political Map View.
- Search using localized names and technical identifiers.

### Flags and interface assets

- Read country flags from the local game or mod.
- Resolve `.gfx` definitions and `.dds` textures.
- Display game icons without distributing Paradox assets.
- Add a local asset cache for faster project loading.

### Adjacencies

- Create, edit and remove adjacency entries.
- Support sea crossings, canals and through-province connections.
- Validate endpoints and invalid adjacency records.
- Preview changes directly on the map.

### Project validation

- Validate province colors and IDs.
- Detect missing or duplicate state assignments.
- Check coastal provinces and naval bases.
- Validate state, strategic region and continent references.
- Produce a clear report with direct navigation to each problem.

### Strategic Regions

- Display region boundaries and membership.
- Create and edit strategic regions.
- Assign and remove states.
- Edit weather and related region data.
- Validate and save region files safely.

### Continents

- Inspect continent assignments.
- Assign provinces to continents.
- Create and rename continent definitions.
- Validate continent IDs and province references.

### Later tools

Possible additions after the core editor is stable:

- Rivers editing
- Supply network editing
- Heightmap tools
- Tree map editing
- Project autosave and workspace recovery
- External file change detection
- Procedural island and province generation
- Steam Workshop publishing helpers

Planned features are not guaranteed and may change based on testing and
community feedback.

## Documentation

- [User Guide](docs/USER_GUIDE.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Privacy](docs/PRIVACY.md)
- [Changelog](CHANGELOG.md)

## Building from source

Install a current Rust toolchain and run:

```sh
cargo build --release
```

To create the portable Windows package:

```powershell
.\scripts\package-windows.ps1
```

The packaging script creates the ZIP, SHA-256 checksum and release manifest in
`dist/`.

## Reporting issues

When reporting a problem, include:

- application version;
- Windows version;
- workspace and tool being used;
- steps to reproduce the problem;
- relevant local log output;
- screenshots when the problem is visual.

Logs are stored under:

```text
%LOCALAPPDATA%\HOI4MapEditor\logs
```

Do not upload an entire mod unless it is necessary and you have permission to
share it.

## Credits

Based on
[HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
by ScottyThePilot.

Developed and extended by Adrian Costa.

HOI4 Map Editor is an unofficial community project and is not affiliated with
or endorsed by Paradox Interactive.

The project retains the original MIT license and all applicable third-party
notices.

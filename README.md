# HOI4 Map Editor

A visual province and state editor for Hearts of Iron IV mods.

Paint provinces, edit terrain, organize states, and review changes without manually editing large bitmaps, CSV rows, or PDXScript files.

HOI4 Map Editor is based on [ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor) and extends it with state editing, map views, project validation, backups, rollback, recovery, and safer file updates.

[Download the latest preview](https://github.com/AdriianCOE/HOI4-Map-Editor/releases)

> **Preview software:** keep an independent backup of every mod you edit.

![Province Map View](images/hoi4me_color.png)
![State Map View](images/hoi4me_state.png)

## Features

### Province editing

- Paint, fill, recolor, and lasso-select provinces directly on the map.
- Edit terrain, province type, coastal status, continent, RGB color, and definition data.
- Inspect province IDs, colors, terrain, state assignments, and map diagnostics.
- Search and focus provinces by technical properties.
- Undo and redo geographic changes.
- Include `provinces.bmp` and `definition.csv` candidates in the validated,
  coordinated Save Project transaction.

### Map compatibility

- Custom and non-vanilla map dimensions are supported by the editor; this does
  not assert that every size is valid for Hearts of Iron IV itself.
- Sparse Province IDs are supported: existing positive IDs are preserved, gaps
  are allowed, new IDs follow the current maximum, and IDs are not compacted
  automatically.
- Opening a project can report actionable compatibility diagnostics for malformed
  map/definition data, missing colors or definitions, unusual dimensions,
  invalid Province references, and State/project structure issues. Sparse IDs
  are informational compatibility data, not a load blocker.

### State editing

- Select provinces with Select, Brush, Lasso, or Fill.
- Assign, move, or unassign provinces between states.
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
- Keep changes in memory until they are reviewed and applied.
- Undo, redo, or discard state changes.

### Map views

Switch between:

- Province Colors
- Province Types
- Terrain
- Continents
- Coastal Provinces
- States
- Political

Political View resolves country colors, localized country names, and available
normal flags from the user's mod/base-game installation. It remains read-only:
missing metadata falls back safely to country tags and deterministic colors.
Country metadata is loaded once per project/base-game/language change, not per
frame. Resolution is layer-first: the mod's `common/countries/colors.txt`, then
its per-country definitions, then the equivalent base-game sources. The
`color` entry is used (never `color_ui`). Localization scans every
`localisation/<language>/**/*.yml` file; it prefers `TAG`, then the country's
initial ruling-ideology key, and uses the raw tag only as a final fallback.

Optional overlays include:

- Resources
- Rivers
- Adjacencies
- Province IDs
- Province Borders
- State Borders
- Custom Image Overlay

Resources is a read-only overlay for fully loaded State projects. It shows the
current working State resource quantities, including unsaved edits. Icons are
resolved at runtime from the mod/base-game `common/resources` definitions and
`GFX_resources_strip`; missing or custom icons fall back to text. No Paradox
assets are distributed with the editor.

### Search and navigation

Use `Ctrl+F` to find provinces and states by:

- ID;
- state name;
- province color;
- terrain;
- type;
- owner or controller;
- continent;
- coastal status;
- current state assignment.

Selecting a result focuses and zooms the map automatically.

### Safe file updates

**Save Project** reviews every pending Province Map and State change together.

- prepares `provinces.bmp`, `definition.csv`, and all affected
  `history/states/*.txt` candidates before the first mod write;
- validates the combined candidate in a physical temporary copy;
- creates one verified backup and durable journal;
- replaces each affected file atomically in deterministic order;
- rolls back the coordinated operation when a later step fails;
- reloads and verifies the complete project before clearing dirty state;
- detects external file changes and supports interrupted-save recovery.

This is a transactional, rollback-capable project save. Individual files are
published atomically; it is not one filesystem-atomic operation for the entire
multi-file project. Unix builds also persist critical file and directory-entry
changes around the save journal.

Existing state files retain unrelated comments, formatting, unknown fields, and unsupported blocks.

### Settings and languages

The interface is available in:

- English
- Português do Brasil
- Español
- Français
- Русский
- 简体中文

Application settings use the platform configuration directory:

```text
Windows: %APPDATA%\HOI4MapEditor\config.toml
Linux:   $XDG_CONFIG_HOME/HOI4MapEditor/config.toml
         (or ~/.config/HOI4MapEditor/config.toml)
```

Optional project settings are stored in:

```text
<mod>/.hoi4-map-editor/project.toml
```

State backups and recovery data are stored in:

```text
<mod>/.hoi4-state-editor/
```

## Installation and supported platforms

| Platform | Status | Baseline |
| --- | --- | --- |
| Windows x86_64 | Supported preview | Windows desktop x64 |
| Linux x86_64 | Supported preview | Ubuntu 22.04-compatible glibc desktop with OpenGL |

Linux support does not cover ARM, musl distributions, older glibc systems, or
every Linux distribution. The desktop stack may use X11 and/or Wayland according
to the installed runtime libraries.

### Windows

1. Download the Windows x64 ZIP from [GitHub Releases](https://github.com/AdriianCOE/HOI4-Map-Editor/releases).
2. Extract the ZIP into a writable folder.
3. Run `HOI4 Map Editor.exe`.
4. Select **File → Open HOI4 Mod...**
5. Choose the root folder of your mod.

Do not run the application directly from inside the ZIP.

### Linux

1. Download `HOI4-Map-Editor-v<version>-linux-x86_64.tar.gz` from GitHub Releases.
2. Verify its adjacent `.sha256` file, extract it into a writable folder, and
   run `./hoi4-map-editor`.
3. Select **File → Open HOI4 Mod...** and choose the root folder of your mod.

Do not run the application from inside the tarball. `xdg-open` is optional for
opening files/folders. Clipboard integration uses `wl-copy`, `xclip`, or `xsel`
when available; the editor still launches if they are absent. Additional
Cyrillic/CJK glyph coverage depends on installed system fonts.

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

The States workspace additionally uses `history/states/*.txt`.

## Basic workflow

```text
Open Mod
→ Edit Provinces and/or States
→ View Changes if desired
→ Save Project
→ Validate and round-trip the candidate
→ Ready to Save
→ Explicit Save Project
→ Reload and Verify
```

**Export Province Map As...** remains separate and never clears project dirty
state.

## Essential shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open a mod |
| `Ctrl+S` | Review and Save Project |
| `Ctrl+Shift+S` | Export Province Map As |
| `Ctrl+1` | Provinces workspace |
| `Ctrl+2` | States workspace |
| `Ctrl+Tab` | Switch workspace |
| `Ctrl+F` | Search |
| `1`–`7` | Change Map View; Resources is toggled as an overlay |
| `B` | Brush |
| `F` | Fill |
| `L` | Lasso |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo |
| `Esc` | Cancel the current action |
| `H` | Reset zoom |

Map shortcuts are suspended while typing in a field, picker, or search box.

## Current limitations

- Linux preview support is limited to x86_64 glibc desktops built and tested on
  the Ubuntu 22.04 baseline; ARM, musl, and older glibc environments are not
  supported.
- The Validation Results window currently offers basic all-domain results;
  advanced report export and persistent ignore policies are planned.
- State localizations and broader game-data localization are not loaded yet.
  Political View and the Resources overlay load only the presentation metadata
  they need at runtime.
- Adjacencies can be inspected and preserved, but the full editing workspace is
  still planned; invalid references require external repair.
- Strategic Regions and Continents do not yet have complete dedicated editing workspaces.
- Rivers, supply networks, heightmaps, and tree maps are not directly editable.
- The executable is not currently digitally signed, so Windows SmartScreen may display a warning for early preview builds.

The project does not distribute proprietary Hearts of Iron IV assets.

## Planned development

### Game data integration

- Load localized state, building, resource, terrain, and category names.
- Resolve data from both the mod and the local Hearts of Iron IV installation.
- Use mod files as overrides over vanilla data.
- Search using localized names and technical identifiers.

### Broader interface assets

- Extend runtime `.gfx`/`.dds` support beyond the currently used country flags
  and resource strip.
- Display additional game icons without distributing Paradox assets.
- Add broader local asset caching where it improves first-use latency.

### Adjacencies

- Create, edit, and remove adjacency entries.
- Support sea crossings, canals, and through-province connections.
- Validate endpoints and malformed adjacency records.
- Preview changes directly on the map.

### Project validation

- Validate province colors and IDs.
- Detect missing or duplicate state assignments.
- Check coastal provinces and naval bases.
- Validate state, strategic region, and continent references.
- Produce a report with direct navigation to each problem.

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
- Project autosave and unsaved-workspace recovery
- Procedural island and province generation
- Steam Workshop publishing helpers

Planned features are not guaranteed and may change based on testing and community feedback.

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

The packaging script creates the ZIP, SHA-256 checksum, and release manifest in `dist/`.

To create and validate the portable Linux package on Linux:

```sh
bash scripts/package-linux.sh
```

See [Linux packaging](docs/development/LINUX_PACKAGING.md) for the exact
allowlist, runtime expectations, and manual desktop smoke gate.

## Reporting issues

When reporting a problem, include:

- application version;
- operating system and version;
- workspace and tool being used;
- steps to reproduce the problem;
- relevant local log output;
- screenshots when the problem is visual.

Logs are stored under:

```text
Windows: %LOCALAPPDATA%\HOI4MapEditor\logs
Linux:   $XDG_STATE_HOME/HOI4MapEditor/logs
         (or ~/.local/state/HOI4MapEditor/logs)
```

Do not upload an entire mod unless it is necessary and you have permission to share it.

## Support

<a href="https://ko-fi.com/adriiancoe">
  <img src="https://raw.githubusercontent.com/AdriianCOE/AdriianCOE/refs/heads/main/support_me_on_kofi_badge.png" alt="Support Adrian Costa on Ko-fi" width="180">
</a>

Support helps with development, testing, documentation, and future updates.

## Credits

Based on [HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor) by ScottyThePilot.

Developed and extended by Adrian Costa.

HOI4 Map Editor is an unofficial community project and is not affiliated with or endorsed by Paradox Interactive.

The project retains the original MIT license and all applicable third-party notices.

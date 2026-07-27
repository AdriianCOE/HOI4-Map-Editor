# HOI4 Map Editor

[Latest Release](https://github.com/AdriianCOE/hoi4_state_editor/releases/latest)

Paint provinces, select them with a brush or lasso, and organize them into states — all visually, without hand-editing bitmaps or text files. HOI4 Map Editor is an unofficial visual editor for Hearts of Iron IV province maps and state history files, built to make map and state work fast and safe.

Based on [ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor) and extended with state editing, political views, validation, backups, and transactional file updates.

> **Preview software:** keep an independent backup of every mod you edit.

![Province Map Mode](images/hoi4me_color.png)
![Terrain Map Mode](images/hoi4me_state.png)

## Features

**Province editing**
- Paint, fill, lasso-select, and recolor provinces directly on the map.
- Edit terrain, coastal flags, and other province data with instant visual feedback.
- Built-in diagnostics catch map errors and warnings before you save.

**State editing**
- Select provinces with Brush, Lasso, or Fill and assign them to a state in a few clicks.
- Edit state properties, buildings, and victory points.
- Create, remove, Undo, Redo, or Discard state edits — everything stays in memory until you're ready to apply it.

**Map Views & overlays**
- Switch between Province Colors, Province Types, Terrain, Continents, Coastal, States, and Political views.
- Layer independent visual overlays on top of any view.

**Safety by design**
- Lossless State Patch Preview and validation in a temporary workspace before anything touches your mod.
- Atomic Province Save and transactional State Apply, both with backup and rollback.

**Interface**
- Application and project settings, with a 6-language interface: English, Português do Brasil, Español, Français, Русский, and 简体中文.

## Install and open a mod

1. [Download](https://github.com/AdriianCOE/hoi4_state_editor/releases/latest) and extract the Windows x64 ZIP into a writable folder.
2. Run `HOI4 Map Editor.exe`.
3. Choose **File → Open HOI4 Mod...** and select your mod's root folder.

A province project needs `map/provinces.bmp` and `map/definition.csv`. The States workspace additionally uses `history/states/*.txt`.

## Safe workflow

```text
Edit in memory
→ Review State Changes
→ Validate in Temporary Workspace
→ Apply State Changes with Backup
→ Reload and Verify
```

- **Save Province Map** writes only `provinces.bmp` and `definition.csv`.
- **Apply State Changes** writes only `history/states/*.txt`.
- **Export Province Map** creates a copy and never changes the open project.

Existing state files retain unrelated comments, formatting, unknown fields, and unsupported blocks. State transactions use `<mod>/.hoi4-state-editor/`. Optional project preferences use `<mod>/.hoi4-map-editor/project.toml`.

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
- Game-data localization, flags, icons, `.gfx`, and `.dds` loading are not yet implemented.
- Adjacencies, Strategic Regions, and Continents are not complete dedicated editing workspaces.
- No proprietary Hearts of Iron IV assets are distributed.

See the [User Guide](docs/USER_GUIDE.md), [Troubleshooting](docs/TROUBLESHOOTING.md), and [Privacy](docs/PRIVACY.md).

## Credits

Based on HOI4 Province Editor by ScottyThePilot. Developed and extended by Adrian Costa.

HOI4 Map Editor is an unofficial community project and is not affiliated with or endorsed by Paradox Interactive. The project retains the original MIT license and third-party notices.

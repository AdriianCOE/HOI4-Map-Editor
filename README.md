# HOI4 Map Editor

Province and State Editing Toolkit

A visual editor for Hearts of Iron IV province maps and state history files.

HOI4 Map Editor combines the geographic tools inherited from
[ScottyThePilot's HOI4 Province Editor](https://github.com/ScottyThePilot/hoi4_province_editor)
with safe, transactional editing of `history/states/*.txt`.

## Current status

The core through Phase 4C is complete: project discovery, lossless state
parsing, state visualization, in-memory editing, patch preview, isolated
round-trip validation, backup, transactional Save, rollback, and crash
recovery.

Phase 5A.1.3 continues UX refinement and establishes the Windows preview
release pipeline. Phase 5B sprite, `.gfx`, `.dds`, and localisation resolution
is not implemented.

This is experimental software. Keep an independent backup of every mod.

## Features

### Province editing

- Legacy HOI4 Province Editor compatibility mode.
- Geographic editing of `provinces.bmp`.
- Brush, fill, lasso, recolour, coastal, adjacency, and diagnostic tools.
- Direct map-folder and map-ZIP workflows.

Province editing changes geographic files and can damage a map if used
incorrectly. Back up the mod before using it.

### State editing

- States and Political map views.
- State selection, Ctrl+click province selection, State Lasso, State Brush,
  and semantic State Fill.
- State properties, owner/controller, cores, claims, resources, state
  buildings, victory points, and province buildings.
- Creation and removal of states in memory.
- Undo, Redo, and global Discard.
- Lossless patch preview and isolated round-trip validation.
- Verified backup, transactional Save, rollback, and interrupted-save recovery.

## State editing workflow

```text
Edit in memory
→ Generate Patch Preview
→ Validate in Temporary Workspace
→ Save with Backup
→ Reload and Verify
```

1. Open a mod root.
2. Select a state and, when needed, a target state.
3. Edit properties or province assignments in memory.
4. Generate a Patch Preview.
5. Validate the exact current plan in a temporary workspace.
6. Save only a current Safe plan with a current Passed validation.

Existing state files are patched through their syntax-aware source documents.
Unchanged comments, spacing, line endings, BOM, ordering, unknown fields,
repeated keys, and dated blocks are preserved. Newly created states use a
canonical representation because they have no original document.

## Safety model

State editing and geographic province editing are separate subsystems:

- A state project treats `provinces.bmp`, `definition.csv`, `adjacencies.csv`,
  and `rivers.bmp` as read-only geographic inputs.
- State changes remain in memory until the validated Save workflow runs.
- Save is blocked for stale, `ReviewRequired`, or `Blocked` patch plans.
- Save requires the exact current `Passed` round-trip validation report.
- Backups, manifests, journals, and transaction reports are stored under
  `<mod>/.hoi4-state-editor/`.
- A commit failure triggers journal-driven rollback. Incomplete rollback keeps
  recovery data and blocks further editing until recovery completes.
- Only direct `history/states/*.txt` paths covered by the plan may be written.

The project does not distribute proprietary Hearts of Iron IV assets.

## Map views and overlays

The UI keeps three independent concepts:

- **Workspace** — the data being edited: Provinces or States.
- **Map View** — the mutually exclusive renderer colouring the map.
- **Overlays** — read-only visual layers composed over the current Map View.

Changing a Map View or overlay does not change the workspace, selection,
working state, dirty state, active State Brush/Lasso/Fill, or project files.
Each workspace remembers its last Map View. Provinces starts with Province
Colors; States starts with States. Overlays remain shared and are preserved
when switching workspaces.

The canonical Map Views are:

1. Province Colors
2. Province Types
3. Terrain / Biome
4. Continents
5. Coastal Provinces
6. States
7. Political

States uses deterministic colours by state ID and state diagnostics. Political
uses effective owner colours; when country colour metadata is unavailable,
tags receive deterministic fallback colours.

Independent overlays are available from **View** and the compact sidebar:

- Rivers;
- Adjacencies, including connection type, explicit endpoints, through-province
  markers, hover highlight, and the active adjacency-edit preview;
- Province IDs;
- Province Borders;
- State Borders;
- one read-only Image Overlay.

Image Overlay accepts BMP, PNG, JPG, and JPEG. It follows map zoom and pan,
does not participate in hit testing, and must exactly match the dimensions of
`provinces.bmp`. A mismatch reports both sizes and is blocked. Opacity changes
reuse the existing texture. **Use project heightmap** loads
`map/heightmap.bmp`; DDS remains deferred to Phase 5B.

Selection, target, Lasso, Brush, Fill, diagnostics, and tooltips remain
contextual visual feedback rather than user-configured base views.

The main menu is limited to **File | Edit | View | Tools | Help**. The compact
bar below it contains the Provinces/States workspace switch, Map View,
Overlays, **Review Changes**, and **Apply to Mod**. Edit actions and the left
tool strip follow the active workspace; Lasso, Brush, and Fill options appear
only while that State tool is active. Advanced patch, validation, save-report,
and recovery diagnostics remain under **Tools**.

The State Inspector is an overlay drawer. Expanding or collapsing it does not
resize the map viewport, change zoom or pan, recenter the selection, or modify
the working state.

## Keyboard shortcuts

| Shortcut | Action | Availability |
| --- | --- | --- |
| `Ctrl+1` | Provinces workspace | Loaded projects |
| `Ctrl+2` | States workspace | State projects |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Cycle workspace | Loaded projects |
| `1` | Province Colors | All loaded maps |
| `2` | Province Types | All loaded maps |
| `3` | Terrain / Biome | All loaded maps |
| `4` | Continents | All loaded maps |
| `5` | Coastal Provinces | All loaded maps |
| `6` | States | State projects |
| `7` | Political | State projects |
| `8` | States, hidden legacy alias for `6` | Compatibility |
| `9` | Cycle province label mode | Loaded projects |
| `F3` | Cycle developer diagnostics | State projects |
| `B` | State Brush; Paint Bucket in Province view | Contextual |
| `F` | State Fill from the hovered province | State projects |
| `L` | State Lasso; legacy lasso in Province view | Contextual |
| `Shift+L` | State Lasso Add mode | State projects |
| `Alt+L` | State Lasso Remove mode | State projects |
| `Enter` | Confirm Fill/Lasso preview or finish a legacy tool | Contextual |
| `Esc` | Cancel picker, Brush, Lasso, Fill, selection, or active tool | Contextual |
| `M` | Move selected provinces to target state | State projects |
| `Delete` | Unassign selected provinces | State projects |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo | Loaded projects |
| `Ctrl+Shift+D` | Discard all in-memory state edits | State projects |
| `Ctrl+O` | Open HOI4 mod or compatible map folder | All |
| `Ctrl+Alt+O` | Open legacy map file/archive | Province projects |
| `Ctrl+S` | Transactional State Save when eligible; legacy map Save otherwise | Contextual |
| `H` | Reset zoom | All |

Global map and tool shortcuts are suspended while a text field, numeric field,
search picker, New State dialog, or other modal input owns focus.

The compact **Map View: _current view_** selector exposes the same seven
actions and marks the active entry. The main **View** menu contains the same
Map Views plus overlay, panel, definitions, zoom, and Image Overlay
configuration actions.

## Download

Preview builds are intended to be published through GitHub Releases as a
Windows x64 portable ZIP:

```text
HOI4-Map-Editor-v0.1.0-windows-x64.zip
HOI4-Map-Editor-v0.1.0-windows-x64.zip.sha256
```

The current repository prepares these artifacts but does not publish a
release automatically. When a preview is available, download both files from
GitHub Releases and optionally verify the ZIP with:

```powershell
Get-FileHash .\HOI4-Map-Editor-v0.1.0-windows-x64.zip -Algorithm SHA256
```

## Portable installation

1. Download the Windows x64 ZIP.
2. Extract it into a writable folder.
3. Do not run the executable directly from inside the ZIP.
4. Start `HOI4 Map Editor.exe`.
5. Choose the root folder of the mod.

The application requires a supported 64-bit Windows installation and read
access to the mod. Applying changes also requires write permission. State
changes remain in memory until **Apply to Mod**; Review, isolated validation,
backup, transactional application, reload, verification, and rollback remain
part of that flow. Legacy Province editing uses its own geographic save path.

## Opening a mod

Choose **File → Open HOI4 Mod...** or press `Ctrl+O`, then select the mod root:

```text
mod-root/
├─ map/
│  ├─ provinces.bmp
│  ├─ definition.csv
│  ├─ heightmap.bmp      (optional overlay)
│  ├─ adjacencies.csv    (optional)
│  └─ rivers.bmp         (optional)
└─ history/
   └─ states/
      └─ *.txt
```

The loader reports detected capabilities such as Province map, State history,
Heightmap, and Definition catalog. One mod root can expose both province and
state tools; it is not split into two applications.

Only direct `.txt` files under `history/states/` are part of State Save. ZIP
input remains a legacy province-map compatibility path.

## Building from source

Install a current Rust toolchain, then run:

```sh
cargo build --release
```

To build the portable package and checksum locally:

```powershell
.\scripts\package-windows.ps1
```

The script also writes `dist/RELEASE_MANIFEST.txt` with the package version,
size, and SHA-256. Operational documentation is available in the
[user guide](docs/USER_GUIDE.md), [known limitations](docs/KNOWN_LIMITATIONS.md),
[troubleshooting guide](docs/TROUBLESHOOTING.md), [privacy statement](docs/PRIVACY.md),
and [release checklist](docs/RELEASE_CHECKLIST.md).

The executable remains `hoi4_state_editor` under `target/release` for technical
compatibility. The public product name and window title are HOI4 Map Editor
v0.1.0.

## Known limitations

- Autosave, HOI4 launch/testing, graphical backup restore, and advanced backup
  retention are not implemented.
- Public preview packages currently target Windows x64 only.
- Country colour metadata is not always available; Political view then uses a
  stable fallback derived from the owner tag.
- State projects do not edit geographic pixels or open mod ZIPs.
- Localised picker display names and Phase 5B sprite/`.gfx`/`.dds` resolution
  are not implemented.
- The crate, executable, repository URL, backup directory, and some internal
  types retain legacy technical names to avoid breaking paths and scripts.
- Crash logs are local-only under `%LOCALAPPDATA%\HOI4MapEditor\logs`; there is
  no telemetry or automatic upload.

## Project history and credits

Based on and forked from ScottyThePilot's HOI4 Province Editor. The repository
history, original credits, and MIT licence are preserved.

Developed and extended by Adrian Costa. HOI4 Map Editor is an unofficial
community tool and is not affiliated with or endorsed by Paradox Interactive.

Bundled third-party assets retain their original notices:

- [Tabler Icons](https://github.com/tabler/tabler-icons)
- [css.gg](https://github.com/astrit/css.gg)
- Inconsolata under the SIL Open Font License in
  `assets/Inconsolata-OFL.txt`

## Development roadmap

Core complete:

- Phase 0
- Phase 1A
- Phase 1B
- Phase 2A
- Phase 2B
- Phase 3A
- Phase 3B
- Phase 3C
- Phase 3D
- Phase 4A
- Phase 4B
- Phase 4C

UX refinement in progress:

- Phase 5A
- Phase 5A.1
- Phase 5A.1.1
- Phase 5A.1.2
- Phase 5A.1.3

Release foundation:

- Phase 11A preview packaging and Windows CI foundation

Phase 5B is future work and is not claimed as complete.

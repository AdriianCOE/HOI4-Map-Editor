# User guide

## Open a mod

Use **File → Open HOI4 Mod...** and select the mod root, not its `map` or
`history` subfolder. A detected province map enables the Provinces workspace;
`history/states/*.txt` also enables the States workspace.

## Workspaces and Map Views

A Workspace chooses what you edit. A Map View changes only the rendered base
map. Borders, labels, rivers, adjacencies, and Image Overlay are independent
visual layers and never mark a project as modified.

## Province workspace

- **Save Current Workspace** runs **Save Province Map**.
- **Export Province Map As...** creates a folder copy.
- **Export Province Map Archive...** creates a ZIP copy.

Province Save prepares, validates, and backs up complete BMP/CSV candidates
before replacing the mod files atomically. Exports never change the current
project or its modified indicator.

## States workspace

Select a state from the map or Inspector. Use the Inspector for properties,
owner/controller, cores, claims, resources, buildings, and victory points.
Brush, Lasso, and Fill preview province-level changes in memory. `Esc` cancels;
`Enter` confirms an applicable preview.

Apply State Changes regenerates a stale preview, validates a temporary copy,
creates a backup, commits, reloads, and verifies. `ReviewRequired` needs
explicit review; `Blocked` changes cannot be applied.

## Settings and project configuration

**Edit → Settings...** controls the UI language, last-project behavior,
remembered Workspace/Map Views/overlays, tooltip delay, undo history, and Undo
view behavior. Press `Tab` or the arrow keys to move, `Left`/`Right` to adjust
numeric choices, `Enter` to activate, and `Esc` to cancel. Settings use a draft:
nothing is written before **Save**.

Global settings are optional and created only after an explicit Settings save:

```text
%APPDATA%\HOI4MapEditor\config.toml
```

**File → Project Settings...** is available for a loaded mod. It controls
Province ID preservation, coastal calculation on Province Save, extra map
warnings, and their threshold. Project settings are optional and created only
after an explicit save:

```text
<mod>/.hoi4-map-editor/project.toml
```

Built-in terrains work without a file. A project file may override them or add
custom terrains:

```toml
schema-version = 1

[terrain.volcanic]
color = [55, 44, 33]
type = "land"
```

Valid types are `land`, `sea`, and `lake`; every RGB component must be 0–255.
Unknown keys and comments are preserved when known settings are changed.
Invalid files are never overwritten without explicit restore confirmation and
a `.bak` backup. Saving either settings domain does not save maps, apply states,
clear dirty state, or create an Undo command.

The UI supports `en-US` and `pt-BR`. This setting translates the editor only;
technical HOI4 identifiers and mod content are not translated.

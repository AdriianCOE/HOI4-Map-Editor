# User guide

## Open a mod

Use **File → Open HOI4 Mod...** and select the mod root, not its `map` or
`history` subfolder. A detected province map enables the Provinces workspace;
`history/states/*.txt` also enables the States workspace.

## Workspaces, Map Views, and Overlays

A Workspace chooses what you edit. A Map View changes only the rendered base
map. Borders, labels, rivers, adjacencies, and Image Overlay are independent
visual layers and never mark a project as modified.

The shell keeps these decisions separate: the top menu contains global
commands, the workspace bar shows the active domain, tool, selection, and
unsaved-work summary, the left rail contains the current domain's tools, and
the Inspector and Problems panels remain supporting views over the map. Hover
controls for a short explanation and any available shortcut.

**Country Borders** is a read-only overlay. It highlights boundaries between
adjacent land provinces when their effective State owners differ. It changes no
selection, project data, dirty state, or save behavior.

**Political** is a read-only Map View. It uses country colors, localized names,
and flags resolved from the local mod/base-game installation when available.

**Resources** is a read-only overlay for fully loaded State projects. It
visualizes the current working State resource quantities over the selected base
map, so inspector edits appear before Save Project. Resource icons are loaded
from the local mod/base-game installation; unresolved or custom icons use a
textual fallback.

**Strategic Regions** opens in read-only selection mode. It colors working
memberships, draws boundaries, and opens the selected region in the Strategic
Regions panel. In this map view, enable **Edit Strategic Regions** and choose
Select, Brush, Fill, or Lasso to assign whole Provinces to the selected region.
Brush and Lasso collect a temporary deterministic set, and Fill traverses the
geographic Province adjacency graph; each completed operation is one undoable
in-memory transaction. Middle-click picks only a unique working membership.

No Strategic Region file changes until **Save Project**. Mutation stays
disabled for incomplete data, duplicate region IDs, unsafe comment-protected
fields, and archive-backed sources that cannot be safely changed. `Esc`, a
tool change, a map-view change, or a project switch discards any active draft.

When Strategic Region loading is incomplete, the panel states how many
effective files loaded and directs you to **Project Problems** for the source.
Known memberships still keep their normal region color. Gray hatching means
unknown coverage, amber hatching means unassigned under complete coverage, and
magenta hatching means multiple known memberships. These are diagnostics only;
they never change mod data.

**Manpower** uses a sequential blue-to-yellow scale with a robust 90th-percentile
upper bound, so one outlier does not flatten the rest of the map. The legend
shows zero, medium, high, and missing values. **State Category** assigns a
stable high-contrast categorical palette to the categories present in the
loaded project and lists them in its map legend.

## Province workspace

- **Save Project** reviews and saves pending province-map and state changes
  together.
- **Export Province Map As...** creates a folder copy.
- **Export Province Map Archive...** creates a ZIP copy.

When Save Project includes province-map changes, it prepares, validates, and
backs up complete BMP/CSV candidates before the coordinated commit. Exports
never change the current project or its modified indicator.

## States workspace

Select a state from the map or Inspector. Use the Inspector for properties,
owner/controller, cores, claims, resources, buildings, and victory points. Use
the remove control beside a Core or Claim tag to remove that individual tag;
the change remains undoable until Save Project.

The **History** tab uses the earliest valid bookmark date found through the
active project source graph (mod, then compatible lower sources). It applies
the editor-supported dated history commands only up to that initial date and
labels owner/controller as declared, dated, or implicit from owner. If no
bookmark can be resolved, the Inspector explicitly reports the direct-history
fallback and does not guess a date. Dated source blocks are never flattened.
The save planner protects only edits whose values conflict with applicable
dated history: Owner and Controller, Cores and Claims by country tag, Victory
Points by province, and buildings by their State or Province key. Unrelated
direct values remain editable. The Inspector marks the selected State as
dated-history protected and affected State or Province drafts explain that the
displayed values may be effective dated values the editor cannot safely
rewrite.
Brush, Lasso, and Fill preview province-level changes in memory. `Esc` cancels;
`Enter` confirms an applicable preview.

View Changes summarizes pending working edits without saving. Save Project
regenerates stale previews, validates and round-trips a temporary copy, creates
backups and a durable journal, waits for explicit final confirmation, commits
the affected files, reloads, and verifies. `ReviewRequired` needs explicit
review; `Blocked` changes cannot be applied. Individual replacements are
staged safely, but this preview does not claim project-wide filesystem
atomicity across all files and platforms.

## Settings and project configuration

**Edit → Settings...** controls the UI language, last-project behavior,
remembered Workspace/Map Views/overlays, tooltip delay, undo history, and Undo
view behavior. Press `Tab` or the arrow keys to move, `Left`/`Right` to adjust
numeric choices, `Enter` to activate, and `Esc` to cancel. Settings use a draft:
nothing is written before **Save**.

### Changing the interface language

Open **Edit → Settings...**, select the **Language** row, and press
`Enter`/`Left`/`Right` to cycle between English, Português do Brasil,
Español, Français, Русский, and 简体中文 (shown by their native names, not
language codes). The interface updates immediately so you can preview a
language before saving. Press **Save** to keep it, or **Cancel** to revert.
The choice persists in `config.toml` and applies the next time the editor
opens. Technical paths, file names, HOI4 identifiers, and mod content are
never translated.

Global settings are optional and created only after an explicit Settings save:

```text
Windows: %APPDATA%\HOI4MapEditor\config.toml
Linux:   $XDG_CONFIG_HOME/HOI4MapEditor/config.toml
         (or ~/.config/HOI4MapEditor/config.toml)
```

**File → Project Settings...** is available for a loaded mod. It controls
Province ID preservation, coastal calculation when Save Project includes
province-map changes, extra map warnings, and their threshold. Project
settings are optional and created only after an explicit save:

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

## Logs and reporting a problem

Local logs are written under:

```text
Windows: %LOCALAPPDATA%\HOI4MapEditor\logs
Linux:   $XDG_STATE_HOME/HOI4MapEditor/logs
         (or ~/.local/state/HOI4MapEditor/logs)
```

If something goes wrong, include the application version (see **Help → About
HOI4 Map Editor**), the exact error text, the relevant log lines, and a
minimal reproducer if you have one. Do not share your whole mod, backup
directory, usernames, or credentials. Report issues on the project's GitHub
repository.

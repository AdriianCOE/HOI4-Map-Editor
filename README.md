# HOI4 State Editor

Graphical state editor for Hearts of Iron IV mods.

This project is an early-stage fork of
[ScottyThePilot/hoi4_province_editor](https://github.com/ScottyThePilot/hoi4_province_editor).
It still inherits most of the original Province Editor, including BMP/CSV
loading, OpenGL rendering, camera controls, province picking, borders, lasso,
selective texture updates, diagnostics, and undo/redo.

The new editor opens a mod root and uses:

```text
mod-root/
├─ map/
│  ├─ provinces.bmp
│  ├─ definition.csv
│  ├─ adjacencies.csv  (optional)
│  └─ rivers.bmp       (optional)
└─ history/
   └─ states/
      └─ *.txt
```

The intended data flow is:

```text
provinces.bmp -> RGB -> definition.csv -> province ID -> state
```

For state projects, the geographic map is a read-only visual base.
`history/states/*.txt` is parsed and indexed without being modified.
State rendering and selection use an in-memory working model. Province-to-state
associations and selected state properties can be changed temporarily,
inspected, undone, redone, or discarded. States can also be created or removed
from the current session, but they are not serialized and no mod file is
created, deleted, renamed, or written.

## Current status

- Mod-root discovery and validation are implemented.
- Province map loading and rendering remain inherited.
- State-domain models are prepared without a fake parser.
- State files are discovered deterministically and parsed into a generic
  PDXScript syntax tree with source spans and preserved trivia.
- Typed state data, state IDs, province assignments, diagnostics, and loading
  summaries are built without changing the map texture.
- A cached state-map view assigns deterministic colors by state ID, draws
  inter-state borders, highlights diagnostic provinces, and supports
  read-only state selection and inspection.
- The state view supports in-memory transactional reassignment of selected
  land provinces to an existing target state, plus explicit unassignment,
  undo, redo, and discard.
- A state-specific polygon lasso selects whole land provinces without painting
  pixels. It provides Replace, Add, and Remove modes; Centroid, Any
  Intersection, and Majority inclusion criteria; and a cached preview that
  must be confirmed before the edit selection changes.
- Keys `7` and `8` switch between province and state views.
- In state view, Ctrl+click toggles province selection for the edit session,
  normal click selects the target state, the Edit menu can select every
  province in that target, `M` moves the selected provinces to it, `Delete`
  unassigns them, `Esc` clears the current edit selection, and
  `Ctrl+Shift+D` discards the session.
- In state view, `L` starts the state lasso. Click adds map-anchored polygon
  points, clicking the first point or pressing `Enter` creates the preview,
  another `Enter` confirms only the selection, and `Esc` cancels drawing or
  preview. Shift selects Add mode and Alt selects Remove mode. The State Lasso
  menu exposes the same controls and inclusion criteria.
- Confirming a lasso never marks the project dirty and never enters edit
  history. A later Move or Unassign passes the confirmed province IDs to the
  same transactional command used by Ctrl+click selection, producing one
  undo entry for the whole batch.
- The Edit menu opens an explicit temporary property draft for a valid selected
  state. General values, owner/controller, cores, claims, resources, and
  state-level buildings are validated and enter the working session only after
  `Apply to session`. `Discard draft` drops typing without changing history.
- Each property Apply is one command in the same ordered undo/redo history as
  Move and Unassign. Draft typing is not dirty; confirmed working values are
  compared with the immutable baseline, and global discard restores both
  province assignments and properties.
- A separately tracked active province can open a temporary provincial-data
  draft. Victory points and custom province buildings are validated and
  applied together as one command in the same history. Those confirmed values
  follow the province through Move and Unassign without regenerating the map.
- `Edit > New State` creates either an empty state or a state containing the
  current province selection. IDs are suggested deterministically and rejected
  when occupied or reserved. The state exists only in the working session and
  has no backing file.
- `Edit > Remove state from session` either moves all provinces atomically to
  another active state or leaves them temporarily unassigned. Loaded states
  become session tombstones; undo restores their properties, victory points,
  province buildings, target eligibility, and visual state.
- Create and Remove each produce exactly one command in the unified history.
  Redo keeps created IDs reserved, and global discard restores the loaded
  baseline with zero created or removed states.
- Geographic painting, recoloring, metadata changes, adjacency changes, and
  map saving are blocked when a state project is open. `Ctrl+S` remains
  intentionally blocked for state projects because the serializer is not part
  of this phase. The property editor never uses a Save action.
- Opening a direct `map/` folder or map ZIP remains available as an explicitly
  legacy, editable compatibility mode.

See [Architecture](docs/ARCHITECTURE.md) and
[Migration plan](docs/MIGRATION_PLAN.md).

## Safety

This is experimental software. Keep backups of every mod before opening or
editing it. The state-project path does not save `provinces.bmp`,
`definition.csv`, `adjacencies.csv`, `rivers.bmp`, or state files in this
phase. State edits are held only in memory until the edit session is discarded
or the application is closed.

## Building

1. Install a current Rust toolchain.
2. Clone this repository.
3. Run `cargo build --release`.
4. Find the executable under `target/release`.

## License and credits

The project remains licensed under the MIT License. Original copyright,
history, and credit belong to ScottyThePilot and the contributors to
`hoi4_province_editor`.

Bundled icons/assets also retain their original licenses and credits:

- [Tabler Icons](https://github.com/tabler/tabler-icons)
- [css.gg](https://github.com/astrit/css.gg)

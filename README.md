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
from the current session. A disposable patch preview can show the exact
span-based changes and canonical content planned for new states. That plan can
also be applied and reloaded inside an isolated temporary workspace for
round-trip validation. A current fully Safe plan whose exact validation status
is `Passed` can then be saved transactionally after explicit confirmation.

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
- The State Brush is separate from geographic painting. In state view, `B`
  activates Assign mode for the current target state, and the State Brush menu can
  activate Assign or Unassign. The brush samples cursor movement in map
  coordinates, previews whole province IDs, and applies only on mouse release.
  Each stroke produces at most one `ReassignProvinces` command; no-op provinces
  are skipped, sea/lake/ID-zero provinces are ignored, and ambiguous or invalid
  states are blocked.
- State Brush assignment and unassignment reuse the same province reassignment
  command as Move/Unassign, so victory points and province buildings follow the
  province and Undo/Redo restore the full stroke atomically.
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
- The Patch Preview menu derives a `ProjectPatchPlan` from the immutable
  baseline versus the current working state. Existing files use byte-span
  Replace/Insert/Delete operations over a copy of their exact original bytes;
  new states use an in-memory canonical renderer; removed states produce only
  planned removals. Each file is marked Safe, Review required, or Blocked and
  includes semantic changes, operations, diagnostics, and a textual diff.
- Preview output is parsed and compared semantically before it can be marked
  safe. Lossy UTF-8, changed source files, duplicate authoritative bindings,
  ambiguous dated history, overlaps, and unsafe fragment transfers block the
  affected file. Any edit, Undo, Redo, or Discard makes an older preview stale.
- The Patch Preview menu can validate a current Safe plan in an isolated
  workspace under the system temporary directory. It copies the two map inputs
  and every direct `history/states/*.txt` file as real bytes, applies the plan
  only to the candidate copy, reloads it through the normal map and state
  loaders, and compares the full semantic model, indexes, province coverage,
  structural diagnostics, and bytes.
- Validation rejects stale, Blocked, unsafe, colliding, or externally changed
  plans before creating the workspace. ReviewRequired plans need the explicit
  review action and can finish only as `PassedWithReview`. The default policy
  removes temporary workspaces after pass, failure, or cancellation; an
  opt-in diagnostic policy may retain a failed workspace and reports its exact
  path.
- State Save is authorized only by the exact current plan and its exact current
  `Passed` report. `PassedWithReview`, stale results, drafts, active
  lasso/brush interactions, externally changed sources, net-zero plans, and
  interrupted transactions remain ineligible.
- `Ctrl+S` and `Patch Preview > Save state files` show the operation counts,
  project root, planned backup location, and a warning before changing state
  files. `Save As` remains unavailable for state projects.
- Each confirmed Save acquires an exclusive
  `.hoi4-state-editor/save.lock`, persists a deterministic journal, copies and
  verifies every modified/removed source in a timestamped backup, writes and
  verifies same-directory stage files, then commits modified, created, and
  removed paths in deterministic order.
- Existing files are renamed to transaction-specific rollback siblings before
  replacement/removal. Those rollback files remain until a real project reload
  and global semantic, index, coverage, victory-point, building, diagnostic,
  byte, and map-input comparison succeeds.
- Failures after commit begins trigger reverse journal-driven rollback.
  Incomplete rollback keeps the lock, journal, backup, and critical report.
  On the next open, an interrupted lock blocks editing until the explicit
  recovery action performs a verified rollback.
- A successful post-save reload becomes the new edit baseline with empty
  Undo/Redo and dirty state. Backups and transaction reports remain; stage and
  rollback siblings are removed.
- Geographic painting, recoloring, metadata changes, adjacency changes, and
  map saving are blocked when a state project is open. State Save touches only
  direct `history/states/*.txt` paths; the property editor never uses a Save
  action.
- Opening a direct `map/` folder or map ZIP remains available as an explicitly
  legacy, editable compatibility mode.

See [Architecture](docs/ARCHITECTURE.md) and
[Migration plan](docs/MIGRATION_PLAN.md).

## Safety

This is experimental software. Keep independent backups of every mod.
Round-trip validation still writes only to a newly created directory under the
system temporary directory. State Save is a separate, explicitly confirmed
operation and writes only the Safe direct `history/states/*.txt` operations
that passed the exact current validation. It never saves `provinces.bmp`,
`definition.csv`, `adjacencies.csv`, or `rivers.bmp`.

Verified backups, journals, and reports are stored under
`<mod>/.hoi4-state-editor/`. Same-directory temporary files use suffixes after
`.txt` (`.hse-stage-*` and `.hse-rollback-*`) so HOI4 does not interpret them
as states. Backups are not removed automatically.

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

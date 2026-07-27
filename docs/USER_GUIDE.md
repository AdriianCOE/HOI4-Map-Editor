# User guide

## Open a mod

Use **File → Open HOI4 Mod...** and select the mod root, not its `map` or
`history` subfolder. A detected province map enables the Provinces workspace;
`history/states/*.txt` also enables the States workspace.

Keep an independent backup. Province-map saving and transactional State Apply
are separate systems.

## Workspaces and Map Views

A **Workspace** chooses what you edit: Provinces or States. A **Map View**
changes only map colouring. Overlays such as borders, labels, rivers,
adjacencies, and Image Overlay are read-only layers over the current view.

## Province workspace

The legacy tools edit geographic province data. Brush, fill, lasso, coastal,
adjacency, and diagnostic actions can affect map files.

- **File → Save Current Workspace** runs **Save Province Map** and writes the
  current mod's geographic files.
- **Export Province Map As...** creates a folder copy.
- **Export Province Map Archive...** creates a ZIP copy.

Exports never change the open project or clear its modified indicator. Province
saves prepare and validate complete files before replacing the originals.

## States workspace

Select a state from the map or Inspector. Create a state from the current
province selection or create it empty, then use:

- **Inspector** for properties, owner/controller, cores, claims, resources,
  state buildings, victory points, and province buildings;
- **Brush** to assign every touched land province in one transaction;
- **Lasso** to replace, add, or remove province selections;
- **Fill** to preview and assign a connected region.

Brush, Lasso, and Fill preview in memory. `Esc` cancels; `Enter` confirms
applicable previews. Undo, Redo, and Discard operate on the in-memory session.

## Image Overlay

Load BMP, PNG, JPG, or JPEG through the View controls. The image must have the
same dimensions as `provinces.bmp`. It follows map pan and zoom but is never
written to the mod.

## Review and Apply

Use this sequence:

```text
Edit in memory
→ Review State Changes
→ Validate in Temporary Workspace
→ Apply State Changes with Backup
→ Reload and Verify
```

Apply State Changes automatically regenerates a stale preview and runs the required temporary
validation. `ReviewRequired` changes need the explicit **Validate and Continue**
action and a current `PassedWithReview` result. `Blocked` changes cannot be
applied. State backups, journals, and recovery records are stored under
`<mod>/.hoi4-state-editor/`.

If an interrupted transaction is detected, complete recovery before editing
again. Keep the recovery directory until the project reloads successfully.

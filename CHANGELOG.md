# Changelog

## v0.1.0-preview.4 - Unreleased

### Added

- Political View with country colors, localized country names, runtime flags,
  mod-over-base color resolution, and deterministic fallbacks when metadata is
  incomplete.
- Resources overlay for fully loaded State projects, showing working State
  resource quantities, runtime resource icons, and textual fallback for missing
  or custom icons.
- Linux x86_64 preview support, including XDG configuration/log locations,
  desktop integration fallbacks, CI, and a portable Linux package path.
- Editor support for custom and small map dimensions without vanilla-size or
  64-multiple assumptions.
- Editor support for sparse Province IDs: existing IDs and gaps are preserved,
  new IDs are allocated after the current maximum, and IDs are not compacted
  automatically.
- Improved State compatibility for real-world mods, including high/sparse
  State IDs, `State={...}` blocks, and integral decimal values such as
  `steel = 20.000`.
- View Changes and Project Problems surfaces for reviewing pending edits and
  current diagnostics without saving.
- Province assignment to States before first Save, including stable new
  Province IDs and unambiguous split inheritance.

### Changed

- Project opening defers Political and Resources presentation work so normal
  project load does not eagerly pay those view costs.
- Save Project prepares, validates, round-trips, and verifies coordinated
  candidates before the explicit final write.
- Save blockers and project diagnostics are easier to separate from existing
  warnings and to navigate when an actionable target exists.
- Coastal Province flags are recalculated during Save Project preparation after
  relevant Province geometry changes.
- Newly created Province IDs are stable immediately and use max existing
  Province ID + 1.
- Project switching replaces project-scoped map/session/cache state instead of
  reusing data from a previously opened project.
- Resources is presented as an overlay rather than as a standalone Map View.
- Cross-platform paths, clipboard handling, system-font fallback discovery, and
  Unix Save Project durability barriers were tightened for preview use.

### Fixed

- Political color parsing now uses `common/countries/colors.txt` and country
  `color` entries correctly, with localized ideology-name fallback.
- Create Province candidate/index handling no longer causes a round-trip
  mismatch before Save Project.
- Save Blocked action routing no longer reaches Project Saved from a
  view/details action.
- Missing or incomplete State loading no longer collapses into misleading
  secondary Unassigned diagnostics alone.
- State files using `State={...}` capitalization and integral decimal resource
  values are accepted.
- Small-map diagnostics no longer assume vanilla HOI4 dimensions.
- Save and reload no longer compact sparse Province IDs.
- Adjacency lookup no longer assumes dense Province IDs.
- Stale map, river, Political, Resources, validation, selection, and save
  baseline data are cleared when switching projects.

### Limitations

- The Linux preview targets Ubuntu 22.04-compatible x86_64 glibc desktops with
  an OpenGL-capable X11 or Wayland desktop stack; ARM, musl, older glibc, and
  every-distribution support are outside this preview baseline.
- Political and Resources presentation are read-only. The first activation may
  still perform a short one-time runtime asset load.
- Sparse Province ID support is an editor-side guarantee; this changelog does
  not claim that every sparse-ID layout is valid in the HOI4 engine.
- Custom and small map support means the editor avoids vanilla-size
  assumptions; it does not guarantee every arbitrary map configuration is valid
  in-game.
- Adjacencies can be inspected and preserved but do not yet have an editing UI.
- Merge Brush, direct resource painting, Country editing, Strategic Regions,
  rivers, heightmaps, supply networks, and tree maps do not yet have complete
  dedicated editing support.

## v0.1.0-preview.2 - Unreleased

### Added

- Unified Save Project review and coordinated Province Map/State transaction.
- Reusable Validation Core with stable diagnostic codes and combined-candidate
  Province, State, and cross-domain checks.
- Tools → Validate Project with a basic results window, map navigation, and
  source-file opening.
- Combined backup, durable journal, verified rollback, and interrupted-save
  recovery.

### Changed

- Ctrl+S now reviews all pending project changes regardless of workspace.
- Province and State candidates are generated and validated together before
  the first write to the mod.
- Dirty baselines are promoted only after reload and full verification.

### Fixed

- Province dirty state now reflects differences from the loaded baseline,
  including Undo and Redo.
- First-run and partially supported project messages are clearer.
- New project-save and validation strings are present in all six bundled
  language catalogs.

No public release, tag, push, or Workshop update has been performed.

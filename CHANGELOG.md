# Changelog

## v0.1.0-preview.3 - Unreleased

### Added

- Linux x86_64 preview support, including XDG configuration/log locations,
  desktop integration fallbacks, CI, and a portable Linux package.
- Support for custom and small map dimensions in the editor.
- Stable sparse Province IDs: existing IDs and gaps are preserved, and new IDs
  are allocated after the current maximum without automatic compaction.
- Compatibility diagnostics for map/definition data, Province references,
  unusual dimensions, and project structure.

### Improved

- Cross-platform paths, clipboard handling, and system-font fallback discovery.
- State, adjacency, validation, Save Project, reload, rollback, and recovery
  handling for sparse Province IDs.
- Unix Save Project durability barriers for critical files and directory entries.
- Windows/Linux CI and audit-friendly Windows ZIP and Linux tar.gz packaging.

### Fixed

- Small-map diagnostics no longer assume vanilla HOI4 dimensions.
- Save and reload no longer compact sparse Province IDs.
- Adjacency lookup no longer assumes dense Province IDs.

### Limitations

- The Linux preview targets x86_64 glibc desktops built and tested on Ubuntu
  22.04; ARM, musl, and older glibc environments are not supported.
- Adjacencies can be inspected and preserved but do not yet have an editing UI.
- Strategic Regions, Continents, rivers, heightmaps, supply networks, and tree
  maps do not yet have complete dedicated editing support.

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

# Changelog

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

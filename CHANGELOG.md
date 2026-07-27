# Changelog

## Unreleased

### Added

- Province editing inherited from HOI4 Province Editor.
- State Workspace with States and Political views.
- State Inspector, Lasso, Brush, Fill, Image Overlay, creation, removal, and property editing.
- Lossless state patch preview and isolated round-trip validation.
- Transactional state save with backup, rollback, and interrupted-save recovery.
- Windows x64 allowlist packaging with checksum and release manifest.
- User guide, known limitations, troubleshooting, privacy, and release checklist.

### Changed

- The public product name is HOI4 Map Editor.
- Provinces and States use explicit workspaces with independent remembered Map Views.
- The State Inspector is an overlay drawer and no longer resizes the map viewport.
- Large inherited map samples were replaced with minimal artificial fixtures.
- Release workflows now use read-only permissions and credential-free checkout.

### Fixed

- State creation clears temporary province/tool selections only after a successful transaction.
- Focused inputs and modal controls suppress global map and workspace shortcuts.
- Package output path validation no longer accepts sibling directories sharing
  the repository name prefix.

### Known Issues

- Adjacencies, Strategic Regions, Continents, Rivers, and Supply Network do not yet have complete dedicated editors.
- Localised picker labels and `.gfx`/`.dds` sprite resolution are not implemented.
- Real-mod smoke tests require a local controlled fixture and do not run in public CI.

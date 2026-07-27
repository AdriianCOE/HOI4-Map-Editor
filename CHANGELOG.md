# Changelog

## [0.1.0-preview.1] - 2026-07-27

First public preview of HOI4 Map Editor: a standalone tool for editing Hearts
of Iron IV province maps and state history files. This is preview software —
keep an independent backup of your mod.

### Province editing

- Color editing, terrain/definition editing, and coastal recalculation.
- Atomic Province Map Save: BMP/CSV candidates are prepared, validated, and
  backed up before the mod files are replaced.
- Export Province Map as a folder or ZIP archive without touching the open
  project.
- Undo and Redo across province edits.
- Contextual search with focused navigation to a province on the map.

### State editing

- State creation and removal, and province-to-state assignment.
- State Brush, Lasso, and Fill tools for assigning or unassigning provinces.
- Owner, controller, colors, cores, claims, resources, buildings, and victory
  points editing in the State Inspector.
- Review State Changes → Validate in a temporary workspace → Apply State
  Changes with backup, rollback, and interrupted-save recovery.
- Isolated round-trip validation before any state file is written.

### Interface

- Separate Provinces and States workspaces with independent remembered Map
  Views and overlays.
- Map Views (Province Colors, Province Types, Terrain, Continents, Coastal,
  States, Political) and overlays (Rivers, Adjacencies, Province/State
  Borders, Image Overlay).
- State Inspector as an overlay drawer that no longer resizes the map
  viewport.
- Application Settings and Project Settings, including a language selector.
- Six interface languages: English, Português do Brasil, Español, Français,
  Русский, and 简体中文. Missing translations fall back to English rather
  than showing a blank or broken label.
- Contextual search with focused map navigation.

### Known limitations

- Windows x64 only.
- Game localization and metadata (province/state names from HOI4's own
  localization files) are not yet loaded; the editor shows raw identifiers.
- Game flags and icons (`.gfx`/`.dds`) are not yet loaded.
- Adjacencies does not yet have a complete dedicated editing workspace.
- Strategic Regions and Continents do not yet have dedicated workspaces.
- This is preview software: keep an independent backup of your mod
  regardless of the safeguards above.

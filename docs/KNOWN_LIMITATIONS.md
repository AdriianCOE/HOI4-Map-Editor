# Known limitations

- Transactional Apply currently covers direct `history/states/*.txt` files.
  The legacy geographic province save flow is separate.
- Adjacencies are available as an overlay and legacy tool, not a complete
  dedicated editor.
- Real localisation resolution and Phase 5B `.gfx`/`.dds` assets are not
  implemented.
- Strategic Regions and Continents are not editable workspaces.
- There is no direct editor for heightmap, trees, rivers, or supply networks.
- State projects do not edit geographic pixels or open mod ZIPs.
- Public preview packaging targets Windows x64.
- Visual validation is still required on multiple display scales and clean
  Windows machines.
- The inherited icon spritesheet has documented attribution but lacks an exact
  upstream revision record.
- The crate, executable, repository URL, and `.hoi4-state-editor` transaction
  directory retain legacy technical names for compatibility.

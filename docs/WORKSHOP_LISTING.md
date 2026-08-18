# Steam Workshop listing draft (not published)

Reference copy for whoever publishes this item to the Steam Workshop. This
file is not used by the build or the packager; it is prepared text only.

## Short description (blurb, ~300 chars)

Unofficial standalone editor for HOI4 province maps and state history
files. Province colors/terrain, state creation and properties, validated
coordinated saves with backup/rollback, and a 6-language interface.
Preview software - keep an independent backup of your mod.

## Long description

HOI4 Map Editor is an unofficial, independently developed tool for editing
Hearts of Iron IV province maps (`provinces.bmp` / `definition.csv`) and
state history files (`history/states/*.txt`). It is a **standalone
application** - it does not need to be enabled in a playset, and it does
not modify the game itself.

**Features**

- Province brush, fill, lasso, recolor, terrain/definition editing, and
  coastal recalculation.
- Coordinated Save Project: province-map and state candidates are validated
  and backed up before the mod's files are replaced.
- State creation, removal, and province assignment via Brush, Lasso, and
  Fill.
- State Inspector for owner, controller, colors, cores, claims, resources,
  buildings, and victory points.
- Review Project Changes -> validate in a temporary workspace -> Save Project
  with backup, rollback, and interrupted-save recovery.
- Contextual search with focused map navigation.
- Six interface languages: English, Portugues do Brasil, Espanol, Francais,
  Russkiy, and Simplified Chinese.

**How to run it**

After subscribing, open this item's folder in your Steam Workshop content
directory and run `HOI4 Map Editor.exe` directly. It is not loaded by the
game and needs no playset entry.

**Known limitations**

- The Workshop staging flow currently packages the Windows executable only.
  Linux x86_64 previews are distributed as portable tarballs through GitHub
  Releases, not through Workshop content.
- Game localization/metadata, flags, and icons (`.gfx`/`.dds`) are not yet
  loaded; the editor shows raw identifiers.
- Adjacencies, Strategic Regions, and Continents do not yet have complete
  dedicated editing workspaces.

**This is preview software.** Keep an independent backup of your mod,
separate from the automatic backups the editor creates during a save.

Source and releases: https://github.com/AdriianCOE/HOI4-Map-Editor

Unofficial community project. Not affiliated with or endorsed by Paradox
Interactive. No Hearts of Iron IV or Paradox assets are distributed.

## Suggested tags

`Utilities`

(HOI4 Workshop tooling is generally limited to gameplay-content tags;
`Utilities` is the closest fit for a non-gameplay standalone tool. Confirm
against the current Workshop tag list at publish time.)

## Changelog (for the Workshop "Change Notes" field)

Copy the matching release section from `CHANGELOG.md` at publish time.

## Screenshot checklist (none captured yet - required before publishing)

- [ ] Provinces workspace, Province Colors view, a province selected with
      the Inspector visible.
- [ ] States workspace, Political view, a state with owner/controller/cores
      set, State Inspector open.
- [ ] Review Project Changes / Save Project dialog mid-flow.
- [ ] Settings dialog with the Language row showing a non-English language
      selected (demonstrates the 6-language UI).
- [ ] Search with a focused province result.

All screenshots must come from a test/fixture mod, never from a real
in-progress mod, and must not include any Paradox Interactive proprietary
map art, flags, or game assets beyond what the user's own test mod already
legally includes.

## Before publishing (do not skip)

- Steam Workshop credentials available and the publishing user is signed
  in.
- Explicit confirmation from the project owner to publish (not just to
  prepare).
- Real screenshots captured per the checklist above.
- `thumbnail.png` reviewed and approved (currently a placeholder original
  icon - see `scripts/workshop/thumbnail.png`).
- The downloaded-from-Steam copy of this package tested end-to-end after
  upload, not just the local staged folder.

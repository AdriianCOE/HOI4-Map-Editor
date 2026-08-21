# Linux packaging

The first official Linux preview format is a portable `tar.gz`, not AppImage,
Flatpak, Snap, deb, rpm, or AUR metadata. It is intentionally easy to inspect
and extract before testing on a clean machine.

## Support baseline

The supported baseline is an Ubuntu 22.04-compatible x86_64, glibc-based Linux
desktop. It needs an OpenGL-capable graphics stack and the X11 and/or Wayland
runtime libraries selected by the current glutin/winit stack. This is not a
claim of support for every Linux distribution: musl systems, ARM, and older
glibc environments are outside this first preview baseline.

## Build and validate

Run this on Linux from a clean checkout:

```sh
bash scripts/package-linux.sh
```

The script runs an isolated `cargo build --release --locked` with
`CARGO_TARGET_DIR=target/package-linux`, release debug information disabled, and
Rust path remapping for the repository and active home directory. It derives the
version from `cargo metadata`; do not edit a package filename manually. The
default output directory is ignored by Git (`dist/`).

For release version `0.1.0-preview.4`, the outputs are:

```text
dist/HOI4-Map-Editor-v0.1.0-preview.4-linux-x86_64.tar.gz
dist/HOI4-Map-Editor-v0.1.0-preview.4-linux-x86_64.tar.gz.sha256
```

The archive contains one top-level directory with this allowlist only:

```text
HOI4-Map-Editor-v…-linux-x86_64/
  hoi4-map-editor
  LICENSE
  THIRD_PARTY_NOTICES.md
  README-LINUX.txt
  MANIFEST.txt
```

`MANIFEST.txt` records the version, target, architecture, commit when available,
payload byte sizes, and SHA-256 hashes. The adjacent checksum verifies the final
archive:

```sh
cd dist
sha256sum -c HOI4-Map-Editor-v…-linux-x86_64.tar.gz.sha256
```

The packager rejects unexpected files, missing executable permission, a
non-x86_64 ELF, ELF RPATH/RUNPATH, unresolved `ldd` dependencies, archive/list
mismatches, and common private/development paths. It never copies the repository
tree, local configurations, mods, game assets, system libraries, or system
commands into the archive.

## Runtime expectations

The executable relies on normal target-system dynamic libraries. `ldd` is run
and printed during every package build; its exact output is the audit record for
that artifact. Fundamental dependencies are the glibc/runtime libraries reported
there. Graphics/window dependencies are the OpenGL and X11/Wayland libraries
reported there. Do not bundle glibc or arbitrary shared libraries.

`xdg-open` is optional and is used for opening files/folders. Clipboard support
uses `wl-copy` on Wayland when available, then `xclip` or `xsel` for X11/XWayland.
Those commands are optional; an absent clipboard backend is reported by the app
but does not prevent startup. None is bundled.

The primary Inconsolata font is embedded in the executable and is covered by
`THIRD_PARTY_NOTICES.md`. Cyrillic/CJK fallback comes from installed system fonts
when present; Noto, DejaVu, Liberation, and WenQuanYi are never copied into the
package.

## Smoke test gate

After extracting the archive outside the repository, start
`./hoi4-map-editor` from a writable directory on a supported X11 or Wayland
desktop. Confirm the window opens and closes, Open Mod can choose a folder, and
a small test project can complete a Save Project cycle. This GUI smoke test is a
manual release gate; the packaging script deliberately does not invent a CLI or
pretend that a headless CI runner can validate desktop startup.

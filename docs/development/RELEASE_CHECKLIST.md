# Release checklist

GitHub Releases is the first preview channel. Do not publish until every
applicable item is confirmed.

## Automated CI gates

- [ ] GitHub Actions passes on the supported Windows x86_64 and Linux x86_64 runners.
- [ ] Formatting: `cargo fmt --check` passes.
- [ ] Compile check: `cargo check --locked` passes.
- [ ] Tests: `cargo test --locked` passes.
- [ ] Clippy: `cargo clippy --locked --all-targets --all-features` passes.
- [ ] Release build: `cargo build --release --locked` passes.
- [ ] Linux portable archive validation: `bash scripts/package-linux.sh` passes
      on the Ubuntu 22.04 CI runner.
- [ ] `git diff --check` passes.

CI validates committed source on Windows x86_64 and Linux x86_64 with format,
compile/check, tests, Clippy, and release-build gates. Linux CI does not imply
Linux packaging or complete desktop-runtime support, and does not replace the
manual release gates below.

## Manual build, package, and desktop gates

- [ ] The same Rust gates pass in a clean clone made from committed history.
- [ ] Portable ZIP was produced by `scripts/package-windows.ps1`.
- [ ] ZIP contains only the executable, README, LICENSE, changelog, and notices.
- [ ] Embedded `en-US`, `pt-BR`, `es-ES`, `fr-FR`, `ru-RU`, and `zh-CN`
      catalogs work without external locale files.
- [ ] Package contains no personal, project, or legacy configuration file.
- [ ] SHA-256 checksum matches the ZIP.
- [ ] `RELEASE_MANIFEST.txt` matches the ZIP name, version, size, and SHA-256.
- [ ] The ZIP was extracted outside the repository, its allowlist rechecked,
      and the executable launch-smoke-tested.
- [ ] Package contains no `.rcs`, backups, logs, absolute local paths, private fixtures, or proprietary game/mod assets.
- [ ] Version matches `Cargo.toml`, the window title, About, ZIP, checksum, and changelog.
- [ ] Portable Linux tarball was produced by `scripts/package-linux.sh`.
- [ ] Linux tarball contains only its documented allowlist and has a verified
      adjacent SHA-256 checksum and manifest.
- [ ] Linux tarball was extracted outside the repository and launch-smoke-tested
      on the Ubuntu 22.04-compatible x86_64 desktop baseline.

## Manual Windows validation

- [ ] Extract the ZIP onto a clean supported Windows x86_64 machine and launch
      the executable from a writable folder.
- [ ] Open a known mod; perform a Province edit and a State edit; validate, Save
      Project, close/reopen, and verify the result.
- [ ] Exercise a backup/recovery scenario where practical.
- [ ] Confirm logs are created under `%LOCALAPPDATA%\HOI4MapEditor\logs` and
      settings are created only after explicit Save.

## Manual Linux validation

- [ ] Extract the tarball onto a clean Ubuntu 22.04-compatible x86_64 glibc
      desktop, launch `hoi4-map-editor`, and confirm OpenGL/window initialization.
- [ ] Open a mod; perform a Province edit and a State edit; validate, Save
      Project, close/reopen, and verify the result.
- [ ] Confirm `xdg-open` works when installed and clipboard behavior is graceful
      with the available `wl-copy`, `xclip`, or `xsel` backend.
- [ ] Confirm config and logs use the documented XDG paths.
- [ ] Record whether the smoke ran under Wayland, X11/XWayland, or both. One
      available supported desktop session is sufficient for this preview gate.

## Shared manual validation

- [ ] Test with at least two independently backed-up mods.
- [ ] Review Project Changes, combined temporary validation, backup, Save
      Project, reload, rollback, and recovery are exercised.
- [ ] Province-only, state-only, and combined dirty-save flows are tested
      separately.

## Store and communication preparation

- [ ] MIT licence and ScottyThePilot credit are present.
- [ ] All screenshots and promotional assets are original or properly licensed.
- [ ] Description states that the tool is unofficial and not endorsed by Paradox Interactive.
- [ ] Known limitations and update policy are documented.
- [ ] User guide, troubleshooting, privacy, changelog, checksum, and rollback information are current.
- [ ] The inherited spritesheet provenance limitation has been resolved or accepted explicitly.
- [ ] Dependency licence metadata has received a final manual review.

Steam integration is not implemented. GitHub remains the preview distribution
channel until clean-machine and multi-mod validation are complete.

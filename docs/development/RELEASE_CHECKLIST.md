# Release checklist

GitHub Releases is the first preview channel. Do not publish until every
applicable item is confirmed.

## Build and package

- [ ] `cargo check`, tests, Clippy, and release build pass on Windows x64.
- [ ] The same gates pass in a clean clone made from committed history.
- [ ] Portable ZIP was produced by `scripts/package-windows.ps1`.
- [ ] ZIP contains only the executable, README, LICENSE, changelog, and notices.
- [ ] Embedded `en-US`, `pt-BR`, `es-ES`, `fr-FR`, `ru-RU`, and `zh-CN`
      catalogs work without external locale files.
- [ ] Package contains no personal, project, or legacy configuration file.
- [ ] SHA-256 checksum matches the ZIP.
- [ ] `RELEASE_MANIFEST.txt` matches the ZIP name, version, size, and SHA-256.
- [ ] The ZIP was extracted outside the repository and its allowlist rechecked.
- [ ] Package contains no `.rcs`, backups, logs, absolute local paths, private fixtures, or proprietary game/mod assets.
- [ ] Version matches `Cargo.toml`, the window title, About, ZIP, checksum, and changelog.

## Product validation

- [ ] Test on a clean supported Windows machine from a writable extracted folder.
- [ ] Test with at least two independently backed-up mods.
- [ ] Review Project Changes, combined temporary validation, backup, Save Project, reload, rollback, and recovery are exercised.
- [ ] Province-only, state-only, and combined dirty-save flows are tested separately.
- [ ] Logs are created under `%LOCALAPPDATA%\HOI4MapEditor\logs`.
- [ ] Global settings are created only after explicit Save and project settings
      only after explicit Project Settings Save.

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

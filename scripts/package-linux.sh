#!/usr/bin/env bash
# Build and validate the portable Linux preview archive. Nothing is published.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/package-linux.sh [--output-directory DIR] [--skip-build]

Builds the official Linux x86_64 preview archive in an ignored directory under
the repository. The script deliberately packages only an explicit allowlist.
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
output_argument="dist"
skip_build=false

while (($#)); do
    case "$1" in
        --output-directory)
            (($# >= 2)) || die "--output-directory requires a value"
            output_argument="$2"
            shift 2
            ;;
        --skip-build)
            skip_build=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

for command in cargo python3 tar gzip sha256sum file readelf ldd grep find sort stat cmp; do
    require_command "$command"
done

if [[ "$output_argument" = /* ]]; then
    output_directory="$output_argument"
else
    output_directory="$repo_root/$output_argument"
fi
mkdir -p -- "$output_directory"
output_directory="$(cd "$output_directory" && pwd -P)"
package_output_root="$repo_root/dist"
mkdir -p -- "$package_output_root"
package_output_root="$(cd "$package_output_root" && pwd -P)"

case "$output_directory/" in
    "$package_output_root/"*) ;;
    *) die "output directory must stay inside ignored dist/: $output_directory" ;;
esac

metadata="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1)"
version="$(printf '%s' "$metadata" | python3 -c '
import json, sys
packages = [p for p in json.load(sys.stdin)["packages"] if p["name"] == "hoi4_state_editor"]
if len(packages) != 1:
    raise SystemExit("expected exactly one hoi4_state_editor package")
print(packages[0]["version"])
')" || die "unable to determine the package version from cargo metadata"
[[ -n "$version" ]] || die "package version is empty"

package_name="HOI4-Map-Editor-v${version}-linux-x86_64"
stage="$output_directory/$package_name"
archive="$output_directory/$package_name.tar.gz"
checksum="$archive.sha256"
package_target="$repo_root/target/package-linux"
binary="$package_target/release/hoi4_state_editor"
commit="$(git -C "$repo_root" rev-parse --verify HEAD 2>/dev/null || printf 'unavailable')"
source_date_epoch="${SOURCE_DATE_EPOCH:-$(git -C "$repo_root" log -1 --format=%ct 2>/dev/null || printf 0)}"
[[ "$source_date_epoch" =~ ^[0-9]+$ ]] || die "SOURCE_DATE_EPOCH must be an integer"

cleanup_directory() {
    local path="$1"
    case "$path/" in
        "$repo_root/"*) rm -rf -- "$path" ;;
        *) die "refusing to remove a path outside the repository: $path" ;;
    esac
}

cleanup_directory "$stage"
if ! "$skip_build"; then
    cleanup_directory "$package_target"
fi
rm -f -- "$archive" "$checksum"
mkdir -p -- "$output_directory"

if ! "$skip_build"; then
    # Packaging builds are isolated from ordinary target/release outputs and
    # remove local source/home paths from compiler metadata where supported.
    encoded_flags="--remap-path-prefix=$repo_root=."
    if [[ -n "${HOME:-}" ]]; then
        encoded_flags+=$'\x1f'"--remap-path-prefix=$HOME=<HOME>"
    fi
    (
        export CARGO_TARGET_DIR="$package_target"
        export CARGO_PROFILE_RELEASE_DEBUG=0
        export CARGO_ENCODED_RUSTFLAGS="$encoded_flags"
        unset RUSTFLAGS
        cargo build --manifest-path "$repo_root/Cargo.toml" --release --locked
    )
fi

[[ -f "$binary" ]] || die "release executable not found: $binary"

mkdir -p -- "$stage"
install -m 0755 -- "$binary" "$stage/hoi4-map-editor"
install -m 0644 -- "$repo_root/LICENSE" "$stage/LICENSE"
install -m 0644 -- "$repo_root/THIRD_PARTY_NOTICES.md" "$stage/THIRD_PARTY_NOTICES.md"

cat > "$stage/README-LINUX.txt" <<EOF
HOI4 Map Editor $version — Linux x86_64 preview

Extract this archive to a writable directory and run ./hoi4-map-editor.
Do not run it from inside the archive.

Support baseline: Ubuntu 22.04-compatible x86_64, glibc-based desktop with
OpenGL and the X11 and/or Wayland runtime libraries required by this build.
This package does not support musl distributions, ARM Linux, or older glibc
systems unless separately tested.

Optional integrations are not bundled: xdg-open opens files/folders; wl-copy
is preferred for Wayland clipboard support; xclip or xsel are X11 fallbacks.
The application still starts when the optional clipboard commands are absent.

The primary Inconsolata font is embedded in the executable. Extra Cyrillic/CJK
glyph coverage uses installed system fonts when available; no system fonts are
bundled. See THIRD_PARTY_NOTICES.md for bundled-font licensing.
EOF

file_size() {
    stat --format=%s -- "$1"
}

file_hash() {
    sha256sum -- "$1" | awk '{print $1}'
}

payload_files=(hoi4-map-editor LICENSE THIRD_PARTY_NOTICES.md README-LINUX.txt)
{
    printf 'Application: HOI4 Map Editor\n'
    printf 'Version: %s\n' "$version"
    printf 'Platform: Linux\nArchitecture: x86_64\n'
    printf 'Build commit: %s\n' "$commit"
    printf 'SOURCE_DATE_EPOCH: %s\n' "$source_date_epoch"
    printf '\nPayload files (name, bytes, SHA-256):\n'
    for name in "${payload_files[@]}"; do
        printf '%s\t%s\t%s\n' "$name" "$(file_size "$stage/$name")" "$(file_hash "$stage/$name")"
    done
    printf '\nMANIFEST.txt is generated from this deterministic payload list.\n'
    printf 'The final archive SHA-256 is written next to the archive.\n'
} > "$stage/MANIFEST.txt"

expected_files=(LICENSE MANIFEST.txt README-LINUX.txt THIRD_PARTY_NOTICES.md hoi4-map-editor)
assert_allowlist() {
    local directory="$1"
    local actual
    actual="$(find "$directory" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)"
    local expected
    expected="$(printf '%s\n' "${expected_files[@]}" | LC_ALL=C sort)"
    [[ "$actual" == "$expected" ]] || {
        printf 'expected files:\n%s\nactual files:\n%s\n' "$expected" "$actual" >&2
        die "package allowlist mismatch"
    }
}

assert_allowlist "$stage"
assert_manifest() {
    local directory="$1"
    local name
    for name in "${payload_files[@]}"; do
        local expected_record
        expected_record="$(printf '%s\t%s\t%s' "$name" "$(file_size "$directory/$name")" "$(file_hash "$directory/$name")")"
        grep -Fqx -- "$expected_record" "$directory/MANIFEST.txt" ||
            die "manifest does not describe packaged file: $name"
    done
}
assert_manifest "$stage"
[[ -x "$stage/hoi4-map-editor" ]] || die "packaged executable lost its executable bit"

file_description="$(file -b -- "$stage/hoi4-map-editor")"
[[ "$file_description" == *"ELF 64-bit"* && "$file_description" == *"x86-64"* ]] || \
    die "packaged executable is not an x86_64 ELF: $file_description"
readelf -h -- "$stage/hoi4-map-editor" | grep -Eq 'Machine:.*(X86-64|x86-64)' || \
    die "ELF machine is not x86_64"
if readelf -d -- "$stage/hoi4-map-editor" | grep -Eq '(RPATH|RUNPATH)'; then
    die "packaged executable contains a runtime library search path"
fi
ldd_output="$(ldd "$stage/hoi4-map-editor")" || die "ldd failed for packaged executable"
printf '%s\n' "$ldd_output"
grep -Fq 'not found' <<<"$ldd_output" && die "a required dynamic library was not found"

privacy_scan() {
    local package_file
    while IFS= read -r -d '' package_file; do
        grep -aFq -- "$repo_root" "$package_file" &&
            die "package contains the absolute repository path: ${package_file##*/}"
        if [[ -n "${HOME:-}" ]] && grep -aFq -- "$HOME" "$package_file"; then
            die "package contains the active home path: ${package_file##*/}"
        fi
        if grep -aEq '(/home/[^[:space:][:cntrl:]]+)|(/Users/[^[:space:][:cntrl:]]+)|([A-Za-z]:[\\/](Users|Documents)[\\/])|(Documents[\\/]Paradox Interactive)|\bAzarya\b' "$package_file"; then
            die "package contains a private or development path: ${package_file##*/}"
        fi
    done < <(find "$stage" -maxdepth 1 -type f -print0)
}

privacy_scan

# GNU tar with an explicit epoch, sorted names and normalized ownership makes
# archive contents stable when the same payload is rebuilt. Build bytes may vary.
tar --format=posix --sort=name --mtime="@$source_date_epoch" --owner=0 --group=0 \
    --numeric-owner -C "$output_directory" -cf - "$package_name" | gzip -n > "$archive"
sha256sum -- "$archive" > "$checksum"
(cd "$output_directory" && sha256sum -c -- "$(basename "$checksum")")

extraction_root="$(mktemp -d)"
trap 'rm -rf -- "$extraction_root"' EXIT
tar -xzf "$archive" -C "$extraction_root"
extracted="$extraction_root/$package_name"
[[ -d "$extracted" ]] || die "archive is missing its top-level directory"
assert_allowlist "$extracted"
assert_manifest "$extracted"
[[ -x "$extracted/hoi4-map-editor" ]] || die "extracted executable lost its executable bit"
for name in "${expected_files[@]}"; do
    cmp -- "$stage/$name" "$extracted/$name" || die "extracted file differs: $name"
done

printf 'Package: %s\nChecksum: %s\n' "$archive" "$checksum"
printf 'Validated: allowlist, privacy scan, ELF x86_64, dependencies, checksum, clean extraction.\n'
printf 'GUI smoke test remains a manual release gate on a supported Linux desktop.\n'

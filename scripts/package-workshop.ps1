[CmdletBinding()]
param(
    [string]$OutputDirectory = "dist"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$manifest = Join-Path $repoRoot "Cargo.toml"
$metadata = cargo metadata --manifest-path $manifest --no-deps --format-version 1 | ConvertFrom-Json
$package = $metadata.packages | Where-Object name -eq "hoi4_state_editor" | Select-Object -First 1
if (-not $package) {
    throw "Unable to find the hoi4_state_editor package."
}

$version = $package.version
$dist = Join-Path $repoRoot $OutputDirectory
$stage = Join-Path $dist "workshop-v$version"
$packageTarget = Join-Path $repoRoot "target\package-windows"
$binary = Join-Path $packageTarget "release\hoi4_state_editor.exe"

if (-not (Test-Path -LiteralPath $binary)) {
    throw "Release executable not found: $binary. Run scripts\package-windows.ps1 first (or build --release with CARGO_TARGET_DIR set to target\package-windows)."
}

function Test-RepositoryPath([string]$Path) {
    $root = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd("\")
    $candidate = [System.IO.Path]::GetFullPath($Path)
    return $candidate.Equals($root, [System.StringComparison]::OrdinalIgnoreCase) -or
        $candidate.StartsWith("$root\", [System.StringComparison]::OrdinalIgnoreCase)
}

if (-not (Test-RepositoryPath $stage)) {
    throw "Refusing to write a workshop package target outside the repository."
}
if (Test-Path -LiteralPath $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "HOI4 Map Editor.exe")
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $stage
Copy-Item -LiteralPath (Join-Path $repoRoot "CHANGELOG.md") -Destination $stage
Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") -Destination $stage
Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\workshop\README.txt") -Destination $stage
Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\workshop\thumbnail.png") -Destination $stage

$descriptor = @"
version="$version"
tags={
	"Utilities"
}
name="HOI4 Map Editor (Standalone Tool - Preview $version)"
picture="thumbnail.png"
supported_version="*"
"@
Set-Content -LiteralPath (Join-Path $stage "descriptor.mod") -Value $descriptor -Encoding utf8NoBOM

Write-Output "Workshop package staged at: $stage"
Write-Output "This folder is prepared only. It is not uploaded or published."

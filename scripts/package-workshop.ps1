[CmdletBinding()]
param([string]$OutputDirectory = "dist")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$metadata = cargo metadata --manifest-path (Join-Path $repoRoot "Cargo.toml") --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq "hoi4_map_editor" | Select-Object -First 1).version
if (-not $version) { throw "Unable to determine package version." }

$stage = Join-Path (Join-Path $repoRoot $OutputDirectory) "workshop-v$version"
$binary = Join-Path $repoRoot "target\package-windows\release\hoi4_map_editor.exe"
$resolvedRoot = [IO.Path]::GetFullPath($repoRoot).TrimEnd("\")
$resolvedStage = [IO.Path]::GetFullPath($stage)
if (-not $resolvedStage.StartsWith("$resolvedRoot\", [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing Workshop target outside repository."
}
if (-not (Test-Path -LiteralPath $binary)) {
    throw "Release executable not found. Run scripts\package-windows.ps1 first."
}
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "HOI4 Map Editor.exe")
foreach ($file in @("LICENSE", "CHANGELOG.md", "THIRD_PARTY_NOTICES.md")) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $file) -Destination $stage
}
@"
HOI4 Map Editor is a standalone utility. It does not need a playset entry and this folder is not gameplay content.

Run HOI4 Map Editor.exe directly, then choose Open HOI4 Mod.
"@ | Set-Content -LiteralPath (Join-Path $stage "README.txt") -Encoding utf8NoBOM
@"
version="$version"
tags={
	"Utilities"
}
name="HOI4 Map Editor (Standalone Tool - Preview $version)"
supported_version="*"
"@ | Set-Content -LiteralPath (Join-Path $stage "descriptor.mod") -Encoding utf8NoBOM

Write-Output "Workshop package staged at: $stage"
Write-Output "Prepared locally only; nothing was uploaded or published."

[CmdletBinding()]
param(
    [string]$OutputDirectory = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$metadata = cargo metadata --manifest-path (Join-Path $repoRoot "Cargo.toml") --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq "hoi4_map_editor" | Select-Object -First 1).version
if (-not $version) { throw "Unable to determine package version." }

$packageName = "HOI4-Map-Editor-v$version-windows-x64"
$dist = Join-Path $repoRoot $OutputDirectory
$stage = Join-Path $dist "$packageName-staging"
$zipPath = Join-Path $dist "$packageName.zip"
$checksumPath = "$zipPath.sha256"
$releaseManifestPath = Join-Path $dist "RELEASE_MANIFEST.txt"
$packageTarget = Join-Path $repoRoot "target\package-windows"

function Test-RepositoryPath([string]$Path) {
    $root = [IO.Path]::GetFullPath($repoRoot).TrimEnd("\")
    $candidate = [IO.Path]::GetFullPath($Path)
    $candidate.Equals($root, [StringComparison]::OrdinalIgnoreCase) -or
        $candidate.StartsWith("$root\", [StringComparison]::OrdinalIgnoreCase)
}

function Get-Sha256([string]$Path) {
    $getFileHash = Get-Command Get-FileHash -ErrorAction SilentlyContinue
    if ($getFileHash) {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    }

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [IO.File]::ReadAllBytes($Path)
        return ([BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace("-", "")
    } finally {
        $sha256.Dispose()
    }
}

if (-not (Test-RepositoryPath $dist)) { throw "Refusing package directory outside repository: $dist" }
New-Item -ItemType Directory -Force -Path $dist | Out-Null
foreach ($target in @($stage, $zipPath, $checksumPath, $releaseManifestPath)) {
    if (-not (Test-RepositoryPath $target)) { throw "Refusing package target outside repository: $target" }
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
}

if (-not $SkipBuild) {
    $previousTarget = $env:CARGO_TARGET_DIR
    $previousDebug = $env:CARGO_PROFILE_RELEASE_DEBUG
    $previousRustFlags = $env:RUSTFLAGS
    $previousEncodedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
    try {
        $env:CARGO_TARGET_DIR = $packageTarget
        $env:CARGO_PROFILE_RELEASE_DEBUG = "0"
        $env:RUSTFLAGS = $null
        $env:CARGO_ENCODED_RUSTFLAGS = @(
            "--remap-path-prefix=$repoRoot=."
            "--remap-path-prefix=$env:USERPROFILE=<USERPROFILE>"
        ) -join [char]0x1f
        cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed." }
    } finally {
        $env:CARGO_TARGET_DIR = $previousTarget
        $env:CARGO_PROFILE_RELEASE_DEBUG = $previousDebug
        $env:RUSTFLAGS = $previousRustFlags
        $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedRustFlags
    }
}

$binary = Join-Path $packageTarget "release\hoi4_map_editor.exe"
if (-not (Test-Path -LiteralPath $binary)) { throw "Release executable not found: $binary" }

New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "HOI4 Map Editor.exe")
$allowed = @("HOI4 Map Editor.exe", "README.md", "LICENSE", "CHANGELOG.md", "THIRD_PARTY_NOTICES.md")
foreach ($file in $allowed | Where-Object { $_ -ne "HOI4 Map Editor.exe" }) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $file) -Destination $stage
}

foreach ($packageFile in Get-ChildItem -LiteralPath $stage -File) {
    $content = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($packageFile.FullName))
    if ($content -match "[A-Za-z]:[\\/]Users[\\/]" -or $content -match "Documents[\\/]Paradox Interactive" -or $content -match "\bAzarya\b") {
        throw "Package contains a private or mod-specific path: $($packageFile.Name)"
    }
}

Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath -CompressionLevel Optimal
Remove-Item -LiteralPath $stage -Recurse -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    $entries = @($archive.Entries | ForEach-Object FullName)
    if (@($entries | Where-Object { $_ -notin $allowed }).Count) { throw "Package contains an unexpected file." }
    foreach ($required in $allowed) { if ($required -notin $entries) { throw "Package is missing $required." } }
} finally { $archive.Dispose() }

$hash = Get-Sha256 $zipPath
"$hash  $([IO.Path]::GetFileName($zipPath))" | Set-Content -LiteralPath $checksumPath -Encoding ascii
@(
    "Name: HOI4 Map Editor"
    "Version: $version"
    "Build date (UTC): $([DateTime]::UtcNow.ToString('yyyy-MM-dd'))"
    "File: $([IO.Path]::GetFileName($zipPath))"
    "Size: $((Get-Item -LiteralPath $zipPath).Length)"
    "SHA-256: $hash"
) | Set-Content -LiteralPath $releaseManifestPath -Encoding ascii

Write-Output "Package: $zipPath"
Write-Output "Checksum: $checksumPath"
Write-Output "Manifest: $releaseManifestPath"

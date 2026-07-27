[CmdletBinding()]
param(
    [string]$OutputDirectory = "dist",
    [switch]$SkipBuild
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
$packageName = "HOI4-Map-Editor-v$version-windows-x64"
$dist = Join-Path $repoRoot $OutputDirectory
$stage = Join-Path $dist "$packageName-staging"
$zipPath = Join-Path $dist "$packageName.zip"
$checksumPath = "$zipPath.sha256"
$releaseManifestPath = Join-Path $dist "RELEASE_MANIFEST.txt"
$packageTarget = Join-Path $repoRoot "target\package-windows"

function Test-RepositoryPath([string]$Path) {
    $root = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd("\")
    $candidate = [System.IO.Path]::GetFullPath($Path)
    return $candidate.Equals($root, [System.StringComparison]::OrdinalIgnoreCase) -or
        $candidate.StartsWith("$root\", [System.StringComparison]::OrdinalIgnoreCase)
}

New-Item -ItemType Directory -Force -Path $dist | Out-Null
foreach ($target in @($stage, $zipPath, $checksumPath, $releaseManifestPath)) {
    if (-not (Test-RepositoryPath $target)) {
        throw "Refusing to replace a package target outside the repository."
    }
    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Recurse -Force
    }
}

if (-not $SkipBuild) {
    $previousDebug = $env:CARGO_PROFILE_RELEASE_DEBUG
    $previousRustFlags = $env:RUSTFLAGS
    $previousEncodedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
    $previousTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_PROFILE_RELEASE_DEBUG = "0"
        $env:CARGO_TARGET_DIR = $packageTarget
        $cargoExecutable = (Get-Command cargo -ErrorAction Stop).Source
        $pathPrefixes = @(
            [Environment]::GetFolderPath("UserProfile"),
            $env:USERPROFILE,
            (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $cargoExecutable)))
        )
        if ($repoRoot -match "^[A-Za-z]:\\Users\\[^\\]+") {
            $pathPrefixes += $Matches[0]
        }
        $pathPrefixes = $pathPrefixes |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            Select-Object -Unique

        $separator = [char]0x1f
        $encodedRustFlags = @()
        foreach ($prefix in $pathPrefixes) {
            $encodedRustFlags += "--remap-path-prefix", "$prefix=."
            $normalizedPrefix = $prefix.Replace("\", "/")
            if ($normalizedPrefix -ne $prefix) {
                $encodedRustFlags += "--remap-path-prefix", "$normalizedPrefix=."
            }
        }
        $encodedRustFlags += "--remap-path-scope", "all"

        $env:RUSTFLAGS = $null
        $env:CARGO_ENCODED_RUSTFLAGS = $encodedRustFlags -join $separator
        cargo build --manifest-path $manifest --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed."
        }
    }
    finally {
        $env:CARGO_PROFILE_RELEASE_DEBUG = $previousDebug
        $env:RUSTFLAGS = $previousRustFlags
        $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedRustFlags
        $env:CARGO_TARGET_DIR = $previousTarget
    }
}

$binary = Join-Path $packageTarget "release\hoi4_state_editor.exe"
if (-not (Test-Path -LiteralPath $binary)) {
    throw "Release executable not found: $binary"
}

New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "HOI4 Map Editor.exe")
foreach ($file in @("README.md", "LICENSE", "CHANGELOG.md", "THIRD_PARTY_NOTICES.md")) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $file) -Destination $stage
}

$binaryText = [System.Text.Encoding]::ASCII.GetString(
    [System.IO.File]::ReadAllBytes((Join-Path $stage "HOI4 Map Editor.exe"))
)
$binaryUtf8Text = [System.Text.Encoding]::UTF8.GetString(
    [System.IO.File]::ReadAllBytes((Join-Path $stage "HOI4 Map Editor.exe"))
)
if ($binaryText -match "[A-Za-z]:[\\/]Users[\\/]") {
    throw "The packaged executable contains an absolute user path."
}
$legacyConfigName = "hoi4pe_" + "config.toml"
if ($binaryText.Contains($legacyConfigName)) {
    throw "The packaged executable still references the legacy configuration file."
}
$portugueseSettings = "Configura$([char]0x00E7)$([char]0x00F5)es"
foreach ($embeddedText in @("Settings", $portugueseSettings, "Province and State Editing Toolkit")) {
    if (-not $binaryUtf8Text.Contains($embeddedText)) {
        throw "The packaged executable is missing embedded UI catalog text: $embeddedText"
    }
}
foreach ($textFile in Get-ChildItem -LiteralPath $stage -File |
    Where-Object Extension -in ".md", ".txt") {
    $text = Get-Content -LiteralPath $textFile.FullName -Raw
    if ($text -match "[A-Za-z]:[\\/]Users[\\/]" -or
        $text -match "Documents[\\/]Paradox Interactive" -or
        $text -match "\bAzarya\b") {
        throw "The package text contains a private or mod-specific path."
    }
}

Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath -CompressionLevel Optimal
Remove-Item -LiteralPath $stage -Recurse -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    $allowed = @(
        "HOI4 Map Editor.exe",
        "README.md",
        "LICENSE",
        "CHANGELOG.md",
        "THIRD_PARTY_NOTICES.md"
    )
    $entries = @($archive.Entries | ForEach-Object FullName)
    if (@($entries | Where-Object { $_ -notin $allowed }).Count -ne 0) {
        throw "The package contains an unexpected file."
    }
    foreach ($required in $allowed) {
        if ($required -notin $entries) {
            throw "The package is missing $required."
        }
    }
}
finally {
    $archive.Dispose()
}

$hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
"$hash  $([System.IO.Path]::GetFileName($zipPath))" |
    Set-Content -LiteralPath $checksumPath -Encoding ascii
$verifiedHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
if ($verifiedHash -ne $hash) {
    throw "The package checksum changed after it was generated."
}

$buildDate = if ($env:SOURCE_DATE_EPOCH) {
    [DateTimeOffset]::FromUnixTimeSeconds([long]$env:SOURCE_DATE_EPOCH).UtcDateTime
} else {
    [DateTime]::UtcNow
}
@(
    "Name: HOI4 Map Editor",
    "Version: $version",
    "Build date (UTC): $($buildDate.ToString('yyyy-MM-dd'))",
    "File: $([System.IO.Path]::GetFileName($zipPath))",
    "Size: $((Get-Item -LiteralPath $zipPath).Length)",
    "SHA-256: $hash"
) | Set-Content -LiteralPath $releaseManifestPath -Encoding ascii

Write-Output "Package: $zipPath"
Write-Output "Checksum: $checksumPath"
Write-Output "Manifest: $releaseManifestPath"

[CmdletBinding()]
param(
    [string]$ReferenceRoot,
    [switch]$StrictKnownBaseline
)

$ErrorActionPreference = 'Stop'

if ($ReferenceRoot) {
    $package = Join-Path -Path $ReferenceRoot -ChildPath 'package.json'
    if (-not (Test-Path -LiteralPath $package -PathType Leaf)) {
        throw "Herbix reference root does not contain package.json: $ReferenceRoot"
    }
    Write-Host 'Herbix execution is optional and snapshot-based; this script does not start VS Code or mutate the reference checkout.'
} else {
    Write-Host 'No Herbix checkout configured. Running deterministic Rust normalization/snapshot tests only.'
}

if ($StrictKnownBaseline) {
    Write-Host 'Strict baseline mode is enforced by approved ExpectedDifference records in the snapshot comparison API.'
}

cargo test --locked compatibility_harness

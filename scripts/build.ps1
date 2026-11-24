param([switch]$Release)
$ErrorActionPreference = 'Stop'
# This script lives in scripts/; paths below are relative to the repository root.
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    $cargoArgs = @('build', '--locked', '--workspace')
    if ($Release) { $cargoArgs += '--release' }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
} finally { Pop-Location }

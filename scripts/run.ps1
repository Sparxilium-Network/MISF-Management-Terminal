param([Parameter(ValueFromRemainingArguments = $true)][string[]]$AppArguments)

$ErrorActionPreference = 'Stop'
# This script lives in scripts/; paths below are relative to the repository root.
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    & cargo run --locked -p misf -- @AppArguments
    if ($LASTEXITCODE -ne 0) { throw 'MISF exited with an error' }
} finally { Pop-Location }

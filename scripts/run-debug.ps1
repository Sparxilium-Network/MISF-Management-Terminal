# Only named options bind here, so app arguments such as --help pass through unchanged.
[CmdletBinding(PositionalBinding = $false)]
param(
    # Isolated test profile; relative paths are from the repository root.
    [string]$Dir = '.test-data/debug-profile',
    # Allow real authenticators in that profile; use a test security key.
    [switch]$AllowPasskey,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$AppArguments
)
# Runs MISF from source in debug mode: its own host store, settings and logs in $Dir,
# loopback SSH only and, unless -AllowPasskey, no real Passkey.
$ErrorActionPreference = 'Stop'
# This script lives in scripts/; paths below are relative to the repository root.
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    $profileDir = (Resolve-Path -LiteralPath $Dir).Path
    $debugArgs = @("--debug.dir=$profileDir")
    if ($AllowPasskey) { $debugArgs += '--debug.allow-passkey' }
    Write-Output "Debug profile: $profileDir"
    & cargo run --locked -p misf -- @debugArgs @AppArguments
    if ($LASTEXITCODE -ne 0) { throw 'MISF exited with an error' }
} finally { Pop-Location }

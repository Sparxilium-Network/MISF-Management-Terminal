param([switch]$Check)
# Regenerates licenses/THIRD_PARTY_LICENSES.html from Cargo.lock with cargo-about,
# and records which Cargo.lock it was built from so packaging can reject a stale copy.
# Requires: cargo install --locked cargo-about --features cli; cargo install --locked cargo-deny
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    $stampPath = 'licenses/THIRD_PARTY_LICENSES.lock-sha256'
    # Line endings are normalized so Windows and Linux checkouts agree.
    $lock = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'Cargo.lock')) -replace "`r`n", "`n"
    $hash = [System.BitConverter]::ToString(
        [System.Security.Cryptography.SHA256]::Create().ComputeHash([System.Text.Encoding]::UTF8.GetBytes($lock))
    ).Replace('-', '').ToLowerInvariant()
    if ($Check) {
        $recorded = if (Test-Path -LiteralPath $stampPath) { (Get-Content -LiteralPath $stampPath -Raw).Trim() } else { '' }
        if ($recorded -ne $hash) {
            throw 'licenses/THIRD_PARTY_LICENSES.html is out of date with Cargo.lock; run ./licenses.ps1'
        }
        return
    }
    cargo deny --locked check licenses
    if ($LASTEXITCODE -ne 0) { throw 'cargo deny found a dependency license that is not allowed' }
    cargo about generate --locked --fail about.hbs --output-file licenses/THIRD_PARTY_LICENSES.html
    if ($LASTEXITCODE -ne 0) { throw 'cargo about failed' }
    Set-Content -LiteralPath $stampPath -Value $hash -Encoding ascii -NoNewline
    Write-Output "Wrote licenses/THIRD_PARTY_LICENSES.html for Cargo.lock $hash"
} finally { Pop-Location }

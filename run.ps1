$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo run --locked -p misf
    if ($LASTEXITCODE -ne 0) { throw 'MISF exited with an error' }
} finally { Pop-Location }

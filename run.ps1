$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo run --locked -p msif
    if ($LASTEXITCODE -ne 0) { throw 'MSIF exited with an error' }
} finally { Pop-Location }

$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & "$PSScriptRoot/build.ps1" -Release
    $outputPath = Join-Path $PSScriptRoot 'dist/MISF'
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    Copy-Item -LiteralPath 'target/release/misf.exe' -Destination "$outputPath/misf.exe"
    Copy-Item -LiteralPath 'README.md' -Destination $outputPath
    Write-Output "Portable app: $outputPath/misf.exe"
} finally { Pop-Location }

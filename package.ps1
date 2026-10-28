$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & "$PSScriptRoot/build.ps1" -Release
    $outputPath = Join-Path $PSScriptRoot 'dist/MSIF'
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    Copy-Item -LiteralPath 'target/release/msif.exe' -Destination "$outputPath/msif.exe"
    Copy-Item -LiteralPath 'README.md' -Destination $outputPath
    Write-Output "Portable app: $outputPath/msif.exe"
} finally { Pop-Location }

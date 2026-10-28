$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & "$PSScriptRoot/build.ps1" -Release
    $outputPath = Join-Path $PSScriptRoot 'dist/MISF'
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    Copy-Item -LiteralPath 'target/release/misf.exe' -Destination "$outputPath/misf.exe"
    Copy-Item -LiteralPath 'README.md' -Destination $outputPath
    New-Item -ItemType Directory -Path "$outputPath/licenses/LUKSbox" -Force | Out-Null
    Get-ChildItem -LiteralPath 'third_party/LUKSbox' -File | Copy-Item -Destination "$outputPath/licenses/LUKSbox"
    Copy-Item -LiteralPath 'THIRD_PARTY_NOTICES.md' -Destination $outputPath
    New-Item -ItemType Directory -Path "$outputPath/docs" -Force | Out-Null
    Get-ChildItem -LiteralPath 'docs' -Filter '*.md' -File | Copy-Item -Destination "$outputPath/docs"
    Write-Output "Portable app: $outputPath/misf.exe"
} finally { Pop-Location }

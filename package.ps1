param([switch]$SkipBuild, [switch]$Archive)
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    if (-not $SkipBuild) { & "$PSScriptRoot/build.ps1" -Release }
    $onWindows = $env:OS -eq 'Windows_NT'
    if (-not $onWindows -and -not $IsLinux) { throw 'Packaging supports Windows and Linux only' }
    $binaryName = if ($onWindows) { 'misf.exe' } else { 'misf' }
    $platformName = if ($onWindows) { 'windows-x86_64' } else { 'linux-x86_64' }
    # Archive staging must not overwrite a locally running portable app.
    $outputPath = if ($Archive) {
        Join-Path $PSScriptRoot "dist/package-$platformName"
    } else { Join-Path $PSScriptRoot 'dist/MISF' }
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    Copy-Item -LiteralPath "target/release/$binaryName" -Destination "$outputPath/$binaryName"
    Copy-Item -LiteralPath 'README.md' -Destination $outputPath
    New-Item -ItemType Directory -Path "$outputPath/licenses/LUKSbox" -Force | Out-Null
    Get-ChildItem -LiteralPath 'third_party/LUKSbox' -File | Copy-Item -Destination "$outputPath/licenses/LUKSbox"
    Copy-Item -LiteralPath 'THIRD_PARTY_NOTICES.md' -Destination $outputPath
    New-Item -ItemType Directory -Path "$outputPath/docs" -Force | Out-Null
    Get-ChildItem -LiteralPath 'docs' -Filter '*.md' -File | Copy-Item -Destination "$outputPath/docs"
    Write-Output "Portable app: $outputPath/$binaryName"
    if ($Archive) {
        $archiveBase = Join-Path $PSScriptRoot "dist/MISF-Management-Terminal-$platformName"
        # Only package the explicit application files, never other contents of dist/MISF.
        $packageFiles = @($binaryName, 'README.md', 'THIRD_PARTY_NOTICES.md', 'docs', 'licenses')
        if ($onWindows) {
            $archivePath = "$archiveBase.zip"
            $packagePaths = $packageFiles | ForEach-Object { Join-Path $outputPath $_ }
            Compress-Archive -LiteralPath $packagePaths -DestinationPath $archivePath -Force
        } else {
            $archivePath = "$archiveBase.tar.gz"
            & tar -czf $archivePath -C $outputPath @packageFiles
            if ($LASTEXITCODE -ne 0) { throw 'Creating Linux archive failed' }
        }
        $digest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $archiveName = Split-Path -Leaf $archivePath
        Set-Content -LiteralPath "$archivePath.sha256" -Value "$digest  $archiveName" -Encoding ascii
        Write-Output "Archive: $archivePath"
    }
} finally { Pop-Location }

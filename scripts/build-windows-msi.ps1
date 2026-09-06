<#
Build a local MSI from the current checkout and existing bundled workers.
Example: ./scripts/build-windows-msi.ps1 -WixBinDir ./target/tools/wix
WiX 3.14.1 portable binaries may be unpacked into target/tools/wix; no global
installation is required. Worker build recipes are in .github/workflows/release.yml.
#>
[CmdletBinding()]
param(
    [string]$WixBinDir,
    [string]$OutputDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $manifest = Get-Content Cargo.toml -Raw
    $match = [regex]::Match($manifest, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"')
    if (-not $match.Success) { throw 'Cargo.toml must contain a stable x.y.z package version.' }
    $version = $match.Groups[1].Value
    $parts = $version.Split('.')
    if ([int]$parts[0] -gt 255 -or [int]$parts[1] -gt 255 -or [int]$parts[2] -gt 65535) {
        throw "Version $version exceeds Windows Installer limits."
    }
    if (-not $WixBinDir) {
        if ($env:WIX) { $WixBinDir = Join-Path $env:WIX 'bin' }
        else { $WixBinDir = 'target/tools/wix' }
    }
    $candle = (Resolve-Path (Join-Path $WixBinDir 'candle.exe')).Path
    $light = (Resolve-Path (Join-Path $WixBinDir 'light.exe')).Path
    if (-not $OutputDirectory) { $OutputDirectory = "dist/v$version" }
    New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
    $outputDir = (Resolve-Path $OutputDirectory).Path

    if (-not $SkipBuild) {
        & cargo build --locked --offline --release --bin OpenCADStudio --jobs 2
        if ($LASTEXITCODE -ne 0) { throw 'Application release build failed.' }
        & cargo build --locked --offline --release -p dwg-thumbnailer-win --jobs 2
        if ($LASTEXITCODE -ne 0) { throw 'Thumbnail provider release build failed.' }
    }
    $exe = (Resolve-Path target/release/OpenCADStudio.exe).Path
    $classifier = (Resolve-Path target/release/ocs-lidar-classifier.exe).Path
    $projWorker = (Resolve-Path target/release/ocs-proj-worker.exe).Path
    $projGrid = (Resolve-Path target/release/proj-data/us_noaa_g2018u0.tif).Path
    $null = Resolve-Path target/release/dwg_thumbnailer_win.dll
    $binaryVersion = (Get-Item $exe).VersionInfo.ProductVersion
    if ($binaryVersion -ne $version) {
        throw "Refusing to package stale executable $binaryVersion as $version. Rebuild without -SkipBuild."
    }
    $gridHash = (Get-FileHash -LiteralPath $projGrid -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($gridHash -ne 'fa9a407ac7ee3f5a3694008e4bcd09ce9cc250452f0c3b11700a4960340abce2') {
        throw 'Bundled PROJ grid checksum does not match the pinned release grid.'
    }
    & $projWorker --self-test
    if ($LASTEXITCODE -ne 0) { throw 'Bundled PROJ worker self-test failed.' }
    & $classifier --help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Bundled classifier could not start.' }

    $objDir = Join-Path $outputDir 'wix'
    New-Item -ItemType Directory -Force $objDir | Out-Null
    $icon = (Resolve-Path packaging/windows/AppIcon.ico).Path
    $license = (Resolve-Path packaging/windows/License.rtf).Path
    $msiPath = Join-Path $outputDir "OpenCADStudio-$version-windows-x86_64-installer.msi"
    & $candle -arch x64 "-dVersion=$version" "-dSource=$exe" `
        "-dClassifier=$classifier" "-dProjWorker=$projWorker" "-dProjGrid=$projGrid" `
        "-dIcon=$icon" "-dLicense=$license" `
        packaging/windows/main.wxs packaging/windows/ui.wxs -out ($objDir + '\')
    if ($LASTEXITCODE -ne 0) { throw 'WiX candle failed.' }
    & $light (Join-Path $objDir 'main.wixobj') (Join-Path $objDir 'ui.wixobj') `
        -ext WixUIExtension -sval -out $msiPath
    if ($LASTEXITCODE -ne 0) { throw 'WiX light or installer validation failed.' }

    $hash = (Get-FileHash -LiteralPath $msiPath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $(Split-Path $msiPath -Leaf)" | Set-Content -Encoding ascii `
        (Join-Path $outputDir "OpenCADStudio-$version-windows-x86_64-SHA256SUMS.txt")
    Write-Output "Built and validated: $msiPath"
}
finally { Pop-Location }

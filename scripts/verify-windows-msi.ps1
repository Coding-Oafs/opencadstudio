<# Inspect installer metadata and verify every extracted application payload. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$MsiPath,
    [string]$WixBinDir = 'target/tools/wix',
    [string]$OutputDirectory = 'target/msi-verification'
)
$ErrorActionPreference = 'Stop'
$packagePath = (Resolve-Path $MsiPath).Path
$projectRoot = Split-Path $PSScriptRoot -Parent
$dark = (Resolve-Path (Join-Path $WixBinDir 'dark.exe')).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$outputDir = (Resolve-Path $OutputDirectory).Path
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($packagePath, 0)
$view = $database.OpenView('SELECT `Property`, `Value` FROM `Property`')
$view.Execute()
$properties = @{}
while ($record = $view.Fetch()) { $properties[$record.StringData(1)] = $record.StringData(2) }
$view.Close()
$expectedVersion = (Get-Item (Join-Path $projectRoot 'target/release/OpenCADStudio.exe')).VersionInfo.ProductVersion
if ($properties.ProductVersion -ne $expectedVersion) { throw 'MSI version does not match the release executable.' }
if ($properties.UpgradeCode -ne '{BD6BCEAB-6C7A-4CE2-A62D-6EF06A92F8B1}') { throw 'Installer upgrade identity changed.' }
$view = $database.OpenView('SELECT `Key`, `Name`, `Value` FROM `Registry`')
$view.Execute()
$registry = @{}
$extensions = @()
while ($record = $view.Fetch()) {
    if (-not $record.StringData(2)) { $registry[$record.StringData(1)] = $record.StringData(3) }
}
$view.Close()
foreach ($extension in @('dwg', 'dxf', '3mf')) {
    $progId = "OpenCADStudio.$($extension.ToUpperInvariant())"
    if ($registry[".$extension"] -ne $progId) { throw "Missing .$extension association." }
    if ($registry["$progId\shell\open\command"] -ne '"[#OpenCADStudioEXE]" "%1"') {
        throw "Incorrect .$extension open command."
    }
    $extensions += $extension
}
$xmlPath = Join-Path $outputDir 'installer.wxs'
$extractDir = Join-Path $outputDir 'extracted'
& $dark -nologo -sui -x $extractDir $packagePath -o $xmlPath
if ($LASTEXITCODE -ne 0) { throw 'MSI extraction failed.' }
[xml]$xml = Get-Content -LiteralPath $xmlPath -Raw
$namespace = [Xml.XmlNamespaceManager]::new($xml.NameTable)
$namespace.AddNamespace('w', 'http://schemas.microsoft.com/wix/2006/wi')
$payloadDir = Join-Path $outputDir 'payload'
New-Item -ItemType Directory -Force (Join-Path $payloadDir 'proj-data') | Out-Null
$expected = @{
    OpenCADStudioEXE = @('target/release/OpenCADStudio.exe', 'OpenCADStudio.exe')
    LidarClassifierEXE = @('target/release/ocs-lidar-classifier.exe', 'ocs-lidar-classifier.exe')
    ProjWorkerEXE = @('target/release/ocs-proj-worker.exe', 'ocs-proj-worker.exe')
    ProjGridGEOID18 = @('target/release/proj-data/us_noaa_g2018u0.tif', 'proj-data/us_noaa_g2018u0.tif')
    DwgThumbDll = @('target/release/dwg_thumbnailer_win.dll', 'dwg_thumbnailer_win.dll')
    DwgIco = @('packaging/windows/dwg.ico', 'dwg.ico')
    DxfIco = @('packaging/windows/dxf.ico', 'dxf.ico')
}
$files = @()
foreach ($id in $expected.Keys) {
    $node = $xml.SelectSingleNode("//w:File[@Id='$id']", $namespace)
    if (-not $node) { throw "Missing MSI payload: $id" }
    $source = Join-Path $projectRoot $expected[$id][0]
    $hash = (Get-FileHash -LiteralPath $node.Source -Algorithm SHA256).Hash
    if ($hash -ne (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash) {
        throw "Extracted MSI payload differs from release input: $id"
    }
    Copy-Item -LiteralPath $node.Source -Destination (Join-Path $payloadDir $expected[$id][1])
    $files += @{ id=$id; name=$expected[$id][1]; sha256=$hash.ToLowerInvariant() }
}
$report = @{
    passed=$true; package=$packagePath; properties=$properties; associations=$extensions
    payload=$files; msiSha256=(Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    signature=(Get-AuthenticodeSignature -LiteralPath $packagePath).Status.ToString()
}
$report | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $outputDir 'msi-report.json') -Encoding utf8
Write-Output "PASS: MSI $expectedVersion, upgrade identity, file associations and $($files.Count) matching payload files."
Write-Output "Extracted app for smoke testing: $(Join-Path $payloadDir 'OpenCADStudio.exe')"

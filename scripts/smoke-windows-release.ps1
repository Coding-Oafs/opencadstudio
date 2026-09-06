<# Exercise the actual packaged binary through its own CAD command dispatcher. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ExePath,
    [string]$OutputDirectory = 'target/release-smoke'
)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $ExePath).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$scratch = (Resolve-Path $OutputDirectory).Path
$info = [Diagnostics.ProcessStartInfo]::new($exe, '--serve')
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardInput = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$info.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
$info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
$info.Environment['APPDATA'] = Join-Path $scratch 'config'
$info.WorkingDirectory = $scratch
$process = [Diagnostics.Process]::Start($info)
$stderr = $process.StandardError.ReadToEndAsync()
$transcript = [Collections.Generic.List[object]]::new()
function Read-Response {
    $read = $process.StandardOutput.ReadLineAsync()
    if (-not $read.Wait(30000)) { throw 'Timed out waiting for the CAD automation response.' }
    $line = $read.Result
    if (-not $line) { throw 'CAD automation ended without a response.' }
    return ($line | ConvertFrom-Json)
}
function Request($request) {
    $process.StandardInput.WriteLine(($request | ConvertTo-Json -Compress))
    $process.StandardInput.Flush()
    $reply = Read-Response
    $transcript.Add(@{ request = $request; response = $reply })
    if (-not $reply.ok) { throw "CAD request failed: $($reply | ConvertTo-Json -Compress)" }
    return $reply
}
function Expect($actual, $expected, [string]$context) {
    if ($actual -ne $expected) { throw "$context expected $expected, received $actual" }
}
try {
    $ready = Read-Response
    Expect $ready.ready $true 'Automation startup'
    Expect $ready.version (Get-Item $exe).VersionInfo.ProductVersion 'Runtime version'
    Expect (Request @{op='new'}).total 0 'New drawing'
    Expect (Request @{op='run'; cmd='LINE 0,0 10,10 10,20'}).added 2 'Polyline-style LINE input'
    Expect (Request @{op='run'; cmd='CIRCLE 5,5 3'}).added 1 'Circle'
    Expect (Request @{op='select'; type='Circle'}).selected 1 'Circle selection'
    $null = Request @{op='run'; cmd='MOVE 0,0 100,0'}
    $circle = (Request @{op='query'; type='Circle'}).entities[0]
    Expect $circle.center[0] 105 'Move geometry'
    Expect $circle.radius 3 'Circle radius'
    Expect (Request @{op='select'; type='Line'}).selected 2 'Line selection'
    Expect (Request @{op='run'; cmd='ERASE'}).entities 1 'Erase'
    Expect (Request @{op='undo'}).total 3 'Undo erase'
    Expect (Request @{op='redo'}).total 1 'Redo erase'
    Expect (Request @{op='undo'}).total 3 'Restore geometry'
    foreach ($extension in @('dwg', 'dxf')) {
        $drawing = Join-Path $scratch "Drawing round trip.$extension"
        $null = Request @{op='save'; path=$drawing}
        if ((Get-Item $drawing).Length -eq 0) { throw 'Drawing save produced an empty file.' }
        Expect (Request @{op='new'}).total 0 'Reset between files'
        Expect (Request @{op='undo'}).total 0 'History isolation'
        Expect (Request @{op='open'; path=$drawing}).total 3 "$extension round trip"
        $circle = (Request @{op='query'; type='Circle'}).entities[0]
        Expect $circle.center[0] 105 "$extension coordinates"
        Expect $circle.radius 3 "$extension radius"
    }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(10000)) { throw 'CAD automation did not exit after EOF.' }
    Expect $process.ExitCode 0 'Clean exit'
    @{ version=$ready.version; executable=$exe; passed=$true; requests=$transcript.Count; transcript=$transcript } |
        ConvertTo-Json -Depth 15 | Set-Content (Join-Path $scratch 'smoke-report.json') -Encoding utf8
    Write-Output "PASS: $($transcript.Count) CAD requests, DWG/DXF round trips and clean exit ($($ready.version))."
}
finally {
    if (-not $process.HasExited) { $process.StandardInput.Close() }
    if ($process.HasExited) {
        $stderr.Result | Set-Content (Join-Path $scratch 'stderr.log') -Encoding utf8
        $process.Dispose()
    }
}

param(
    [string]$Exe = "C:\Users\danof\OneDrive\Documents\GitHub\opencadstudio\target\debug\OpenCADStudio.exe",
    [string]$Offset = "0x63c1007"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Dbg {
    [DllImport("dbghelp.dll", SetLastError = true)]
    public static extern bool SymSetOptions(uint options);
    [DllImport("dbghelp.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool SymInitialize(IntPtr handle, string searchPath, bool invade);
    [DllImport("dbghelp.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern ulong SymLoadModuleEx(IntPtr handle, IntPtr file, string imageName,
        string moduleName, ulong baseAddress, uint size, IntPtr data, uint flags);
    [DllImport("dbghelp.dll", SetLastError = true)]
    public static extern bool SymFromAddr(IntPtr handle, ulong address, out ulong displacement,
        byte[] symbol);
    [DllImport("dbghelp.dll", SetLastError = true)]
    public static extern bool SymGetLineFromAddr64(IntPtr handle, ulong address,
        out ulong displacement, byte[] line);
}
"@

[Dbg]::SymSetOptions(0x10) | Out-Null  # SYMOPT_LOAD_LINES
$ok = [Dbg]::SymInitialize([IntPtr]::Zero, (Split-Path $Exe), $false)
if (-not $ok) { Write-Output "SymInitialize failed"; exit 1 }

$stream = [System.IO.File]::OpenRead($Exe)
$handle = $stream.Handle
$base = [Dbg]::SymLoadModuleEx([IntPtr]::Zero, $handle, $Exe, "OpenCADStudio", [UInt64]0, [UInt32]0, [IntPtr]::Zero, 0)
$stream.Close()
if ($base -eq 0) { Write-Output ("SymLoadModuleEx failed: " + [Runtime.InteropServices.Marshal]::GetLastWin32Error()); exit 1 }
Write-Output ("module base: 0x" + $base.ToString("x"))

$address = $base + [Convert]::ToUInt64($Offset, 16)

$symbol = New-Object byte[] 8192
[Array]::Copy([BitConverter]::GetBytes([UInt32]88), 0, $symbol, 0, 4)    # SizeOfStruct
[Array]::Copy([BitConverter]::GetBytes([UInt32]512), 0, $symbol, 80, 4) # MaxNameLen
$displacement = [UInt64]0
if ([Dbg]::SymFromAddr([IntPtr]::Zero, $address, [ref]$displacement, $symbol)) {
    $nameLen = [BitConverter]::ToUInt32($symbol, 76)
    $name = [System.Text.Encoding]::Unicode.GetString($symbol, 84, $nameLen * 2)
    Write-Output ("SYMBOL: " + $name + " +0x" + $displacement.ToString("x"))
} else {
    Write-Output ("SymFromAddr failed: " + [Runtime.InteropServices.Marshal]::GetLastWin32Error())
}

$lineStruct = New-Object byte[] 4096
[Array]::Copy([BitConverter]::GetBytes([UInt32]40), 0, $lineStruct, 0, 4)
$displacement2 = [UInt64]0
if ([Dbg]::SymGetLineFromAddr64([IntPtr]::Zero, $address, [ref]$displacement2, $lineStruct)) {
    $lineNum = [BitConverter]::ToUInt32($lineStruct, 16)
    $filePtr = [BitConverter]::ToUInt64($lineStruct, 8)
    $fileName = [Runtime.InteropServices.Marshal]::PtrToStringUni([IntPtr][Int64]$filePtr)
    Write-Output ("LINE: " + $fileName + ":" + $lineNum)
} else {
    Write-Output ("SymGetLineFromAddr64 failed: " + [Runtime.InteropServices.Marshal]::GetLastWin32Error())
}

param(
    [string]$Exe = "C:\Users\danof\OneDrive\Documents\GitHub\opencadstudio\target\debug\OpenCADStudio.exe",
    [string]$Offset = "0x3aa5cee"
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Dbg3 {
    [DllImport("dbghelp.dll")] public static extern bool SymSetOptions(uint o);
    [DllImport("dbghelp.dll", CharSet = CharSet.Unicode)] public static extern bool SymInitialize(IntPtr h, string p, bool i);
    [DllImport("dbghelp.dll", CharSet = CharSet.Unicode)] public static extern ulong SymLoadModuleEx(IntPtr h, IntPtr f, string img, string moduleName, ulong baseAddress, uint size, IntPtr d, uint flags);
    [DllImport("dbghelp.dll")] public static extern bool SymGetLineFromAddr64(IntPtr h, ulong a, out ulong disp, byte[] line);
}
"@
[Dbg3]::SymSetOptions(0x10) | Out-Null
[Dbg3]::SymInitialize([IntPtr]::Zero, (Split-Path $Exe), $false) | Out-Null
$stream = [System.IO.File]::OpenRead($Exe)
$base = [Dbg3]::SymLoadModuleEx([IntPtr]::Zero, $stream.Handle, $Exe, "OCS", [UInt64]0, [UInt32]0, [IntPtr]::Zero, 0)
$stream.Close()
$address = $base + [Convert]::ToUInt64($Offset, 16)
$line = New-Object byte[] 4096
[Array]::Copy([BitConverter]::GetBytes([UInt32]40), 0, $line, 0, 4)
$disp = [UInt64]0
$ok = [Dbg3]::SymGetLineFromAddr64([IntPtr]::Zero, $address, [ref]$disp, $line)
$lineNum = [BitConverter]::ToUInt32($line, 16)
$filePtr = [BitConverter]::ToUInt64($line, 24)
$fileName = ""
if ($filePtr -gt 0x10000) {
    try { $fileName = [Runtime.InteropServices.Marshal]::PtrToStringAnsi([IntPtr][Int64]$filePtr) } catch {}
}
Write-Output ("LINE: " + $fileName + ":" + $lineNum)

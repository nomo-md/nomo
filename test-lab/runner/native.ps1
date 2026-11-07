param([ValidateSet('input','screenshot','environment','check-processes')][string]$Action, [int]$RootProcessId, [string]$Text, [string]$Keys, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
if ($Action -eq 'environment') {
  @{ os=([Environment]::OSVersion.VersionString); powershell=$PSVersionTable.PSVersion.ToString(); cpu=@(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors);
     memoryBytes=(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory;
     gpu=@(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,AdapterRAM) } | ConvertTo-Json -Depth 6
  exit
}
if ($Action -eq 'check-processes') {
  $owned=@($Text | ConvertFrom-Json);$alive=@()
  foreach($entry in $owned){$p=Get-CimInstance Win32_Process -Filter "ProcessId = $([int]$entry.pid)";if($p -and $p.CreationDate.ToUniversalTime() -eq ([DateTime]$entry.created).ToUniversalTime()){$alive+=$entry}}
  @{survivors=$alive} | ConvertTo-Json -Depth 4 -Compress
  exit
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TestWindow {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] data, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public UNION u; }
  [StructLayout(LayoutKind.Explicit)] public struct UNION { [FieldOffset(0)] public KEYBOARD ki; [FieldOffset(0)] public MOUSE mi; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBOARD { public ushort vk,scan; public uint flags,time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSE { public int x,y; public uint data,flags,time; public UIntPtr extra; }
  public static void Key(ushort vk, ushort scan, uint flags) {
    var i=new INPUT {type=1,u=new UNION {ki=new KEYBOARD {vk=vk,scan=scan,flags=flags}}};
    if(SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)))!=1) throw new Exception("SendInput failed");
  }
}
'@
$handle = [TestWindow]::GetForegroundWindow(); [uint32]$foregroundPid = 0
[void][TestWindow]::GetWindowThreadProcessId($handle, [ref]$foregroundPid)
if ($foregroundPid -ne $RootProcessId) { throw "测试窗口失焦，已拒绝发送输入/截取其他窗口（PID $foregroundPid）" }
if ($Action -eq 'input') {
  if ($Text) { foreach ($character in $Text.ToCharArray()) {
    [uint32]$currentPid=0; [void][TestWindow]::GetWindowThreadProcessId([TestWindow]::GetForegroundWindow(),[ref]$currentPid)
    if ($currentPid -ne $RootProcessId) { throw '输入期间测试窗口失焦' }
    [TestWindow]::Key(0,[uint16]$character,4); [TestWindow]::Key(0,[uint16]$character,6); Start-Sleep -Milliseconds 15
  } }
  if ($Keys) {
    $map=@{ 'CTRL'=17; 'SHIFT'=16; 'A'=65; 'Z'=90; 'Y'=89; 'S'=83; 'END'=35; 'HOME'=36; 'PAGEDOWN'=34; 'PAGEUP'=33; 'BACKSPACE'=8; 'ENTER'=13; 'TAB'=9; 'ESC'=27 }
    $codes=@($Keys.ToUpperInvariant().Split('+') | ForEach-Object { if (!$map.ContainsKey($_)) { throw "不支持按键：$_" }; $map[$_] })
    try { foreach ($code in $codes) { [TestWindow]::Key($code,0,0) } }
    finally { [array]::Reverse($codes); foreach ($code in $codes) { [TestWindow]::Key($code,0,2) } }
  }
  '{"sent":true,"ime":false}'
} else {
  Add-Type -AssemblyName System.Drawing
  $rect=[TestWindow+RECT]::new(); [void][TestWindow]::GetWindowRect($handle,[ref]$rect)
  $bitmap=[Drawing.Bitmap]::new($rect.Right-$rect.Left,$rect.Bottom-$rect.Top)
  $graphics=[Drawing.Graphics]::FromImage($bitmap)
  try { $graphics.CopyFromScreen($rect.Left,$rect.Top,0,0,$bitmap.Size); $bitmap.Save($OutputPath,[Drawing.Imaging.ImageFormat]::Png) }
  finally { $graphics.Dispose(); $bitmap.Dispose() }
  '{"captured":true}'
}

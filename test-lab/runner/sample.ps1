param([int]$RootProcessId, [string]$OutputPath, [string]$StopPath, [int]$IntervalMs = 500)
$ErrorActionPreference = 'Stop'
# 高频路径只使用本机进程 API；WMI/GPU 查询在独立低频采样器中执行。
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class NomoProcesses {
 [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] public struct ENTRY {
  public uint size,usage,pid; public UIntPtr heap; public uint module,threads,parent; public int priority; public uint flags;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=260)] public string exe;
 }
 [StructLayout(LayoutKind.Sequential)] public struct IO { public ulong readOps,writeOps,otherOps,readBytes,writeBytes,otherBytes; }
 [DllImport("kernel32.dll")] static extern IntPtr CreateToolhelp32Snapshot(uint flags,uint pid);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern bool Process32FirstW(IntPtr h,ref ENTRY e);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern bool Process32NextW(IntPtr h,ref ENTRY e);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
 [DllImport("kernel32.dll")] public static extern bool GetProcessIoCounters(IntPtr h,out IO io);
 [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access,bool inherit,int pid);
 [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr h,out long created,out long exit,out long kernel,out long user);
 [StructLayout(LayoutKind.Sequential)] public struct MEMORY {
  public uint size,faults; public UIntPtr peakWorkingSet,workingSet,peakPaged,paged,peakNonPaged,nonPaged,pagefile,peakPagefile,privateUsage;
 }
 [DllImport("psapi.dll")] static extern bool GetProcessMemoryInfo(IntPtr h,ref MEMORY counters,uint size);
 public sealed class Sample { public string created; public double cpuMs; public ulong privateBytes,workingSet,workingSetPeak; public IO io; public bool hasIo; }
 public static Sample Read(int pid) {
  var h=OpenProcess(0x410,false,pid); if(h==IntPtr.Zero) throw new Exception("OpenProcess failed");
  try {
   long created,exit,kernel,user; if(!GetProcessTimes(h,out created,out exit,out kernel,out user)) throw new Exception("GetProcessTimes failed");
   var m=new MEMORY{size=(uint)Marshal.SizeOf(typeof(MEMORY))}; if(!GetProcessMemoryInfo(h,ref m,m.size))throw new Exception("GetProcessMemoryInfo failed");
   IO io; var hasIo=GetProcessIoCounters(h,out io);
   return new Sample{created=DateTime.FromFileTimeUtc(created).ToString("o"),cpuMs=(kernel+user)/10000d,privateBytes=m.privateUsage.ToUInt64(),workingSet=m.workingSet.ToUInt64(),workingSetPeak=m.peakWorkingSet.ToUInt64(),io=io,hasIo=hasIo};
  }finally{CloseHandle(h);}
 }
 public static Dictionary<int,int> Tree(int root) {
  var all=new Dictionary<int,int>(); var found=new Dictionary<int,int>{{root,0}};
  var h=CreateToolhelp32Snapshot(2,0); if(h==new IntPtr(-1)) throw new Exception("Process snapshot failed");
  try { var e=new ENTRY{size=(uint)Marshal.SizeOf(typeof(ENTRY))}; if(Process32FirstW(h,ref e)) do { all[(int)e.pid]=(int)e.parent; } while(Process32NextW(h,ref e)); }
  finally {CloseHandle(h);}
  bool added; do {added=false;foreach(var p in all)if(found.ContainsKey(p.Value)&&!found.ContainsKey(p.Key)){found[p.Key]=p.Value;added=true;}}while(added);
  return found;
 }
}
'@
$writer = [IO.StreamWriter]::new($OutputPath, $true, [Text.UTF8Encoding]::new($false)); $writer.AutoFlush = $true
$clock=[Diagnostics.Stopwatch]::StartNew(); $known=@{}; $previous=@{}; $lastDiscovery=-2000; $ready=$false
$counterPath="$OutputPath.counters.json"; $identity=@{}
try {
 while (!(Test-Path -LiteralPath $StopPath)) {
  $started=$clock.Elapsed.TotalMilliseconds
  if ($started-$lastDiscovery -ge 1000) {
   $tree=[NomoProcesses]::Tree($RootProcessId)
   foreach($processId in $tree.Keys){if(!$known.ContainsKey($processId)){$known[$processId]=$tree[$processId]}}
   $lastDiscovery=$started
  }
  $slow=$null
  if(Test-Path -LiteralPath $counterPath){try{$slow=Get-Content -LiteralPath $counterPath -Raw | ConvertFrom-Json}catch{}}
  # ConvertFrom-Json 可能已将 ISO 字符串转成 DateTime；不要再次经字符串转换丢失时区。
  $age=if($slow){([DateTime]::UtcNow-([DateTime]$slow.timestamp).ToUniversalTime()).TotalMilliseconds}else{$null}
  $fresh=$slow -and $age -le 15000
  $rows=@()
  foreach($processId in @($known.Keys)) {
   try {
    $p=[NomoProcesses]::Read($processId)
    $created=$p.created
    if($identity.ContainsKey($processId)-and $identity[$processId] -ne $created){throw 'PID reused'}
    $identity[$processId]=$created
    $cpu=$p.cpuMs; $now=$clock.Elapsed.TotalMilliseconds; $percent=$null
    if($previous.ContainsKey($processId)){$old=$previous[$processId];if($now -gt $old.Time){$percent=[Math]::Max(0,($cpu-$old.Cpu)/($now-$old.Time)*100)}}
    $previous[$processId]=@{Time=$now;Cpu=$cpu}
    $counter=if($fresh){@($slow.processes | Where-Object { $_.pid -eq $processId -and [Math]::Abs((([DateTime]$_.created).ToUniversalTime()-([DateTime]$created).ToUniversalTime()).TotalMilliseconds) -lt 1 }) | Select-Object -First 1}else{$null}
    $role=if($processId -eq $RootProcessId){'host'}elseif($counter){$counter.role}elseif($known[$processId] -eq $RootProcessId){'browser'}else{'webview-unclassified'}
    $io=$p.io; $hasIo=$p.hasIo
    $rows+=@{pid=$processId;parentPid=$known[$processId];role=$role;created=$created;cpuMs=$cpu;cpuCorePercent=$percent;
      cpuMachinePercent=$(if($null -ne $percent){$percent/[Environment]::ProcessorCount}else{$null});
      privateBytes=$p.privateBytes;workingSet=$p.workingSet;workingSetPeak=$p.workingSetPeak;
      workingSetPrivate=$(if($counter){$counter.workingSetPrivate}else{$null});
      ioReadBytes=$(if($hasIo){$io.readBytes}else{$null});ioWriteBytes=$(if($hasIo){$io.writeBytes}else{$null})}
   }catch{$rows+=@{pid=$processId;role='unknown';exited=$true};$known.Remove($processId);$previous.Remove($processId)}
  }
  $row=@{timestamp=[DateTime]::UtcNow.ToString('o');elapsedMs=$clock.Elapsed.TotalMilliseconds;samplerCostMs=$clock.Elapsed.TotalMilliseconds-$started;
    logicalProcessors=[Environment]::ProcessorCount;processes=$rows;countersAgeMs=$age;slowSamplerCostMs=$(if($slow){$slow.costMs}else{$null});
    gpuMemory=@(if($fresh){$slow.gpu});gpuUnavailable=$(if($fresh){$slow.gpuUnavailable}else{'低频 GPU 计数尚未就绪或已过期'});
    countersUnavailable=$(if($fresh){$slow.error}else{'低频私有工作集计数尚未就绪或已过期'})}
  $writer.WriteLine(($row | ConvertTo-Json -Depth 7 -Compress))
  if(!$ready -and $known.ContainsKey($RootProcessId)){
    [IO.File]::WriteAllText("$OutputPath.ready",(@{pid=$RootProcessId;at=$row.timestamp}|ConvertTo-Json -Compress));$ready=$true
  }
  if(!$known.ContainsKey($RootProcessId)){break}
  Start-Sleep -Milliseconds ([int][Math]::Max(1,$IntervalMs-($clock.Elapsed.TotalMilliseconds-$started)))
 }
}finally{$writer.Dispose()}

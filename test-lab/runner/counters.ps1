param([int]$RootProcessId,[string]$OutputPath,[string]$StopPath,[int]$IntervalMs=5000)
$ErrorActionPreference='Stop'
while(!(Test-Path -LiteralPath $StopPath)) {
 $clock=[Diagnostics.Stopwatch]::StartNew();$errorText=$null;$gpuError=$null;$rows=@();$gpu=@()
 try {
  $all=@(Get-CimInstance Win32_Process);$ids=[Collections.Generic.HashSet[int]]::new();[void]$ids.Add($RootProcessId)
  do{$added=$false;foreach($p in $all){if($ids.Contains([int]$p.ParentProcessId)-and $ids.Add([int]$p.ProcessId)){$added=$true}}}while($added)
  $private=@{};foreach($p in Get-CimInstance Win32_PerfRawData_PerfProc_Process){if($ids.Contains([int]$p.IDProcess)){$private[[int]$p.IDProcess]=[double]$p.WorkingSetPrivate}}
  foreach($p in $all | Where-Object {$ids.Contains([int]$_.ProcessId)}) {
   $role=if($p.ProcessId -eq $RootProcessId){'host'}elseif($p.CommandLine -match '--type=([^ ]+)'){$Matches[1]}else{'browser'}
   $rows+=@{pid=$p.ProcessId;created=$p.CreationDate.ToUniversalTime().ToString('o');role=$role;workingSetPrivate=$private[[int]$p.ProcessId]}
  }
  try{$gpu=@(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory | Where-Object {$_.Name -match '^pid_(\d+)_' -and $ids.Contains([int]$Matches[1])} | Select-Object Name,DedicatedUsage,SharedUsage,TotalCommitted)
    if(!$gpu.Count){$gpuError='No matching GPU memory counter instances'}
  }catch{$gpuError=$_.Exception.Message}
 }catch{$errorText=$_.Exception.Message}
 $result=@{timestamp=[DateTime]::UtcNow.ToString('o');costMs=$clock.Elapsed.TotalMilliseconds;processes=$rows;gpu=$gpu;gpuUnavailable=$gpuError;error=$errorText}
 [IO.File]::WriteAllText("$OutputPath.tmp",($result | ConvertTo-Json -Depth 6 -Compress))
 [IO.File]::Move("$OutputPath.tmp",$OutputPath,$true)
 $until=[Math]::Max(0,$IntervalMs-$clock.Elapsed.TotalMilliseconds)
 for($elapsed=0;$elapsed -lt $until -and !(Test-Path -LiteralPath $StopPath);$elapsed+=100){Start-Sleep -Milliseconds 100}
}

param([switch]$Check)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$nomoReferences = @(Get-ChildItem -LiteralPath (Join-Path $PSHOME 'ref') -Filter '*.dll' | ForEach-Object FullName)
$nomoReferences += [Drawing.Bitmap].Assembly.Location
$nomoReferences += @(Get-ChildItem -LiteralPath $PSHOME -Filter 'System.Private.Windows.*.dll' | ForEach-Object FullName)
Add-Type -Path (Join-Path $PSScriptRoot 'opening-capture.cs') -ReferencedAssemblies $nomoReferences
if ($Check) { '{"compiled":true}'; exit }
$nomoCapture = [OpeningCapture]::new()
'{"ready":true}'
try {
  while ($null -ne ($line = [Console]::ReadLine())) {
    try {
      $request = $line | ConvertFrom-Json
      $result = switch ($request.method) {
        'ping' { @{clock=[OpeningCapture]::Now()} }
        'start' { $nomoCapture.Start([int]$request.pid, [string]$request.path) }
        'mark' { $nomoCapture.Mark() }
        'region' { $nomoCapture.SetRegion([double]$request.x,[double]$request.y,[double]$request.width,[double]$request.height,[double]$request.scale) }
        'input' { $nomoCapture.TypeProbe() }
        'stop' { $nomoCapture.Stop() }
        'save' { $nomoCapture.SaveFrame([int]$request.index, [string]$request.path); @{saved=$true} }
        default { throw 'Unsupported capture command' }
      }
      @{ok=$true;result=$result} | ConvertTo-Json -Depth 8 -Compress
    } catch { @{ok=$false;error=$_.Exception.Message} | ConvertTo-Json -Compress }
  }
} finally { $nomoCapture.Dispose() }

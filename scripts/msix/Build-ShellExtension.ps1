#Requires -Version 7.0
[CmdletBinding()]
param()
. (Join-Path $PSScriptRoot 'Common.ps1')
$tools = Initialize-NomoMsixTools
$work = New-NomoMsixWorkDirectory 'shell'
$project = Join-Path $NomoRepositoryRoot 'src-gpui/windows-shell-extension/NomoShellExtension.vcxproj'
$output = Join-Path $work 'bin/'
$objects = Join-Path $work 'obj/'
$log = Join-Path $work 'build.log'
& $tools.MSBuild $project /nologo /m:1 /t:Build /p:Configuration=Release /p:Platform=x64 "/p:PlatformToolset=$($tools.PlatformToolset)" "/p:OutDir=$output" "/p:IntDir=$objects" /v:minimal "/flp:logfile=$log;verbosity=normal" | Out-Host
if ($LASTEXITCODE -ne 0) { throw "Shell extension build failed; see $log" }
$dll = Join-Path $output 'NomoShellExtension.dll'
Assert-NomoX64Pe $dll -Dll
$exports = & dumpbin /nologo /exports $dll
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Shell extension exports.' }
foreach ($name in @('DllGetClassObject', 'DllCanUnloadNow')) {
    if (-not ($exports -match "\s$name\s*$")) { throw "Shell extension is missing $name" }
}
if (@(Get-NomoRuntimeImports $dll).Count -ne 0) { throw 'Shell extension must use the static C++ runtime.' }
$result = [ordered]@{
    dll = $dll
    pdb = Join-Path $output 'NomoShellExtension.pdb'
    sha256 = (Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash.ToLowerInvariant()
    architecture = 'x64'
    configuration = 'Release'
    log = $log
    registered = $false
}
$json = $result | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $work 'shell-build-report.json'), $json, [Text.UTF8Encoding]::new($false))
# 唯一成功输出供 MSIX 构建读取；日志走 Host，不注册 DLL 或启动应用。
Write-Output $json

param(
  [ValidateSet('check','build','test')][string]$Mode = 'build',
  [ValidateSet('release','debug')][string]$Configuration = 'release',
  [switch]$DefaultBuild,
  [string]$Filter = 'automation'
)
$ErrorActionPreference = 'Stop'
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '../..'))
if ($IsWindows) {
  $nomoVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
  $nomoVs = & $nomoVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  if (!$nomoVs) { throw '未找到 MSVC x64 构建工具' }
  & (Join-Path $nomoVs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
}
$nomoArgs = @($Mode, '--locked', '--bin', 'nomo')
if (!$DefaultBuild) { $nomoArgs += @('--features', 'automation') }
if ($Mode -ne 'check' -and $Configuration -eq 'release') { $nomoArgs += '--release' }
if ($Mode -eq 'test') { $nomoArgs += @($Filter, '--', '--test-threads=1') }
& cargo @nomoArgs
exit $LASTEXITCODE

[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'Run Windows packaging in PowerShell 7 on Windows.' }
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $root
try {
    # 普通终端与 CI 共用入口，只在当前进程中准备 x64 构建环境。
    $missingTools = @('cl.exe', 'link.exe', 'dumpbin.exe', 'rc.exe') | Where-Object {
        -not (Get-Command $_ -CommandType Application -ErrorAction SilentlyContinue)
    }
    if ($missingTools -or [string]::IsNullOrWhiteSpace($env:VCToolsRedistDir) -or $env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
            throw 'Install Visual Studio C++ Build Tools and Windows SDK before packaging.'
        }
        $vsPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($vsPath)) {
            throw 'No Visual Studio installation with x64 C++ tools was found.'
        }
        $devShell = Join-Path $vsPath 'Common7/Tools/Launch-VsDevShell.ps1'
        if (-not (Test-Path -LiteralPath $devShell -PathType Leaf)) { throw "Missing $devShell; repair Visual Studio Build Tools." }
        & $devShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
    }
    foreach ($tool in @('cargo.exe', 'cl.exe', 'link.exe', 'dumpbin.exe', 'rc.exe')) {
        if (-not (Get-Command $tool -CommandType Application -ErrorAction SilentlyContinue)) {
            throw "Missing $tool; install Rust MSVC, Visual Studio C++ Build Tools and Windows SDK."
        }
    }
    if (-not (Get-Command fxc.exe -CommandType Application -ErrorAction SilentlyContinue) -and
        -not ($env:GPUI_FXC_PATH -and (Test-Path -LiteralPath $env:GPUI_FXC_PATH -PathType Leaf))) {
        throw 'Missing fxc.exe; install Windows SDK or set GPUI_FXC_PATH.'
    }
    if ([string]::IsNullOrWhiteSpace($env:VCToolsRedistDir)) { throw 'Visual Studio did not provide VCToolsRedistDir; repair the C++ tools installation.' }
    $makensis = Get-Command makensis -CommandType Application -ErrorAction SilentlyContinue
    if ($makensis) { $compiler = $makensis.Source }
    else { $compiler = Join-Path ${env:ProgramFiles(x86)} 'NSIS/makensis.exe' }
    if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) { throw 'Install NSIS 3 and add makensis to PATH.' }

    $metadataText = & cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
    $metadata = $metadataText | ConvertFrom-Json
    $package = $metadata.packages | Where-Object name -EQ 'nomo-gpui'
    $version = $package.version
    $target = 'x86_64-pc-windows-msvc'
    if (-not $SkipBuild) {
        & cargo build --locked --release --target $target --bin nomo
        if ($LASTEXITCODE -ne 0) { throw 'Cargo release build failed' }
    }
    $binary = Join-Path $metadata.target_directory "$target/release/nomo.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Missing $binary" }
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($binary))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Not a Windows PE executable' }
        $reader.BaseStream.Position = 0x3c
        $peOffset = $reader.ReadUInt32()
        $reader.BaseStream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'Expected an x64 PE executable' }
    } finally { $reader.Dispose() }
    if ((Get-Item -LiteralPath $binary).VersionInfo.ProductVersion -ne $version) { throw 'Executable version does not match Cargo' }
    # Rust MSVC 使用动态 VC 运行库；便携包必须能在未装 Build Tools 的机器上加载。
    $dependencyLines = & dumpbin /nologo /dependents $binary
    if ($LASTEXITCODE -ne 0) { throw 'dumpbin dependency inspection failed.' }
    $importedDlls = @($dependencyLines | ForEach-Object {
        if ($_ -match '^\s*([\w.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
    } | Sort-Object -Unique)
    $runtimeImports = @($importedDlls | Where-Object { $_ -match '^(vcruntime|msvcp|concrt)' })
    if ($runtimeImports.Count -ne 1 -or $runtimeImports[0] -ne 'vcruntime140.dll') {
        throw "VC runtime imports changed; update the native package payload: $runtimeImports"
    }
    $runtimeDirectories = @(Get-ChildItem -LiteralPath (Join-Path $env:VCToolsRedistDir 'x64') -Directory -Filter 'Microsoft.VC*.CRT')
    if ($runtimeDirectories.Count -ne 1) { throw 'Expected one x64 VC redistributable CRT directory.' }
    $runtime = Join-Path $runtimeDirectories[0].FullName 'vcruntime140.dll'
    if (-not (Test-Path -LiteralPath $runtime -PathType Leaf)) { throw 'Missing redistributable vcruntime140.dll' }
    $output = Join-Path $root '.artifacts/packages'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $stage = Join-Path $root ('.artifacts/package-work/windows-' + [guid]::NewGuid().ToString('N'))
    $app = Join-Path $stage 'Nomo'
    New-Item -ItemType Directory -Path (Join-Path $app 'samples/assets') -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $app 'nomo.exe')
    Copy-Item -LiteralPath $runtime -Destination (Join-Path $app 'vcruntime140.dll')
    Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $app 'LICENSE')
    Copy-Item -LiteralPath (Join-Path $root 'src-gpui/assets/icons/LICENSE') -Destination (Join-Path $app 'LICENSE-Lucide')
    Copy-Item -LiteralPath (Join-Path $root 'src-gpui/assets/licenses/LICENSE-STIX') -Destination (Join-Path $app 'LICENSE-STIX')
    Copy-Item -LiteralPath (Join-Path $root 'src-gpui/assets/licenses/LICENSE-Rendering') -Destination (Join-Path $app 'LICENSE-Rendering')
    Copy-Item -LiteralPath (Join-Path $root 'src-gpui/assets/licenses/LICENSE-Export') -Destination (Join-Path $app 'LICENSE-Export')
    Copy-Item -LiteralPath (Join-Path $root 'sample.md') -Destination (Join-Path $app 'samples/sample.md')
    Copy-Item -LiteralPath (Join-Path $root 'assets/128x128.png') -Destination (Join-Path $app 'samples/assets/128x128.png')
    $installer = Join-Path $output "Nomo_${version}_x64-setup.exe"
    $icon = Join-Path $root 'src-gpui/assets/app/icon.ico'
    $numericVersion = ($version -split '[-+]')[0] + '.0'
    & $compiler '/V3' "/DVERSION=$version" "/DNUMERICVERSION=$numericVersion" "/DPAYLOAD=$app" "/DOUTPUT=$installer" "/DAPPICON=$icon" (Join-Path $PSScriptRoot 'packaging/windows.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed' }
    Compress-Archive -LiteralPath $app -DestinationPath (Join-Path $output "Nomo_${version}_x64.zip") -Force
    Write-Output "Windows packages: $output"
} finally {
    Pop-Location
}

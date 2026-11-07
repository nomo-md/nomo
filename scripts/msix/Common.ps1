#Requires -Version 7.0
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'Run MSIX packaging in PowerShell 7 on Windows.' }
$NomoRepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$NomoMsixArtifactsRoot = Join-Path $NomoRepositoryRoot '.artifacts/msix'

function New-NomoMsixWorkDirectory([string]$Purpose) {
    if ($Purpose -notmatch '^[a-z-]+$') { throw 'Invalid MSIX work directory name.' }
    $path = Join-Path $NomoMsixArtifactsRoot ($Purpose + '-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $path | Out-Null
    return $path
}

function Resolve-NomoSdkTool([string]$ToolName) {
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    foreach ($sdk in (Get-ChildItem -LiteralPath $sdkBin -Directory |
            Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' |
            Sort-Object { [version]$_.Name } -Descending)) {
        $tool = Join-Path $sdk.FullName "x64/$ToolName"
        if (Test-Path -LiteralPath $tool -PathType Leaf) { return $tool }
    }
    throw "Missing Windows SDK tool: $ToolName"
}

function Initialize-NomoMsixTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { throw 'Install Visual Studio C++ Build Tools and Windows SDK.' }
    $vsPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 Microsoft.Component.MSBuild -property installationPath
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($vsPath)) { throw 'No x64 C++ Build Tools installation found.' }
    & (Join-Path $vsPath 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Host
    $msbuild = Join-Path $vsPath 'MSBuild/Current/Bin/MSBuild.exe'
    if (-not (Test-Path -LiteralPath $msbuild -PathType Leaf)) { throw 'MSBuild is missing.' }
    foreach ($name in @('cl.exe', 'link.exe', 'dumpbin.exe', 'rc.exe')) {
        if (-not (Get-Command $name -CommandType Application -ErrorAction SilentlyContinue)) { throw "Missing $name" }
    }
    $toolset = Get-ChildItem -Path (Join-Path $vsPath 'MSBuild/Microsoft/VC/v*/Platforms/x64/PlatformToolsets/*') -Directory |
        Where-Object Name -Match '^v\d+$' | Sort-Object { [int]$_.Name.Substring(1) } -Descending | Select-Object -First 1 -ExpandProperty Name
    if (-not $toolset) { throw 'No x64 C++ platform toolset found.' }
    return [pscustomobject]@{ MSBuild = $msbuild; PlatformToolset = $toolset }
}

function Assert-NomoX64Pe([string]$Path, [switch]$Dll) {
    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { throw "Invalid PE: $Path" }
        $stream.Position = 0x3c
        $offset = $reader.ReadUInt32()
        if ($offset -gt $stream.Length - 24) { throw "Invalid PE header offset: $Path" }
        $stream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) { throw "Expected x64 PE: $Path" }
        $stream.Position = $offset + 22
        $isDll = ($reader.ReadUInt16() -band 0x2000) -ne 0
        if ($isDll -ne $Dll.IsPresent) { throw "Unexpected PE image kind: $Path" }
    } finally { $reader.Dispose() }
}

function Get-NomoRuntimeImports([string]$Path) {
    $lines = & dumpbin /nologo /dependents $Path
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect PE imports: $Path" }
    return @($lines | ForEach-Object {
        if ($_ -match '^\s*([\w.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
    } | Where-Object { $_ -match '^(vcruntime|msvcp|concrt)' } | Sort-Object -Unique)
}

function Read-NomoMsixIdentity {
    $path = Join-Path $NomoRepositoryRoot 'scripts/packaging/msix/package-identity.json'
    $identity = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    foreach ($field in @('state', 'name', 'displayName', 'publisher', 'publisherDisplayName', 'applicationId', 'packageFamilyName', 'storeProductId')) {
        if ([string]::IsNullOrWhiteSpace([string]$identity.$field)) { throw "Missing package identity: $field" }
    }
    if ($identity.state -ne 'partnerCenter' -or $identity.applicationId -cne 'Nomo' -or
        $identity.name -cne '7D729C8F.NomoMarkdown' -or $identity.publisher -cne 'CN=B28DFEE9-0867-4248-BB11-F280549EFAB0' -or
        $identity.displayName -cne 'Nomo Markdown' -or $identity.publisherDisplayName -cne '清羽晚安' -or
        $identity.packageFamilyName -cne '7D729C8F.NomoMarkdown_zq15d798m6gb8' -or $identity.storeProductId -cne '9P1G24GK650Z') {
        throw 'MSIX identity does not match the existing Nomo product.'
    }
    return $identity
}

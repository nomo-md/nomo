#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackageRoot,
    [Parameter(Mandatory)][string]$MakePriPath
)
. (Join-Path $PSScriptRoot 'Common.ps1')
$package = (Resolve-Path -LiteralPath $PackageRoot).Path
$allowed = [IO.Path]::GetFullPath($NomoMsixArtifactsRoot).TrimEnd('\') + '\'
if (-not $package.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { throw 'PRI staging must be inside .artifacts/msix.' }
$manifest = Join-Path $package 'AppxManifest.xml'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw 'Package manifest is required before building PRI.' }
$source = Join-Path $NomoRepositoryRoot 'scripts/packaging/msix/Assets'
$assets = Join-Path $package 'Assets'
New-Item -ItemType Directory -Path $assets | Out-Null
foreach ($name in @('StoreLogo', 'Square44x44Logo', 'Square71x71Logo', 'Square150x150Logo')) {
    foreach ($suffix in @('', '.scale-125', '.scale-150', '.scale-200', '.scale-400')) {
        Copy-Item -LiteralPath (Join-Path $source "$name$suffix.png") -Destination $assets
    }
}
foreach ($size in @(16, 20, 24, 30, 32, 36, 40, 44, 48, 60, 64, 72, 80, 96, 256)) {
    foreach ($suffix in @('', '_altform-unplated', '_altform-lightunplated')) {
        Copy-Item -LiteralPath (Join-Path $source "Square44x44Logo.targetsize-$size$suffix.png") -Destination $assets
    }
}
$config = Join-Path (Split-Path -Parent $package) 'icon-priconfig.xml'
& $MakePriPath createconfig /cf $config /dq en-US
if ($LASTEXITCODE -ne 0) { throw 'MakePRI configuration failed.' }
[xml]$xml = Get-Content -LiteralPath $config -Raw
$packaging = $xml.SelectSingleNode('/resources/packaging')
if ($packaging) { $packaging.ParentNode.RemoveChild($packaging) | Out-Null }
$xml.Save($config)
$pri = Join-Path (Split-Path -Parent $package) 'resources.pri'
& $MakePriPath new /pr $package /cf $config /mn $manifest /of $pri
if ($LASTEXITCODE -ne 0) { throw 'MakePRI resource indexing failed.' }
Copy-Item -LiteralPath $pri -Destination (Join-Path $package 'resources.pri')

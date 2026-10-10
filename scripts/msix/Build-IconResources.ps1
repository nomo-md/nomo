[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PackageRoot,
    [Parameter(Mandatory)]
    [string]$MakePriPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$packageDirectory = [IO.Path]::GetFullPath($PackageRoot)
$iconsDirectory = Join-Path $repoRoot 'src-tauri\icons'
$variantDirectory = Join-Path $iconsDirectory 'nomo\windows'
$manifestPath = Join-Path $packageDirectory 'AppxManifest.xml'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "生成图标 PRI 前必须提供包清单：$manifestPath"
}
if (-not (Test-Path -LiteralPath $MakePriPath -PathType Leaf)) {
    throw "找不到 MakePRI：$MakePriPath"
}

$assetsDirectory = Join-Path $packageDirectory 'Assets'
New-Item -ItemType Directory -Path $assetsDirectory -Force | Out-Null
foreach ($asset in @('StoreLogo.png', 'Square44x44Logo.png', 'Square71x71Logo.png', 'Square150x150Logo.png')) {
    Copy-Item -LiteralPath (Join-Path $iconsDirectory $asset) -Destination (Join-Path $assetsDirectory $asset)
}

# 任务栏按目标像素尺寸及浅深色外观选图；缺失资源应在打包时明确失败。
foreach ($size in @(16, 20, 24, 30, 32, 36, 40, 44, 48, 60, 64, 72, 80, 96, 256)) {
    foreach ($suffix in @('', '_altform-unplated', '_altform-lightunplated')) {
        $asset = "Square44x44Logo.targetsize-$size$suffix.png"
        if (-not (Test-Path -LiteralPath (Join-Path $variantDirectory $asset) -PathType Leaf)) {
            throw "缺少图标资源 $asset，请先运行 pnpm icons:generate。"
        }
    }
}
foreach ($name in @('StoreLogo', 'Square44x44Logo', 'Square71x71Logo', 'Square150x150Logo')) {
    foreach ($scale in @(125, 150, 200, 400)) {
        $asset = "$name.scale-$scale.png"
        if (-not (Test-Path -LiteralPath (Join-Path $variantDirectory $asset) -PathType Leaf)) {
            throw "缺少图标资源 $asset，请先运行 pnpm icons:generate。"
        }
    }
}
Get-ChildItem -LiteralPath $variantDirectory -Filter '*.png' -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $assetsDirectory $_.Name)
}

$configPath = Join-Path (Split-Path -Parent $packageDirectory) 'icon-priconfig.xml'
& $MakePriPath createconfig /cf $configPath /dq en-US /o
if ($LASTEXITCODE -ne 0) {
    throw "MakePRI 创建配置失败，退出码：$LASTEXITCODE"
}
[xml]$config = Get-Content -LiteralPath $configPath -Raw
# 所有缩放资源随主包交付，不生成需要另行分发的资源包。
$packaging = $config.SelectSingleNode('/resources/packaging')
if ($packaging) {
    $packaging.ParentNode.RemoveChild($packaging) | Out-Null
}
$config.Save($configPath)
$priPath = Join-Path $packageDirectory 'resources.pri'
$generatedPriPath = Join-Path (Split-Path -Parent $packageDirectory) 'icon-resources.pri'
# 保留包内相对路径（Files/Assets/...），清除旧索引并在包目录之外生成。
if (Test-Path -LiteralPath $priPath -PathType Leaf) {
    Remove-Item -LiteralPath $priPath -Force
}
& $MakePriPath new /pr $packageDirectory /cf $configPath /mn $manifestPath /of $generatedPriPath /o
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $generatedPriPath -PathType Leaf)) {
    throw "MakePRI 图标资源索引生成失败，退出码：$LASTEXITCODE"
}
Copy-Item -LiteralPath $generatedPriPath -Destination $priPath -Force

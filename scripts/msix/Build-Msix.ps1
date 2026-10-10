[CmdletBinding()]
param(
    [ValidateSet('Dev', 'Store')]
    [string]$Mode = 'Dev',
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [switch]$SkipApplicationBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$artifactsRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot '.artifacts\msix'))
$stagingRoot = Join-Path $artifactsRoot 'staging'
$outputRoot = Join-Path $artifactsRoot 'output'
$identityPath = Join-Path $repoRoot 'src-tauri\msix\package-identity.json'
$manifestTemplatePath = Join-Path $repoRoot 'src-tauri\msix\AppxManifest.xml.template'

function Reset-TaskDirectory([string]$Path) {
    $resolved = [IO.Path]::GetFullPath($Path)
    $allowedPrefix = $artifactsRoot.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝清理 MSIX 工作目录之外的路径：$resolved"
    }
    if (Test-Path -LiteralPath $resolved) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    New-Item -ItemType Directory -Path $resolved -Force | Out-Null
}

function Resolve-WindowsSdkTool([string]$ToolName) {
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $candidates = Get-ChildItem -LiteralPath $sdkBin -Directory -ErrorAction Stop |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending
    foreach ($candidate in $candidates) {
        $tool = Join-Path $candidate.FullName "x64\$ToolName"
        if (Test-Path -LiteralPath $tool) {
            return $tool
        }
    }
    throw "找不到 Windows SDK 工具：$ToolName"
}

function Assert-NonEmpty([string]$Name, $Value) {
    if ([string]::IsNullOrWhiteSpace([string]$Value)) {
        throw "MSIX 身份字段不能为空：$Name"
    }
}

function Assert-ProductionFrontendEmbedded([string]$ExecutablePath, [string]$FrontendDirectory) {
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        throw "找不到待验证的 Tauri 应用程序：$ExecutablePath"
    }

    $indexPath = Join-Path $FrontendDirectory 'index.html'
    if (-not (Test-Path -LiteralPath $indexPath -PathType Leaf)) {
        throw "找不到生产前端入口：$indexPath"
    }

    $indexHtml = Get-Content -LiteralPath $indexPath -Raw
    $entryPaths = @(
        [regex]::Matches($indexHtml, '(?i)(?:src|href)\s*=\s*"(?<path>[^"]+)"') |
            ForEach-Object {
                $path = $_.Groups['path'].Value.Split('?', 2)[0].Split('#', 2)[0]
                $path.Replace('\', '/').TrimStart([char[]]@('.', '/'))
            } |
            Where-Object { $_ -like 'assets/*' } |
            Sort-Object -Unique
    )
    if ($entryPaths.Count -eq 0) {
        throw "生产前端入口未引用任何 assets 资源：$indexPath"
    }

    $executableText = [Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($ExecutablePath))
    $missingEntries = @(
        $entryPaths | Where-Object {
            $embeddedPath = '/' + $_
            $executableText.IndexOf($embeddedPath, [StringComparison]::Ordinal) -lt 0 -and
                $executableText.IndexOf($_, [StringComparison]::Ordinal) -lt 0
        }
    )
    if ($missingEntries.Count -gt 0) {
        throw "Tauri 应用程序未嵌入生产前端资源，疑似开发构建：$ExecutablePath；缺少：$($missingEntries -join ', ')"
    }

    return $entryPaths
}

function Resolve-PackageFamilyName([string]$Name, [string]$Publisher) {
    if (-not ('NomoMsix.PackageIdentityNative' -as [type])) {
        $nativeSource = @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace NomoMsix
{
    public static class PackageIdentityNative
    {
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode, Pack = 4)]
        private struct PackageId
        {
            public uint Reserved;
            public uint ProcessorArchitecture;
            public ulong Version;
            [MarshalAs(UnmanagedType.LPWStr)] public string Name;
            [MarshalAs(UnmanagedType.LPWStr)] public string Publisher;
            [MarshalAs(UnmanagedType.LPWStr)] public string ResourceId;
            [MarshalAs(UnmanagedType.LPWStr)] public string PublisherId;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        private static extern int PackageFamilyNameFromId(
            ref PackageId packageId,
            ref uint packageFamilyNameLength,
            StringBuilder packageFamilyName);

        public static string GetPackageFamilyName(string name, string publisher)
        {
            var packageId = new PackageId { Name = name, Publisher = publisher };
            uint length = 0;
            int result = PackageFamilyNameFromId(ref packageId, ref length, null);
            const int ErrorInsufficientBuffer = 122;
            if (result != ErrorInsufficientBuffer || length == 0)
            {
                throw new InvalidOperationException(
                    $"计算 Package Family Name 长度失败，Windows 错误码：{result}");
            }

            var buffer = new StringBuilder((int)length);
            result = PackageFamilyNameFromId(ref packageId, ref length, buffer);
            if (result != 0)
            {
                throw new InvalidOperationException(
                    $"计算 Package Family Name 失败，Windows 错误码：{result}");
            }
            return buffer.ToString();
        }
    }
}
'@
        Add-Type -TypeDefinition $nativeSource -Language CSharp
    }

    return [NomoMsix.PackageIdentityNative]::GetPackageFamilyName($Name, $Publisher)
}

function Compress-ToExtension([string]$SourceDirectory, [string]$DestinationPath) {
    $temporaryZip = "$DestinationPath.zip"
    if (Test-Path -LiteralPath $temporaryZip) {
        Remove-Item -LiteralPath $temporaryZip -Force
    }
    if (Test-Path -LiteralPath $DestinationPath) {
        Remove-Item -LiteralPath $DestinationPath -Force
    }
    Compress-Archive -Path (Join-Path $SourceDirectory '*') -DestinationPath $temporaryZip -CompressionLevel Optimal
    Move-Item -LiteralPath $temporaryZip -Destination $DestinationPath
}

$identity = Get-Content -LiteralPath $identityPath -Raw | ConvertFrom-Json
foreach ($field in @('state', 'name', 'displayName', 'publisher', 'publisherDisplayName', 'applicationId')) {
    Assert-NonEmpty $field $identity.$field
}
if ($identity.state -notin @('placeholder', 'partnerCenter')) {
    throw "MSIX 身份状态无效：$($identity.state)"
}
if ($identity.applicationId -ne 'Nomo') {
    throw 'MSIX ApplicationId 必须保持为 Nomo，以匹配 Shell 扩展运行时 AUMID。'
}
if ($identity.state -eq 'placeholder') {
    if (
        $identity.name -ne 'NomoMarkdown.Dev' -or
        $identity.publisher -ne 'CN=Nomo Development' -or
        $identity.publisherDisplayName -ne 'Nomo Development' -or
        -not [string]::IsNullOrWhiteSpace([string]$identity.packageFamilyName) -or
        -not [string]::IsNullOrWhiteSpace([string]$identity.storeProductId)
    ) {
        throw '占位身份必须使用仓库约定的开发值，且不得填写 PFN 或 Store Product ID。'
    }
}

if ($Mode -eq 'Store') {
    if ($identity.state -ne 'partnerCenter') {
        throw 'Store 构建已阻止：package-identity.json 仍为占位身份。'
    }
    Assert-NonEmpty 'packageFamilyName' $identity.packageFamilyName
    Assert-NonEmpty 'storeProductId' $identity.storeProductId
    if (
        $identity.name -match '(?i)\.dev$' -or
        $identity.displayName -match '(?i)development' -or
        $identity.publisher -match '(?i)development' -or
        $identity.publisherDisplayName -match '(?i)development'
    ) {
        throw 'Store 构建已阻止：身份配置仍包含开发占位值。'
    }
    $expectedPfnPattern = '^' + [regex]::Escape([string]$identity.name) + '_[a-z0-9]{13}$'
    if ([string]$identity.packageFamilyName -cnotmatch $expectedPfnPattern) {
        throw 'Store 构建已阻止：packageFamilyName 与 Partner Center Name 不一致或格式无效。'
    }
    $calculatedPfn = Resolve-PackageFamilyName ([string]$identity.name) ([string]$identity.publisher)
    if (-not [string]::Equals(
        [string]$identity.packageFamilyName,
        $calculatedPfn,
        [StringComparison]::Ordinal
    )) {
        throw "Store 构建已阻止：packageFamilyName 与 Name/Publisher 计算结果不一致，预期为 $calculatedPfn。"
    }
    if ([string]$identity.storeProductId -notmatch '^[A-Za-z0-9]{12}$') {
        throw 'Store 构建已阻止：storeProductId 格式无效。'
    }
}

$packageJson = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$tauriConfig = Get-Content -LiteralPath (Join-Path $repoRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$cargoToml = Get-Content -LiteralPath (Join-Path $repoRoot 'src-tauri\Cargo.toml') -Raw
$cargoVersionMatch = [regex]::Match($cargoToml, '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"')
if (-not $cargoVersionMatch.Success) {
    throw '无法从 src-tauri/Cargo.toml 读取版本号。'
}

$appVersion = [string]$packageJson.version
if ($appVersion -ne [string]$tauriConfig.version -or $appVersion -ne $cargoVersionMatch.Groups[1].Value) {
    throw "版本号不一致：package.json=$appVersion tauri.conf.json=$($tauriConfig.version) Cargo.toml=$($cargoVersionMatch.Groups[1].Value)"
}
if ($Mode -eq 'Store' -and $Configuration -ne 'Release') {
    throw 'Store MSIX 只允许使用 Release 配置。'
}
$frontendDistValue = [string]$tauriConfig.build.frontendDist
Assert-NonEmpty 'build.frontendDist' $frontendDistValue
if ($frontendDistValue -match '^[a-z][a-z0-9+.-]*://') {
    throw "MSIX 构建要求 build.frontendDist 为本地生产目录，当前为：$frontendDistValue"
}
$frontendDist = [IO.Path]::GetFullPath((Join-Path (Join-Path $repoRoot 'src-tauri') $frontendDistValue))
if ($appVersion -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
    throw "MSIX 只接受稳定三段 SemVer，当前为：$appVersion"
}
$semverMajor = [int]$Matches[1]
$semverMinor = [int]$Matches[2]
$semverPatch = [int]$Matches[3]
$msixSegments = @(($semverMajor + 1), $semverMinor, $semverPatch, 0)
if ($msixSegments | Where-Object { $_ -lt 0 -or $_ -gt 65535 }) {
    throw "MSIX 版本字段超出 0..65535：$($msixSegments -join '.')"
}
$msixVersion = $msixSegments -join '.'

Reset-TaskDirectory $stagingRoot
Reset-TaskDirectory $outputRoot

if (-not $SkipApplicationBuild) {
    Push-Location $repoRoot
    try {
        & pnpm.cmd tauri build --target x86_64-pc-windows-msvc --no-bundle
        if ($LASTEXITCODE -ne 0) {
            throw "Tauri x64 Release 构建失败，退出码：$LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

& (Join-Path $PSScriptRoot 'Build-ShellExtension.ps1') -Configuration $Configuration -RunTests

$applicationTarget = Join-Path $repoRoot 'src-tauri\target\x86_64-pc-windows-msvc\release'
$applicationPath = Join-Path $applicationTarget 'nomo.exe'
if (-not (Test-Path -LiteralPath $applicationPath -PathType Leaf)) {
    throw "找不到本次 x64 Tauri Release 输出，拒绝回退到可能过期的其他目标目录：$applicationPath"
}
$embeddedFrontendEntries = @(Assert-ProductionFrontendEmbedded $applicationPath $frontendDist)
$applicationSha256 = (Get-FileHash -LiteralPath $applicationPath -Algorithm SHA256).Hash.ToLowerInvariant()

Copy-Item -LiteralPath $applicationPath -Destination (Join-Path $stagingRoot 'nomo.exe')
$resourcesPath = Join-Path $applicationTarget 'resources'
if (Test-Path -LiteralPath $resourcesPath) {
    Copy-Item -LiteralPath $resourcesPath -Destination (Join-Path $stagingRoot 'resources') -Recurse
}

$shellOutput = Join-Path $repoRoot "src-tauri\windows-shell-extension\bin\x64\$Configuration"
Copy-Item -LiteralPath (Join-Path $shellOutput 'NomoShellExtension.dll') -Destination (Join-Path $stagingRoot 'NomoShellExtension.dll')

$manifest = Get-Content -LiteralPath $manifestTemplatePath -Raw
$replacements = @{
    '{{PACKAGE_NAME}}' = [Security.SecurityElement]::Escape([string]$identity.name)
    '{{DISPLAY_NAME}}' = [Security.SecurityElement]::Escape([string]$identity.displayName)
    '{{PUBLISHER}}' = [Security.SecurityElement]::Escape([string]$identity.publisher)
    '{{PUBLISHER_DISPLAY_NAME}}' = [Security.SecurityElement]::Escape([string]$identity.publisherDisplayName)
    '{{APPLICATION_ID}}' = [Security.SecurityElement]::Escape([string]$identity.applicationId)
    '{{PACKAGE_VERSION}}' = $msixVersion
}
foreach ($entry in $replacements.GetEnumerator()) {
    $manifest = $manifest.Replace($entry.Key, $entry.Value)
}
if ($manifest -match '\{\{[A-Z_]+\}\}') {
    throw 'AppxManifest.xml 仍包含未替换的模板字段。'
}
$manifestPath = Join-Path $stagingRoot 'AppxManifest.xml'
[IO.File]::WriteAllText($manifestPath, $manifest, [Text.UTF8Encoding]::new($false))

[xml]$manifestXml = $manifest
$namespaceManager = [Xml.XmlNamespaceManager]::new($manifestXml.NameTable)
$namespaceManager.AddNamespace('f', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
$namespaceManager.AddNamespace('uap', 'http://schemas.microsoft.com/appx/manifest/uap/windows10')
$namespaceManager.AddNamespace('com', 'http://schemas.microsoft.com/appx/manifest/com/windows10')
$namespaceManager.AddNamespace('desktop4', 'http://schemas.microsoft.com/appx/manifest/desktop/windows10/4')
$namespaceManager.AddNamespace('desktop5', 'http://schemas.microsoft.com/appx/manifest/desktop/windows10/5')
$namespaceManager.AddNamespace('rescap', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities')
$identityNode = $manifestXml.SelectSingleNode('/f:Package/f:Identity', $namespaceManager)
if (
    -not $identityNode -or
    $identityNode.Name -ne [string]$identity.name -or
    $identityNode.Publisher -ne [string]$identity.publisher -or
    $identityNode.Version -ne $msixVersion -or
    $identityNode.ProcessorArchitecture -ne 'x64'
) {
    throw '生成的 AppxManifest.xml 身份验证失败。'
}
$displayNameNode = $manifestXml.SelectSingleNode('/f:Package/f:Properties/f:DisplayName', $namespaceManager)
$publisherDisplayNameNode = $manifestXml.SelectSingleNode(
    '/f:Package/f:Properties/f:PublisherDisplayName',
    $namespaceManager
)
$visualElementsNode = $manifestXml.SelectSingleNode(
    '/f:Package/f:Applications/f:Application/uap:VisualElements',
    $namespaceManager
)
if (
    -not $displayNameNode -or
    $displayNameNode.InnerText -ne [string]$identity.displayName -or
    -not $publisherDisplayNameNode -or
    $publisherDisplayNameNode.InnerText -ne [string]$identity.publisherDisplayName -or
    -not $visualElementsNode -or
    $visualElementsNode.DisplayName -ne [string]$identity.displayName
) {
    throw '生成的 AppxManifest.xml 显示名称验证失败。'
}
$applicationNode = $manifestXml.SelectSingleNode('/f:Package/f:Applications/f:Application', $namespaceManager)
if (
    -not $applicationNode -or
    $applicationNode.Id -ne 'Nomo' -or
    $applicationNode.Executable -ne 'nomo.exe' -or
    $applicationNode.EntryPoint -ne 'Windows.FullTrustApplication'
) {
    throw '生成的 AppxManifest.xml 应用入口验证失败。'
}
$targetFamily = $manifestXml.SelectSingleNode('/f:Package/f:Dependencies/f:TargetDeviceFamily', $namespaceManager)
if (
    -not $targetFamily -or
    $targetFamily.Name -ne 'Windows.Desktop' -or
    $targetFamily.MinVersion -ne '10.0.19045.0' -or
    $targetFamily.MaxVersionTested -ne '10.0.26200.0'
) {
    throw '生成的 AppxManifest.xml Windows.Desktop 版本范围验证失败。'
}
$manifestFileTypes = @(
    $manifestXml.SelectNodes('//uap:FileTypeAssociation/uap:SupportedFileTypes/uap:FileType', $namespaceManager) |
        ForEach-Object { $_.InnerText }
)
$expectedFileTypes = @('.md', '.markdown', '.txt', '.json')
if (Compare-Object -ReferenceObject $expectedFileTypes -DifferenceObject $manifestFileTypes) {
    throw "生成的 AppxManifest.xml 文件关联不一致：$($manifestFileTypes -join ', ')"
}
$expectedCommands = @{
    'BC62B998-15B3-4A1A-A4EC-35BFCF652D70' = 'NomoOpenMarkdown'
    '345A9E7E-91CD-4C79-B69C-EFC7CF8E8408' = @('NomoOpenFolder', 'NomoOpenFolderBackground')
}
foreach ($classId in $expectedCommands.Keys) {
    $classNode = $manifestXml.SelectSingleNode("//com:Class[@Id='$classId']", $namespaceManager)
    if (-not $classNode -or $classNode.Path -ne 'NomoShellExtension.dll' -or $classNode.ThreadingModel -ne 'STA') {
        throw "生成的 AppxManifest.xml COM 类验证失败：$classId"
    }
    foreach ($verbId in @($expectedCommands[$classId])) {
        $verbNode = $manifestXml.SelectSingleNode("//desktop5:Verb[@Id='$verbId' and @Clsid='$classId']", $namespaceManager)
        if (-not $verbNode) {
            throw "生成的 AppxManifest.xml Explorer 命令验证失败：$verbId"
        }
    }
}
$fullTrustCapability = $manifestXml.SelectSingleNode('/f:Package/f:Capabilities/rescap:Capability[@Name="runFullTrust"]', $namespaceManager)
if (-not $fullTrustCapability) {
    throw '生成的 AppxManifest.xml 缺少 runFullTrust。'
}

& (Join-Path $PSScriptRoot 'Build-IconResources.ps1') -PackageRoot $stagingRoot -MakePriPath (Resolve-WindowsSdkTool 'makepri.exe')

$makeAppx = Resolve-WindowsSdkTool 'makeappx.exe'
$signTool = Resolve-WindowsSdkTool 'signtool.exe'
$modeSuffix = if ($Mode -eq 'Dev') { 'DEV-NOT-FOR-STORE' } else { 'STORE' }
$msixName = "Nomo_${appVersion}_x64_$modeSuffix.msix"
$msixPath = Join-Path $outputRoot $msixName

& $makeAppx pack /o /h SHA256 /d $stagingRoot /p $msixPath
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx 打包失败，退出码：$LASTEXITCODE"
}

$packageValidationRoot = Join-Path $artifactsRoot 'package-validation'
Reset-TaskDirectory $packageValidationRoot
& $makeAppx unpack /o /p $msixPath /d $packageValidationRoot
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx 回读验证失败，退出码：$LASTEXITCODE"
}
foreach ($requiredPackageFile in @('AppxManifest.xml', 'nomo.exe', 'NomoShellExtension.dll', 'resources.pri')) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageValidationRoot $requiredPackageFile))) {
        throw "MSIX 回读验证缺少必要文件：$requiredPackageFile"
    }
}
$packagedApplicationPath = Join-Path $packageValidationRoot 'nomo.exe'
$packagedFrontendEntries = @(Assert-ProductionFrontendEmbedded $packagedApplicationPath $frontendDist)
$packagedApplicationSha256 = (Get-FileHash -LiteralPath $packagedApplicationPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($packagedApplicationSha256 -ne $applicationSha256) {
    throw "MSIX 回读后的 nomo.exe 与已验证的 Tauri Release 输出不一致：expected=$applicationSha256 actual=$packagedApplicationSha256"
}
if (Compare-Object -ReferenceObject $embeddedFrontendEntries -DifferenceObject $packagedFrontendEntries) {
    throw 'MSIX 回读后的生产前端资源清单与已验证的 Tauri Release 输出不一致。'
}
$forbiddenPackageFiles = Get-ChildItem -LiteralPath $packageValidationRoot -Recurse -File |
    Where-Object {
        $_.Extension -in @('.pfx', '.p12', '.pem', '.key', '.cer') -or
        $_.Name -match '(?i)password|secret'
    }
if ($forbiddenPackageFiles) {
    throw "MSIX 包含禁止的证书或秘密文件：$($forbiddenPackageFiles.FullName -join ', ')"
}

$signed = $false
$certificateOutputPath = $null
if ($Mode -eq 'Dev') {
    $certificateInfo = (& (Join-Path $PSScriptRoot 'New-DevCertificate.ps1')) | ConvertFrom-Json
    $certificatePassword = (Get-Content -LiteralPath $certificateInfo.PasswordPath -Raw).Trim()
    & $signTool sign /fd SHA256 /f $certificateInfo.PfxPath /p $certificatePassword $msixPath
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool 开发签名失败，退出码：$LASTEXITCODE"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $msixPath
    if (-not $signature.SignerCertificate -or $signature.SignerCertificate.Subject -ne [string]$identity.publisher) {
        throw '开发 MSIX 已签名，但签名证书 Publisher 与清单不一致。'
    }
    $certificateOutputPath = Join-Path $outputRoot 'NomoDevelopment.cer'
    Copy-Item -LiteralPath $certificateInfo.CerPath -Destination $certificateOutputPath
    $signed = $true
}

$symbolsStaging = Join-Path $artifactsRoot 'symbols'
Reset-TaskDirectory $symbolsStaging
$symbolCandidates = @(
    (Join-Path $applicationTarget 'nomo.pdb'),
    (Join-Path $shellOutput 'NomoShellExtension.pdb')
)
foreach ($symbol in $symbolCandidates) {
    if (Test-Path -LiteralPath $symbol) {
        Copy-Item -LiteralPath $symbol -Destination (Join-Path $symbolsStaging (Split-Path $symbol -Leaf))
    }
}

$appxSymPath = $null
$uploadPath = $null
if ($Mode -eq 'Store') {
    if (-not (Get-ChildItem -LiteralPath $symbolsStaging -File -ErrorAction SilentlyContinue)) {
        throw 'Store 构建缺少 PDB，无法生成 .appxsym。'
    }
    $appxSymPath = Join-Path $outputRoot "Nomo_${appVersion}_x64.appxsym"
    Compress-ToExtension $symbolsStaging $appxSymPath

    $uploadStaging = Join-Path $artifactsRoot 'upload'
    Reset-TaskDirectory $uploadStaging
    Copy-Item -LiteralPath $msixPath -Destination (Join-Path $uploadStaging (Split-Path $msixPath -Leaf))
    Copy-Item -LiteralPath $appxSymPath -Destination (Join-Path $uploadStaging (Split-Path $appxSymPath -Leaf))
    $uploadPath = Join-Path $outputRoot "Nomo_${appVersion}_x64.msixupload"
    Compress-ToExtension $uploadStaging $uploadPath
}

$report = [ordered]@{
    mode = $Mode
    identityState = [string]$identity.state
    packageName = [string]$identity.name
    packageDisplayName = [string]$identity.displayName
    publisher = [string]$identity.publisher
    publisherDisplayName = [string]$identity.publisherDisplayName
    configuredPackageFamilyName = $identity.packageFamilyName
    storeProductIdConfigured = -not [string]::IsNullOrWhiteSpace([string]$identity.storeProductId)
    applicationVersion = $appVersion
    msixVersion = $msixVersion
    architecture = 'x64'
    minWindowsVersion = '10.0.19045.0'
    maxWindowsVersionTested = '10.0.26200.0'
    webView2 = 'Evergreen'
    applicationSha256 = $applicationSha256
    embeddedFrontendEntryCount = $embeddedFrontendEntries.Count
    embeddedFrontendEntries = $embeddedFrontendEntries
    signedForSideload = $signed
    storeUploadAllowed = $Mode -eq 'Store'
    msix = $msixPath
    appxSym = $appxSymPath
    msixUpload = $uploadPath
    sha256 = (Get-FileHash -LiteralPath $msixPath -Algorithm SHA256).Hash.ToLowerInvariant()
}
$reportPath = Join-Path $outputRoot 'msix-validation-report.json'
[IO.File]::WriteAllText(
    $reportPath,
    ($report | ConvertTo-Json -Depth 5),
    [Text.UTF8Encoding]::new($false)
)

Write-Host "MSIX 构建完成：$msixPath"
if ($Mode -eq 'Dev') {
    Write-Host "开发证书：$certificateOutputPath"
    Write-Host "安装命令：pwsh -NoProfile -File scripts/msix/Install-DevMsix.ps1 -PackagePath `"$msixPath`" -CertificatePath `"$certificateOutputPath`""
} else {
    Write-Host "Store 上传文件：$uploadPath"
}
Write-Host "验证报告：$reportPath"

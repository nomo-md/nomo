#Requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateSet('Review', 'Store')][string]$Mode = 'Review',
    [switch]$SkipApplicationBuild,
    [string]$ApplicationPath,
    [string]$ExpectedApplicationSha256
)
. (Join-Path $PSScriptRoot 'Common.ps1')
$identity = Read-NomoMsixIdentity
$cargo = Get-Content -LiteralPath (Join-Path $NomoRepositoryRoot 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($cargo, '(?ms)^\[workspace\.package\]\s*\r?\n(?:(?!^\[).)*?^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (-not $versionMatch.Success) { throw 'Root Cargo.toml must declare a stable workspace.package.version.' }
$version = $versionMatch.Groups[1].Value
$parts = @($version.Split('.') | ForEach-Object { [long]$_ })
# 沿用原包版本映射，避免 GPUI 切换造成 Windows 版本倒退；第四段留给 Store。
$packageParts = @(($parts[0] + 1), $parts[1], $parts[2], 0)
if ($packageParts | Where-Object { $_ -lt 0 -or $_ -gt 65535 }) { throw 'MSIX version exceeds the 16-bit component range.' }
$packageVersion = $packageParts -join '.'
if ($SkipApplicationBuild) {
    if ([string]::IsNullOrWhiteSpace($ApplicationPath) -or $ExpectedApplicationSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw '-SkipApplicationBuild requires -ApplicationPath and the SHA-256 of the verified Release executable.'
    }
} elseif ($ApplicationPath -or $ExpectedApplicationSha256) {
    throw 'Explicit executable inputs require -SkipApplicationBuild.'
}
$tools = Initialize-NomoMsixTools
$makeAppx = Resolve-NomoSdkTool 'makeappx.exe'
$makePri = Resolve-NomoSdkTool 'makepri.exe'
if (-not $SkipApplicationBuild) {
    Push-Location $NomoRepositoryRoot
    try {
        $metadataText = & cargo metadata --no-deps --format-version 1 --locked
        if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
        $metadata = $metadataText | ConvertFrom-Json
        $package = @($metadata.packages | Where-Object name -EQ 'nomo-gpui')
        if ($package.Count -ne 1 -or $package[0].version -ne $version) { throw 'Cargo package version does not match the root workspace.' }
        & cargo build --locked --release --target x86_64-pc-windows-msvc --bin nomo
        if ($LASTEXITCODE -ne 0) { throw 'Native Nomo Release build failed.' }
        $ApplicationPath = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release/nomo.exe'
    } finally { Pop-Location }
}
$application = (Resolve-Path -LiteralPath $ApplicationPath).Path
if ([IO.Path]::GetFileName($application) -ine 'nomo.exe' -or (Split-Path (Split-Path $application -Parent) -Leaf) -ine 'release') {
    throw 'Package input must be a Release nomo.exe.'
}
Assert-NomoX64Pe $application
if ((Get-Item -LiteralPath $application).VersionInfo.ProductVersion -ne $version) { throw 'Executable product version does not match Cargo.' }
$applicationHash = (Get-FileHash -LiteralPath $application -Algorithm SHA256).Hash.ToLowerInvariant()
if ($SkipApplicationBuild -and $applicationHash -ine $ExpectedApplicationSha256) { throw 'Executable SHA-256 does not match the verified Release input.' }
$runtimeImports = @(Get-NomoRuntimeImports $application)
if ($runtimeImports.Count -ne 1 -or $runtimeImports[0] -ne 'vcruntime140.dll') { throw 'Native runtime imports changed; review the MSIX runtime payload.' }
if ([string]::IsNullOrWhiteSpace($env:VCToolsRedistDir)) { throw 'The Visual C++ redistributable directory is unavailable.' }
$crt = @(Get-ChildItem -LiteralPath (Join-Path $env:VCToolsRedistDir 'x64') -Directory -Filter 'Microsoft.VC*.CRT')
if ($crt.Count -ne 1) { throw 'Expected one x64 redistributable CRT directory.' }
$runtime = Join-Path $crt[0].FullName 'vcruntime140.dll'
Assert-NomoX64Pe $runtime -Dll
$shell = (& (Join-Path $PSScriptRoot 'Build-ShellExtension.ps1')) | ConvertFrom-Json
$work = New-NomoMsixWorkDirectory 'package'
$stage = Join-Path $work 'payload'
$output = Join-Path $work 'output'
New-Item -ItemType Directory -Path $stage, $output | Out-Null
Copy-Item -LiteralPath $application -Destination (Join-Path $stage 'nomo.exe')
Copy-Item -LiteralPath $runtime -Destination (Join-Path $stage 'vcruntime140.dll')
Copy-Item -LiteralPath $shell.dll -Destination (Join-Path $stage 'NomoShellExtension.dll')
Copy-Item -LiteralPath (Join-Path $NomoRepositoryRoot 'LICENSE') -Destination $stage
Copy-Item -LiteralPath (Join-Path $NomoRepositoryRoot 'src-gpui/assets/icons/LICENSE') -Destination (Join-Path $stage 'LICENSE-Lucide')
foreach ($name in @('STIX', 'Rendering', 'Export', 'Windows')) {
    Copy-Item -LiteralPath (Join-Path $NomoRepositoryRoot "src-gpui/assets/licenses/LICENSE-$name") -Destination $stage
}
New-Item -ItemType Directory -Path (Join-Path $stage 'samples/assets') | Out-Null
Copy-Item -LiteralPath (Join-Path $NomoRepositoryRoot 'sample.md') -Destination (Join-Path $stage 'samples/sample.md')
Copy-Item -LiteralPath (Join-Path $NomoRepositoryRoot 'assets/128x128.png') -Destination (Join-Path $stage 'samples/assets/128x128.png')
$manifest = Get-Content -LiteralPath (Join-Path $NomoRepositoryRoot 'scripts/packaging/msix/AppxManifest.xml.template') -Raw
$replacements = @{
    '{{PACKAGE_NAME}}' = $identity.name; '{{PUBLISHER}}' = $identity.publisher
    '{{DISPLAY_NAME}}' = $identity.displayName; '{{PUBLISHER_DISPLAY_NAME}}' = $identity.publisherDisplayName
    '{{APPLICATION_ID}}' = $identity.applicationId; '{{PACKAGE_VERSION}}' = $packageVersion
}
foreach ($entry in $replacements.GetEnumerator()) {
    $manifest = $manifest.Replace($entry.Key, [Security.SecurityElement]::Escape([string]$entry.Value))
}
if ($manifest -match '\{\{[A-Z_]+\}\}') { throw 'Unresolved package manifest placeholder.' }
$manifestPath = Join-Path $stage 'AppxManifest.xml'
[IO.File]::WriteAllText($manifestPath, $manifest, [Text.UTF8Encoding]::new($false))
[xml]$xml = $manifest
$ns = [Xml.XmlNamespaceManager]::new($xml.NameTable)
foreach ($entry in @{
    f = 'http://schemas.microsoft.com/appx/manifest/foundation/windows10'
    uap = 'http://schemas.microsoft.com/appx/manifest/uap/windows10'
    uap3 = 'http://schemas.microsoft.com/appx/manifest/uap/windows10/3'
    com = 'http://schemas.microsoft.com/appx/manifest/com/windows10'
    desktop5 = 'http://schemas.microsoft.com/appx/manifest/desktop/windows10/5'
}.GetEnumerator()) { $ns.AddNamespace($entry.Key, $entry.Value) }
$applicationNode = $xml.SelectSingleNode('/f:Package/f:Applications/f:Application', $ns)
if ($applicationNode.Id -cne 'Nomo' -or $applicationNode.Executable -cne 'nomo.exe' -or $applicationNode.EntryPoint -cne 'Windows.FullTrustApplication') { throw 'Unexpected package application entrypoint.' }
$fileTypes = @($xml.SelectNodes('//uap:SupportedFileTypes/uap:FileType', $ns) | ForEach-Object InnerText)
if (Compare-Object -ReferenceObject @('.md', '.markdown') -DifferenceObject $fileTypes) { throw 'MSIX associations must contain only Markdown.' }
$association = $xml.SelectSingleNode('//uap3:FileTypeAssociation', $ns)
if ($association.Parameters -cne '"%1"') { throw 'File activation must quote its path argument.' }
$commands = @{
    'NomoOpenMarkdown' = @('BC62B998-15B3-4A1A-A4EC-35BFCF652D70', '*')
    'NomoOpenFolder' = @('345A9E7E-91CD-4C79-B69C-EFC7CF8E8408', 'Directory')
    'NomoOpenFolderBackground' = @('345A9E7E-91CD-4C79-B69C-EFC7CF8E8408', 'Directory\Background')
}
foreach ($entry in $commands.GetEnumerator()) {
    $classId, $itemType = $entry.Value
    $verb = $xml.SelectSingleNode("//desktop5:Verb[@Id='$($entry.Key)']", $ns)
    $class = $xml.SelectSingleNode("//com:Class[@Id='$classId']", $ns)
    if (-not $verb -or $verb.Clsid -cne $classId -or $verb.ParentNode.Type -cne $itemType -or
        -not $class -or $class.Path -cne 'NomoShellExtension.dll' -or $class.ThreadingModel -cne 'STA') {
        throw "Invalid Explorer command registration: $($entry.Key)"
    }
}
& (Join-Path $PSScriptRoot 'Build-IconResources.ps1') -PackageRoot $stage -MakePriPath $makePri | Out-Host
if ((Get-FileHash -LiteralPath (Join-Path $stage 'nomo.exe') -Algorithm SHA256).Hash -ine $applicationHash) { throw 'Application changed while staging.' }
if ((Get-FileHash -LiteralPath (Join-Path $stage 'NomoShellExtension.dll') -Algorithm SHA256).Hash -ine $shell.sha256) { throw 'Shell extension changed while staging.' }
$suffix = if ($Mode -eq 'Store') { 'STORE-UNSIGNED' } else { 'REVIEW-UNSIGNED' }
$msix = Join-Path $output "Nomo_${version}_x64_$suffix.msix"
& $makeAppx pack /h SHA256 /d $stage /p $msix | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx packaging or manifest validation failed.' }
$unpacked = Join-Path $work 'unpacked'
& $makeAppx unpack /p $msix /d $unpacked | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx package readback failed.' }
# 回读核对整个白名单载荷，证书、密钥和旧 WebView 资源没有进入包的复制入口。
$payloadFiles = @(Get-ChildItem -LiteralPath $stage -File -Recurse)
foreach ($file in $payloadFiles) {
    $relative = [IO.Path]::GetRelativePath($stage, $file.FullName)
    $copy = Join-Path $unpacked $relative
    if (-not (Test-Path -LiteralPath $copy -PathType Leaf) -or
        (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash) {
        throw "MSIX readback does not match payload: $relative"
    }
}
$unexpected = @(Get-ChildItem -LiteralPath $unpacked -Recurse -File | Where-Object {
    $_.Extension -in @('.pfx', '.p12', '.pem', '.key', '.cer') -or $_.Name -match '(?i)password|secret|AppxSignature'
})
if ($unexpected.Count -gt 0) { throw 'Unsigned MSIX contains forbidden signing or secret files.' }
$symbols = $null
$upload = $null
if ($Mode -eq 'Store') {
    $symbolStage = Join-Path $work 'symbols'
    $uploadStage = Join-Path $work 'upload'
    New-Item -ItemType Directory -Path $symbolStage, $uploadStage | Out-Null
    foreach ($pdb in @([IO.Path]::ChangeExtension($application, '.pdb'), $shell.pdb)) {
        if (-not (Test-Path -LiteralPath $pdb -PathType Leaf)) { throw "Store symbols are missing: $pdb" }
        Copy-Item -LiteralPath $pdb -Destination $symbolStage
    }
    $symbols = Join-Path $output "Nomo_${version}_x64.appxsym"
    Compress-Archive -Path (Join-Path $symbolStage '*') -DestinationPath "$symbols.zip"
    Move-Item -LiteralPath "$symbols.zip" -Destination $symbols
    Copy-Item -LiteralPath $msix, $symbols -Destination $uploadStage
    $upload = Join-Path $output "Nomo_${version}_x64.msixupload"
    Compress-Archive -Path (Join-Path $uploadStage '*') -DestinationPath "$upload.zip"
    Move-Item -LiteralPath "$upload.zip" -Destination $upload
}
$report = [ordered]@{
    mode = $Mode; packageName = $identity.name; packageFamilyName = $identity.packageFamilyName
    applicationVersion = $version; msixVersion = $packageVersion; architecture = 'x64'
    applicationSource = $application; applicationSha256 = $applicationHash; shellSha256 = $shell.sha256
    inputVerification = $(if ($SkipApplicationBuild) { 'caller-supplied Release SHA-256' } else { 'cargo --locked --release --target x86_64-pc-windows-msvc --bin nomo' })
    payloadFilesVerified = $payloadFiles.Count; signed = $false; installed = $false
    msix = $msix; appxSym = $symbols; msixUpload = $upload
    sha256 = (Get-FileHash -LiteralPath $msix -Algorithm SHA256).Hash.ToLowerInvariant()
    validation = 'PE, version, imports, manifest schema, and payload readback; no installation or desktop acceptance'
}
$json = $report | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText((Join-Path $output 'msix-validation-report.json'), $json, [Text.UTF8Encoding]::new($false))
Write-Output $json

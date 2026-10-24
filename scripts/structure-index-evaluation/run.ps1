param([string]$OutputDirectory = '.artifacts/structure-index-20261024')

$ErrorActionPreference = 'Stop'
$evaluationRoot = $PSScriptRoot
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $evaluationRoot '..\..'))
Push-Location -LiteralPath $repositoryRoot
$previousOracleOutput = $env:NOMO_STRUCTURE_EVALUATION_OUTPUT
try {
    $evaluationOutput = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputDirectory))
    New-Item -ItemType Directory -Path $evaluationOutput -Force | Out-Null
    & node (Join-Path $evaluationRoot 'prepare-corpus.mjs') $evaluationOutput
    if ($LASTEXITCODE -ne 0) { throw '语料准备失败。' }

    $env:NOMO_STRUCTURE_EVALUATION_OUTPUT = $evaluationOutput
    & pnpm exec vitest run --config scripts/structure-index-evaluation/vitest.config.ts scripts/structure-index-evaluation/legacy-oracle.test.ts
    if ($LASTEXITCODE -ne 0) { throw '旧实现对照采集失败。' }

    & cargo test --locked --manifest-path (Join-Path $evaluationRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw '共享原生适配器回归失败。' }
    @{ passed = $true; command = 'cargo test --locked --manifest-path scripts/structure-index-evaluation/Cargo.toml' } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evaluationOutput 'adapter-tests.json') -Encoding utf8NoBOM

    & cargo run --release --locked --manifest-path (Join-Path $evaluationRoot 'Cargo.toml') -- `
        (Join-Path $evaluationOutput 'commonmark-0.31.2.json') `
        (Join-Path $evaluationOutput 'gfm-0.29.json') `
        (Join-Path $evaluationOutput 'kernel-report.json')
    if ($LASTEXITCODE -ne 0) { throw '原生解析器实验执行失败。' }
    & node (Join-Path $evaluationRoot 'compare-html.mjs') (Join-Path $evaluationOutput 'kernel-report.json')
    if ($LASTEXITCODE -ne 0) { throw 'HTML 差异分类失败。' }
    & node (Join-Path $evaluationRoot 'compare-structure.mjs') $evaluationOutput
    if ($LASTEXITCODE -ne 0) { throw '结构事实对照失败。' }

    $environment = [ordered]@{
        baseline = (& git rev-parse HEAD)
        branch = (& git branch --show-current)
        rust = (& rustc --version)
        cargo = (& cargo --version)
        node = (& node --version)
        pnpm = (& pnpm --version)
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)
        generatedAt = [DateTimeOffset]::Now.ToString('o')
    }
    $environment | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evaluationOutput 'environment.json') -Encoding utf8NoBOM
    & node (Join-Path $evaluationRoot 'report.mjs') $evaluationOutput
    if ($LASTEXITCODE -ne 0) { throw '实验报告生成失败。' }
    Write-Output "实验执行完成；选型门槛见 $evaluationOutput\REPORT.md，执行成功不代表候选已通过。"
} finally {
    $env:NOMO_STRUCTURE_EVALUATION_OUTPUT = $previousOracleOutput
    Pop-Location
}

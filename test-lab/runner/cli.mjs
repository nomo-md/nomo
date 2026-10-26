import { mkdir, readdir, rm, realpath, lstat } from 'node:fs/promises';
import { join, resolve, basename } from 'node:path';
import { pathToFileURL } from 'node:url';
import { lab, repo, json, saveJson, exists, stamp, run, hashJson, hashFile, walk, within } from './common.mjs';
import { importDataset, loadDataset, prepareFixtures } from './dataset.mjs';
import { ensureBuild, buildApp } from './build.mjs';
import { catalogue, selectCases, executeCase, supportedExpectations } from './cases.mjs';
import { TestHost } from './host.mjs';
import { summarizeProcesses, summarizeSteps, performanceMetrics, absoluteWarnings, writeReport, aggregateCases, compareReports } from './report.mjs';
import { samplingQuality, baselineEligibility } from './statistics.mjs';

export function parseArgs(argv) {
  const values = {}, flags = new Set(['help', 'list', 'dry-run', 'no-build', 'debug', 'release', 'fail-fast']);
  const options = new Set(['config', 'suite', 'case', 'tag', 'dataset', 'repetitions', 'cycles', 'sample-ms', 'soak-minutes', 'baseline', 'profile', 'source', 'name', 'run']);
  let command = 'run';
  if (argv[0] && !argv[0].startsWith('-')) command = argv.shift();
  if (!['run', 'build', 'import', 'baseline', 'clean'].includes(command)) throw new Error(`未知命令：${command}`);
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i].replace(/^--/, '');
    if (!argv[i].startsWith('--')) throw new Error(`无效参数：${argv[i]}`);
    if (flags.has(key)) values[key] = true;
    else if (options.has(key)) { if (!argv[i + 1] || argv[i + 1].startsWith('--')) throw new Error(`缺少参数值：${key}`); values[key] = argv[++i]; }
    else throw new Error(`未知参数：${key}`);
  }
  if (values.debug && values.release) throw new Error('debug 与 release 不能同时指定');
  return { command, values };
}
export function validateConfig(config) {
  const keys = ['schemaVersion', 'dataset', 'suites', 'configuration', 'repetitions', 'cycles', 'sampleMs', 'startupTimeoutMs', 'actionTimeoutMs', 'stressTimeoutMs',
    'soakMinutes', 'failFast', 'profile', 'warnings', 'caseIds', 'tags', 'baseline', 'samplingQuality', 'budgets'];
  for (const key of Object.keys(config)) if (!keys.includes(key)) throw new Error(`未知配置字段：${key}`);
  if (config.schemaVersion !== 1) throw new Error('不支持的配置版本');
  for (const key of ['suites', 'caseIds', 'tags']) if (config[key] !== undefined && (!Array.isArray(config[key]) || !config[key].every(v => typeof v === 'string' && v.length))) throw new Error(`无效配置数组：${key}`);
  for (const [key, min, max] of [['repetitions', 1, 100], ['cycles', 1, 10000], ['sampleMs', 100, 10000], ['startupTimeoutMs', 1000, 300000],
    ['actionTimeoutMs', 1000, 300000], ['stressTimeoutMs', 1000, 600000], ['soakMinutes', .01, 1440]])
    if (!Number.isFinite(config[key]) || config[key] < min || config[key] > max || (key !== 'soakMinutes' && !Number.isInteger(config[key]))) throw new Error(`无效配置：${key}`);
  if (!['release', 'debug'].includes(config.configuration) || !['cpu', 'heap', 'both'].includes(config.profile)) throw new Error('无效构建/诊断配置');
  if (typeof config.failFast !== 'boolean') throw new Error('failFast 必须为布尔值');
  for (const key of ['relative', 'durationMs', 'privateBytes', 'cpuMachinePercent']) if (!Number.isFinite(config.warnings[key]) || config.warnings[key] < 0) throw new Error(`无效告警阈值：${key}`);
  for (const group of ['warnings', 'budgets', 'samplingQuality']) for (const [key, value] of Object.entries(config[group] ?? {}))
    if (!Number.isFinite(value) || value < 0) throw new Error(`无效配置：${group}.${key}`);
  if (!Number.isInteger(config.samplingQuality?.minimumSamples) || config.samplingQuality.minimumSamples < 3 || config.samplingQuality.minimumCoverage > 1 || config.samplingQuality.maxMeanSamplerFraction > 1) throw new Error('无效采样质量配置');
  return config;
}

export async function main(argv = process.argv.slice(2)) {
  const { command, values } = parseArgs([...argv]);
  if (values.help) {
    console.log(`Nomo test-lab（Windows）\n  pnpm test:desktop [run] --suite standard --no-build\n  pnpm test:desktop --list | --dry-run\n  pnpm test:desktop build [--debug]\n  pnpm test:desktop import --source <目录> --name <名称>\n  pnpm test:desktop baseline --run <运行ID> --name <基线名>\n  pnpm test:desktop clean --run <运行ID>\n筛选：--suite a,b --case ID --tag 标签 --dataset main --config 文件\n参数：--repetitions 3 --cycles 20 --soak-minutes 30 --sample-ms 500\n诊断：--suite diagnostics --case ID --profile cpu|heap|both\n比较：--baseline 名称；首次失败即停止：--fail-fast`); return; }
  const defaults = await json(join(lab, 'suites/default.json'));
  const supplied = values.config ? await json(resolve(values.config)) : {};
  const config = { ...defaults, ...supplied, warnings: { ...defaults.warnings, ...supplied.warnings },
    samplingQuality: { ...defaults.samplingQuality, ...supplied.samplingQuality }, budgets: { ...defaults.budgets, ...supplied.budgets } };
  // PowerShell 的 pnpm shim 可能把未引用的逗号列表合成空格列表。
  const split = value => value.split(/[,\s]+/).filter(Boolean);
  for (const [arg, key] of [['suite', 'suites'], ['case', 'caseIds'], ['tag', 'tags']]) if (values[arg]) config[key] = split(values[arg]);
  for (const [arg, key] of [['repetitions', 'repetitions'], ['cycles', 'cycles'], ['sample-ms', 'sampleMs'], ['soak-minutes', 'soakMinutes']]) if (values[arg]) config[key] = Number(values[arg]);
  for (const key of ['dataset', 'baseline', 'profile']) if (values[key]) config[key] = values[key];
  if (values.debug) config.configuration = 'debug'; if (values.release) config.configuration = 'release';
  if (values['fail-fast']) config.failFast = true;
  validateConfig(config);
  if (Number(process.versions.node.split('.')[0]) < 22) throw new Error('测试工具需要 Node.js 22 或更新版本（使用内置 WebSocket/CDP）。');
  if (process.platform !== 'win32' && !values.list && !values['dry-run']) throw new Error('首版仅支持 Windows 实机运行');
  if (command === 'build') { await buildApp(config.configuration); return; }
  if (command === 'import') {
    if (!values.source || !values.name) throw new Error('import 需要 --source 与 --name');
    const imported = await importDataset(resolve(values.source), values.name); console.log(`导入并校验：${imported.samples.length} 个样本，${imported.fingerprint}`); return;
  }
  if (command === 'baseline' || command === 'clean') {
    if (!values.run || !/^[\w.-]+$/.test(values.run)) throw new Error('需要有效的 --run ID');
    const directory = within(join(lab, 'results'), values.run), report = await json(join(directory, 'report.json'));
    if (report.id !== values.run || report.protocol !== 2) throw new Error('不是本工具的运行结果');
    if (command === 'baseline') {
      if (!values.name || !/^[\w-]+$/.test(values.name)) throw new Error('基线需要 --name，限字母数字及短横线');
      const rejected = baselineEligibility(report); if (rejected) throw new Error(rejected);
      const target = join(lab, 'baselines', `${values.name}.json`);
      if (await exists(target)) throw new Error('基线名已存在；使用新名字显式切换，不覆盖固定基线');
      await saveJson(target, { id: report.id, fingerprint: hashJson(report), report }); console.log(`已固定基线：${values.name}`); return;
    }
    if (await exists(join(lab, 'baselines'))) for (const file of await readdir(join(lab, 'baselines'))) {
      if ((await json(join(lab, 'baselines', file))).id === report.id) throw new Error('该运行被基线引用，拒绝清理');
    }
    if ((await lstat(directory)).isSymbolicLink()) throw new Error('拒绝清理符号链接');
    within(await realpath(join(lab, 'results')), await realpath(directory));
    await rm(directory, { recursive: true, force: false }); console.log(`已清理指定运行：${report.id}`); return;
  }
  const dataset = await loadDataset(config.dataset), all = catalogue(dataset);
  const selected = values.list && !values.suite && !values.case && !values.tag ? all : selectCases(all, config);
  if (values.list || values['dry-run']) {
    for (const c of selected) console.log(`${c.id}\t[${c.suites.join(',')}]\t${c.title}`);
    console.log(`共 ${selected.length} 个用例；${values['dry-run'] ? '未启动程序' : '可用 --case 选择'}`); return;
  }
  const baselineFile = config.baseline ? await json(within(join(lab, 'baselines'), `${config.baseline}.json`)) : null;
  if (baselineFile && hashJson(baselineFile.report) !== baselineFile.fingerprint) throw new Error('基线报告指纹不匹配，拒绝比较');
  const baseline = baselineFile?.report ?? null;
  const build = await ensureBuild(config.configuration, { noBuild: Boolean(values['no-build']) });
  const id = stamp(), directory = join(lab, 'results', id); await mkdir(directory, { recursive: true });
  console.log(`运行 ${id}：${selected.length} 个用例；结果目录 ${directory}`);
  const environment = JSON.parse(await run('pwsh', ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'environment']));
  environment.node = process.versions.node;
  const frameworkFiles = [...(await walk(join(lab, 'runner'))).filter(file => !file.endsWith('.test.mjs')),
    ...['src/app/services/automationBridge.ts', 'src/app/services/automationTelemetry.ts', 'src-tauri/src/automation.rs', 'src-tauri/tauri.automation.conf.json'].map(path => join(repo, path))];
  const runnerFingerprint = hashJson(await Promise.all(frameworkFiles.map(async file => [basename(file), await hashFile(file)])));
  const report = { protocol: 2, id, configuration: config.configuration, status: 'running', startedAt: new Date().toISOString(),
    config, build, environment, runnerFingerprint, dataset: dataset.name, datasetFingerprint: dataset.fingerprint, cases: [],
    manualChecks: ['中文 IME 组词、候选、取消与提交', '肉眼视觉闪烁与排版验收'],
    excluded: ['TXT/JSON', '导出', '安装更新', '完整设置回归'] };
  report.measurementFingerprint = hashJson({ runnerFingerprint, profileIsolation: 'per-case-data-cache-webview-v1', configuration: config.configuration, repetitions: config.repetitions, cycles: config.cycles, sampleMs: config.sampleMs, soakMinutes: config.soakMinutes, profile: config.suites.includes('diagnostics') ? config.profile : null });
  const coverage = { dataset: dataset.name, samples: dataset.samples.length, directoryTreeFiles: dataset.tree.length,
    cases: all.map(c => ({ id: c.id, title: c.title, suites: c.suites, samples: c.samples.map(s => s.path), purpose: c.purpose, expected: c.expected,
      implementedExpectationKeys: Object.keys(c.expected ?? {}).filter(key => supportedExpectations.includes(key)),
      assertedExpectationKeys: [], unassertedExpectationKeys: Object.keys(c.expected ?? {}),
      selected: selected.some(s => s.id === c.id), status: 'not-run' })), manualChecks: report.manualChecks, excluded: report.excluded };
  await saveJson(join(directory, 'coverage.json'), coverage);
  let currentHost, interrupted = false, cleanupFailed = false;
  const interrupt = () => { interrupted = true; console.error('正在停止本次测试进程并保存结果…'); void currentHost?.stop().catch(() => { cleanupFailed = true; }); };
  process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
  try {
    outer: for (const test of selected) {
      const repetitions = test.kind === 'performance' || test.kind === 'lifecycle.tabs' || test.kind.startsWith('interaction.') ? config.repetitions : 1;
      for (let round = 1; round <= repetitions; round++) {
        if (interrupted) break outer;
        const key = `${test.id}-${round}`, caseDir = join(directory, key);
        const record = { id: test.id, key, title: test.title, round, kind: test.kind, directory: caseDir,
          startedAt: new Date().toISOString(), status: 'running', assertions: [], steps: [] };
        report.cases.push(record); console.log(`开始 ${test.id} (${round}/${repetitions}) ${test.title}`);
        await writeReport(directory, report);
        const start = performance.now();
        try {
          const fixtures = await prepareFixtures(dataset, caseDir, test.samples, { tree: test.kind === 'structure.directory' });
          const caseConfig = { ...config, actionTimeoutMs: test.suites.includes('stress') ? config.stressTimeoutMs : config.actionTimeoutMs };
          const host = currentHost = new TestHost(build, caseDir, caseConfig, record);
          await host.start(join(fixtures, test.samples[0].path));
          const profiling = config.suites.includes('diagnostics');
          if (profiling && !host.cdp) throw new Error('深度诊断要求 CDP，连接不可用');
          if (profiling && ['heap', 'both'].includes(config.profile)) await host.cdp.heap(join(caseDir, 'before.heapsnapshot'));
          if (profiling && ['cpu', 'both'].includes(config.profile)) await host.cdp.cpuStart();
          const workloadStarted = performance.now();
          try { await executeCase(test, host, fixtures, config, record); }
          finally {
            record.workloadDurationMs = performance.now() - workloadStarted;
            if (profiling && host.cdp && ['cpu', 'both'].includes(config.profile)) await host.cdp.cpuStop(join(caseDir, 'profile.cpuprofile')).catch(e => { record.profileError = String(e); });
          }
          if (profiling && ['heap', 'both'].includes(config.profile)) await host.cdp.heap(join(caseDir, 'after.heapsnapshot'));
          record.finalState = await host.capture();
          if (record.status === 'running') record.status = record.profileError ? 'failed' : 'passed';
        } catch (error) {
          record.status = interrupted ? 'interrupted' : error.code === 'TIMEOUT' || /超时/.test(String(error)) ? 'timeout' : 'failed';
          record.error = String(error.stack ?? error);
          if (currentHost) {
            // 应用已回复的阶段超时可以继续取诊断；传输超时仍可能有迟到响应，不能插入下一条 RPC。
            if (error.code !== 'TIMEOUT') {
              try { record.finalState = await currentHost.capture(2000); } catch { /* 原失败保留，不能用诊断失败覆盖。 */ }
            }
            try { record.screenshot = await currentHost.screenshot(); } catch (e) { record.screenshotUnavailable = String(e); }
          }
        } finally {
          await currentHost?.stop().catch(e => { record.cleanupError = String(e); record.status = 'failed'; cleanupFailed = true; }); currentHost = null;
          record.durationMs = performance.now() - start; record.finishedAt = new Date().toISOString();
          record.processMetrics = await summarizeProcesses(caseDir, record.measurements).catch(e => ({ unavailable: [String(e)] }));
          const observedDuration = Date.parse(record.measurementEndedAt) - Date.parse(record.measurementStartedAt);
          const expected = 1 + Math.max(record.processMetrics.spanMs ?? 0, Number.isFinite(observedDuration) ? observedDuration : 0) / config.sampleMs;
          record.processMetrics.coverage = Math.min(1, (record.processMetrics.samples ?? 0) / expected);
          record.dataQuality = samplingQuality(record.processMetrics, config.sampleMs, config.samplingQuality);
          if (record.samplerWarning) { record.dataQuality.status = 'invalid'; record.dataQuality.reasons.push(record.samplerWarning); }
          record.operationMetrics = await summarizeSteps(caseDir).catch(e => ({ unavailable: String(e) }));
          record.performanceMetrics = performanceMetrics(record);
          record.metricAvailability = { gpuMemory: Object.keys(record.processMetrics.gpu ?? {}).length ? 'available' : 'unavailable',
            privateWorkingSet: Number.isFinite(record.processMetrics.peakWorkingSetPrivate) ? 'available' : 'unavailable',
            inputLatency: Number.isFinite(record.performanceMetrics.inputP95Ms) ? 'available' : 'unavailable-or-insufficient-samples' };
          for (const metric of record.requiredMetrics ?? []) if (!Number.isFinite(record.performanceMetrics[metric])) {
            record.dataQuality.status = 'invalid'; record.dataQuality.reasons.push(`必需指标缺失：${metric}`);
          }
          if (record.requiredMetrics?.includes('inputP95Ms') && ((record.finalState?.telemetry?.unmatchedInputs ?? 0) || (record.finalState?.telemetry?.pendingInputs ?? 0))) {
            record.dataQuality.status = 'invalid'; record.dataQuality.reasons.push('存在未匹配呈现的输入事件');
          }
          record.performanceWarnings = absoluteWarnings(record, config.budgets);
          await saveJson(join(caseDir, 'case.json'), record);
          console.log(`${record.status.toUpperCase()} ${test.id}，${record.assertions.length} 个断言，${(record.durationMs / 1000).toFixed(1)} 秒${record.error ? '：' + record.error.split('\n')[0] : ''}`);
          report.finishedAt = new Date().toISOString(); await writeReport(directory, report);
        }
        if (cleanupFailed || (config.failFast && record.status !== 'passed')) break outer;
      }
    }
  } finally {
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
    await currentHost?.stop();
    report.finishedAt = new Date().toISOString();
    report.status = interrupted ? 'interrupted' : report.cases.every(c => c.status === 'passed') && selected.every(c => report.cases.some(r => r.id === c.id)) ? 'passed' : 'failed';
    report.webviewVersions = [...new Set(report.cases.flatMap(c => [c.webview?.fileVersion, ...(c.processMetrics?.webviewVersions ?? [])]).filter(Boolean))].sort();
    report.environmentFingerprint = hashJson({ hardware: environment, webview: report.cases.find(c => c.environment)?.environment, webviewVersions: report.webviewVersions });
    report.aggregate = aggregateCases(report.cases);
    report.comparison = compareReports(report, baseline, config.warnings);
    report.dataQuality = report.cases.every(c => c.dataQuality?.status === 'valid') ? 'valid' : 'invalid';
    report.performanceWarnings = [...report.cases.flatMap(c => c.performanceWarnings ?? []), ...report.comparison.warnings];
    report.performanceStatus = report.dataQuality !== 'valid' ? 'unavailable' : report.performanceWarnings.length ? 'warning' : baseline ? report.comparison.status : 'no-baseline';
    if (cleanupFailed) report.abortReason = '进程清理未确认，停止后续用例以免污染结果';
    for (const c of coverage.cases) {
      const cases = report.cases.filter(r => r.id === c.id);
      c.expectationChecks = cases.flatMap(r => r.expectationChecks ?? []);
      c.assertedExpectationKeys = [...new Set(c.expectationChecks.filter(check => check.status === 'passed').map(check => check.key))];
      c.unassertedExpectationKeys = Object.keys(c.expected ?? {}).filter(key => !c.assertedExpectationKeys.includes(key));
      if (cases.length) c.status = cases.every(r => r.status === 'passed') ? 'passed' : cases.find(r => r.status !== 'passed').status;
    }
    await saveJson(join(directory, 'coverage.json'), coverage); await writeReport(directory, report);
    console.log(`功能：${report.status}；数据质量：${report.dataQuality}；性能：${report.performanceStatus}；HTML：${join(directory, 'report.html')}`);
    if (report.status !== 'passed') process.exitCode = interrupted ? 130 : 1;
    else if (report.dataQuality !== 'valid') process.exitCode = 2;
  }
  return report;
}
if (import.meta.url === pathToFileURL(resolve(process.argv[1] ?? '')).href) main().catch(error => { console.error(error.stack ?? error); process.exitCode = 1; });

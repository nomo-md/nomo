import { createReadStream, createWriteStream } from 'node:fs';
import { readdir, writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { join, relative } from 'node:path';
import { quantiles, saveJson } from './common.mjs';
import { Distribution } from './statistics.mjs';

export async function summarizeProcesses(directory, measurements = []) {
  const files = (await readdir(directory)).filter(f => /^process-\d+\.jsonl$/.test(f)).sort((a, b) => Number(a.match(/\d+/)[0]) - Number(b.match(/\d+/)[0]));
  const series = [], privateBytes = new Distribution(), cpu = new Distribution(), roles = {}, gpu = {}; const unavailable = new Set();
  const webviewVersions = new Set();
  const warmAt = Date.parse(measurements.find(m => m.phase === 'initial')?.at), idleAt = Date.parse(measurements.find(m => m.phase === 'idle')?.at);
  let warmPrivate = null, idlePrivate = null;
  let samples = 0, cost = 0, peak = 0, peakWorkingSetPrivate = null, firstPrivate = null, lastPrivate = null;
  let firstAt, lastAt, maxGapMs = 0, sumX = 0, sumY = 0, sumXX = 0, sumXY = 0, slowCost = 0, slowCount = 0;
  const csv = createWriteStream(join(directory, 'process.csv'));
  csv.write('timestamp,pid,role,cpuMs,cpuCorePercent,cpuMachinePercent,privateBytes,workingSetPrivate,workingSet,ioReadBytes,ioWriteBytes\n');
  for (const file of files) for await (const line of createInterface({ input: createReadStream(join(directory, file)), crlfDelay: Infinity })) {
    if (!line.trim()) continue;
    let row; try { row = JSON.parse(line.replace(/^\uFEFF/, '')); } catch { unavailable.add(`truncated sample in ${file}`); continue; }
    const alive = row.processes.filter(p => !p.exited);
    if (!alive.length) continue;
    if (!alive.every(p => Number.isFinite(p.privateBytes))) { unavailable.add('私有提交采样不完整'); continue; }
    const total = alive.reduce((sum, p) => sum + (p.privateBytes ?? 0), 0);
    const totalCpu = alive.every(p => Number.isFinite(p.cpuMachinePercent)) ? alive.reduce((sum, p) => sum + p.cpuMachinePercent, 0) : null;
    firstPrivate ??= total; lastPrivate = total; peak = Math.max(peak, total);
    const at = Date.parse(row.timestamp);
    firstAt ??= at;
    if (lastAt !== undefined) maxGapMs = Math.max(maxGapMs, at - lastAt);
    lastAt = at;
    const x = (at - firstAt) / 60000; sumX += x; sumY += total; sumXX += x * x; sumXY += x * total;
    if (Number.isFinite(warmAt) && at >= warmAt) warmPrivate ??= total;
    if (Number.isFinite(idleAt) && at <= idleAt) idlePrivate = total;
    if (alive.every(p => Number.isFinite(p.workingSetPrivate))) peakWorkingSetPrivate = Math.max(peakWorkingSetPrivate ?? 0, alive.reduce((a, p) => a + p.workingSetPrivate, 0));
    samples++; cost += row.samplerCostMs ?? 0;
    privateBytes.add(total); cpu.add(totalCpu);
    if (Number.isFinite(row.slowSamplerCostMs)) { slowCost += row.slowSamplerCostMs; slowCount++; }
    series.push({ timestamp: row.timestamp, privateBytes: total, cpuMachinePercent: totalCpu });
    if (series.length > 600) { const keep = series.filter((_, i) => i % 2 === 0); series.splice(0, series.length, ...keep); }
    if (row.gpuUnavailable) unavailable.add(`GPU: ${row.gpuUnavailable}`);
    if (row.countersUnavailable) unavailable.add(`Counters: ${row.countersUnavailable}`);
    // Older PowerShell samples unwrapped a single GPU instance into an object.
    const gpuInstances = Array.isArray(row.gpuMemory) ? row.gpuMemory : row.gpuMemory ? [row.gpuMemory] : [];
    for (const instance of gpuInstances) {
      const record = gpu[instance.Name] ??= { dedicatedPeak: null, sharedPeak: null, committedPeak: null };
      for (const [field, key] of [['DedicatedUsage', 'dedicatedPeak'], ['SharedUsage', 'sharedPeak'], ['TotalCommitted', 'committedPeak']])
        if (Number.isFinite(instance[field])) record[key] = Math.max(record[key] ?? 0, instance[field]);
    }
    for (const p of alive) {
      if (p.role === 'browser' && p.fileVersion) webviewVersions.add(p.fileVersion);
      const role = roles[p.role] ??= { privatePeak: 0, workingSetPrivatePeak: null, lastCpuMs: 0 };
      role.privatePeak = Math.max(role.privatePeak, p.privateBytes ?? 0);
      if (Number.isFinite(p.workingSetPrivate)) role.workingSetPrivatePeak = Math.max(role.workingSetPrivatePeak ?? 0, p.workingSetPrivate);
      role.lastCpuMs = Math.max(role.lastCpuMs, p.cpuMs ?? 0);
      csv.write([row.timestamp, p.pid, p.role, p.cpuMs, p.cpuCorePercent, p.cpuMachinePercent, p.privateBytes, p.workingSetPrivate, p.workingSet, p.ioReadBytes, p.ioWriteBytes].join(',') + '\n');
    }
  }
  await new Promise(resolve => csv.end(resolve));
  if (!samples) unavailable.add('未获得进程采样；不可当作零占用');
  const spanMs = lastAt - firstAt, slopeDenominator = samples * sumXX - sumX * sumX;
  return { samples, spanMs: Number.isFinite(spanMs) ? spanMs : null, maxGapMs,
    meanSamplerCostMs: samples ? cost / samples : null, meanSlowSamplerCostMs: slowCount ? slowCost / slowCount : null, privateBytes: privateBytes.summary(),
    cpuMachinePercent: cpu.summary(), privatePeak: samples ? peak : null, peakWorkingSetPrivate,
    memorySlopeBytesPerMinute: spanMs >= 30000 && slopeDenominator > 0 ? (samples * sumXY - sumX * sumY) / slopeDenominator : null,
    firstPrivate, lastPrivate, startupToEndDelta: samples ? lastPrivate - firstPrivate : null,
    warmPrivate, idlePrivate, retainedDelta: warmPrivate !== null && idlePrivate !== null ? idlePrivate - warmPrivate : null, roles, gpu, series, webviewVersions: [...webviewVersions].sort(),
    quantileMethod: '全时段有界直方图，约 1% 相对误差；曲线降采样不影响统计', unavailable: [...unavailable], notice: '离散采样峰值；驻留、私有提交、JS 堆、GPU 不相加；角色峰值不是可相加的同时峰值。' };
}

export async function summarizeSteps(directory) {
  const methods = {}, phases = {}; let samples = 0;
  for await (const line of createInterface({ input: createReadStream(join(directory, 'steps.jsonl')), crlfDelay: Infinity })) {
    if (!line.trim()) continue;
    const step = JSON.parse(line); samples++;
    if (step.status !== 'passed') continue;
    (methods[step.method] ??= new Distribution()).add(step.appDurationMs);
    const key = [step.method, step.stage, step.mode, step.command].filter(Boolean).join(':');
    (phases[key] ??= new Distribution()).add(step.roundTripMs);
  }
  return { samples, methods: Object.fromEntries(Object.entries(methods).map(([key, d]) => [key, d.summary()])),
    phases: Object.fromEntries(Object.entries(phases).map(([key, d]) => [key, d.summary()])) };
}

export function performanceMetrics(record) {
  const telemetry = record.finalState?.telemetry, minutes = record.processMetrics?.spanMs / 60000;
  return { retainedBytes: record.processMetrics?.retainedDelta ?? null, memorySlopeBytesPerMinute: record.processMetrics?.memorySlopeBytesPerMinute ?? null,
    inputP95Ms: telemetry?.inputToFrameEstimate?.p95Ms ?? null, frameP95Ms: telemetry?.frameInterval?.p95Ms ?? null,
    longTaskRate: Number.isFinite(telemetry?.longTasks) && minutes > 0 ? telemetry.longTasks / minutes : null,
    longTaskMsPerMinute: Number.isFinite(telemetry?.longTaskMs) && minutes > 0 ? telemetry.longTaskMs / minutes : null };
}

export function absoluteWarnings(record, budgets) {
  return Object.entries(budgets).flatMap(([metric, limit]) => Number.isFinite(record.performanceMetrics?.[metric]) && record.performanceMetrics[metric] > limit
    ? [{ id: record.id, metric, observed: record.performanceMetrics[metric], limit, kind: 'absolute-budget', diagnosticOnly: true }] : []);
}

export function aggregateCases(cases) {
  const grouped = new Map();
  for (const c of cases) { if (!grouped.has(c.id)) grouped.set(c.id, []); grouped.get(c.id).push(c); }
  return [...grouped].map(([id, rows]) => {
    const passed = rows.filter(r => r.status === 'passed');
    const methods = {};
    for (const r of passed) for (const [method, summary] of Object.entries(r.operationMetrics?.methods ?? {})) (methods[method] ??= new Distribution()).merge(summary);
    return { id, title: rows[0].title, runs: rows.length, passed: passed.length, status: passed.length === rows.length ? 'passed' : rows.find(r => r.status !== 'passed').status,
      operationSamplesRetained: passed.reduce((n, r) => n + r.steps.length, 0), operationSamplesEvicted: passed.reduce((n, r) => n + (r.evictedSteps ?? 0), 0),
      startupMs: quantiles(passed.map(r => r.startupMs)), durationMs: quantiles(passed.map(r => r.durationMs)), workloadDurationMs: quantiles(passed.map(r => r.workloadDurationMs)),
      privateBytes: quantiles(passed.map(r => r.processMetrics?.privatePeak)),
      cpuMachinePercent: quantiles(passed.map(r => r.processMetrics?.cpuMachinePercent?.median)),
      dataQuality: passed.every(r => r.dataQuality?.status === 'valid') ? 'valid' : 'invalid',
      performanceMetrics: Object.fromEntries(['retainedBytes', 'memorySlopeBytesPerMinute', 'inputP95Ms', 'frameP95Ms', 'longTaskRate', 'longTaskMsPerMinute']
        .map(key => [key, quantiles(passed.map(r => r.performanceMetrics?.[key]))])),
      methods: Object.fromEntries(Object.entries(methods).map(([key, d]) => [key, d.summary()])) };
  });
}

export function compareReports(current, baseline, limits) {
  if (!baseline) return { status: 'no-baseline', warnings: [], reason: '尚未明确指定性能基线' };
  if (baseline.configuration !== 'release' || current.configuration !== 'release') return { status: 'incomparable', reason: '性能基线只比较 Release', warnings: [] };
  for (const key of ['datasetFingerprint', 'environmentFingerprint', 'measurementFingerprint'])
    if (baseline[key] !== current[key]) return { status: 'incomparable', reason: `${key} 不一致`, warnings: [] };
  const warnings = [], comparisons = [], missing = [];
  for (const row of current.aggregate) {
    const before = baseline.aggregate.find(b => b.id === row.id && b.status === 'passed');
    if (!before || row.status !== 'passed') continue;
    if (row.dataQuality !== 'valid' || before.dataQuality !== 'valid') { missing.push({ id: row.id, reason: '采样质量未通过' }); continue; }
    for (const metric of ['startupMs', 'workloadDurationMs', 'privateBytes', 'cpuMachinePercent', 'retainedBytes', 'memorySlopeBytesPerMinute', 'inputP95Ms', 'frameP95Ms', 'longTaskRate', 'longTaskMsPerMinute']) {
      const a = (before[metric] ?? before.performanceMetrics?.[metric])?.median, b = (row[metric] ?? row.performanceMetrics?.[metric])?.median;
      if (!Number.isFinite(a) || !Number.isFinite(b)) { missing.push({ id: row.id, metric, reason: '指标缺失或样本不足' }); continue; }
      const delta = b - a, ratio = a > 0 ? delta / a : (delta > 0 ? Infinity : 0);
      const floor = limits[metric] ?? limits.durationMs;
      const entry = { id: row.id, metric, before: a, after: b, delta, relative: Number.isFinite(ratio) ? ratio : null, warning: delta >= floor && ratio >= limits.relative };
      comparisons.push(entry); if (entry.warning) warnings.push(entry);
    }
  }
  return { status: comparisons.length ? (missing.length ? 'partial' : 'compared') : (missing.length ? 'unavailable' : 'no-matching-cases'), baseline: baseline.id, comparisons, missing, warnings };
}
const escape = value => String(value ?? '').replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const link = (root, path) => relative(root, path).replaceAll('\\', '/').split('/').map(encodeURIComponent).join('/');
function graph(points, key, color) {
  const values = points.filter(p => Number.isFinite(p[key])); if (values.length < 2) return '<span>采样不足</span>';
  const times = values.map(p => Date.parse(p.timestamp)), origin = times[0], span = Math.max(1, times.at(-1) - origin);
  const maximum = Math.max(...values.map(p => p[key]), 1);
  return `<svg viewBox="0 0 600 90" role="img" aria-label="${escape(key)} 时间曲线"><polyline fill="none" stroke="${color}" stroke-width="1.7" points="${values.map((p, i) => `${((times[i] - origin) / span * 596 + 2).toFixed(1)},${(88 - p[key] / maximum * 84).toFixed(1)}`).join(' ')}"/></svg>`;
}
export async function writeReport(directory, report) {
  report.aggregate = aggregateCases(report.cases);
  await saveJson(join(directory, 'report.json'), report);
  await writeFile(join(directory, 'summary.csv'), 'id,status,runs,passed,totalDurationMedianMs,workloadDurationMedianMs,dataQuality,privatePeakMedianBytes,cpuMachineMedianPercent\n' + report.aggregate.map(c => [c.id, c.status, c.runs, c.passed, c.durationMs.median, c.workloadDurationMs.median, c.dataQuality, c.privateBytes.median, c.cpuMachinePercent.median].join(',')).join('\n'));
  const rows = report.cases.map(c => `<tr><td><a href="#${escape(c.key)}">${escape(c.id)}</a><br><small>${escape(c.title)}</small></td><td class="${c.status}">${escape(c.status)}</td><td>${c.round}</td><td>${((c.durationMs ?? 0) / 1000).toFixed(2)} s</td><td>${c.assertions.length}</td><td>${escape(c.error ?? c.reason ?? '')}</td></tr>`).join('');
  const sections = report.cases.map(c => `<details id="${escape(c.key)}"><summary>${escape(c.title)} · ${escape(c.status)} · 第 ${c.round} 轮</summary>
    <p><a href="${link(directory, join(c.directory, 'case.json'))}">用例 JSON</a> · <a href="${link(directory, join(c.directory, 'process.csv'))}">进程 CSV</a>${c.screenshot ? ` · <a href="${link(directory, join(c.directory, c.screenshot))}">失败截图</a>` : ''}</p>
    <p>私有提交峰值：${c.processMetrics?.privatePeak == null ? '不可用' : (c.processMetrics.privatePeak / 1048576).toFixed(1) + ' MiB'}；私有工作集峰值：${c.processMetrics?.peakWorkingSetPrivate == null ? '不可用' : (c.processMetrics.peakWorkingSetPrivate / 1048576).toFixed(1) + ' MiB'}</p>
    <p>私有提交曲线（独立口径）</p>${graph(c.processMetrics?.series ?? [], 'privateBytes', '#245bdb')}
    <p>整机归一化 CPU 曲线</p>${graph(c.processMetrics?.series ?? [], 'cpuMachinePercent', '#bd5723')}
    <pre>${escape(JSON.stringify({ assertions: c.assertions, expectationChecks: c.expectationChecks, observations: c.observations, readingPosition: c.readingPosition, error: c.error, unavailable: c.processMetrics?.unavailable,
      dataQuality: c.dataQuality, metricAvailability: c.metricAvailability, workloadDurationMs: c.workloadDurationMs, performanceMetrics: c.performanceMetrics, performanceWarnings: c.performanceWarnings,
      samplerMeanCostMs: c.processMetrics?.meanSamplerCostMs, slowSamplerMeanCostMs: c.processMetrics?.meanSlowSamplerCostMs,
      gpu: c.processMetrics?.gpu, warmToIdlePrivateDelta: c.processMetrics?.retainedDelta, finalTelemetry: c.finalState?.telemetry,
      stages: c.finalState?.document?.timings, views: c.finalState?.views, methods: report.aggregate.find(a => a.id === c.id)?.methods }, null, 2))}</pre></details>`).join('');
  const html = `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Nomo 测试报告 ${escape(report.id)}</title>
    <style>body{font:15px/1.6 system-ui,sans-serif;margin:32px auto;padding:0 24px;max-width:1280px;color:#18243a;background:#f7f8fb}h1{font-size:28px}table{border-collapse:collapse;width:100%;background:white}td,th{text-align:left;padding:9px;border-bottom:1px solid #dde3eb;vertical-align:top}small{color:#55647b}a{color:#245bdb}.passed{color:#18734c}.failed,.timeout{color:#b52232}details{background:white;padding:16px;margin:12px 0;border:1px solid #dde3eb}summary{cursor:pointer;font-weight:600}pre{overflow:auto;font-size:12px}svg{display:block;max-width:700px;background:#f4f6fb}nav a{margin-right:16px}</style>
    <h1>Nomo 编辑与性能测试</h1><p>${escape(report.id)} · ${escape(report.configuration)} · 功能：<b>${escape(report.status)}</b> · 数据质量：${escape(report.dataQuality ?? '待汇总')} · 性能：${escape(report.performanceStatus ?? '待汇总')}</p>
    <nav><a href="report.json">完整 JSON</a><a href="summary.csv">汇总 CSV</a><a href="coverage.json">覆盖清单</a></nav>
    <p>${report.cases.length} 次用例运行；${report.cases.filter(c => c.status === 'passed').length} 通过。${escape(report.startedAt)} — ${escape(report.finishedAt ?? '运行中')}</p>
    <details><summary>环境、构建与基线</summary><pre>${escape(JSON.stringify({ build: report.build, environment: report.environment, comparison: report.comparison, config: report.config }, null, 2))}</pre></details>
    <p>状态代表功能断言；性能单独告警或标记不可用。IME 组词和肉眼视觉验收：人工待验。rAF 为帧间隔，输入时间为应用内呈现估计；采样峰值可能漏掉瞬时峰值。不同内存口径不相加。量化分位数显示样本数，三轮不报告 P95/P99。</p>
    <table><thead><tr><th>用例</th><th>状态</th><th>轮次</th><th>耗时</th><th>断言</th><th>原因</th></tr></thead><tbody>${rows}</tbody></table>${sections}</html>`;
  await writeFile(join(directory, 'report.html'), html);
}

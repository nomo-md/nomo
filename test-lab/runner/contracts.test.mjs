import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, validateConfig } from './cli.mjs';
import { selectCases } from './cases.mjs';
import { compareReports, aggregateCases, summarizeProcesses } from './report.mjs';
import { quantiles, json, lab, within, hashOutsideToc, hashJson, run } from './common.mjs';
import { join } from 'node:path';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { prepareFixtures } from './dataset.mjs';
import { Distribution, samplingQuality, baselineEligibility } from './statistics.mjs';
import { assertMarkdown, inlineResults } from './oracles.mjs';
import { stopOwnedProcess } from './process-lifecycle.mjs';
import { summarizeSteps, performanceMetrics, absoluteWarnings } from './report.mjs';

test('选择器拒绝未知测试集、用例和零匹配', () => {
  const cases = [{ id: 'edit', suites: ['editing'], tags: ['small'] }, { id: 'large', suites: ['stress'], tags: ['large'] }];
  assert.throws(() => selectCases(cases, { suites: ['typo'] }), /未知测试集/);
  assert.throws(() => selectCases(cases, { suites: ['editing'], caseIds: ['missing'] }), /未知用例/);
  assert.throws(() => selectCases(cases, { suites: ['editing'], tags: ['large'] }), /没有用例/);
  assert.deepEqual(selectCases(cases, { suites: ['editing', 'stress'], tags: ['large'] }).map(c => c.id), ['large']);
});
test('深度诊断必须指定用例', () => assert.throws(() => selectCases([], { suites: ['diagnostics'] }), /必须指定/));
test('非法参数及配置在启动程序前失败', async () => {
  assert.throws(() => parseArgs(['--typo']), /未知参数/);
  assert.throws(() => parseArgs(['--case']), /缺少参数值/);
  assert.throws(() => parseArgs(['--debug', '--release']), /不能同时/);
  const config = await json(join(lab, 'suites/default.json'));
  assert.throws(() => validateConfig({ ...config, cycles: -1 }), /cycles/);
  assert.throws(() => validateConfig({ ...config, failFast: 'false' }), /布尔值/);
  assert.throws(() => validateConfig({ ...config, typo: 1 }), /未知配置字段/);
});
test('路径边界拒绝逃逸和绝对路径跨目录', () => {
  assert.throws(() => within(join(lab, 'results'), '../apps/file'), /超出目录/);
  assert.throws(() => within(join(lab, 'results'), join(lab, 'apps')), /超出目录/);
});
test('三次测量不输出伪 P95/P99；缺失不当作零', () => {
  assert.deepEqual(quantiles([10, 30, 20]), { n: 3, min: 10, median: 20, max: 30, p95: null, p99: null });
  assert.equal(quantiles([null, undefined, NaN]).median, null);
});
const row = (duration, memory) => ({ id: 'case', status: 'passed', dataQuality: 'valid', durationMs: { median: duration }, workloadDurationMs: { median: duration }, startupMs: { median: 1 }, privateBytes: { median: memory }, cpuMachinePercent: { median: 1 } });
const report = rows => ({ configuration: 'release', datasetFingerprint: 'd', environmentFingerprint: 'e', measurementFingerprint: 'm', aggregate: rows });
const limits = { relative: .2, durationMs: 10, privateBytes: 20 * 1048576, cpuMachinePercent: 5 };
test('不同环境不能被宣称性能可比', () => {
  assert.equal(compareReports({ ...report([]), environmentFingerprint: 'different' }, report([]), limits).status, 'incomparable');
  assert.equal(compareReports({ ...report([]), configuration: 'debug' }, report([]), limits).status, 'incomparable');
});

test('准确正文断言拒绝错误替换，不能仅凭摘要变化通过', () => {
  const expected = `# Editing\n\n${inlineResults.toggleBold} beta\n\n`;
  assertMarkdown(expected, expected, 'bold');
  for (const wrong of ['# Editing\n\nWRONG beta\n\n', '# Editing\n\n**Alpha** LOST\n\n', '# Editing\n\n*Alpha* beta\n\n'])
    assert.throws(() => assertMarkdown(wrong, expected, 'bold'));
});

test('零采样、高开销、采样空洞不能进入基线', async () => {
  const config = await json(join(lab, 'suites/default.json'));
  const good = { samples: 30, meanSamplerCostMs: 10, maxGapMs: 510, coverage: .98, privateBytes: { n: 30 }, cpuMachinePercent: { n: 29 } };
  assert.equal(samplingQuality(good, 500, config.samplingQuality).status, 'valid');
  for (const bad of [{ samples: 0 }, { meanSamplerCostMs: 223 }, { maxGapMs: 10000 }, { coverage: .1 }, { cpuMachinePercent: { n: 0 } }]) {
    const quality = samplingQuality({ ...good, ...bad }, 500, config.samplingQuality);
    assert.equal(quality.status, 'invalid');
    assert.match(baselineEligibility({ status: 'passed', configuration: 'release', config, cases: [{ dataQuality: quality }] }), /采样质量/);
  }
  assert.match(baselineEligibility({ status: 'passed', configuration: 'release', config, cases: [{ dataQuality: { status: 'valid' }, requiredMetrics: ['inputP95Ms'], performanceMetrics: {} }] }), /必需/);
});

test('输入延迟、内存保留量与增长速度退化会告警，缺失不会当作通过', async () => {
  const config = await json(join(lab, 'suites/default.json'));
  const before = row(100, 100), after = row(100, 100);
  before.performanceMetrics = { inputP95Ms: { median: 10 }, retainedBytes: { median: 0 }, memorySlopeBytesPerMinute: { median: 0 } };
  after.performanceMetrics = { inputP95Ms: { median: 300 }, retainedBytes: { median: 300 * 1048576 }, memorySlopeBytesPerMinute: { median: 10 * 1048576 } };
  const compared = compareReports(report([after]), report([before]), config.warnings);
  assert.deepEqual(compared.warnings.map(w => w.metric).sort(), ['inputP95Ms', 'memorySlopeBytesPerMinute', 'retainedBytes'].sort());
  assert.equal(compared.status, 'partial'); assert.ok(compared.missing.some(m => m.metric === 'frameP95Ms'));
  assert.equal(absoluteWarnings({ id: 'x', performanceMetrics: { retainedBytes: 200 * 1048576 } }, config.budgets).length, 1);
});

test('清理必须等待真正退出：kill 报错或超时不能假成功', async () => {
  const never = new Promise(() => {});
  await assert.rejects(() => stopOwnedProcess({ pid: 1, finished: never }, { terminate: () => { throw new Error('denied'); }, timeoutMs: 5 }), /无法确认.*denied/);
  await assert.rejects(() => stopOwnedProcess({ pid: 2, finished: never }, { terminate: () => {}, timeoutMs: 5 }), /无法确认/);
  let complete; const finished = new Promise(resolve => { complete = resolve; });
  const result = await stopOwnedProcess({ pid: 3, finished }, { terminate: () => { complete({ code: 1 }); }, timeoutMs: 50 });
  assert.equal(result.code, 1);
});

test('Windows 存活验证识别真实进程及 JSON 日期转换，拒绝旧创建时间', { skip: process.platform !== 'win32' }, async () => {
  const script = join(lab, 'runner/native.ps1').replaceAll("'", "''");
  const command = `$p=Get-CimInstance Win32_Process -Filter "ProcessId = $PID"; $entry=@{pid=$PID;created=$p.CreationDate.ToUniversalTime().ToString('o')};
    $alive=& '${script}' -Action check-processes -Text (ConvertTo-Json -InputObject @($entry) -Compress);
    $entry.created=$p.CreationDate.ToUniversalTime().AddMinutes(-1).ToString('o');
    $stale=& '${script}' -Action check-processes -Text (ConvertTo-Json -InputObject @($entry) -Compress);
    @{alive=($alive|ConvertFrom-Json).survivors.Count;stale=($stale|ConvertFrom-Json).survivors.Count}|ConvertTo-Json -Compress`;
  const result = JSON.parse(await run('pwsh', ['-NoProfile', '-Command', command]));
  assert.equal(result.alive, 1); assert.equal(result.stale, 0);
});

test('超过旧上限的后期退化仍进入全时段分位统计', () => {
  const d = new Distribution(); for (let i = 0; i < 5000; i++) d.add(10); for (let i = 0; i < 5000; i++) d.add(1000);
  const summary = d.summary(); assert.equal(summary.n, 10000); assert.ok(summary.p95 >= 990);
  const merged = new Distribution(); merged.merge(summary); merged.merge(summary);
  assert.equal(merged.summary().n, 20000); assert.ok(merged.summary().p95 >= 990);
});

test('UTF-16、images、中文资源目录均完整复制，操作统计读取全部磁盘明细', async () => {
  const directory = await mkdtemp(join(lab, '.cache', 'tool-v2-contract-'));
  try {
    const source = join(directory, 'source'); await mkdir(join(source, '图片'), { recursive: true }); await mkdir(join(source, 'images'));
    await writeFile(join(source, 'one.md'), Buffer.from('\ufeff![x](images/a.png) ![y](图片/b.png)', 'utf16le'));
    await writeFile(join(source, 'images/a.png'), 'image A'); await writeFile(join(source, '图片/b.png'), 'image B');
    const root = await prepareFixtures({ root: source, tree: [], manifest: { assets: [{ path: 'images/a.png' }, { path: '图片/b.png' }] } }, join(directory, 'run'), [{ path: 'one.md' }]);
    assert.equal(await readFile(join(root, 'images/a.png'), 'utf8'), 'image A'); assert.equal(await readFile(join(root, '图片/b.png'), 'utf8'), 'image B');
    const steps = Array.from({ length: 10000 }, (_, i) => ({ method: 'insert', status: 'passed', appDurationMs: i < 5000 ? 10 : 1000, roundTripMs: i < 5000 ? 20 : 1100 }));
    await writeFile(join(directory, 'steps.jsonl'), steps.map(JSON.stringify).join('\n'));
    const summary = await summarizeSteps(directory); assert.equal(summary.methods.insert.n, 10000); assert.ok(summary.methods.insert.p95 >= 990);
    const times = steps.map((_, i) => new Date(Date.UTC(2026, 0, 1) + i * 500).toISOString());
    await writeFile(join(directory, 'process-1.jsonl'), times.map((timestamp, i) => JSON.stringify({ timestamp, samplerCostMs: 1, processes: [{ pid: 1, role: 'host', privateBytes: i < 5000 ? 100 : 10000, cpuMachinePercent: 1 }] })).join('\n'));
    const processSummary = await summarizeProcesses(directory); assert.equal(processSummary.privateBytes.n, 10000); assert.ok(processSummary.privateBytes.p95 >= 9900);
    assert.ok(processSummary.memorySlopeBytesPerMinute > 0);
  } finally { within(join(lab, '.cache'), directory); await rm(directory, { recursive: true, force: true }); }
});
test('PowerShell JSON 字段顺序变化不影响环境指纹', () => {
  assert.equal(hashJson({ cpu: [{ name: 'CPU', cores: 8 }], memory: 100 }), hashJson({ memory: 100, cpu: [{ cores: 8, name: 'CPU' }] }));
});
test('性能警告需要同时满足相对和绝对门槛', () => {
  assert.equal(compareReports(report([row(2, 100)]), report([row(1, 100)]), limits).warnings.length, 0);
  assert.equal(compareReports(report([row(130, 100)]), report([row(100, 100)]), limits).warnings.length, 1);
});
test('功能失败不混进性能通过样本', () => {
  const result = aggregateCases([{ id: 'x', title: 'x', status: 'failed', steps: [], durationMs: 1 }, { id: 'x', title: 'x', status: 'passed', steps: [], durationMs: 30 }]);
  assert.equal(result[0].status, 'failed'); assert.equal(result[0].durationMs.median, 30); assert.equal(result[0].privateBytes.median, null);
});

test('目录负载和选中样本重叠时只复制一次，原件保持不变', async () => {
  await mkdir(join(lab, '.cache'), { recursive: true });
  const directory = await mkdtemp(join(lab, '.cache', 'fixture-contract-'));
  try {
    const source = join(directory, 'source'); await mkdir(source);
    await writeFile(join(source, 'one.md'), '# original\n');
    const sample = { path: 'one.md' };
    const root = await prepareFixtures({ root: source, tree: [sample], manifest: { assets: [] } }, join(directory, 'run'), [sample], { tree: true });
    await writeFile(join(root, 'one.md'), '# edited\n');
    assert.equal(await readFile(join(source, 'one.md'), 'utf8'), '# original\n');
    // 标记横跨流块边界，TOC 之外任何变化仍应被检出。
    const head = 'a'.repeat(65530), tail = '\n# body\n';
    await writeFile(join(source, 'one.md'), head + '<!-- toc -->old<!-- /toc -->' + tail);
    const before = await hashOutsideToc(join(source, 'one.md'), 'prefix');
    await writeFile(join(root, 'one.md'), 'prefix' + head + '<!-- toc -->new entry\n'.repeat(1) + '<!-- /toc -->' + tail);
    assert.deepEqual(await hashOutsideToc(join(root, 'one.md')), before);
    await writeFile(join(root, 'one.md'), 'prefix' + head + '<!-- toc -->new<!-- /toc -->changed');
    assert.notEqual((await hashOutsideToc(join(root, 'one.md'))).sha256, before.sha256);
    for (const big of [false, true]) {
      const encode = text => { const b = Buffer.from('\ufeff' + text, 'utf16le'); return big ? b.swap16() : b; };
      await writeFile(join(source, 'one.md'), encode('<!-- toc -->old<!-- /toc -->正文'));
      await writeFile(join(root, 'one.md'), encode('prefix<!-- toc -->new<!-- /toc -->正文'));
      assert.deepEqual(await hashOutsideToc(join(root, 'one.md')), await hashOutsideToc(join(source, 'one.md'), 'prefix'));
    }
  } finally {
    within(join(lab, '.cache'), directory);
    await rm(directory, { recursive: true, force: true });
  }
});

test('保留量使用预热后基线，GPU 独立统计，空采样明确不可用', async () => {
  await mkdir(join(lab, '.cache'), { recursive: true });
  const directory = await mkdtemp(join(lab, '.cache', 'metrics-contract-'));
  try {
    const empty = await summarizeProcesses(directory);
    assert.equal(empty.privatePeak, null); assert.equal(empty.retainedDelta, null);
    assert.ok(empty.unavailable.length);
    const times = [0, 1, 2].map(i => `2026-01-01T00:00:0${i}.0000000Z`);
    const rows = [100, 200, 250].map((privateBytes, i) => ({ timestamp: times[i], processes: [{ pid: 1, role: 'host', privateBytes, cpuMachinePercent: i ? 2 : null }],
      gpuMemory: [{ Name: 'pid_1_adapter', DedicatedUsage: 10 + i, SharedUsage: 20, TotalCommitted: 30 }] }));
    rows[1].gpuMemory = rows[1].gpuMemory[0]; // Historical PowerShell singleton representation.
    await writeFile(join(directory, 'process-1.jsonl'), rows.map(JSON.stringify).join('\n'));
    const result = await summarizeProcesses(directory, [{ phase: 'initial', at: '2026-01-01T00:00:01.000Z' }, { phase: 'idle', at: '2026-01-01T00:00:02.000Z' }]);
    assert.equal(result.retainedDelta, 50); assert.equal(result.startupToEndDelta, 150);
    assert.equal(result.privatePeak, 250); assert.equal(result.gpu.pid_1_adapter.dedicatedPeak, 12);
    assert.equal(result.cpuMachinePercent.n, 2);
  } finally {
    within(join(lab, '.cache'), directory); await rm(directory, { recursive: true, force: true });
  }
});

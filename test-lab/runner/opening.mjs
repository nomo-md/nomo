import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdir, readFile, writeFile, unlink } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { TestHost } from './host.mjs';
import { prepareFixtures } from './dataset.mjs';
import { sourceFingerprint } from './build.mjs';
import { lab, repo, saveJson, hashFile, hashJson, pause, stamp, quantiles, run } from './common.mjs';

// 复用正式打开/等待/关闭入口；桌面采样只存在于此显式测试命令中。
class ScreenCapture {
  constructor(directory) {
    this.directory = directory;
    this.child = spawn('pwsh', ['-NoProfile', '-File', join(lab, 'runner/opening-capture.ps1')],
      { cwd: repo, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    this.stderr = '';
    this.child.stderr.on('data', data => { this.stderr = (this.stderr + data).slice(-6000); });
    this.ready = new Promise((resolveReady, reject) => { this.resolveReady = resolveReady; this.rejectReady = reject; });
    this.lines = createInterface({ input: this.child.stdout });
    this.lines.on('line', line => {
      try {
        const value = JSON.parse(line);
        if (value.ready) { this.resolveReady(); return; }
        const pending = this.pending; this.pending = null;
        if (!pending) throw new Error('采样器返回了非预期响应');
        clearTimeout(pending.timer);
        if (value.ok) pending.resolve(value.result); else pending.reject(new Error(value.error));
      } catch (error) { this.rejectReady(error); this.pending?.reject(error); }
    });
    this.finished = new Promise(resolveExit => {
      this.child.once('error', error => { this.rejectReady(error); this.pending?.reject(error); });
      this.child.once('close', code => {
        this.exited = true; resolveExit(code);
        const error = new Error(`桌面采样器退出 (${code}) ${this.stderr}`);
        this.rejectReady(error); this.pending?.reject(error);
      });
    });
  }
  async initialize() {
    let timer;
    try { await Promise.race([this.ready, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('桌面采样器初始化超时')), 20000); })]); }
    finally { clearTimeout(timer); }
    const samples = [];
    for (let i = 0; i < 5; i++) {
      const before = performance.now(), { clock } = await this.call('ping'), after = performance.now();
      samples.push({ offset: (before + after) / 2 - clock, uncertaintyMs: (after - before) / 2 });
    }
    this.sync = samples.sort((a, b) => a.uncertaintyMs - b.uncertaintyMs)[0];
  }
  call(method, params = {}) {
    if (this.pending || this.exited) return Promise.reject(new Error('采样器不可用或已有在途请求'));
    return new Promise((resolveCall, reject) => {
      this.pending = { resolve: resolveCall, reject, timer: setTimeout(() => {
        this.pending = null; reject(new Error(`桌面采样器 ${method} 超时`));
      }, 10000) };
      this.child.stdin.write(JSON.stringify({ method, ...params }) + '\n');
    });
  }
  async close() {
    this.child.stdin.end();
    let timer;
    try {
      const code = await Promise.race([this.finished, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('采样器退出超时')), 7000); })]);
      if (code !== 0) throw new Error(`采样器异常退出 (${code}): ${this.stderr}`);
    } catch (error) { this.child.kill(); await this.finished; throw error; }
    finally { clearTimeout(timer); this.lines.close(); }
  }
}

function crop(bounds, capture, scale) {
  if (!bounds || !Object.values(bounds).every(Number.isFinite)) throw new Error('缺少实际编辑视口坐标');
  const sx = capture.width / capture.fullWidth * scale, sy = capture.height / capture.fullHeight * scale;
  // 略去滚动条、边框与视口外 UI；正文内部不屏蔽图片、图表或文本。
  const x0 = Math.ceil((bounds.x + 8) * sx), y0 = Math.ceil((bounds.y + 8) * sy);
  const x1 = Math.floor((bounds.x + bounds.width - 12) * sx), y1 = Math.floor((bounds.y + bounds.height - 8) * sy);
  if (x0 < 0 || y0 < 0 || x1 > capture.width || y1 > capture.height || x1 - x0 < 40 || y1 - y0 < 40)
    throw new Error(`编辑视口不在捕获客户区内：${JSON.stringify({ bounds, width: capture.fullWidth, height: capture.fullHeight, scale })}`);
  const pixels = [];
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) pixels.push(y * capture.width + x);
  return { pixels, x0, y0, x1, y1 };
}

export function analyzeOpeningPixels(raw, capture, timing, bounds, scale, blank) {
  if (capture.failure) throw new Error(capture.failure);
  const { frames, width, height } = capture, size = width * height;
  if (frames.length < 8 || raw.length !== size * frames.length) throw new Error('屏幕采样不足或像素流长度不匹配');
  const frame = index => raw.subarray(index * size, (index + 1) * size);
  const { pixels, ...roi } = crop(bounds, capture, scale);
  const baseline = frame(timing.baseline), complete = frame(timing.complete), echo = frame(timing.echo);
  const preInput = frame(timing.preInput ?? timing.complete);
  const counts = new Uint32Array(256);
  for (const p of pixels) counts[complete[p]]++;
  const background = counts.indexOf(Math.max(...counts));
  let ink = pixels.filter(p => complete[p] < background - 30 && Math.abs(complete[p] - baseline[p]) > 22);
  if (!blank) {
    if (ink.length < 8) throw new Error('无法从最终首屏识别与旧文档不同的文字，首屏可见计时不可用');
    const firstRow = Math.floor(ink[0] / width);
    ink = ink.filter(p => Math.floor(p / width) <= firstRow + 30);
  } else {
    ink = pixels.filter(p => Math.abs(complete[p] - baseline[p]) > 22);
    if (ink.length < 8) throw new Error('空白编辑区与等待页无法区分');
  }
  const matches = (image, reference, mask, tolerance = 22) => {
    let matched = 0;
    for (const p of mask) if (Math.abs(image[p] - reference[p]) <= tolerance) matched++;
    return matched / mask.length;
  };
  const at = index => frames[index][1] + timing.offset - timing.started;
  const begin = index => frames[index][0] + timing.offset - timing.started;
  const completeMatch = image => {
    let changed = 0;
    const allowed = Math.max(5, Math.ceil(pixels.length * .0005));
    for (const p of pixels) if (Math.abs(image[p] - complete[p]) > 12 && ++changed > allowed) return false;
    return true;
  };
  let visible = -1;
  for (let i = timing.baseline + 1; i <= timing.complete; i++) {
    if (begin(i) < 0 || (blank && at(i) < timing.readableMs)) continue;
    if (matches(frame(i), complete, ink) >= .90) { visible = i; break; }
  }
  if (visible < 0) throw new Error('未捕获到首屏目标内容');
  let full = timing.complete;
  while (full > visible && completeMatch(frame(full - 1))) full--;
  if (at(timing.complete) - at(full) < 100) throw new Error('首屏没有足够的稳定画面样本');
  const echoMask = pixels.filter(p => Math.abs(echo[p] - preInput[p]) > 22);
  if (echoMask.length < 3) throw new Error('按键已进入正文，但未取得可区分的屏幕回显');
  let echoIndex = -1;
  for (let i = (timing.preInput ?? timing.complete) + 1; i <= timing.echo; i++) {
    if (begin(i) < timing.inputMs) continue;
    if (matches(frame(i), echo, echoMask) >= .95) { echoIndex = i; break; }
  }
  if (echoIndex < 0) throw new Error('没有捕获到真实按键回显');
  // 稳定窗口和按键属于后验验证，不把人为等待包装成用户必须等待的时间。
  // 完整首屏必须与已观察到的可编辑/资源就绪状态同时成立。
  let interactive = full;
  while (interactive <= timing.complete && begin(interactive) < timing.readyMs) interactive++;
  if (interactive > timing.complete) throw new Error('没有捕获到就绪状态之后的完整首屏');
  const bracket = index => [Math.max(0, index ? begin(index - 1) : 0), at(index)];
  return {
    firstVisibleMs: at(visible), completeInteractiveMs: at(interactive), contentCompleteMs: at(full), inputEchoMs: at(echoIndex),
    firstVisibleBracketMs: bracket(visible), completeInteractiveBracketMs: bracket(interactive),
    readyObservedMs: timing.readyMs, inputSentMs: timing.inputMs, confirmationOverheadMs: at(echoIndex) - at(interactive),
    sampleCount: frames.length, sampleGapMs: quantiles(frames.slice(1).map((f, i) => f[0] - frames[i][0])),
    captureCostMs: quantiles(frames.map(f => f[1] - f[0])), clockUncertaintyMs: timing.uncertaintyMs,
    roi, visibleMaskPixels: ink.length, inputMaskPixels: echoMask.length,
    evidenceFrames: { baseline: timing.baseline, firstVisible: visible, complete: interactive,
      preInput: timing.preInput ?? timing.complete, inputEcho: echoIndex },
  };
}

const samePath = (a, b) => a && resolve(a.replace(/^\\\\\?\\/, '')).toLowerCase() === resolve(b).toLowerCase();
const labels = { blank: '空白', text: '5 MiB 纯文本', mixed: '1 MiB 混合', images: '图片瀑布流', diagrams: '七类 Mermaid', code: '29 种代码语言', 'four-tabs': '四标签（打开第 4 份）' };
async function waitOpeningReady(host, path, started) {
  const deadline = performance.now() + host.config.actionTimeoutMs;
  let readableMs, readyMs, previous;
  while (performance.now() < deadline) {
    const state = await host.raw('diagnostics');
    if (state.failure || state.panes?.some(p => p.error || p.referencesFailed) || state.images?.failed || state.embeds?.failed)
      throw new Error('首屏状态或资源加载失败');
    const target = samePath(state.document?.path, path), at = performance.now() - started;
    if (target && state.ready?.readable) readableMs ??= at;
    const ready = target && state.ready?.stable && state.ready?.editable && !state.images.pending && !state.embeds.pending;
    const key = JSON.stringify([state.document?.revision, state.reading, state.panes?.map(p => [p.scrollTop, p.contentHeight, p.views])]);
    if (!ready || key !== previous) readyMs = undefined;
    previous = key;
    if (ready) {
      readyMs ??= at;
      if (at - readyMs >= 150 && readableMs !== undefined) return { state, readableMs, readyMs };
    }
    await pause(20);
  }
  throw new Error('打开后首屏资源及可编辑状态未在预算内就绪');
}
async function waitNaturalScreen(host, capture, ready, path) {
  const deadline = performance.now() + 15000;
  let bounds = ready.panes[0].viewportBounds;
  await capture.call('region', { ...bounds, scale: host.record.environment.scaleFactor });
  while (performance.now() < deadline) {
    const sample = await capture.call('mark');
    if (sample.stableMs >= 150) {
      const state = await host.raw('diagnostics');
      if (state.failure || state.images.failed || state.embeds.failed) throw new Error('自然首屏加载失败');
      const current = state.panes?.[0]?.viewportBounds;
      if (JSON.stringify(current) !== JSON.stringify(bounds)) {
        bounds = current;
        await capture.call('region', { ...bounds, scale: host.record.environment.scaleFactor });
      } else if (samePath(state.document?.path, path) && state.ready?.stable && state.ready?.editable && !state.images.pending && !state.embeds.pending) {
        const confirmed = await capture.call('mark');
        if (confirmed.stableMs >= 150) return { sample: confirmed, bounds };
      }
    }
    await pause(25);
  }
  throw new Error('自然首屏未在预算内取得稳定像素和资源就绪状态');
}
export async function runOpeningSuite(config, tests, dataset, build) {
  const id = `opening-${stamp()}`, directory = join(lab, 'results', id);
  await mkdir(directory, { recursive: true });
  const measurementFiles = ['opening.mjs', 'opening-capture.cs', 'opening-capture.ps1', 'host.mjs'];
  const fingerprint = async () => hashJson(await Promise.all(measurementFiles.map(async f => [f, await hashFile(join(lab, 'runner', f))])));
  const report = { id, status: 'running', startedAt: new Date().toISOString(), build, dataset: dataset.name, datasetFingerprint: dataset.fingerprint,
    runnerFingerprint: await fingerprint(), repetitions: config.repetitions, cases: [],
    environment: JSON.parse(await run('pwsh', ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'environment'])),
    definition: {
      start: '应用已运行，从测试器发出正式打开请求开始；包含文件 RPC/25 ms 控制轮询，不包含进程启动和文件选择。',
      firstVisible: '前台桌面像素出现最终首屏首部文字（90% 特征匹配）；空白文档须同时确认文档身份及可读状态。',
      completeInteractive: '首屏像素完整且已观察到可编辑、已请求资源成功就绪的首个画面；以随后稳定画面及真实 Q 按键回显校验，成功后撤销并校验原文。',
      limits: '稳定确认及按键验证是后验检查，其额外等待不计入主列，inputEchoMs 单独记录验证结束。两项仍含 RPC 及离散采样误差，是观测上界；不等于显示器光学测量或离屏全文加载。',
      isolation: '各场景各轮新进程/独立 profile；六个单文档场景从轻量等待文档打开，四标签场景先打开前三份。未清空系统文件缓存。',
    } };
  let host, capture, interrupted = false;
  const interrupt = () => { interrupted = true; };
  process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
  console.log(`打开体验采样已启动：${id}；${tests.length} 个场景 × ${config.repetitions} 轮`);
  try {
    outer: for (const test of tests) for (let round = 1; round <= config.repetitions; round++) {
      if (interrupted) break outer;
      const key = `${test.id.replace('performance.', '')}-${round}`, caseDir = join(directory, key);
      const row = { id: test.id, title: labels[test.id.replace('performance.', '')], round, status: 'running', directory: caseDir, steps: [], assertions: [] };
      report.cases.push(row);
      try {
        row.phase = 'prepare';
        const fixtures = await prepareFixtures(dataset, caseDir, test.samples);
        const baselinePath = join(fixtures, 'opening-wait.md');
        await writeFile(baselinePath, '# NOMO OPENING BASELINE\n\nWaiting for the next document.\n\nThis is the previous document, not the target screen.\n');
        host = new TestHost(build, caseDir, config, row);
        await host.start(baselinePath);
        let initial = await host.wait('resources');
        const baselineId = initial.activeTabId;
        if (test.samples.length > 1) {
          for (const sample of test.samples.slice(0, -1)) { await host.call('open', { path: join(fixtures, sample.path) }); await host.wait('resources'); }
          const third = (await host.raw('diagnostics')).activeTabId;
          await host.call('switch', { tabId: baselineId }); await host.call('close'); await host.call('switch', { tabId: third });
        }
        await host.raw('focus'); await host.wait('resources');
        row.phase = 'capture-start';
        capture = new ScreenCapture(caseDir); await capture.initialize();
        const dimensions = await capture.call('start', { pid: host.child.pid, path: join(caseDir, 'screen.gray') });
        await pause(180);
        const baseline = await capture.call('mark');
        if (baseline.index < 2) throw new Error('等待页屏幕采样不足');
        const target = test.samples.at(-1), path = join(fixtures, target.path);
        const started = performance.now();
        row.phase = 'open-readable';
        await host.raw('open', { path });
        row.phase = 'resources';
        const { state: ready, readableMs, readyMs } = await waitOpeningReady(host, path, started);
        assert.equal(ready.images.failed, 0, '首屏请求图片加载失败');
        assert.equal(ready.embeds.failed, 0, '首屏请求公式/图表渲染失败');
        assert.equal(ready.ready.editable, true, '文档不可编辑');
        assert.equal(ready.tabs.length, test.samples.length > 1 ? 4 : 2, '打开标签数量不符');
        // 先固定自然打开后的首屏，避免后续选区/焦点准备改变画面，抬高完成时点。
        row.phase = 'natural-screen';
        const { sample: complete, bounds: completeBounds } = await waitNaturalScreen(host, capture, ready, path);
        await host.call('select', { fromByte: 0 }); await host.raw('focus');
        const before = await host.raw('readRange', { fromByte: 0, toByte: Math.min(512, ready.document.byteLength) });
        await host.wait('resources'); await pause(150);
        const preInput = await capture.call('mark');
        row.phase = 'native-input';
        const input = await capture.call('input');
        const changed = await host.wait('editable', config.actionTimeoutMs, s => s.document?.revision !== ready.document.revision);
        assert.equal(changed.document.byteLength, ready.document.byteLength + 1, '真实按键必须只插入一个字符');
        const after = await host.raw('readRange', { fromByte: 0, toByte: Math.min(513, changed.document.byteLength) });
        // readRange 可能向 UTF-8 边界外扩，比较共同前缀并确认恰好插入一个 Q。
        const insertion = [...after.text].some((character, index) => character === 'Q' &&
          [...after.text].filter((_, i) => i !== index).join('').startsWith(before.text));
        assert.ok(insertion, '真实 Q 按键没有进入预期正文首部');
        await host.wait('stable'); await pause(150);
        const echo = await capture.call('mark'), samples = await capture.call('stop');
        row.phase = 'analyze-pixels';
        const timing = { started, baseline: baseline.index, complete: complete.index, preInput: preInput.index, echo: echo.index,
          readableMs, readyMs, inputMs: input.clock + capture.sync.offset - started, ...capture.sync };
        row.metrics = analyzeOpeningPixels(await readFile(samples.rawPath), samples, timing,
          completeBounds, row.environment.scaleFactor, ready.document.byteLength === 0);
        row.capture = dimensions;
        row.measurement = { timing, bounds: completeBounds, imageEntries: ready.images.entries, embedEntries: ready.embeds.entries };
        await saveJson(join(caseDir, 'screen-timeline.json'), { ...samples, ...row.measurement, rawPath: undefined });
        for (const [name, index] of Object.entries(row.metrics.evidenceFrames))
          await capture.call('save', { index, path: join(caseDir, `${name}.png`) });
        await capture.close(); capture = null;
        await unlink(samples.rawPath);
        row.phase = 'restore-and-close';
        await host.call('undo');
        assert.equal((await host.raw('digest')).sha256, target.sha256, '撤销后正文与原始样本不符');
        assert.equal(await hashFile(path), target.sha256, '原始文档副本被意外保存');
        row.assertions = ['目标文档身份', '首屏资源无失败', '桌面目标像素可见', '首屏画面稳定', '真实按键正文及屏幕回显', '撤销及磁盘正文一致'];
        while ((await host.raw('diagnostics')).tabs.length) await host.call('close', { discard: true });
        await host.call('exit'); await host.waitExit();
        row.status = 'passed';
        row.phase = 'complete';
      } catch (error) {
        row.status = 'failed'; row.error = String(error.message ?? error);
      } finally {
        if (capture) { try { await capture.close(); } catch (e) { row.cleanupError = String(e); } capture = null; }
        if (host) { try { await host.stop(); } catch (e) { row.cleanupError = String(e); } host = null; }
        if (row.cleanupError) row.status = 'failed';
        await saveJson(join(caseDir, 'case.json'), row);
      }
      if (row.cleanupError || (config.failFast && row.status !== 'passed')) break outer;
    }
  } finally {
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
    report.finishedAt = new Date().toISOString();
    report.runnerUnchanged = report.runnerFingerprint === await fingerprint();
    report.sourceUnchanged = build.sourceFingerprint === await sourceFingerprint();
    report.status = !interrupted && report.runnerUnchanged && report.sourceUnchanged && report.cases.length === tests.length * config.repetitions && report.cases.every(c => c.status === 'passed') ? 'passed' : 'failed';
    report.aggregate = tests.map(test => {
      const rows = report.cases.filter(c => c.id === test.id), passed = rows.filter(c => c.status === 'passed');
      return { id: test.id, title: labels[test.id.replace('performance.', '')], runs: rows.length, passed: passed.length,
        firstVisibleMs: quantiles(passed.map(c => c.metrics.firstVisibleMs)), completeInteractiveMs: quantiles(passed.map(c => c.metrics.completeInteractiveMs)),
        contentCompleteMs: quantiles(passed.map(c => c.metrics.contentCompleteMs)), confirmationOverheadMs: quantiles(passed.map(c => c.metrics.confirmationOverheadMs)),
        maxCaptureGapMs: Math.max(0, ...passed.map(c => c.metrics.sampleGapMs.max)), errors: rows.filter(c => c.error).map(c => c.error) };
    });
    await saveJson(join(directory, 'report.json'), report);
    const summary = { ...report, cases: report.cases.map(({ id, title, round, status, phase, error, metrics, cleanupError, cleanupVerified, environment }) =>
      ({ id, title, round, status, phase, error, metrics, cleanupError, cleanupVerified, environment })) };
    await saveJson(join(directory, 'summary.json'), summary);
    const format = q => q.n ? `${Math.round(q.median)}（${Math.round(q.min)}–${Math.round(q.max)}）` : '未测得';
    const markdown = `# 文档打开体验采样\n\n状态：${report.status}；${report.cases.filter(c => c.status === 'passed').length}/${tests.length * config.repetitions} 轮通过。单位 ms，展示中位数（最小–最大）。\n\n` +
      '| 场景 | 首屏可见 | 首屏完整且交互已验证 |\n| --- | ---: | ---: |\n' +
      report.aggregate.map(c => `| ${c.title} | ${format(c.firstVisibleMs)} | ${format(c.completeInteractiveMs)} |`).join('\n') +
      `\n\n${Object.values(report.definition).join('\n\n')}\n\n构建：${build.id}。结果与证据见同目录 summary.json 和各用例文件夹。\n`;
    await writeFile(join(directory, 'report.md'), markdown);
    console.log(JSON.stringify({ id, status: report.status, summary: join(directory, 'summary.json'), report: join(directory, 'report.md') }));
    if (report.status !== 'passed') process.exitCode = 1;
  }
  return report;
}

import { mkdir, writeFile, rename, unlink, readFile, appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import net from 'node:net';
import { launch, run, lab, pause, within } from './common.mjs';
import { connectCdp } from './cdp.mjs';
import { stopOwnedProcess } from './process-lifecycle.mjs';

async function freePort() {
  const server = net.createServer(); await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
export class TestHost {
  constructor(build, directory, config, record) { Object.assign(this, { build, directory, config, record }); this.serial = 0; this.boot = 0; this.stopping = false; }
  async start(path) {
    const started = performance.now();
    this.boot++; this.exit = undefined; this.stopping = false; this.stopTask = null;
    await mkdir(this.directory, { recursive: true });
    for (const name of ['ready.json', 'request.json', 'response.json', 'sampler.stop']) await unlink(join(this.directory, name)).catch(e => { if (e.code !== 'ENOENT') throw e; });
    const port = await freePort();
    this.child = launch(this.build.binary, path ? [path] : [], { log: join(this.directory, `app-${this.boot}.log`), env: {
      ...process.env, NOMO_AUTOMATION_DIR: this.directory, NOMO_AUTOMATION_CDP_PORT: String(port),
      // WebView2 最终使用此进程级参数；实机确认 config.data_directory 会被运行时默认路径覆盖。
      WEBVIEW2_USER_DATA_FOLDER: join(this.directory, 'profile', 'webview') } });
    this.child.once('exit', (code, signal) => { this.exit = { code, signal }; });
    this.child.once('error', error => { this.exit = { error: String(error) }; });
    this.record.processes ??= []; this.record.processes.push({ pid: this.child.pid, boot: this.boot, port, startedAt: new Date().toISOString() });
    this.sampler = launch('pwsh', ['-NoProfile', '-File', join(lab, 'runner/sample.ps1'), '-RootProcessId', String(this.child.pid),
      '-OutputPath', join(this.directory, `process-${this.boot}.jsonl`), '-StopPath', join(this.directory, 'sampler.stop'), '-IntervalMs', String(this.config.sampleMs)],
      { log: join(this.directory, `sampler-${this.boot}.log`) });
    this.counters = launch('pwsh', ['-NoProfile', '-File', join(lab, 'runner/counters.ps1'), '-RootProcessId', String(this.child.pid),
      '-OutputPath', join(this.directory, `process-${this.boot}.jsonl.counters.json`), '-StopPath', join(this.directory, 'sampler.stop')],
      { log: join(this.directory, `counters-${this.boot}.log`) });
    const connecting = connectCdp(port).then(async cdp => {
      if (this.stopping) { cdp.close(); return; }
      this.cdp = cdp; this.record.cdp = 'available';
      cdp.on('Runtime.exceptionThrown', event => {
        void appendFile(join(this.directory, `webview-${this.boot}.jsonl`), JSON.stringify(event) + '\n').catch(() => {});
      });
      await cdp.call('Runtime.enable');
    }).catch(error => { this.record.cdp = { unavailable: String(error) }; });
    const ready = await this.waitFile('ready.json', this.config.startupTimeoutMs);
    this.record.startupMs = performance.now() - started;
    if (ready.result?.protocol !== 2 || ready.result?.pid !== this.child.pid) throw new Error('测试接口版本或 PID 不匹配');
    this.record.environment = ready.result.environment;
    this.record.profile = ready.result.profile;
    await connecting;
    // 短用例可能早于首次 GPU/WMI 采样结束；独立核对真实版本和数据目录，不能依赖配置意图。
    this.record.webview = JSON.parse(await run('pwsh', ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'webview', '-RootProcessId', String(this.child.pid)]));
    within(join(this.directory, 'profile', 'webview'), this.record.webview.userDataDirectory);
    if (!this.record.webview.fileVersion) throw new Error('WebView2 实际版本不可用');
    const sampling = await this.waitFile(`process-${this.boot}.jsonl.ready`, this.config.startupTimeoutMs);
    if (sampling.pid !== this.child.pid) throw new Error('采样器身份不匹配');
    this.record.measurementStartedAt = new Date().toISOString();
    return ready;
  }
  async waitFile(name, timeout) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (this.exit) throw new Error(`测试进程退出：${JSON.stringify(this.exit)}`);
      try { return JSON.parse(await readFile(join(this.directory, name), 'utf8')); }
      catch (e) { if (e.code !== 'ENOENT') throw e; }
      await pause(25);
    }
    const error = new Error(`等待 ${name} 超时（${timeout} ms）`); error.code = 'TIMEOUT'; throw error;
  }
  async call(method, params = {}, timeout = this.config.actionTimeoutMs) {
    const id = `${this.boot}-${++this.serial}`, started = performance.now();
    if (this.record.steps.length >= 8192) {
      this.record.steps.splice(0, 4096); this.record.evictedSteps = (this.record.evictedSteps ?? 0) + 4096;
    }
    const step = { id, method, startedAt: new Date().toISOString(), status: 'running',
      stage: params.stage, mode: params.mode, target: params.path ?? params.tabId, command: params.command?.type,
      fromByte: params.fromByte, toByte: params.toByte, byte: params.byte, revision: params.revision,
      query: typeof params.query === 'string' ? params.query.slice(0, 160) : undefined,
      textBytes: typeof params.text === 'string' ? Buffer.byteLength(params.text) : undefined };
    this.record.steps.push(step);
    try {
      await writeFile(join(this.directory, 'last-step.json'), JSON.stringify(step));
      await writeFile(join(this.directory, 'request.tmp'), JSON.stringify({ id, method, params }));
      await rename(join(this.directory, 'request.tmp'), join(this.directory, 'request.json'));
      const response = await this.waitFile('response.json', timeout);
      await unlink(join(this.directory, 'response.json'));
      if (response.id !== id) throw new Error('响应 ID 不匹配；停止发送操作');
      step.appDurationMs = response.durationMs;
      if (!response.ok) throw new Error(response.error);
      step.status = 'passed'; return response.result;
    } catch (error) { step.status = error.code === 'TIMEOUT' ? 'timeout' : 'failed'; step.error = String(error); throw error; }
    finally {
      step.roundTripMs = performance.now() - started;
      await appendFile(join(this.directory, 'steps.jsonl'), JSON.stringify(step) + '\n');
      await writeFile(join(this.directory, 'last-step.json'), JSON.stringify(step));
    }
  }
  async native(params) {
    await this.call('focus');
    const args = ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'input', '-RootProcessId', String(this.child.pid)];
    if (params.text) args.push('-Text', params.text);
    if (params.keys) args.push('-Keys', params.keys);
    return run('pwsh', args);
  }
  async screenshot(name = 'failure.png') {
    if (this.cdp) {
      const result = await this.cdp.call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, 5000);
      await writeFile(join(this.directory, name), Buffer.from(result.data, 'base64')); return name;
    }
    await run('pwsh', ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'screenshot', '-RootProcessId', String(this.child.pid), '-OutputPath', join(this.directory, name)]);
    return name;
  }
  async capture(timeout = this.config.actionTimeoutMs) {
    const data = await this.call('diagnostics', {}, timeout);
    data.telemetry = await this.call('telemetry', {}, timeout);
    if (this.cdp) try { data.jsHeap = await this.cdp.call('Runtime.getHeapUsage', {}, 3000); } catch (e) { data.jsHeap = { unavailable: String(e) }; }
    else data.jsHeap = { unavailable: 'CDP unavailable' };
    return data;
  }
  async readingAnchors() {
    if (!this.cdp) throw new Error('阅读位置几何测量需要本次测试进程的 CDP');
    const result = await this.cdp.call('Runtime.evaluate', { returnByValue: true, expression: `(() =>
      [...document.querySelectorAll('.markdown-session-scroll')].filter(root => {
        const r = root.getBoundingClientRect(); return r.width > 0 && r.height > 0;
      }).map(root => {
        const bounds = root.getBoundingClientRect();
        const blocks = root.querySelectorAll('.ProseMirror p,.ProseMirror h1,.ProseMirror h2,.ProseMirror h3,.ProseMirror h4,.ProseMirror h5,.ProseMirror h6,.cm-line');
        const block = [...blocks].find(node => {
          const r = node.getBoundingClientRect(); return node.textContent.trim() && r.width > 0 && r.bottom > bounds.top + 4 && r.top < bounds.bottom;
        });
        return { source: Boolean(root.querySelector('.cm-editor')), scrollTop: root.scrollTop, scrollHeight: root.scrollHeight,
          anchor: block ? { text: block.textContent.trim().slice(0, 256), node: block.tagName, top: block.getBoundingClientRect().top - bounds.top } : null };
      }))()` });
    if (result.exceptionDetails) throw new Error('阅读位置几何测量失败');
    return result.result.value;
  }
  async waitExit(timeout = 30000) {
    const deadline = Date.now() + timeout;
    while (!this.exit && Date.now() < deadline) await pause(50);
    if (!this.exit) throw new Error('正常退出超时');
    if (this.exit.code !== 0) throw new Error(`正常退出返回异常：${JSON.stringify(this.exit)}`);
  }
  stop() {
    // 信号处理和 finally 可能同时清理；后一个调用必须等待同一次清理完成。
    return this.stopTask ??= this.stopProcesses();
  }
  async stopProcesses() {
    this.stopping = true;
    this.cdp?.close(); this.cdp = null;
    await writeFile(join(this.directory, 'sampler.stop'), 'stop');
    this.record.measurementEndedAt = new Date().toISOString();
    const errors = [];
    try { await stopOwnedProcess(this.child, { terminate: child => run('taskkill', ['/PID', String(child.pid), '/T', '/F']) }); }
    catch (error) { errors.push(String(error)); }
    for (const worker of [this.sampler, this.counters]) {
      try {
        const result = await stopOwnedProcess(worker, { graceMs: 7000, terminate: child => { child.kill(); } });
        if (result?.code !== 0) this.record.samplerWarning = `采样器异常退出：${result?.code}`;
      } catch (error) { errors.push(String(error)); }
    }
    if (this.record.webview?.ownedProcesses) {
      try {
        const checked = JSON.parse(await run('pwsh', ['-NoProfile', '-File', join(lab, 'runner/native.ps1'), '-Action', 'check-processes', '-Text', JSON.stringify(this.record.webview.ownedProcesses)]));
        if (checked.survivors.length) errors.push(`本次应用子进程仍存活：${checked.survivors.map(p => p.pid).join(',')}`);
      } catch (error) { errors.push(String(error)); }
    }
    if (this.record.processes?.length) this.record.processes.at(-1).exit = this.exit ?? { unavailable: true };
    this.record.cleanupVerified = errors.length === 0;
    if (errors.length) { const error = new Error(errors.join('; ')); error.code = 'CLEANUP_FAILED'; throw error; }
  }
}

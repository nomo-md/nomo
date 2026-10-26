import { openSync, writeSync, closeSync } from 'node:fs';
import { saveJson, pause } from './common.mjs';
export async function connectCdp(port, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(1000) })).json();
      const page = pages.find(p => p.type === 'page' && /tauri|localhost/.test(p.url));
      if (!page?.webSocketDebuggerUrl) throw new Error('页面尚未就绪');
      const ws = new WebSocket(page.webSocketDebuggerUrl);
      await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
      let id = 0; const requests = new Map(); const listeners = new Map();
      ws.onmessage = event => {
        const message = JSON.parse(event.data);
        if (message.id) { const request = requests.get(message.id); if (!request) return; requests.delete(message.id); clearTimeout(request.timer);
          message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result); }
        else listeners.get(message.method)?.(message.params);
      };
      ws.onclose = () => { for (const r of requests.values()) { clearTimeout(r.timer); r.reject(new Error('CDP 连接断开')); } requests.clear(); };
      const call = (method, params = {}, timeout = 30000) => new Promise((resolve, reject) => {
        const serial = ++id, timer = setTimeout(() => { requests.delete(serial); reject(new Error(`CDP 超时：${method}`)); }, timeout);
        requests.set(serial, { resolve, reject, timer }); ws.send(JSON.stringify({ id: serial, method, params }));
      });
      return { call, close: () => ws.close(), on: (event, listener) => { listeners.set(event, listener); return () => listeners.delete(event); },
        async heap(path) {
          const fd = openSync(path, 'w'); let writeError;
          listeners.set('HeapProfiler.addHeapSnapshotChunk', p => { try { writeSync(fd, p.chunk); } catch (e) { writeError = e; } });
          try { await call('HeapProfiler.enable'); await call('HeapProfiler.takeHeapSnapshot', { reportProgress: false }, 180000); }
          finally { listeners.delete('HeapProfiler.addHeapSnapshotChunk'); closeSync(fd); }
          if (writeError) throw writeError;
        },
        async cpuStart() { await call('Profiler.enable'); await call('Profiler.start'); },
        async cpuStop(path) { const result = await call('Profiler.stop'); await saveJson(path, result.profile); },
      };
    } catch { await pause(100); }
  }
  throw new Error('无法连接本次测试进程的 CDP 端口');
}

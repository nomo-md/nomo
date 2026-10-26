const diagnostics = import.meta.env.VITE_NOMO_AUTOMATION === '1' ? new Set<() => { active: number; queued: number }>() : undefined;
export function getRenderQueueDiagnostics() {
  return diagnostics ? [...diagnostics].map(read => read()) : [];
}
/** 限制昂贵增强任务并发；取消排队项时立即释放闭包，已运行的库调用只等待其结束。 */
export function createRenderTaskQueue(concurrency: number) {
  let active = 0;
  const queue: Array<() => void> = [];
  // 队列与渲染器模块同生命周期，仅测试构建登记计数读取器，不持有排队任务。
  diagnostics?.add(() => ({ active, queued: queue.length }));
  let deadline = 0;
  let resume: ReturnType<typeof setTimeout> | undefined;
  function drain() {
    if (resume !== undefined) return;
    // 库调用不可抢占；只在任务之间让出主线程，避免连续微任务把多次增强渲染连成一个长任务。
    if (queue.length && active < concurrency && performance.now() >= deadline) {
      resume = setTimeout(() => {
        resume = undefined;
        deadline = performance.now() + 4;
        drain();
      }, 0);
      return;
    }
    while (active < concurrency && queue.length) queue.shift()!();
    if (!active && !queue.length) deadline = 0;
  }
  return function run<T>(task: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    return new Promise((resolve, reject) => {
      let started = false;
      const abort = () => {
        if (started) return;
        const index = queue.indexOf(start);
        if (index >= 0) queue.splice(index, 1);
        if (!queue.length && resume !== undefined) { clearTimeout(resume); resume = undefined; }
        signal?.removeEventListener('abort', abort);
        reject(new DOMException('Render cancelled', 'AbortError'));
      };
      const start = () => {
        started = true;
        active++;
        Promise.resolve().then(() => {
          signal?.throwIfAborted();
          return task();
        }).then(resolve, reject).finally(() => {
          signal?.removeEventListener('abort', abort);
          active--;
          drain();
        });
      };
      if (signal?.aborted) { abort(); return; }
      signal?.addEventListener('abort', abort, { once: true });
      if (!active && !queue.length && resume === undefined) deadline = performance.now() + 4;
      queue.push(start);
      drain();
    });
  };
}

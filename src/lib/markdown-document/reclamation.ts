interface Reclamation { release(): void; resolve(): void; reject(error: unknown): void; }
const queue: Reclamation[] = [];
let timer: ReturnType<typeof setTimeout> | undefined;

/** 隐藏或最小化窗口不依赖 rAF 收口；截止时间只调度清理，不取消保护操作。 */
export function afterMarkdownClosePaint(): Promise<void> {
  return new Promise(resolve => {
    let frame = 0, done = false;
    const finish = () => { if (done) return; done = true; clearTimeout(deadline); cancelAnimationFrame(frame); resolve(); };
    const deadline = setTimeout(finish, 250);
    frame = requestAnimationFrame(() => setTimeout(finish, 0));
  });
}

/** 与可见内容调度分离；每个宏任务最多工作 4ms，不因连续输入永久暂停。 */
function schedule() {
  if (timer !== undefined || !queue.length) return;
  timer = setTimeout(() => {
    timer = undefined;
    const started = performance.now();
    do {
      const item = queue.shift()!;
      try { item.release(); item.resolve(); } catch (error) { item.reject(error); }
    } while (queue.length && performance.now() - started < 4);
    schedule();
  }, 0);
}
export function reclaimMarkdownViews(releases: Array<() => void>): Promise<void> {
  const pending = releases.map(release => new Promise<void>((resolve, reject) => queue.push({ release, resolve, reject })));
  schedule();
  // 每项均带拒绝处理，单项失败时其余已接管的视图仍会释放。
  return Promise.allSettled(pending).then(results => {
    const failed = results.find(result => result.status === 'rejected');
    if (failed?.status === 'rejected') throw failed.reason;
  });
}

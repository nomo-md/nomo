export enum MarkdownTaskPriority {
  Input,
  VisibleText,
  NearViewport,
  VisibleEnhancement,
  Prefetch,
  Index,
  RemainingMount,
}
interface Task {
  owner: object;
  key: string;
  priority: MarkdownTaskPriority;
  run(): void;
  generation: number;
}

/** 应用窗口共享的协作调度。正文所有权和任务所有权分离，卸载只取消该视图的任务。 */
export class MarkdownWorkScheduler {
  private tasks: Task[] = [];
  private ownerGenerations = new WeakMap<object, number>();
  private paused = new WeakSet<object>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private scheduledAt = Infinity;
  private busyUntil = 0;
  constructor(
    private readonly budgetMilliseconds = 4,
    private readonly now: () => number = () => performance.now(),
  ) {}

  enqueue(owner: object, key: string, priority: MarkdownTaskPriority, run: () => void) {
    const generation = this.ownerGenerations.get(owner) ?? 0;
    const previous = this.tasks.findIndex((task) => task.owner === owner && task.key === key);
    const task = { owner, key, priority, run, generation };
    if (previous >= 0) this.tasks[previous] = task;
    else this.tasks.push(task);
    this.schedule();
  }
  interacting(milliseconds = 120) {
    this.busyUntil = Math.max(this.busyUntil, this.now() + milliseconds);
  }
  pause(owner: object, paused: boolean) {
    if (paused) this.paused.add(owner);
    else {
      this.paused.delete(owner);
      this.schedule();
    }
  }
  cancel(owner: object) {
    this.ownerGenerations.set(owner, (this.ownerGenerations.get(owner) ?? 0) + 1);
    this.tasks = this.tasks.filter((task) => task.owner !== owner);
    this.paused.delete(owner);
    if (!this.tasks.length && this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
      this.scheduledAt = Infinity;
    }
  }
  private schedule() {
    const active = this.tasks.filter(task => !this.paused.has(task.owner));
    if (!active.length) return;
    const now = this.now();
    const delay = active.some(task => task.priority <= MarkdownTaskPriority.NearViewport)
      ? 0 : Math.max(0, this.busyUntil - now);
    const scheduledAt = now + delay;
    if (this.timer !== null) {
      if (this.scheduledAt <= scheduledAt) return;
      clearTimeout(this.timer);
    }
    this.scheduledAt = scheduledAt;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.scheduledAt = Infinity;
      this.flush();
    }, delay);
  }
  private flush() {
    const started = this.now();
    this.tasks.sort((a, b) => a.priority - b.priority);
    let consumed = 0;
    while (this.tasks.length && this.now() - started < this.budgetMilliseconds) {
      const index = this.tasks.reduce(
        (best, task, index) =>
          !this.paused.has(task.owner) &&
          (this.now() >= this.busyUntil || task.priority <= MarkdownTaskPriority.NearViewport) &&
          (best < 0 || task.priority < this.tasks[best].priority)
            ? index
            : best,
        -1,
      );
      if (index < 0) break;
      const task = this.tasks.splice(index, 1)[0];
      if (task.generation !== (this.ownerGenerations.get(task.owner) ?? 0)) continue;
      try {
        task.run();
      } catch (error) {
        queueMicrotask(() => {
          throw error;
        });
      }
      // now 在确定性的测试时钟上可能不变化；同一批最多处理 64 个独立任务。
      if (++consumed >= 64) break;
    }
    this.schedule();
  }
}

export const markdownWorkScheduler = new MarkdownWorkScheduler();

/** 同一帧先读取几何，再统一写入，最后只恢复一次阅读锚点。 */
export class MarkdownLayoutBatch {
  private pending = new Map<object, { measure(): unknown; write(measurement: unknown): void }>();
  private frame: number | null = null;
  constructor(
    private readonly captureAnchor: () => unknown,
    private readonly restoreAnchor: (anchor: unknown) => void,
  ) {}
  enqueue<T>(owner: object, measure: () => T, write: (measurement: T) => void) {
    this.pending.set(owner, { measure, write: (value) => write(value as T) });
    if (this.frame === null) this.frame = requestAnimationFrame(() => this.flush());
  }
  cancel(owner: object) {
    this.pending.delete(owner);
    if (!this.pending.size && this.frame !== null) {
      cancelAnimationFrame(this.frame);
      this.frame = null;
    }
  }
  private flush() {
    this.frame = null;
    const tasks = [...this.pending.values()];
    this.pending.clear();
    const anchor = this.captureAnchor();
    const measurements = tasks.map((task) => task.measure());
    tasks.forEach((task, index) => task.write(measurements[index]));
    this.restoreAnchor(anchor);
  }
}

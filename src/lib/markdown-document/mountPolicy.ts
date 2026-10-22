import type { DocumentIndexProgress } from './protocol';

export type MarkdownMountMode = 'windowed' | 'progressive-full';
export const DEFAULT_MARKDOWN_MOUNT_LIMITS = Object.freeze({
  bytes: 500_000,
  layoutUnits: 2_000,
  enhancementObjects: 128,
  returnBytes: 400_000,
  returnLayoutUnits: 1_600,
  returnEnhancementObjects: 100,
  idleMilliseconds: 2_000,
});

/** 阈值只改变 DOM 驻留；完整正文、历史和编辑权限均不属于本策略。 */
export class MarkdownMountPolicy {
  private mode: MarkdownMountMode = 'windowed';
  private limited = false;
  private belowSince: number | null = null;
  get windowLimited() { return this.mode === 'windowed' && this.limited; }
  constructor(private readonly limits = DEFAULT_MARKDOWN_MOUNT_LIMITS) {}
  update(progress: DocumentIndexProgress, now: number, interacting: boolean): MarkdownMountMode {
    const high =
      progress.totalBytes > this.limits.bytes ||
      progress.layoutUnits > this.limits.layoutUnits ||
      progress.enhancementObjects > this.limits.enhancementObjects;
    if (high) {
      this.limited = true;
      this.mode = 'windowed';
      this.belowSince = null;
      return this.mode;
    }
    if (progress.state !== 'ready' || progress.metricsComplete === false) {
      this.belowSince = null;
      this.mode = 'windowed';
      return this.mode;
    }
    if (!this.limited) {
      this.mode = 'progressive-full';
      return this.mode;
    }
    const low =
      progress.totalBytes <= this.limits.returnBytes &&
      progress.layoutUnits <= this.limits.returnLayoutUnits &&
      progress.enhancementObjects <= this.limits.returnEnhancementObjects;
    if (!low || interacting) {
      this.belowSince = null;
      return this.mode;
    }
    this.belowSince ??= now;
    if (now - this.belowSince >= this.limits.idleMilliseconds) {
      this.mode = 'progressive-full';
      this.limited = false;
    }
    return this.mode;
  }
}

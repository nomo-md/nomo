import type { DocumentIndexProgress } from './protocol';

export type MarkdownMountMode = 'windowed' | 'progressive-full';
export const DEFAULT_MARKDOWN_MOUNT_LIMITS = Object.freeze({
  bytes: 1_048_576,
  layoutUnits: 6_000,
  returnBytes: 838_860,
  returnLayoutUnits: 4_800,
  idleMilliseconds: 2_000,
});

/** 阈值只改变 DOM 驻留；完整正文、历史和编辑权限均不属于本策略。 */
export class MarkdownMountPolicy {
  private mode: MarkdownMountMode = 'windowed';
  private limited = false;
  private belowSince: number | null = null;
  private pressure = false;
  private incomplete = false;
  get windowLimited() { return this.mode === 'windowed' && (this.limited || this.pressure); }
  get restriction() { return this.pressure ? 'resource-pressure' : this.limited ? 'document-size' : this.mode === 'windowed' ? this.incomplete ? 'structure-incomplete' : 'index-pending' : null; }
  constructor(private readonly limits = DEFAULT_MARKDOWN_MOUNT_LIMITS) {}
  update(progress: DocumentIndexProgress, now: number, interacting: boolean, pressure = false): MarkdownMountMode {
    this.pressure = pressure;
    this.incomplete = progress.state === 'ready' && progress.metricsComplete === false;
    const high =
      progress.totalBytes > this.limits.bytes ||
      progress.layoutUnits > this.limits.layoutUnits;
    if (high) {
      this.limited = true;
      this.mode = 'windowed';
      this.belowSince = null;
      return this.mode;
    }
    if (pressure || progress.state !== 'ready' || progress.metricsComplete === false) {
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
      progress.layoutUnits <= this.limits.returnLayoutUnits;
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

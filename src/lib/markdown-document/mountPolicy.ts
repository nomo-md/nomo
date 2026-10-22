import type { DocumentIndexProgress } from './protocol';
import { DEFAULT_MARKDOWN_MOUNT_LIMITS, type MarkdownMountLimits } from './windowingLimits';
export { DEFAULT_MARKDOWN_MOUNT_LIMITS } from './windowingLimits';

export type MarkdownMountMode = 'windowed' | 'progressive-full';

/** 阈值只改变 DOM 驻留；完整正文、历史和编辑权限均不属于本策略。 */
export class MarkdownMountPolicy {
  private mode: MarkdownMountMode = 'windowed';
  private limited = false;
  private belowSince: number | null = null;
  private pressure = false;
  private incomplete = false;
  get windowLimited() { return this.mode === 'windowed' && (this.limited || this.pressure); }
  get restriction() { return this.pressure ? 'resource-pressure' : this.limited ? 'document-size' : this.mode === 'windowed' ? this.incomplete ? 'structure-incomplete' : 'index-pending' : null; }
  constructor(private limits: MarkdownMountLimits = DEFAULT_MARKDOWN_MOUNT_LIMITS) {}
  get recoveryAt() { return this.belowSince === null ? null : this.belowSince + this.limits.idleMilliseconds; }
  updateLimits(limits: MarkdownMountLimits) {
    if (Object.keys(limits).every(key => limits[key as keyof MarkdownMountLimits] === this.limits[key as keyof MarkdownMountLimits])) return false;
    this.limits = { ...limits };
    this.belowSince = null;
    return true;
  }
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
      this.belowSince = null;
    }
    return this.mode;
  }
}

import type { DocumentIndexProgress } from './protocol';
import { DEFAULT_MARKDOWN_MOUNT_LIMITS, MARKDOWN_RESIDENT_LIMITS, type MarkdownMountLimits } from './windowingLimits';
export { MARKDOWN_RESIDENT_LIMITS } from './windowingLimits';

export interface MarkdownResidentCost { views: number; sourceBytes: number; domNodes: number; imageBytes: number; }
export const UNKNOWN_IMAGE_BYTES = 8 * 1024 * 1024;
const RECOVERY_LOW_WATERMARK = 0.7;
const DOCUMENT_REDUCTION_RATIO = 0.8;
// 连续滚动的事件间隙也保持让步；与宿主的 300 ms 安静期一致，只推迟可选准入。
const OPTIONAL_INTERACTION_QUIET_MS = 300;
export const emptyResidentCost = (): MarkdownResidentCost => ({ views: 0, sourceBytes: 0, domNodes: 0, imageBytes: 0 });
const keys = Object.keys(MARKDOWN_RESIDENT_LIMITS) as (keyof MarkdownResidentCost)[];
function add(target: MarkdownResidentCost, value: MarkdownResidentCost, sign = 1) {
  for (const key of keys) target[key] += sign * value[key];
}
type DocumentEntry = {
  progress?: DocumentIndexProgress; pressured: boolean; eligible: boolean;
  baseline?: { bytes: number; units: number }; interaction: number; lowSince?: number;
  timer?: ReturnType<typeof setTimeout>;
};
type HostEntry = { documentId: string; activity: number; changed(): void };
export interface ResidentToken { readonly id: number; }
type Entry = { host: object; cost: MarkdownResidentCost; required: boolean; reserved: boolean };
export type BudgetReservation = { kind: 'admitted'; token: ResidentToken } | { kind: 'waiting' | 'denied' };

/** 一个 WebView 内共享；只保存活动登记和标量，不保存正文、DOM 或编辑器对象。 */
export class MarkdownResidentBudget {
  private hosts = new Map<object, HostEntry>();
  private documents = new Map<string, DocumentEntry>();
  private entries = new Map<ResidentToken, Entry>();
  private actual = emptyResidentCost();
  private reserved = emptyResidentCost();
  private unknown?: ResidentToken;
  private waiting = new Set<object>();
  private idleWaiting = new Set<object>();
  private optionalAfter = -Infinity;
  private optionalTimer?: ReturnType<typeof setTimeout>;
  private notifications = new Set<object>();
  private queued = false;
  private sequence = 0;
  private reasons = new Set<keyof MarkdownResidentCost>();
  private mountLimits: MarkdownMountLimits = DEFAULT_MARKDOWN_MOUNT_LIMITS;
  private currentLimits: Readonly<MarkdownResidentCost>;
  constructor(limits: Readonly<MarkdownResidentCost> = MARKDOWN_RESIDENT_LIMITS, private readonly now = () => performance.now()) {
    this.currentLimits = { ...limits };
  }
  get limits() { return this.currentLimits; }

  /** 原位更新以保留预占 token、成本和跨栏压力状态。 */
  updateLimits(limits: Readonly<MarkdownResidentCost>, mountLimits: MarkdownMountLimits) {
    const capacityChanged = keys.some(key => limits[key] !== this.limits[key]);
    const admissionChanged = mountLimits.bytes !== this.mountLimits.bytes || mountLimits.layoutUnits !== this.mountLimits.layoutUnits;
    if (!capacityChanged && !admissionChanged) return;
    const increased = keys.some(key => limits[key] > this.limits[key]) ||
      mountLimits.bytes > this.mountLimits.bytes || mountLimits.layoutUnits > this.mountLimits.layoutUnits;
    this.currentLimits = { ...limits };
    this.mountLimits = { ...mountLimits };
    for (const doc of this.documents.values()) {
      doc.lowSince = undefined;
      if (increased && doc.pressured) doc.eligible = true;
    }
    this.checkPressure();
    this.recoveries();
    for (const host of this.hosts.keys()) this.notify(host);
  }

  register(host: object, documentId: string, changed: () => void) {
    if (this.hosts.has(host)) throw new Error('驻留宿主重复注册。');
    const pressured = [...this.documents.values()].some(doc => doc.pressured);
    this.hosts.set(host, { documentId, activity: 0, changed });
    if (!this.documents.has(documentId)) this.documents.set(documentId, { pressured, eligible: false, interaction: -Infinity });
    if (pressured) this.notify(host);
  }
  pressured(host: object) { return this.doc(host)?.pressured ?? false; }
  private doc(host: object) {
    const entry = this.hosts.get(host);
    return entry && this.documents.get(entry.documentId);
  }
  updateDocument(host: object, progress: DocumentIndexProgress) {
    const doc = this.doc(host);
    if (!doc) return;
    doc.progress = progress;
    if (doc.pressured && !doc.baseline) doc.baseline = { bytes: progress.totalBytes, units: progress.layoutUnits };
    const base = doc.baseline;
    if (base && ((base.bytes > 0 && progress.totalBytes <= base.bytes * DOCUMENT_REDUCTION_RATIO) ||
      (base.units > 0 && progress.metricsComplete !== false && progress.state === 'ready' && progress.layoutUnits <= base.units * DOCUMENT_REDUCTION_RATIO))) doc.eligible = true;
    this.checkRecovery(doc);
  }
  interacting(host: object) {
    const doc = this.doc(host);
    if (!doc) return;
    doc.interaction = this.now();
    this.optionalAfter = doc.interaction + OPTIONAL_INTERACTION_QUIET_MS;
    this.wakeOptional();
    this.checkRecovery(doc);
  }
  holdInteraction(host: object, kind: 'composition' | 'pointer', active: boolean) {
    const entry = this.hosts.get(host);
    if (!entry) return;
    const mask = kind === 'composition' ? 1 : 2;
    entry.activity = active ? entry.activity | mask : entry.activity & ~mask;
    this.interacting(host);
  }
  reserve(host: object, cost: MarkdownResidentCost, required: boolean): BudgetReservation {
    if (!this.hosts.has(host)) return { kind: 'denied' };
    if (!required) {
      if (this.pressured(host)) return { kind: 'denied' };
      if (this.now() < this.optionalAfter || [...this.hosts.values()].some(host => host.activity)) {
        this.idleWaiting.add(host); this.wakeOptional(); return { kind: 'waiting' };
      }
      // 等待另一宿主的未知组核算不等于资源压力，也不忙轮询。
      if (this.unknown) { this.waiting.add(host); return { kind: 'waiting' }; }
      const exceeded = keys.filter(key => this.actual[key] + this.reserved[key] + cost[key] > this.limits[key]);
      if (exceeded.length) { this.pressure(exceeded); return { kind: 'denied' }; }
    }
    const token = Object.freeze({ id: ++this.sequence });
    this.entries.set(token, { host, cost: { ...cost }, required, reserved: true });
    add(this.reserved, cost);
    if (!required) this.unknown = token;
    this.checkPressure(); this.recoveries();
    return { kind: 'admitted', token };
  }
  commit(token: ResidentToken, cost: MarkdownResidentCost) {
    const entry = this.entries.get(token);
    if (!entry) return false;
    add(entry.reserved ? this.reserved : this.actual, entry.cost, -1);
    entry.cost = { ...cost }; entry.reserved = false;
    add(this.actual, cost);
    this.finishUnknown(token);
    this.checkPressure(); this.recoveries();
    return true;
  }
  update(token: ResidentToken, cost: MarkdownResidentCost) {
    const entry = this.entries.get(token);
    if (!entry || keys.every(key => cost[key] === entry.cost[key])) return;
    this.commit(token, cost);
  }
  protect(token: ResidentToken, required: boolean) {
    const entry = this.entries.get(token);
    if (entry) entry.required = required;
  }
  release(token: ResidentToken) {
    const entry = this.entries.get(token);
    if (!entry) return;
    add(entry.reserved ? this.reserved : this.actual, entry.cost, -1);
    this.entries.delete(token);
    this.finishUnknown(token);
    // 自身回收不解除压力锁存。
    this.recoveries();
  }
  unregister(host: object) {
    const entry = this.hosts.get(host);
    if (!entry) return;
    for (const [token, value] of this.entries) if (value.host === host) this.release(token);
    this.hosts.delete(host); this.waiting.delete(host); this.idleWaiting.delete(host); this.notifications.delete(host);
    this.wakeOptional();
    if (![...this.hosts.values()].some(other => other.documentId === entry.documentId)) {
      const doc = this.documents.get(entry.documentId);
      clearTimeout(doc?.timer); this.documents.delete(entry.documentId);
    }
    for (const doc of this.documents.values()) doc.eligible = true;
    this.recoveries();
    if (![...this.documents.values()].some(doc => doc.pressured)) this.reasons.clear();
  }
  private finishUnknown(token: ResidentToken) {
    if (this.unknown !== token) return;
    this.unknown = undefined;
    for (const host of this.waiting) this.notify(host);
    this.waiting.clear();
  }
  private wakeOptional() {
    clearTimeout(this.optionalTimer); this.optionalTimer = undefined;
    if (!this.idleWaiting.size || [...this.hosts.values()].some(host => host.activity)) return;
    const remaining = this.optionalAfter - this.now();
    if (remaining > 0) { this.optionalTimer = setTimeout(() => this.wakeOptional(), remaining); return; }
    for (const host of this.idleWaiting) this.notify(host);
    this.idleWaiting.clear();
  }
  private checkPressure() {
    const exceeded = keys.filter(key => this.actual[key] + this.reserved[key] > this.limits[key]);
    if (exceeded.length) this.pressure(exceeded);
  }
  private pressure(reasons: (keyof MarkdownResidentCost)[]) {
    clearTimeout(this.optionalTimer); this.optionalTimer = undefined; this.idleWaiting.clear();
    reasons.forEach(reason => this.reasons.add(reason));
    for (const doc of this.documents.values()) {
      if (doc.pressured) continue;
      doc.pressured = true; doc.eligible = false; doc.lowSince = undefined;
      if (doc.progress) doc.baseline = { bytes: doc.progress.totalBytes, units: doc.progress.layoutUnits };
    }
    for (const host of this.hosts.keys()) this.notify(host);
    this.recoveries();
  }
  private recoveries() { for (const doc of this.documents.values()) this.checkRecovery(doc); }
  private checkRecovery(doc: DocumentEntry) {
    clearTimeout(doc.timer); doc.timer = undefined;
    if (!doc.pressured) return;
    const progress = doc.progress;
    const low = keys.every(key => this.actual[key] + this.reserved[key] < this.limits[key] * RECOVERY_LOW_WATERMARK);
    const active = [...this.hosts.values()].some(host => host.activity && this.documents.get(host.documentId) === doc);
    if (!doc.eligible || !low || active || !progress || progress.state !== 'ready' || progress.metricsComplete === false ||
      progress.totalBytes > this.mountLimits.bytes || progress.layoutUnits > this.mountLimits.layoutUnits) {
      doc.lowSince = undefined; return;
    }
    const now = this.now();
    doc.lowSince ??= now;
    const remaining = Math.max(doc.lowSince, doc.interaction) + this.mountLimits.idleMilliseconds - now;
    if (remaining > 0) { doc.timer = setTimeout(() => this.checkRecovery(doc), remaining); return; }
    doc.pressured = false; doc.eligible = false; doc.baseline = undefined; doc.lowSince = undefined;
    for (const [host, entry] of this.hosts) if (this.documents.get(entry.documentId) === doc) this.notify(host);
    if (![...this.documents.values()].some(item => item.pressured)) this.reasons.clear();
  }
  private notify(host: object) {
    this.notifications.add(host);
    if (this.queued) return;
    this.queued = true;
    queueMicrotask(() => {
      this.queued = false;
      const hosts = [...this.notifications]; this.notifications.clear();
      for (const owner of hosts) this.hosts.get(owner)?.changed();
    });
  }
  get diagnostics() {
    const protectedCost = emptyResidentCost();
    for (const entry of this.entries.values()) if (entry.required) add(protectedCost, entry.cost);
    const protectedExcess = emptyResidentCost();
    for (const key of keys) protectedExcess[key] = Math.max(0, protectedCost[key] - this.limits[key]);
    return { hosts: this.hosts.size, documents: this.documents.size, entries: this.entries.size,
      limits: { ...this.limits }, actual: { ...this.actual }, reserved: { ...this.reserved }, protectedExcess,
      pressuredDocuments: [...this.documents.values()].filter(doc => doc.pressured).length,
      reasons: [...this.reasons], unknownPreparation: !!this.unknown };
  }
}

const budgets = new WeakMap<Document, MarkdownResidentBudget>();
export function markdownResidentBudget(document: Document) {
  let budget = budgets.get(document);
  if (!budget) { budget = new MarkdownResidentBudget(); budgets.set(document, budget); }
  return budget;
}

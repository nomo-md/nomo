import { MarkdownTaskPriority, markdownWorkScheduler } from './scheduler';
import { UNKNOWN_IMAGE_BYTES, type MarkdownResidentBudget, type MarkdownResidentCost, type ResidentToken } from './residentBudget';

type Tracked = { root: HTMLElement; token: ResidentToken; cost: MarkdownResidentCost; nodes: Set<Node>; images: Map<Element, number> };

/** 每个活动宿主一个 Observer；仅走变化子树，不在滚动或输入时重扫全文。 */
export class MarkdownResidentTracker {
  private roots = new Map<HTMLElement, Tracked>();
  private membership = new WeakMap<Node, Tracked>();
  private errors = new WeakMap<HTMLImageElement, string>();
  private observer?: MutationObserver;
  private records: MutationRecord[] = [];
  private dirty = new Set<Tracked>();
  private quiesced = false;
  constructor(private readonly budget: MarkdownResidentBudget) {}
  track(root: HTMLElement, token: ResidentToken, sourceBytes: number, views: number) {
    this.quiesced = false;
    const tracked: Tracked = { root, token, cost: { sourceBytes, views, domNodes: 0, imageBytes: 0 }, nodes: new Set(), images: new Map() };
    this.roots.set(root, tracked);
    this.walk(root, node => this.move(node, tracked));
    this.budget.commit(token, tracked.cost);
    this.dirty.delete(tracked);
    this.observer ??= new MutationObserver(records => {
      this.records.push(...records);
      markdownWorkScheduler.enqueue(this, 'resources', MarkdownTaskPriority.NearViewport, () => this.flush());
    });
    this.observe(root);
    root.addEventListener('load', this.onImage, true);
    root.addEventListener('error', this.onImage, true);
  }
  update(root: HTMLElement, sourceBytes: number, required: boolean) {
    const tracked = this.roots.get(root);
    if (!tracked) return;
    if (tracked.cost.sourceBytes !== sourceBytes) {
      tracked.cost.sourceBytes = sourceBytes;
      this.budget.update(tracked.token, tracked.cost);
    }
    this.budget.protect(tracked.token, required);
  }
  release(root: HTMLElement) {
    const tracked = this.roots.get(root);
    if (!tracked) return;
    if (!this.quiesced) this.flush();
    this.roots.delete(root); this.dirty.delete(tracked);
    for (const node of tracked.nodes) this.membership.delete(node);
    tracked.nodes.clear(); tracked.images.clear();
    root.removeEventListener('load', this.onImage, true);
    root.removeEventListener('error', this.onImage, true);
    this.budget.release(tracked.token);
    // MutationObserver 没有 unobserve；重绑剩余根，不遍历其后代。
    this.observer?.disconnect();
    if (!this.quiesced) for (const remaining of this.roots.keys()) this.observe(remaining);
  }
  quiesce() {
    this.quiesced = true; this.observer?.disconnect(); this.records = []; this.dirty.clear();
    markdownWorkScheduler.cancel(this);
  }
  dispose() {
    for (const root of [...this.roots.keys()]) this.release(root);
    this.observer?.disconnect(); this.observer = undefined;
    this.records = []; this.dirty.clear();
    markdownWorkScheduler.cancel(this);
  }
  private observe(root: HTMLElement) {
    this.observer?.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['src', 'srcset', 'sizes', 'class'] });
  }
  private walk(node: Node, visit: (node: Node) => void) {
    const stack = [node];
    while (stack.length) {
      const current = stack.pop()!;
      if (current.nodeType === 1 || current.nodeType === 3) visit(current);
      for (let child = current.lastChild; child; child = child.previousSibling) stack.push(child);
    }
  }
  private owner(node: Node): Tracked | undefined {
    for (let parent: Node | null = node; parent; parent = parent.parentNode) {
      const tracked = this.roots.get(parent as HTMLElement);
      if (tracked) return tracked;
    }
    return undefined;
  }
  private signature(image: HTMLImageElement) { return `${image.getAttribute('src') ?? ''}|${image.getAttribute('srcset') ?? ''}`; }
  private imageBytes(image: HTMLImageElement) {
    const signature = this.signature(image);
    if (signature === '|' || this.errors.get(image) === signature) return 0;
    // 已在离屏准备阶段失败的 IMG 不会再次发送 error，也不能永久保留未知估算。
    if (image.complete) return image.naturalWidth * image.naturalHeight * 4;
    return UNKNOWN_IMAGE_BYTES;
  }
  private image(node: Node): node is HTMLImageElement { return node.nodeType === 1 && (node as Element).tagName === 'IMG'; }
  private imageResource(node: Node): node is Element {
    return this.image(node) || (node.nodeType === 1 && (node as Element).classList.contains('image-node'));
  }
  private resourceBytes(node: Element) {
    if (this.image(node)) return this.imageBytes(node);
    // resolve 尚未返回时还没有 IMG；此时同样占用未知图片预留。
    return node.classList.contains('is-loading') && !node.querySelector('img') ? UNKNOWN_IMAGE_BYTES : 0;
  }
  private move(node: Node, next: Tracked | undefined) {
    const old = this.membership.get(node);
    if (old === next) return;
    if (old) {
      old.nodes.delete(node); old.cost.domNodes--;
      if (old.images.has(node as Element)) { old.cost.imageBytes -= old.images.get(node as Element) ?? 0; old.images.delete(node as Element); }
      this.dirty.add(old);
    }
    if (next) {
      this.membership.set(node, next); next.nodes.add(node); next.cost.domNodes++;
      if (this.imageResource(node)) { const bytes = this.resourceBytes(node); next.images.set(node, bytes); next.cost.imageBytes += bytes; }
      this.dirty.add(next);
    } else this.membership.delete(node);
  }
  private refreshImage(image: Element) {
    const tracked = this.membership.get(image);
    if (!tracked) return;
    const bytes = this.resourceBytes(image);
    tracked.cost.imageBytes += bytes - (tracked.images.get(image) ?? 0);
    tracked.images.set(image, bytes); this.dirty.add(tracked);
  }
  private onImage = (event: Event) => {
    if (!(event.target instanceof Node) || !this.image(event.target)) return;
    if (event.type === 'error') this.errors.set(event.target, this.signature(event.target));
    else this.errors.delete(event.target);
    this.refreshImage(event.target);
    markdownWorkScheduler.enqueue(this, 'resources', MarkdownTaskPriority.NearViewport, () => this.flush());
  };
  flush() {
    this.records.push(...(this.observer?.takeRecords() ?? []));
    const records = this.records; this.records = [];
    for (const record of records) {
      if (record.type === 'attributes') {
        if (this.imageResource(record.target) || this.membership.get(record.target)?.images.has(record.target as Element))
          this.refreshImage(record.target as Element);
        continue;
      }
      for (const node of [...record.removedNodes, ...record.addedNodes]) {
        const owner = this.owner(node);
        this.walk(node, child => this.move(child, owner));
      }
    }
    for (const tracked of this.dirty) if (this.roots.has(tracked.root)) {
      for (const image of tracked.images.keys()) if (!this.image(image)) this.refreshImage(image);
      this.budget.update(tracked.token, tracked.cost);
    }
    this.dirty.clear();
  }
}

import type { Node as PmNode } from 'prosemirror-model';
import { Plugin, PluginKey, TextSelection, type Command, type EditorState, type Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet } from 'prosemirror-view';
import { closeHistory } from 'prosemirror-history';
import type { EditorView } from 'prosemirror-view';
import { parseMarkdownWithSyncAnchors } from '../markdown';

interface Entry { defaultOpen: boolean; override?: boolean }
interface SourceEntry extends Entry { end: number }
interface DetailsState { entries: Map<number, Entry>; decorations: DecorationSet }
export const detailsKey = new PluginKey<DetailsState>('details');
export const detailsHistoryBoundary = 'detailsHistoryBoundary';
export type DetailsAction = { pos: number; open: boolean; collapse?: boolean; reset?: boolean };

/** 文档会话仅缓存固定大小的节点位置/状态，不持有 DOM、回调或另一个编辑器。 */
export class DetailsSession {
  entries = new Map<number, Entry>();
  private sourceEntries: Map<number, SourceEntry> | null = null;
  private pendingSource = false;

  fork(previous: string, source: string): DetailsSession {
    const next = new DetailsSession();
    next.entries = new Map([...this.entries].map(([pos, entry]) => [pos, { ...entry }]));
    if (source !== previous) next.beforeSourceChange(previous, source);
    next.prepare(source);
    return next;
  }

  invalidateSource(): void { if (!this.pendingSource) this.sourceEntries = null; }

  beforeSourceChange(previous: string, source: string): void {
    if (!this.sourceEntries) {
      if (![...this.entries.values()].some(entry => entry.override !== undefined)) return;
      this.sourceEntries = new Map();
      for (const anchor of this.offsets(previous)) {
        const entry = this.entries.get(anchor.pos);
        if (entry?.override !== undefined) this.sourceEntries.set(anchor.offset, { ...entry, end: anchor.end });
      }
    }
    // 只在首次进入源码编辑时建立锚点；后续输入仅映射每个折叠块的固定大小记录。
    let prefix = 0;
    while (prefix < previous.length && prefix < source.length && previous[prefix] === source[prefix]) prefix++;
    let oldEnd = previous.length, newEnd = source.length;
    // 后缀独立匹配：同名块插在前面时，共同前缀可能只是新块的标题。
    while (oldEnd > 0 && newEnd > 0 && previous[oldEnd - 1] === source[newEnd - 1]) { oldEnd--; newEnd--; }
    const delta = source.length - previous.length;
    const mapped = new Map<number, SourceEntry>();
    for (const [offset, entry] of this.sourceEntries) {
      if (oldEnd < prefix) {
        const inPrefix = entry.end <= prefix;
        const inSuffix = offset >= oldEnd;
        // 完整块在前后两份中都匹配，身份有歧义时恢复默认，不能串用临时状态。
        if (inPrefix && inSuffix) continue;
        if (inPrefix) mapped.set(offset, entry);
        else if (inSuffix) mapped.set(offset + delta, { ...entry, end: entry.end + delta });
        else if (offset < oldEnd && entry.end > prefix) mapped.set(offset, { ...entry, end: entry.end + delta });
      } else if (entry.end <= prefix) mapped.set(offset, entry);
      else if (offset < prefix && entry.end >= oldEnd) mapped.set(offset, { ...entry, end: entry.end + delta });
      else if (offset >= oldEnd) mapped.set(offset + delta, { ...entry, end: entry.end + delta });
    }
    this.sourceEntries = mapped;
    this.pendingSource = true;
  }

  prepare(source: string): void {
    if (!this.pendingSource) return;
    this.pendingSource = false;
    const entries = new Map<number, Entry>();
    for (const anchor of this.offsets(source)) {
      const entry = this.sourceEntries?.get(anchor.offset);
      if (entry) entries.set(anchor.pos, { defaultOpen: entry.defaultOpen, override: entry.override });
    }
    this.entries = entries;
  }

  private offsets(text: string): Array<{ pos: number; offset: number; end: number }> {
    const starts = [0];
    for (let i = 0; i < text.length; i++) if (text[i] === '\n') starts.push(i + 1);
    const ranges: Array<{ pos: number; offset: number; end: number }> = [];
    const counts = new Map<number, number>();
    for (const anchor of parseMarkdownWithSyncAnchors(text).anchors) {
      if (anchor.kind !== 'details' || anchor.edge !== 'start') continue;
      const start = starts[anchor.fromLine - 1];
      const lineEnd = text.indexOf('\n', start);
      const line = text.slice(start, lineEnd < 0 ? text.length : lineEnd);
      const opening = line.toLowerCase().indexOf('<details');
      // HTML 批量导入的子节点可能只有共同的粗粒度范围，不能把它当成节点身份。
      if (opening < 0 || !/^(?:[ \t>]|[-+*] |\d+[.)] )*$/.test(line.slice(0, opening))) continue;
      const lastLine = starts[anchor.toLine - 1];
      const lastLineEnd = text.indexOf('\n', lastLine);
      const closingLine = text.slice(lastLine, lastLineEnd < 0 ? text.length : lastLineEnd);
      const closing = [...closingLine.matchAll(/<\/details\s*>/ig)].at(-1);
      if (!closing) continue;
      const offset = start + opening;
      ranges.push({ pos: anchor.pos, offset, end: lastLine + closing.index! + closing[0].length });
      counts.set(offset, (counts.get(offset) ?? 0) + 1);
    }
    return ranges.filter(range => counts.get(range.offset) === 1);
  }

  clear(): void { this.entries.clear(); this.sourceEntries = null; this.pendingSource = false; }
}

export function detailsIsOpen(state: EditorState, pos: number): boolean {
  const entry = detailsKey.getState(state)?.entries.get(pos);
  return entry?.override ?? entry?.defaultOpen ?? state.doc.nodeAt(pos)?.attrs.defaultOpen ?? false;
}

export function detailsParent(state: EditorState): { node: PmNode; pos: number; depth: number } | null {
  const { $from } = state.selection;
  if ($from.nodeAfter?.type.name === 'details') return { node: $from.nodeAfter, pos: $from.pos, depth: $from.depth + 1 };
  for (let depth = $from.depth; depth > 0; depth--) {
    if ($from.node(depth).type.name === 'details') return { node: $from.node(depth), pos: $from.before(depth), depth };
  }
  return null;
}

function decoration(doc: PmNode, pos: number, entry: Entry): Decoration {
  return Decoration.node(pos, pos + doc.nodeAt(pos)!.nodeSize, {
    'data-details-open': String(entry.override ?? entry.defaultOpen),
  }, { open: String(entry.override ?? entry.defaultOpen) });
}

export function detailsPlugin(session: DetailsSession, enterBody: Command): Plugin<DetailsState> {
  return new Plugin<DetailsState>({
    key: detailsKey,
    appendTransaction(transactions, _old, state) {
      // 结构操作只有一个正文事务；随后关闭历史分组，避免首次输入把整个新块一并撤销。
      return transactions.some(tr => tr.getMeta(detailsHistoryBoundary))
        ? closeHistory(state.tr).setMeta('addToHistory', false) : null;
    },
    state: {
      init(_, state) {
        const entries = new Map<number, Entry>();
        state.doc.descendants((node, pos) => {
          if (node.type.name !== 'details') return;
          const previous = session.entries.get(pos);
          entries.set(pos, { defaultOpen: node.attrs.defaultOpen,
            override: previous?.defaultOpen === node.attrs.defaultOpen ? previous?.override : undefined });
        });
        session.entries = entries;
        return { entries, decorations: DecorationSet.create(state.doc, [...entries].map(([pos, entry]) => decoration(state.doc, pos, entry))) };
      },
      apply(tr, value, _old, state) {
        if (tr.docChanged || tr.getMeta(detailsKey) || tr.selectionSet) session.invalidateSource();
        const entries = new Map<number, Entry>();
        let decorations = value.decorations.map(tr.mapping, tr.doc);
        for (const [oldPos, entry] of value.entries) {
          const mapped = tr.mapping.mapResult(oldPos, 1);
          const node = tr.doc.nodeAt(mapped.pos);
          if (mapped.deleted || node?.type.name !== 'details') continue;
          entries.set(mapped.pos, node.attrs.defaultOpen === entry.defaultOpen ? entry : { defaultOpen: node.attrs.defaultOpen });
        }
        if (tr.docChanged) {
          const collect = (node: PmNode, pos: number) => {
            if (node.type.name === 'details' && !entries.has(pos)) entries.set(pos, { defaultOpen: node.attrs.defaultOpen });
          };
          tr.mapping.maps.forEach((map, index) => map.forEach((_a, _b, from, to) => {
            const mapping = tr.mapping.slice(index + 1);
            from = Math.max(0, Math.min(tr.doc.content.size, mapping.map(from, -1)));
            to = Math.max(from, Math.min(tr.doc.content.size, mapping.map(to, 1)));
            tr.doc.nodesBetween(from, to, collect);
            const $pos = tr.doc.resolve(from);
            for (let d = 1; d <= $pos.depth; d++) collect($pos.node(d), $pos.before(d));
          }));
        }
        const action = tr.getMeta(detailsKey) as DetailsAction | undefined;
        if (action && entries.has(action.pos)) entries.set(action.pos, { ...entries.get(action.pos)!, override: action.reset ? undefined : action.open });
        // 搜索、历史恢复及程序定位可以进入隐藏正文；显示状态不写入文档历史。
        if (tr.selectionSet && !action?.collapse) {
          for (const $pos of [state.selection.$anchor, state.selection.$head]) {
            for (let d = 1; d <= $pos.depth; d++) {
              if ($pos.node(d).type.name !== 'details_body') continue;
              const pos = $pos.before(d - 1);
              const entry = entries.get(pos);
              if (entry && !(entry.override ?? entry.defaultOpen)) entries.set(pos, { ...entry, override: true });
            }
          }
        }
        for (const [pos, entry] of entries) {
          const existing = decorations.find(pos, pos + 1).filter(d => d.from === pos);
          const next = decoration(tr.doc, pos, entry);
          const oldOpen = existing[0]?.spec.open;
          if (oldOpen !== String(entry.override ?? entry.defaultOpen)) decorations = decorations.remove(existing).add(tr.doc, [next]);
        }
        session.entries = entries;
        return { entries, decorations };
      },
    },
    props: {
      decorations: state => detailsKey.getState(state)?.decorations ?? null,
      handleKeyDown(view, event) {
        if (event.isComposing || view.composing) return false;
        return navigateDetails(view, event, enterBody);
      },
    },
  });
}

/** 箭头只遍历可见正文；显式定位隐藏内容由插件状态展开。 */
function navigateDetails(view: EditorView, event: KeyboardEvent, enterBody: Command): boolean {
  if (!['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight'].includes(event.key) || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey || !view.state.selection.empty) return false;
  const { state } = view;
  const { $from } = state.selection;
  const forward = event.key === 'ArrowDown' || event.key === 'ArrowRight';
  if (!$from.parent.isTextblock) return false;
  const atEdge = event.key === 'ArrowDown' || event.key === 'ArrowUp'
    ? view.endOfTextblock(forward ? 'down' : 'up')
    : $from.parentOffset === (forward ? $from.parent.content.size : 0);
  if (!atEdge) return false;
  const parent = detailsParent(state);
  if (forward && $from.parent.type.name === 'details_summary' && parent && detailsIsOpen(state, parent.pos)) {
    return enterBody(state, view.dispatch, view);
  }
  if (forward && $from.parent.type.name === 'details_summary' && parent && !detailsIsOpen(state, parent.pos)) {
    const selection = TextSelection.near(state.doc.resolve(parent.pos + parent.node.nodeSize), 1);
    view.dispatch(state.tr.setSelection(selection).scrollIntoView());
    return true;
  }
  // 从容器后面向左/上进入折叠块时停在标题，不能触发正文的自动展开。
  if (!forward) {
    for (let d = $from.depth; d > 0; d--) {
      if (d === $from.depth && event.key === 'ArrowLeft' && $from.parentOffset !== 0) break;
      if (d < $from.depth && $from.index(d) !== 0) break;
      const boundary = $from.before(d);
      const previous = state.doc.resolve(boundary).nodeBefore;
      if (previous?.type.name === 'details') {
        const pos = boundary - previous.nodeSize;
        if (!detailsIsOpen(state, pos)) {
          view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, pos + 2 + previous.firstChild!.content.size)).scrollIntoView());
          return true;
        }
      }
    }
  }
  return false;
}

export function setDetailsTemporaryOpen(tr: Transaction, pos: number, open: boolean): Transaction {
  return tr.setMeta(detailsKey, { pos, open, collapse: !open } satisfies DetailsAction).setMeta('addToHistory', false);
}

import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { Plugin, PluginKey } from 'prosemirror-state';
import { Decoration, DecorationSet } from 'prosemirror-view';
import { escapeHtml, sanitizeHtml } from '../utils/html';
import { isInlineSourceText } from '../InlineSourceCodec';

const tableHtmlKey = new PluginKey('tableHtml');

export function tableHtmlBlockPlugin(): Plugin {
  return new Plugin({
    key: tableHtmlKey,
    state: {
      init(_, state) {
        return buildTableHtmlDecorations(state.doc);
      },
      apply(tr, oldSet, _oldState, newState) {
        if (!tr.docChanged) return oldSet;
        return buildTableHtmlDecorations(newState.doc);
      },
    },
    props: {
      decorations(state) {
        return this.getState(state);
      },
    },
  });
}

function buildTableHtmlDecorations(doc: ProseMirrorNode): DecorationSet {
  const decorations: Decoration[] = [];
  const blocks: Array<{ node: ProseMirrorNode; pos: number }> = [];
  doc.forEach((node, offset) => {
    blocks.push({ node, pos: offset });
  });
  let i = 0;
  while (i < blocks.length) {
    const tableResult = tryParseTable(blocks, i);
    if (tableResult) {
      const { from, to, html } = tableResult;
      const widget = document.createElement('div');
      widget.className = 'table-widget';
      widget.setAttribute('contenteditable', 'false');
      widget.innerHTML = html;
      decorations.push(Decoration.widget(from, widget, { side: 0 }));
      decorations.push(
        Decoration.inline(
          from,
          to,
          { style: 'display: none' },
          { inclusiveStart: false, inclusiveEnd: false },
        ),
      );
      i = tableResult.nextIndex;
      continue;
    }
    const htmlResult = tryParseHtmlBlock(blocks[i]);
    if (htmlResult) {
      const { pos, node, safeHtml } = htmlResult;
      const widget = document.createElement('span');
      widget.className = 'html-widget';
      widget.setAttribute('contenteditable', 'false');
      renderHtmlWidget(widget, safeHtml);
      decorations.push(Decoration.widget(pos, widget, { side: 0 }));
      decorations.push(
        Decoration.inline(
          pos + 1,
          pos + node.nodeSize - 1,
          { style: 'display: none' },
          { inclusiveStart: false, inclusiveEnd: false },
        ),
      );
    }
    i += 1;
  }
  return decorations.length > 0 ? DecorationSet.create(doc, decorations) : DecorationSet.empty;
}

function tryParseTable(
  blocks: Array<{ node: ProseMirrorNode; pos: number }>,
  startIndex: number,
): { from: number; to: number; html: string; nextIndex: number } | null {
  const rows: string[][] = [];
  let hasSeparator = false;
  let i = startIndex;
  while (i < blocks.length) {
    const block = blocks[i];
    if (block.node.type.name !== 'paragraph') break;
    const text = block.node.textContent.trim();
    if (!text.startsWith('|') || !text.endsWith('|')) break;
    const parts = text
      .slice(1, -1)
      .split('|')
      .map((cell) => cell.trim());
    let cols = 0;
    while (cols < parts.length && parts[cols] !== '') {
      cols++;
    }
    if (cols < 2) break;
    const stride = cols + 1;
    let pos = 0;
    let blockHasValidRows = false;
    while (pos + cols <= parts.length) {
      const cells = parts.slice(pos, pos + cols);
      if (cells.some((cell) => cell === '')) break;
      if (cells.every((cell) => /^:?-{3,}:?$/.test(cell))) {
        hasSeparator = true;
        blockHasValidRows = true;
        pos += stride;
        continue;
      }
      rows.push(cells);
      blockHasValidRows = true;
      pos += stride;
    }
    if (!blockHasValidRows) break;
    i += 1;
  }
  if (rows.length === 0 || i === startIndex) return null;
  let html = '<div class="table-scroll"><table>';
  if (rows.length > 0 && hasSeparator) {
    html +=
      '<thead><tr>' +
      rows[0].map((cell) => `<th>${escapeHtml(cell)}</th>`).join('') +
      '</tr></thead>';
    html +=
      '<tbody>' +
      rows
        .slice(1)
        .map((row) => '<tr>' + row.map((cell) => `<td>${escapeHtml(cell)}</td>`).join('') + '</tr>')
        .join('') +
      '</tbody>';
  } else {
    html +=
      '<tbody>' +
      rows
        .map((row) => '<tr>' + row.map((cell) => `<td>${escapeHtml(cell)}</td>`).join('') + '</tr>')
        .join('') +
      '</tbody>';
  }
  html += '</table></div>';
  const from = blocks[startIndex].pos;
  const lastBlock = blocks[i - 1];
  const to = lastBlock.pos + lastBlock.node.nodeSize;
  return { from, to, html, nextIndex: i };
}

function tryParseHtmlBlock(block: {
  node: ProseMirrorNode;
  pos: number;
}): { pos: number; node: ProseMirrorNode; safeHtml: string } | null {
  // 代码块和 html_block 已有各自的渲染职责，不能再生成 HTML widget
  if (block.node.type.spec.code || block.node.type.name === 'html_block') return null;
  // 六类内联 HTML 定界符由原文编辑插件处理，不能再次整段替换成不可编辑 widget。
  if (/^\s*<\/?(?:strong|b|em|i|code|s|del|strike|u|mark)(?:\s|>)/i.test(block.node.textContent) &&
      Array.from({ length: block.node.childCount }, (_, i) => block.node.child(i)).some(isInlineSourceText)) return null;
  const text = block.node.textContent.trim();
  if (!/^<(\w+)[^>]*>/.test(text)) return null;
  const safeHtml = sanitizeHtml(text);
  if (!safeHtml) return null;
  return { pos: block.pos, node: block.node, safeHtml };
}

function renderHtmlWidget(widget: HTMLElement, safeHtml: string): void {
  const template = document.createElement('template');
  template.innerHTML = safeHtml.trim();
  const root = getSingleRootElement(template.content);

  if (root?.tagName.toLowerCase() === 'p') {
    const align = root.getAttribute('align')?.toLowerCase();
    if (align === 'left' || align === 'center' || align === 'right') {
      widget.classList.add('html-widget-aligned-paragraph');
      widget.style.display = 'block';
      widget.style.textAlign = align;
      widget.style.whiteSpace = 'normal';
    }
    widget.innerHTML = root.innerHTML;
    return;
  }

  widget.innerHTML = safeHtml;
}

function getSingleRootElement(fragment: DocumentFragment): HTMLElement | null {
  const childNodes = Array.from(fragment.childNodes).filter(
    (node) => node.nodeType !== Node.TEXT_NODE || node.textContent?.trim(),
  );
  return childNodes.length === 1 && childNodes[0].nodeType === Node.ELEMENT_NODE
    ? (childNodes[0] as HTMLElement)
    : null;
}

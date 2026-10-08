import { normalizeHeadingTitle, type OutlineItem } from '../outline/outlineService';
import { escapeInlineSourceLiteral } from '../editor-core/InlineSourceCodec';

import { TOC_START_MARKER, TOC_END_MARKER } from './tocMarkers';
export { TOC_START_MARKER, TOC_END_MARKER } from './tocMarkers';

export interface TocBlockRange {
  start: number;
  end: number;
  contentStart: number;
  contentEnd: number;
}

interface MarkdownLine {
  text: string;
  start: number;
  contentEnd: number;
}

export function createTocBlock(markdown: string): string {
  const content = createTocList(markdown);
  return `${TOC_START_MARKER}\n${content}${content ? '\n' : ''}${TOC_END_MARKER}`;
}

export function updateTocBlocks(markdown: string): string {
  if (!markdown.includes(TOC_START_MARKER) || !markdown.includes(TOC_END_MARKER)) {
    return markdown;
  }

  const ranges = extractTocRanges(markdown);
  if (ranges.length === 0) {
    return markdown;
  }

  const content = createTocList(markdown);
  const replacement = `${TOC_START_MARKER}\n${content}${content ? '\n' : ''}${TOC_END_MARKER}`;
  let updated = markdown;
  for (let index = ranges.length - 1; index >= 0; index--) {
    const range = ranges[index];
    updated = `${updated.slice(0, range.start)}${replacement}${updated.slice(range.end)}`;
  }
  return updated;
}

export function hasTocBlock(markdown: string): boolean {
  return extractTocRanges(markdown).length > 0;
}

export function removeTocBlocks(markdown: string): string {
  const ranges = extractTocRanges(markdown);
  let updated = markdown;
  for (let index = ranges.length - 1; index >= 0; index--) {
    const range = ranges[index];
    updated = `${updated.slice(0, range.start)}${updated.slice(range.end)}`;
  }
  return updated;
}

function extractTocRanges(markdown: string): TocBlockRange[] {
  const ranges: TocBlockRange[] = [];
  let startLine: MarkdownLine | null = null;
  let inFence = false;
  let fenceMarker = '';

  for (const line of readMarkdownLines(markdown)) {
    const fenceMatch = /^(\s*)(`{3,}|~{3,})/.exec(line.text);
    if (fenceMatch) {
      const marker = fenceMatch[2];
      if (!inFence) {
        inFence = true;
        fenceMarker = marker[0];
      } else if (marker[0] === fenceMarker) {
        inFence = false;
        fenceMarker = '';
      }
      continue;
    }

    if (inFence) {
      continue;
    }

    const trimmed = line.text.trim();
    if (trimmed === TOC_START_MARKER) {
      startLine = line;
      continue;
    }

    if (trimmed === TOC_END_MARKER && startLine) {
      ranges.push({
        start: startLine.start,
        end: line.contentEnd,
        contentStart: startLine.contentEnd,
        contentEnd: line.start,
      });
      startLine = null;
    }
  }

  return ranges;
}

export function createTocList(markdown: string): string {
  return extractTocItems(markdown)
    .map((item) => `${'  '.repeat(item.level - 1)}- [${escapeLinkText(item.title)}](#${item.id})`)
    .join('\n');
}

export function extractTocItems(markdown: string): OutlineItem[] {
  const body = stripFrontMatter(removeTocBlocks(markdown));
  const usedIds = new Map<string, number>();
  const items: OutlineItem[] = [];
  let inFence = false;
  let fenceMarker = '';

  body.split(/\r?\n/).forEach((line, index) => {
    const fenceMatch = /^(\s*)(`{3,}|~{3,})/.exec(line);
    if (fenceMatch) {
      const marker = fenceMatch[2];
      if (!inFence) {
        inFence = true;
        fenceMarker = marker[0];
      } else if (marker[0] === fenceMarker) {
        inFence = false;
        fenceMarker = '';
      }
      return;
    }

    if (inFence) {
      return;
    }

    const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (!match) {
      return;
    }

    const rawTitle = match[2].trim();
    const title = normalizeHeadingTitle(rawTitle) || rawTitle;
    const baseId = slugifyHeading(title) || `heading-${index + 1}`;
    const seen = usedIds.get(baseId) ?? 0;
    usedIds.set(baseId, seen + 1);

    items.push({
      id: seen === 0 ? baseId : `${baseId}-${seen + 1}`,
      level: match[1].length as OutlineItem['level'],
      title,
      line: index + 1,
    });
  });

  return items;
}

function stripFrontMatter(markdown: string): string {
  if (!markdown.startsWith('---\n')) {
    return markdown;
  }

  const end = markdown.indexOf('\n---\n', 4);
  if (end === -1) {
    return markdown;
  }

  return markdown.slice(end + 5).replace(/^\s+/, '');
}

function readMarkdownLines(markdown: string): MarkdownLine[] {
  const lines: MarkdownLine[] = [];
  let start = 0;

  while (start < markdown.length) {
    const newlineIndex = markdown.indexOf('\n', start);
    if (newlineIndex === -1) {
      lines.push({
        text: markdown.slice(start),
        start,
        contentEnd: markdown.length,
      });
      break;
    }

    const contentEnd =
      newlineIndex > start && markdown[newlineIndex - 1] === '\r' ? newlineIndex - 1 : newlineIndex;
    lines.push({
      text: markdown.slice(start, contentEnd),
      start,
      contentEnd,
    });
    start = newlineIndex + 1;
  }

  return lines;
}

export function slugifyHeading(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}\s-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
}

function escapeLinkText(text: string): string {
  return escapeInlineSourceLiteral(text);
}

import { calculateWritingStats } from './writingStats';
import { analyzeInlineSourceText } from '../editor-core/InlineSourceCodec';
import { createMarkdownTokenizer, inlineTokenSourceRange } from '../editor-core/markdownTokenizer';

const headingTokenizer = createMarkdownTokenizer({ inlineSource: true });

export interface OutlineItem {
  id: string;
  level: 1 | 2 | 3 | 4 | 5 | 6;
  title: string;
  line: number;
}

export interface DocumentStats {
  chars: number;
  words: number;
  visibleChars: number;
  lines: number;
  headings: number;
  readingMinutes: number;
}


export function extractOutline(markdown: string): OutlineItem[] {
  return extractOutlineOnly(markdown);
}

export function calculateDocumentStats(markdown: string): DocumentStats {
  return calculateWritingStats(markdown);
}

export function analyzeMarkdown(markdown: string): { outline: OutlineItem[]; stats: DocumentStats } {
  return { outline: extractOutlineOnly(markdown), stats: calculateWritingStats(markdown) };
}

function extractOutlineOnly(markdown: string): OutlineItem[] {
  const outline: OutlineItem[] = [];
  const usedIds = new Map<string, number>();
  const lines = markdown.split(/\r\n|\r|\n/);
  // 围栏代码块状态追踪：跳过代码块内的 # 标题匹配
  let inFence = false;
  let fenceMarker = '';

  lines.forEach((line, index) => {
    // 检测围栏代码块的开启与关闭（``` 或 ~~~）
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

    // 跳过代码块内的行，避免误识别为标题
    if (inFence) {
      return;
    }

    const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (match) {
      const rawTitle = match[2].trim();
      const title = normalizeHeadingTitle(rawTitle) || rawTitle;
      const baseId = slugifyHeading(title) || `heading-${index + 1}`;
      const seen = usedIds.get(baseId) ?? 0;
      usedIds.set(baseId, seen + 1);

      outline.push({
        id: seen === 0 ? baseId : `${baseId}-${seen + 1}`,
        level: match[1].length as OutlineItem['level'],
        title,
        line: index + 1,
      });
    }
  });

  return outline;
}

function slugifyHeading(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}\s-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
}

export function normalizeHeadingTitle(title: string): string {
  const analysis = analyzeInlineSourceText(title);
  const tokens = headingTokenizer.parseInline(title, {})[0]?.children ?? [];
  const hidden = new Set<number>();
  const images = new Map<number, { to: number; text: string }>();
  for (const token of tokens) {
    const range = inlineTokenSourceRange(token);
    if (!range) continue;
    // A link-looking string inside a code span is still literal code content.
    if (analysis.spans.some((span) => span.type === 'code'
      && range.from >= span.from && range.to <= span.to)) continue;
    if (token.type === 'link_open' || token.type === 'link_close' || token.type === 'image') {
      for (let pos = range.from; pos < range.to; pos++) hidden.add(pos);
      if (token.type === 'image') {
        images.set(range.from, {
          to: range.to,
          text: normalizeHeadingTitle(token.content || token.attrGet('alt') || ''),
        });
      }
    }
  }
  let plain = '';
  const emittedImages = new Set<number>();
  for (let index = 0; index < analysis.visibleText.length; index++) {
    const original = analysis.visiblePositions[index];
    for (const [from, image] of images) {
      if (!emittedImages.has(from) && original >= from) {
        plain += image.text;
        emittedImages.add(from);
      }
    }
    if (!hidden.has(original)) plain += analysis.visibleText[index];
  }
  return plain.trim();
}

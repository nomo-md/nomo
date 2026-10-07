import { calculateWritingStats } from './writingStats';

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
  let plain = title
    .replace(/!\[([^\]]*)\]\([^)]+\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .replace(/`([^`]+)`/g, '$1');

  let previous = '';
  while (plain !== previous) {
    previous = plain;
    plain = plain
      .replace(
        /(^|[^\p{Letter}\p{Number}])(\*\*|__)(\S(?:[\s\S]*?\S)?)\2(?=$|[^\p{Letter}\p{Number}])/gu,
        '$1$3',
      )
      .replace(
        /(^|[^\p{Letter}\p{Number}])(\*|_)(\S(?:[\s\S]*?\S)?)\2(?=$|[^\p{Letter}\p{Number}])/gu,
        '$1$3',
      )
      .replace(
        /(^|[^\p{Letter}\p{Number}])~~(\S(?:[\s\S]*?\S)?)~~(?=$|[^\p{Letter}\p{Number}])/gu,
        '$1$2',
      );
  }

  return plain.replace(/\\([\\`*_[\]{}()#+\-.!>])/g, '$1').trim();
}

import LineBreaker from 'linebreak';
import { englishHyphenationPoints } from './hyphenation';
import type { BreakPoint, LayoutItem, ParagraphInput, TypographyOptions } from './types';

export const isHan = (text: string) => /\p{Script=Han}/u.test(text);
const isLatin = (text: string) => /[\p{Script=Latin}\p{Number}]/u.test(text);
const opening = /^[（［｛《〈「『【〔〖〘〚“‘]$/u;
const closing = /^[）］｝》〉」』】〕〗〙〛”’，。、；：！？]$/u;
const stopping = /^[，。、；：！？]$/u;

export function softBreakText(before: string, after: string): string {
  const left = [...before.replace(/[\p{Mark}\uFE0E\uFE0F]+$/u, '')].at(-1) ?? '';
  const right = [...after][0] ?? '';
  if (!left || !right || /\s/u.test(left) || /\s/u.test(right)) return '';
  return isHan(left) || isHan(right) ? '' : ' ';
}

export function supportedText(text: string): boolean {
  for (const char of text) {
    if (/\p{Letter}/u.test(char) && !/[\p{Script=Han}\p{Script=Latin}]/u.test(char)) return false;
  }
  return true;
}

/** Editable source spaces retain a caret advance, including at visual line edges. */
export function preserveSourceWhitespace(input: ParagraphInput): ParagraphInput {
  if (!input.items.some((item) => item.sourceWhitespace)) return input;
  const breaks = new Map(input.breaks.map((point) => [point.at, point]));
  const items = input.items.map((item, index) => {
    if (!item.sourceWhitespace) return item;
    // UAX #14 normally breaks after a whole space run. Editable runs also need
    // intermediate opportunities so hundreds of real spaces cannot overflow.
    const following = input.items[index + 1]?.text ?? '';
    if (!breaks.has(index + 1) && !closing.test(following) && following !== '\u00a0')
      breaks.set(index + 1, { at: index + 1, penalty: 50 });
    return { ...item, discardable: false };
  });
  return {
    ...input,
    items,
    editableWhitespace: true,
    breaks: [...breaks.values()].sort((a, b) => a.at - b.at),
  };
}

/** UAX #14 and measured Liang candidates supply opportunities; CJK rules apply prohibitions. */
export function createParagraphInput(
  units: LayoutItem[],
  width: number,
  minLineHeight: number,
  options: TypographyOptions,
  forced = new Set<number>(),
  paragraphFontSize?: number,
): ParagraphInput {
  const text = units.map((unit) => unit.text).join('');
  const hyphens = englishHyphenationPoints(
    units
      .map((unit) => (unit.hyphenatable ? unit.text : '\ufffc'.repeat(unit.text.length)))
      .join(''),
    text,
  );
  const opportunities = new Map<number, boolean>();
  const breaker = new LineBreaker(text);
  let next: { position: number; required: boolean } | null;
  while ((next = breaker.nextBreak())) opportunities.set(next.position, next.required);
  const items: LayoutItem[] = [];
  const breaks: BreakPoint[] = [];
  let offset = 0;
  for (let i = 0; i < units.length; i++) {
    const unit = { ...units[i] };
    let before = i - 1,
      after = i + 1;
    while (before >= 0 && !units[before].text) before--;
    while (after < units.length && !units[after].text) after++;
    const previous = units[before];
    const following = units[after];
    const cjk = isHan(unit.text);
    if (opening.test(unit.text))
      unit.trimStart = Math.min(unit.fontSize * 0.5, unit.leadingSpace ?? Infinity);
    if (closing.test(unit.text)) {
      unit.trimEnd = Math.min(
        unit.fontSize * (options.profile === 'zh-TW' ? 0.25 : 0.5),
        unit.trailingSpace ?? Infinity,
      );
      if (stopping.test(unit.text)) unit.hang = unit.fontSize * 0.5;
    }
    // Adjacent punctuation: reduce inner whitespace without scaling the glyph.
    if (
      previous &&
      closing.test(previous.text) &&
      (opening.test(unit.text) || closing.test(unit.text))
    ) {
      const amount = Math.min(
        Math.min(unit.fontSize, previous.fontSize) * 0.25,
        (previous.trailingSpace ?? Infinity) + (unit.leadingSpace ?? 0),
      );
      const last = [...items].reverse().find((item) => !!item.text);
      if (last) {
        last.width = Math.max(0, last.width - amount);
        last.trimEnd = Math.max(0, (last.trimEnd ?? 0) - amount);
      }
    }
    items.push(unit);
    offset += unit.text.length;
    const forcedBreak = forced.has(unit.to);
    if (following && !forcedBreak) {
      const mixed =
        (cjk && isLatin(following.text)) || (isLatin(unit.text) && isHan(following.text));
      const betweenHan = cjk && isHan(following.text);
      const cjkBoundary =
        (cjk || closing.test(unit.text)) && (isHan(following.text) || opening.test(following.text));
      if (mixed || betweenHan || cjkBoundary) {
        const em = Math.min(unit.fontSize, following.fontSize);
        items.push({
          kind: 'glue',
          from: unit.to,
          to: unit.to,
          text: '',
          width: mixed ? em * 0.125 : 0,
          stretch: em * (mixed ? 0.0625 : 0.1),
          shrink: mixed ? em * 0.0625 : 0,
          comfortStretch: em * (mixed ? 0.04 : 0.05),
          comfortShrink: mixed ? em * 0.04 : 0,
          weight: 1,
          discardable: true,
          fontSize: em,
          ascent: 0,
          descent: 0,
        });
      }
    }
    const pair = `${unit.text}${following?.text ?? ''}`;
    const prohibited =
      opening.test(unit.text) ||
      (following && closing.test(following.text)) ||
      pair === '——' ||
      pair === '……' ||
      unit.text === '\u00a0' ||
      following?.text === '\u00a0';
    const automaticHyphen = hyphens.has(offset) && (unit.breakWidth ?? 0) > 0;
    if (
      forcedBreak ||
      (!prohibited && (opportunities.has(offset) || automaticHyphen)) ||
      i === units.length - 1
    ) {
      const hyphenated =
        !forcedBreak && (unit.text === '\u00ad' || automaticHyphen) && i < units.length - 1;
      breaks.push({
        at: items.length,
        penalty: hyphenated ? 50 : 0,
        required: forcedBreak || i === units.length - 1,
        flagged: hyphenated,
        width: hyphenated ? (unit.breakWidth ?? unit.fontSize * 0.35) : 0,
      });
    }
  }
  const totalWidth = units.reduce((sum, unit) => sum + unit.width, 0);
  const codeWidth = units.reduce((sum, unit) => sum + (unit.code ? unit.width : 0), 0);
  const fontSize =
    paragraphFontSize ?? units.find((unit) => unit.fontSize > 0)?.fontSize ?? minLineHeight / 1.75;
  return {
    items,
    breaks,
    width,
    minLineHeight,
    hanging: options.hanging,
    alignment:
      width / fontSize < 20 || codeWidth / Math.max(1, totalWidth) >= 0.35 ? 'ragged' : 'auto',
    tolerance: 100,
    // Protect actual orphan tails without balancing a wide paragraph into two halves.
    lastLineMinWidth: Math.min(width * 0.12, fontSize * 4),
    lastLinePenalty: 500,
  };
}

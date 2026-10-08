import LineBreaker from 'linebreak';
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

/** UAX #14 supplies opportunities; profile tailoring only removes prohibited ones. */
export function createParagraphInput(
  units: LayoutItem[],
  width: number,
  minLineHeight: number,
  options: TypographyOptions,
  forced = new Set<number>(),
): ParagraphInput {
  const text = units.map((unit) => unit.text).join('');
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
          stretch: em * (mixed ? 0.125 : 0.35),
          shrink: mixed ? em * 0.0625 : 0,
          weight: mixed ? 1 : 0.35,
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
    if (forcedBreak || (!prohibited && opportunities.has(offset)) || i === units.length - 1) {
      const hyphenated = unit.text === '\u00ad' && i < units.length - 1;
      breaks.push({
        at: items.length,
        penalty: hyphenated ? 50 : 0,
        required: forcedBreak || i === units.length - 1,
        flagged: hyphenated,
        width: hyphenated ? (unit.breakWidth ?? unit.fontSize * 0.35) : 0,
      });
    }
  }
  return { items, breaks, width, minLineHeight, hanging: options.hanging };
}

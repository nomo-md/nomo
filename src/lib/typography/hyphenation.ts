import Hypher from 'hypher';
import englishPatterns from 'hyphenation.en-us';

let english: Hypher | undefined;
const cache = new Map<string, number[]>();

/** ASCII prose uses visual Liang candidates; callers exclude code, links and other languages. */
export function englishHyphenationPoints(eligibleText: string, source: string): Set<number> {
  const points = new Set<number>();
  for (const match of eligibleText.matchAll(/[A-Za-z]{8,}/g)) {
    const word = match[0];
    const start = match.index;
    if (word.length > 80 || !/^(?:[a-z]+|[A-Z][a-z]+)$/.test(word)) continue;
    if (/[\p{Script=Latin}\p{Number}\p{Mark}_-]/u.test(source[start - 1] ?? '')) continue;
    if (/[\p{Script=Latin}\p{Number}\p{Mark}_-]/u.test(source[start + word.length] ?? '')) continue;
    let left = start;
    let right = start + word.length;
    const delimiter = /[\s，。、；：！？（）《》“”‘’]/u;
    while (left > 0 && !delimiter.test(source[left - 1])) left--;
    while (right < source.length && !delimiter.test(source[right])) right++;
    const token = source.slice(left, right);
    if (/[@/\\]|[A-Za-z0-9]\.[A-Za-z0-9]|(?:https?|www)[.:]/i.test(token)) continue;
    const key = word.toLowerCase();
    let offsets = cache.get(key);
    if (!offsets) {
      english ??= new Hypher(englishPatterns);
      const parts = english.hyphenate(key);
      offsets = [];
      let offset = 0;
      for (const part of parts.slice(0, -1)) {
        offset += part.length;
        offsets.push(offset);
      }
      if (cache.size >= 512) cache.delete(cache.keys().next().value!);
      cache.set(key, offsets);
    }
    for (const offset of offsets) points.add(start + offset);
  }
  return points;
}

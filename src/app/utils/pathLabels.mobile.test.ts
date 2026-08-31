import { afterEach, expect, it, vi } from 'vitest';
import { sameNativePath, pathEqualsOrDescendsFrom } from './pathLabels';

afterEach(() => vi.restoreAllMocks());
it('does not collapse distinct Android import names into the same tab', () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Linux; Android 15');
  expect(sameNativePath('/data/Note.md', '/data/note.md')).toBe(false);
  expect(sameNativePath('/data/é.md', '/data/e\u0301.md')).toBe(false);
  expect(sameNativePath('/data/note.md', '/data/note.md')).toBe(true);
  expect(pathEqualsOrDescendsFrom('/data/import/note.md', '/data/import')).toBe(true);
});
it('preserves existing desktop path comparison behavior', () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Windows NT 10.0');
  expect(sameNativePath('C:\\Notes\\Note.md', 'c:/notes/note.md')).toBe(true);
});

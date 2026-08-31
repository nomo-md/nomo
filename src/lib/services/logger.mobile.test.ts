import { afterEach, expect, it, vi } from 'vitest';
import { enableLogger, disableLogger, logInfo } from './logger';

afterEach(() => {
  disableLogger();
  vi.restoreAllMocks();
});

it('redacts Android URI, path and body before console/buffer/native logging', () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Linux; Android 15');
  const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
  enableLogger();
  logInfo('Import', '打开 /data/user/0/app/files/private name.md', {
    path: '/data/user/0/private.md',
    url: 'https://example.com/?token=secret',
    body: 'private shared text',
    nested: { value: 'private text' },
    bytes: 42,
    failed: false,
  });
  const printed = info.mock.calls.at(-1)!;
  expect(printed[1]).toBe('打开 [redacted]');
  expect(printed[2]).toEqual({
    path: '[redacted]',
    url: '[redacted]',
    body: '[redacted]',
    nested: { value: '[redacted]' },
    bytes: 42,
    failed: false,
  });
  expect(JSON.stringify(printed)).not.toContain('private');
});

it('does not change desktop log context', () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Windows NT 10.0');
  const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
  enableLogger();
  logInfo('Import', 'Open', { path: 'C:/note.md', bytes: 42 });
  expect(info.mock.calls.at(-1)![2]).toEqual({ path: 'C:/note.md', bytes: 42 });
});

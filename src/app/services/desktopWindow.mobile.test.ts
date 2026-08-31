import { afterEach, describe, expect, it, vi } from 'vitest';
import { prepareOpenTargetWindow, syncWindowOpenTargets } from './desktopWindow';
import { isMobilePlatform } from './platform';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => ({ action: 'handled' })) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: vi.fn() }));
afterEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe('mobile compatibility with desktop open-target routing', () => {
  it('keeps Android/iOS opens in the current activity without unsupported IPC', async () => {
    for (const userAgent of ['Linux; Android 15', 'iPhone; CPU iPhone OS 18_0 like Mac OS X']) {
      vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(userAgent);
      expect(isMobilePlatform()).toBe(true);
      const target = { kind: 'documents' as const, paths: ['/data/note.md'] };
      expect(await prepareOpenTargetWindow(true, target, true)).toEqual({
        action: 'open-current',
        target,
      });
      await syncWindowOpenTargets(true, { filePaths: target.paths });
      expect(invoke).not.toHaveBeenCalled();
    }
  });
  it('retains desktop open-target registry calls', async () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Windows NT 10.0; Win64; x64');
    const target = { kind: 'documents' as const, paths: ['C:/note.md'] };
    expect(await prepareOpenTargetWindow(true, target, false)).toEqual({ action: 'handled' });
    await syncWindowOpenTargets(true, { filePaths: target.paths });
    expect(invoke).toHaveBeenCalledWith('prepare_open_target_window', {
      target,
      createIfMissing: false,
    });
    expect(invoke).toHaveBeenCalledWith('sync_window_open_targets', {
      input: { filePaths: target.paths },
    });
  });
});

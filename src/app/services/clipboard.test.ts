import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readEditorClipboard, releasePreparedImage } from './clipboard';

const { invoke, readText } = vi.hoisted(() => ({ invoke: vi.fn(), readText: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText }));

beforeEach(() => {
  invoke.mockReset();
  readText.mockReset();
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined });
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe('clipboard image sources', () => {
  it('keeps Web Clipboard first and never prepares an additional native image', async () => {
    const blob = new Blob(['image'], { type: 'image/png' });
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { read: async () => [{ types: ['image/png'], getType: async () => blob }] },
    });
    const result = await readEditorClipboard(true);
    expect(result.kind).toBe('image');
    if (result.kind === 'image') expect(result.files[0].size).toBe(blob.size);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('keeps the native snapshot identity and explicitly releases it', async () => {
    const image = {
      assetId: 'captured-before-save',
      fileName: 'clipboard.png',
      width: 3840,
      height: 2160,
    };
    invoke.mockResolvedValueOnce(image).mockResolvedValueOnce(undefined);
    const result = await readEditorClipboard(true);
    expect(result).toEqual({ kind: 'native-image', image });
    await releasePreparedImage(image);
    expect(invoke.mock.calls.map((call) => call[0])).toEqual([
      'prepare_clipboard_image',
      'release_prepared_image',
    ]);
    expect(readText).not.toHaveBeenCalled();
  });

  it('falls back to text only when native reading returned no image', async () => {
    invoke.mockResolvedValue(null);
    readText.mockResolvedValue('clipboard text');
    expect(await readEditorClipboard(true)).toEqual({ kind: 'text', text: 'clipboard text' });
    readText.mockClear();
    invoke.mockRejectedValue(new Error('PNG encode failed'));
    await expect(readEditorClipboard(true)).rejects.toThrow('PNG encode failed');
    expect(readText).not.toHaveBeenCalled();
  });

  it('plain-text paste does not read an image blob or call native prepare', async () => {
    const getType = vi.fn(async () => ({ text: async () => 'plain' }));
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { read: async () => [{ types: ['image/png', 'text/plain'], getType }] },
    });
    expect(await readEditorClipboard(true, 'text')).toEqual({ kind: 'text', text: 'plain' });
    expect(getType).toHaveBeenCalledOnce();
    expect(getType).toHaveBeenCalledWith('text/plain');
    expect(invoke).not.toHaveBeenCalled();
  });
});

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_IMAGE_HANDLING_SETTINGS } from '../../lib/services/render';
import { createDesktopImageLoader } from './desktopImageLoader';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../../lib/desktop/tauriStorage', () => ({ isTauriRuntime: () => true }));

beforeEach(() => {
  invoke.mockReset();
});

describe('desktop image transport', () => {
  it.each(['local', 'picgo', 'picgo-core'] as const)(
    'sends %s bytes as the raw IPC body with Unicode metadata',
    async (provider) => {
      const bytes = new Uint8Array([0, 255, 128, 3]);
      invoke.mockResolvedValue({
        markdown_src: './assets/截图.png',
        absolute_path: 'D:/文档/assets/截图.png',
        reused: true,
        url: 'https://images.test/a.gif',
      });
      const result = await createDesktopImageLoader().import(
        { fileName: '截图 %.png', bytes },
        {
          documentPath: 'D:/文档/a.md',
          documentFileName: 'a.md',
          settings: {
            ...DEFAULT_IMAGE_HANDLING_SETTINGS,
            imageInsertStrategy: provider === 'local' ? 'copy-assets' : 'upload',
            uploadProvider: provider === 'picgo-core' ? 'picgo-core' : 'picgo',
          },
        },
      );
      const [command, body, options] = invoke.mock.calls[0];
      expect(command).toBe(
        provider === 'local'
          ? 'import_image_asset'
          : provider === 'picgo'
            ? 'upload_image_via_picgo_server'
            : 'upload_image_via_picgo_core',
      );
      expect(body).toBe(bytes);
      const metadata = JSON.parse(decodeURIComponent(options.headers['x-nomo-image-meta']));
      expect(metadata.file_name).toBe('截图 %.png');
      expect(metadata).not.toHaveProperty('bytes');
      expect(result.markdownSrc).toBe(
        provider === 'local' ? './assets/截图.png' : 'https://images.test/a.gif',
      );
    },
  );

  it('imports a native snapshot by identifier without reading pixels or a file', async () => {
    invoke.mockResolvedValue({ markdown_src: './assets/clipboard.png', reused: true });
    await createDesktopImageLoader().import(
      { fileName: 'clipboard.png', nativeAssetId: 'snapshot-1' },
      { documentPath: 'D:/a.md' },
    );
    expect(invoke).toHaveBeenCalledWith('import_image_asset', {
      nativeAssetId: 'snapshot-1',
      input: {
        document_path: 'D:/a.md',
        document_file_name: 'document.md',
        strategy: 'copy-assets',
        file_name: 'clipboard.png',
      },
    });
  });

  it('rejects an empty image before IPC', async () => {
    await expect(
      createDesktopImageLoader().import(
        { fileName: 'empty.png', bytes: new Uint8Array() },
        { documentPath: 'D:/a.md' },
      ),
    ).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });
});

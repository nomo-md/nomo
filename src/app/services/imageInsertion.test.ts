import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { EditorCore } from '../../lib/editor-core';
import { createImageInsertionHandlers } from './imageInsertion';

const mocks = vi.hoisted(() => ({ importImage: vi.fn(), release: vi.fn(), application: vi.fn() }));
vi.mock('../../lib/editor-core/renderers', () => ({
  getImageLoader: () => ({ import: mocks.importImage }),
}));
vi.mock('./clipboard', () => ({ releasePreparedImage: mocks.release }));
vi.mock('./markdownApplicationSession', () => ({
  getMarkdownApplicationSession: mocks.application,
}));
vi.mock('../../lib/services/logger', () => ({
  createPerfTimer: () => ({ end: vi.fn() }),
  logError: vi.fn(),
  logInfo: vi.fn(),
}));

const nativeImage = { assetId: 'snapshot', fileName: 'clipboard.png', width: 3, height: 1 };

function setup() {
  let nativePath: string | null = 'D:/doc.md';
  let open = true;
  let revision = 0;
  const bookmarks: Array<{ insert: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn> }> =
    [];
  const editor = {
    canExecute: () => true,
    getSnapshot: () => ({
      version: revision,
      contentRevision: revision,
      selection: { anchor: 0, head: 0 },
    }),
    restoreSelectionSnapshot: vi.fn(),
  } as unknown as EditorCore;
  mocks.application.mockReturnValue({
    ready: Promise.resolve(),
    createInsertionBookmark() {
      const bookmark = { insert: vi.fn().mockResolvedValue(undefined), dispose: vi.fn() };
      bookmarks.push(bookmark);
      return bookmark;
    },
  });
  const save = vi.fn().mockResolvedValue(false);
  const status = vi.fn();
  const options: Parameters<typeof createImageInsertionHandlers>[0] = {
    getEditor: () => editor,
    getMode: () => 'semantic',
    getFileName: () => 'doc.md',
    getNativePath: () => nativePath,
    getSourceEditor: () => undefined,
    getImageContext: () => ({ documentPath: nativePath ?? undefined }),
    isDocumentOpen: () => open,
    saveMarkdownFile: save,
    setMarkdown: vi.fn(),
    setStatusMessage: status,
    syncSourceTextareaHeight: vi.fn(),
  };
  return {
    options,
    bookmarks,
    save,
    status,
    setPath: (path: string | null) => {
      nativePath = path;
    },
    close: () => {
      open = false;
    },
    edit: () => {
      revision++;
    },
  };
}

function file(name: string) {
  const source = new File(['image'], name, { type: 'image/png' });
  const read = vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3]).buffer);
  Object.defineProperty(source, 'arrayBuffer', { value: read });
  return { source, read };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.importImage.mockReset();
  mocks.importImage.mockResolvedValue({ markdownSrc: './assets/a.png' });
  mocks.release.mockResolvedValue(undefined);
});

describe('image insertion task ownership', () => {
  it('captures both bookmarks before queueing but only reads one image at a time', async () => {
    const state = setup();
    const handlers = createImageInsertionHandlers(state.options);
    let finish!: (value: { markdownSrc: string }) => void;
    mocks.importImage.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const first = file('first.png');
    const second = file('second.png');
    const one = handlers.insertImageFiles([first.source]);
    const two = handlers.insertImageFiles([second.source]);
    await vi.waitFor(() => expect(mocks.importImage).toHaveBeenCalledTimes(1));
    expect(state.bookmarks).toHaveLength(2);
    expect(first.read).toHaveBeenCalledOnce();
    expect(second.read).not.toHaveBeenCalled();
    let settled = false;
    const drain = handlers.awaitPendingInsertions().then(() => {
      settled = true;
    });
    await Promise.resolve();
    expect(settled).toBe(false);
    finish({ markdownSrc: './assets/first.png' });
    await Promise.all([one, two, drain]);
    expect(second.read).toHaveBeenCalledOnce();
    expect(state.bookmarks.map((bookmark) => bookmark.insert.mock.calls[0][0])).toEqual([
      '\n\n![first.png](./assets/first.png)\n\n',
      '\n\n![second.png](./assets/a.png)\n\n',
    ]);
    expect(state.bookmarks.every((bookmark) => bookmark.dispose.mock.calls.length === 1)).toBe(
      true,
    );
  });

  it('releases a native snapshot when initial save is cancelled', async () => {
    const state = setup();
    state.setPath(null);
    await createImageInsertionHandlers(state.options).insertPreparedImage(nativeImage);
    expect(state.save).toHaveBeenCalledOnce();
    expect(mocks.importImage).not.toHaveBeenCalled();
    expect(mocks.release).toHaveBeenCalledOnce();
    expect(mocks.release).toHaveBeenCalledWith(nativeImage);
    expect(state.bookmarks[0].dispose).toHaveBeenCalledOnce();
  });

  it('uses the captured native identifier after save and preserves it until import settles', async () => {
    const state = setup();
    state.setPath(null);
    state.save.mockImplementation(async () => {
      state.setPath('D:/saved.md');
      return true;
    });
    await createImageInsertionHandlers(state.options).insertPreparedImage(nativeImage);
    expect(mocks.importImage).toHaveBeenCalledWith(
      { fileName: 'clipboard.png', nativeAssetId: 'snapshot' },
      { documentPath: 'D:/saved.md' },
    );
    expect(mocks.release).toHaveBeenCalledOnce();
    expect(state.bookmarks[0].insert).toHaveBeenCalledOnce();
  });

  it.each(['blocked', 'readonly', 'closed', 'failed', 'stale'] as const)(
    'releases native resources on %s without inserting into a different target',
    async (scenario) => {
      const state = setup();
      if (scenario === 'blocked') state.options.canStartInsertion = () => false;
      if (scenario === 'readonly') state.options.getReadonly = () => true;
      if (scenario === 'closed')
        mocks.importImage.mockImplementationOnce(async () => {
          state.close();
          return { markdownSrc: './assets/a.png' };
        });
      if (scenario === 'failed')
        mocks.importImage.mockRejectedValueOnce(new Error('upload failed'));
      if (scenario === 'stale')
        mocks.importImage.mockImplementationOnce(async () => {
          state.edit();
          return { markdownSrc: './assets/a.png' };
        });
      await createImageInsertionHandlers(state.options).insertPreparedImage(
        nativeImage,
        scenario === 'stale'
          ? { selection: { anchor: 0, head: 0 }, contentRevision: 0 }
          : undefined,
      );
      expect(mocks.release).toHaveBeenCalledOnce();
      expect(mocks.release).toHaveBeenCalledWith(nativeImage);
      for (const bookmark of state.bookmarks) {
        expect(bookmark.insert).not.toHaveBeenCalled();
        expect(bookmark.dispose).toHaveBeenCalledOnce();
      }
    },
  );
});

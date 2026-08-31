import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
import { activateDocumentWindow, prepareOpenTargetWindow } from './desktopWindow';

beforeEach(() => {
  vi.resetAllMocks();
});

it('正式打开默认复用目录窗口，文件树预览可以保持原有行为', async () => {
  const target = { kind: 'documents' as const, paths: ['D:/notes/b.md'] };
  await prepareOpenTargetWindow(true, target, false);
  expect(invoke).toHaveBeenLastCalledWith('prepare_open_target_window', {
    target,
    createIfMissing: false,
    reuseDirectoryWindow: true,
  });
  await prepareOpenTargetWindow(true, target, false, { reuseDirectoryWindow: false });
  expect(invoke).toHaveBeenLastCalledWith('prepare_open_target_window', {
    target,
    createIfMissing: false,
    reuseDirectoryWindow: false,
  });
});

describe('activateDocumentWindow', () => {
  it('只激活调用者，不向 IPC 传入任意窗口标签', async () => {
    await activateDocumentWindow(true);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('activate_document_window');
  });

  it('浏览器环境不调用原生窗口操作', async () => {
    await activateDocumentWindow(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('激活失败向调用方传递错误', async () => {
    invoke.mockRejectedValue(new Error('cannot activate'));
    await expect(activateDocumentWindow(true)).rejects.toThrow('cannot activate');
  });
});

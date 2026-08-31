import { describe, expect, it, vi } from 'vitest';
import type { OpenTarget } from './desktopWindow';
import { routeOpenTarget, type OpenTargetRouting } from './openTargetRouting';

function setup() {
  const calls: string[] = [];
  const routing: OpenTargetRouting = {
    syncTargets: vi.fn(async () => {
      calls.push('sync');
    }),
    prepare: vi.fn<OpenTargetRouting['prepare']>(async (target, create) => {
      calls.push(create ? 'reserve' : 'resolve');
      return create
        ? { action: 'create-window', windowLabel: 'window-target', target }
        : { action: 'open-current', target };
    }),
    activateCurrent: vi.fn(async () => {
      calls.push('activate');
    }),
    openCurrent: vi.fn(async () => {
      calls.push('open');
    }),
    createWindow: vi.fn(async () => {
      calls.push('create');
    }),
    isReusableInitialWindow: vi.fn(() => false),
    getBehavior: vi.fn<OpenTargetRouting['getBehavior']>(() => 'new-window'),
    requestChoice: vi.fn(async () => {
      calls.push('ask');
      return null;
    }),
    rememberBehavior: vi.fn(async () => {
      calls.push('remember');
    }),
  };
  return { routing, calls };
}

const targets: OpenTarget[] = [
  { kind: 'documents', paths: ['D:/test/target.md'] },
  { kind: 'folder', path: 'D:/test/folder' },
];

describe.each(targets)('$kind 激活时机', (target) => {
  it('新窗口创建完成前后都不激活旧窗口', async () => {
    const { routing, calls } = setup();
    let finish!: () => void;
    vi.mocked(routing.createWindow).mockImplementation(async () => {
      calls.push('create');
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      expect(routing.activateCurrent).not.toHaveBeenCalled();
    });
    const opening = routeOpenTarget(target, routing);
    await vi.waitFor(() => expect(routing.createWindow).toHaveBeenCalledOnce());
    expect(routing.activateCurrent).not.toHaveBeenCalled();
    finish();
    await opening;
    expect(calls).toEqual(['sync', 'resolve', 'reserve', 'create']);
    expect(routing.openCurrent).not.toHaveBeenCalled();
  });

  it('当前窗口先激活再执行文档操作', async () => {
    const { routing, calls } = setup();
    vi.mocked(routing.getBehavior).mockReturnValue('current-window');
    await routeOpenTarget(target, routing);
    expect(calls).toEqual(['sync', 'resolve', 'activate', 'open']);
  });

  it('初始空窗口仍优先复用', async () => {
    const { routing, calls } = setup();
    vi.mocked(routing.isReusableInitialWindow).mockReturnValue(true);
    await routeOpenTarget(target, routing);
    expect(calls).toEqual(['sync', 'resolve', 'activate', 'open']);
  });

  it('已有目标在其他窗口时不激活接收窗口', async () => {
    const { routing } = setup();
    vi.mocked(routing.prepare).mockResolvedValue({ action: 'handled' });
    await routeOpenTarget(target, routing);
    expect(routing.activateCurrent).not.toHaveBeenCalled();
    expect(routing.createWindow).not.toHaveBeenCalled();
  });

  it('已有目标在当前窗口时先激活再定位', async () => {
    const { routing, calls } = setup();
    vi.mocked(routing.prepare).mockResolvedValue({ action: 'activate-current', target });
    await routeOpenTarget(target, routing);
    expect(calls).toEqual(['sync', 'activate', 'open']);
  });

  it('显示询问前激活，取消不打开也不创建窗口', async () => {
    const { routing, calls } = setup();
    vi.mocked(routing.getBehavior).mockReturnValue('ask-every-time');
    await routeOpenTarget(target, routing);
    expect(calls).toEqual(['sync', 'resolve', 'activate', 'ask']);
    expect(routing.rememberBehavior).not.toHaveBeenCalled();
  });

  it.each(['handled', 'activate-current'] as const)(
    '二次确认返回 %s 时保留已有目标处理',
    async (action) => {
      const { routing, calls } = setup();
      vi.mocked(routing.prepare)
        .mockResolvedValueOnce({ action: 'open-current', target })
        .mockResolvedValueOnce(action === 'handled' ? { action } : { action, target });
      await routeOpenTarget(target, routing);
      expect(routing.prepare).toHaveBeenCalledTimes(2);
      expect(calls).toEqual(action === 'handled' ? ['sync'] : ['sync', 'activate', 'open']);
      expect(routing.createWindow).not.toHaveBeenCalled();
    },
  );

  it.each(['current-window', 'new-window'] as const)(
    '询问选择 %s 后不再次激活旧窗口',
    async (choice) => {
      const { routing, calls } = setup();
      vi.mocked(routing.getBehavior).mockReturnValue('ask-every-time');
      vi.mocked(routing.requestChoice).mockImplementation(async () => {
        calls.push('ask');
        return { choice, remember: true };
      });
      await routeOpenTarget(target, routing);
      expect(calls).toEqual([
        'sync',
        'resolve',
        'activate',
        'ask',
        'remember',
        ...(choice === 'current-window' ? ['open'] : ['reserve', 'create']),
      ]);
      expect(routing.rememberBehavior).toHaveBeenCalledWith(choice);
    },
  );
});

it('混合批次先在同目录窗口增加标签，再处理其他目录，不丢弃剩余文件', async () => {
  const { routing, calls } = setup();
  const current: OpenTarget = { kind: 'documents', paths: ['D:/notes/b.md'] };
  const remaining: OpenTarget = { kind: 'documents', paths: ['D:/other/c.md'] };
  const batch: OpenTarget = { kind: 'documents', paths: ['D:/notes/b.md', 'D:/other/c.md'] };
  vi.mocked(routing.prepare).mockImplementation(async (target, create) => {
    calls.push(create ? 'reserve' : 'resolve');
    if (target === batch)
      return { action: 'activate-current', target: current, remainingTarget: remaining };
    return create
      ? { action: 'create-window', windowLabel: 'window-other', target }
      : { action: 'open-current', target };
  });
  await routeOpenTarget(batch, routing);
  expect(calls).toEqual([
    'sync',
    'resolve',
    'activate',
    'open',
    'sync',
    'resolve',
    'reserve',
    'create',
  ]);
  expect(routing.openCurrent).toHaveBeenCalledTimes(1);
  expect(routing.openCurrent).toHaveBeenCalledWith(current);
  expect(routing.createWindow).toHaveBeenCalledWith('window-other');
});

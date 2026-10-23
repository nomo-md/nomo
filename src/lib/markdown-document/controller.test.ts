import { describe, expect, it } from 'vitest';
import { MarkdownSessionController } from './controller';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';

describe('Markdown 会话协调', () => {
  it('统计等待组字而不确认草稿，取消等待不提交输入', async () => {
    const port = createMemoryMarkdownPort();
    const controller = new MarkdownSessionController(port, await port.openDocument({ text: '正文' }));
    let committed = 0;
    let endComposition!: () => void;
    const detach = controller.attachEdits({
      awaitCompositionEnd: () => new Promise<void>(resolve => { endComposition = resolve; }),
      commitPendingEdits: () => { committed++; },
    });
    const abort = new AbortController();
    const waiting = controller.awaitStatisticsReady(abort.signal);
    abort.abort(); expect(await waiting).toBe(false);
    endComposition(); expect(committed).toBe(0); detach();
    const removeDraft = controller.attachEdits({ awaitCompositionEnd: async () => {},
      hasUnconfirmedDraft: () => true, commitPendingEdits: () => { committed++; } });
    expect(await controller.awaitStatisticsReady(new AbortController().signal)).toBe(false);
    expect(committed).toBe(0); removeDraft(); await controller.close(true);
  });
  it('语义/源码视图连续提交同一队列，挂载回收不切断历史', async () => {
    const port = createMemoryMarkdownPort();
    const controller = new MarkdownSessionController(
      port,
      await port.openDocument({ text: '甲\r\n尾部' }),
    );
    const first = controller.applyEdits({
      viewGeneration: 0,
      historyGroupId: 'semantic',
      edits: [{ fromByte: 0, toByte: 3, insertedText: '乙' }],
    });
    const second = controller.applyEdits({
      viewGeneration: 1,
      historyGroupId: 'source',
      edits: [{ fromByte: 3, toByte: 3, insertedText: '😀' }],
    });
    await Promise.all([first, second]);
    let committed = 0;
    const detach = controller.attachEdits({
      awaitCompositionEnd: async () => {},
      commitPendingEdits: () => {
        committed++;
      },
    });
    detach();
    expect(await controller.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      '乙😀\r\n尾部',
    );
    expect(committed).toBe(0);
    await controller.undo();
    await controller.undo();
    expect(controller.state.committed.dirty).toBe(false);
    await controller.undo(true);
    await controller.undo(true);
    expect(await controller.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      '乙😀\r\n尾部',
    );
    await controller.close(true);
  });
  it('确认丢失时重试同一操作，不重复写入，并继续保留后续输入', async () => {
    const port = createMemoryMarkdownPort();
    const apply = port.applyTransaction;
    let loseAcknowledgement = true;
    port.applyTransaction = async (...args) => {
      const result = await apply(...args);
      if (loseAcknowledgement) {
        loseAcknowledgement = false;
        throw new Error('模拟确认丢失');
      }
      return result;
    };
    const controller = new MarkdownSessionController(
      port,
      await port.openDocument({ text: 'tail' }),
    );
    const first = controller.applyEdits({
      viewGeneration: 0,
      historyGroupId: '1',
      edits: [{ fromByte: 0, toByte: 0, insertedText: 'A' }],
    });
    const next = controller.applyEdits({
      viewGeneration: 1,
      historyGroupId: '2',
      edits: [{ fromByte: 1, toByte: 1, insertedText: 'B' }],
    });
    await expect(first).rejects.toThrow('模拟确认丢失');
    await expect(controller.captureSnapshot()).rejects.toThrow('模拟确认丢失');
    expect(controller.state.pendingTransactions).toBe(2);
    await controller.retryPendingEdits();
    await next;
    expect(await controller.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      'ABtail',
    );
    expect(controller.state.committed.revision).toBe(2);
    await controller.close(true);
  });
  it('保存冻结快照时允许新输入；旧保存成功不能把新内容标记为干净', async () => {
    const port = createMemoryMarkdownPort();
    const save = port.saveSnapshot;
    let release!: () => void;
    let started!: () => void;
    const startedPromise = new Promise<void>((resolve) => {
      started = resolve;
    });
    port.saveSnapshot = async (...args) => {
      started();
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      return save(...args);
    };
    const controller = new MarkdownSessionController(
      port,
      await port.openDocument({ text: 'before' }),
    );
    const saving = controller.save('memory.md');
    await startedPromise;
    await controller.applyEdits({
      viewGeneration: 0,
      historyGroupId: 'edit',
      edits: [{ fromByte: 6, toByte: 6, insertedText: ' after' }],
    });
    release();
    await saving;
    expect(controller.state.committed.dirty).toBe(true);
    await controller.undo();
    expect(controller.state.committed.dirty).toBe(false);
    await controller.close();
  });
  it('全文读取跨块 UTF-8 和 CRLF 边界不重复或遗漏字符', async () => {
    const port = createMemoryMarkdownPort();
    const source = 'a'.repeat(256 * 1024 - 1) + '😀\r\n' + 'b'.repeat(256 * 1024 - 5) + '\r\n尾';
    const controller = new MarkdownSessionController(
      port,
      await port.openDocument({ text: source }),
    );
    expect(await controller.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      source,
    );
    await controller.close();
  });
  it('自动保存等待公式草稿，明确提交后才允许捕获快照', async () => {
    const port = createMemoryMarkdownPort();
    const controller = new MarkdownSessionController(port, await port.openDocument({ text: '' }));
    let pending = true;
    const detach = controller.attachEdits({
      awaitCompositionEnd: async () => {},
      hasUnconfirmedDraft: () => pending,
      commitPendingEdits: () => {
        pending = false;
      },
    });
    await expect(controller.captureSnapshot('automatic')).rejects.toThrow('草稿');
    await controller.withSnapshot(async () => {});
    expect(pending).toBe(false);
    detach();
    await controller.close();
  });
  it('关闭等待输入提交，失败后解除门禁并保留正文', async () => {
    const port = createMemoryMarkdownPort();
    const controller = new MarkdownSessionController(
      port,
      await port.openDocument({ text: '原文' }),
    );
    let finish!: () => void;
    let entered!: () => void;
    const closing = new Promise<void>((resolve) => {
      entered = resolve;
    });
    port.close = async () => {
      entered();
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      throw new Error('关闭存储暂不可用');
    };
    const task = controller.close();
    await closing;
    expect(controller.state.closing).toBe(true);
    await expect(
      controller.applyEdits({
        viewGeneration: 0,
        historyGroupId: 'blocked',
        edits: [{ fromByte: 0, toByte: 0, insertedText: '不可提交' }],
      }),
    ).rejects.toThrow('切换文档状态');
    finish();
    await expect(task).rejects.toThrow('关闭存储暂不可用');
    expect(controller.state.closing).toBe(false);
    await controller.applyEdits({
      viewGeneration: 0,
      historyGroupId: 'after-failure',
      edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }],
    });
    expect(await controller.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      '新原文',
    );
  });
});

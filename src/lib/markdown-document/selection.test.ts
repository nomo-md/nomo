import { describe, expect, it } from 'vitest';
import { MarkdownSessionController } from './controller';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownGlobalSelection } from './selection';
import { utf8Length } from './utf8Text';

async function setup(text: string) {
  const port = createMemoryMarkdownPort();
  const session = new MarkdownSessionController(port, await port.openDocument({ text }));
  const selection = new MarkdownGlobalSelection(session);
  return {
    port,
    session,
    selection,
    read: () => session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot)),
    close: async () => {
      selection.dispose();
      await session.close(true);
    },
  };
}

describe('全文选区与未挂载正文', () => {
  it('整块语义补丁保留事务内光标供后续原生目录派生使用', async () => {
    const { session, selection, close } = await setup('# 原标题\n\n正文');
    selection.set({ anchorByte: 2, headByte: 2 });
    const pending = session.applyEdits({ viewGeneration: 0, historyGroupId: 'heading',
      edits: [{ fromByte: 0, toByte: 11, insertedText: '# 新原标题' }],
      selectionAfter: { anchorByte: 5, headByte: 5 } });
    expect(selection.selection).toEqual({ anchorByte: 5, headByte: 5 });
    await pending;
    expect(selection.selection).toEqual({ anchorByte: 5, headByte: 5 });
    await close();
  });
  it('没有挂载任何视图也能全选、复制、替换和撤销完整正文', async () => {
    const source = '第一段😀\r\n' + '中部\r\n'.repeat(60_000) + '尾部  \n\n';
    const { session, selection, read, close } = await setup(source);
    selection.selectAll();
    expect(selection.visibleRange(0, 20)).toEqual({ fromByte: 0, toByte: 20 });
    expect(await selection.copy()).toBe(source);
    await selection.replaceSelection('新正文');
    expect(await read()).toBe('新正文');
    await session.undo();
    expect(await read()).toBe(source);
    expect(selection.selection).toEqual({ anchorByte: 0, headByte: utf8Length(source) });
    expect(session.state.committed.dirty).toBe(false);
    await close();
  });
  it('剪贴板写入失败或者等待期间继续输入，都不能删除旧选区', async () => {
    const { session, selection, read, close } = await setup('保留正文');
    selection.selectAll();
    await expect(
      selection.cut(async () => {
        throw new Error('剪贴板失败');
      }),
    ).rejects.toThrow('剪贴板失败');
    expect(await read()).toBe('保留正文');
    await expect(
      selection.cut(async (copied) => {
        expect(copied).toBe('保留正文');
        await session.applyEdits({
          viewGeneration: 0,
          historyGroupId: 'new-input',
          edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }],
        });
      }),
    ).rejects.toThrow('正文或选区已经变化');
    expect(await read()).toBe('新保留正文');
    await close();
  });
  it('等待剪贴板期间仅改变光标也不删除原选区；完整剪切作为一条历史', async () => {
    const { session, selection, read, close } = await setup('首屏\r\n尾部');
    selection.selectAll();
    await expect(
      selection.cut(async () => {
        selection.set({ anchorByte: 0, headByte: 0 });
      }),
    ).rejects.toThrow('正文或选区已经变化');
    expect(await read()).toBe('首屏\r\n尾部');
    selection.set({ anchorByte: 8, headByte: 14 });
    expect(await selection.copy()).toBe('尾部');
    await selection.cut(async () => {});
    expect(await read()).toBe('首屏\r\n');
    await session.undo();
    expect(await read()).toBe('首屏\r\n尾部');
    await close();
  });
});

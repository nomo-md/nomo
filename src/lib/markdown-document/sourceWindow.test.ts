// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownSessionController } from './controller';
import { MarkdownSourceWindow } from './sourceWindow';
import { MarkdownSemanticWindow } from './semanticWindow';

it('PM 和 CM 交替编辑共用全文、保存点及撤销，混合换行不随视图转换改变', async () => {
  const original = '+ 正文\r\n+ 第二行\n\r\n尾部  \n';
  const port = createMemoryMarkdownPort();
  const session = new MarkdownSessionController(port, await port.openDocument({ text: original }));
  const target = document.createElement('div');
  document.body.append(target);
  const errors: unknown[] = [];
  const options = {
    target,
    onStale() {},
    onHistory() {},
    onFailure: (error: unknown) => errors.push(error),
  };
  const initial = await port.readWindow(
    session.documentId,
    0,
    0,
    session.state.committed.byteLength,
  );
  const semantic = new MarkdownSemanticWindow(session, initial, options);
  semantic.activate();
  await semantic.whenReady;
  semantic.core.restoreSelectionSnapshot({ anchor: 3, head: 3 });
  semantic.core.execute({ type: 'insertText', text: '新' });
  await session.flushEdits();
  semantic.dispose();
  const current = await port.readWindow(
    session.documentId,
    session.state.committed.revision,
    0,
    session.state.committed.byteLength,
  );
  const source = new MarkdownSourceWindow(session, current, options);
  const offset = source.view.state.doc.toString().indexOf('尾部');
  source.view.dispatch({ changes: { from: offset, to: offset + 2, insert: '尾部😀' } });
  await session.flushEdits();
  source.dispose();
  expect(errors).toEqual([]);
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    original.replace('正文', '新正文').replace('尾部', '尾部😀'),
  );
  await session.undo();
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    original.replace('正文', '新正文'),
  );
  await session.undo();
  expect(session.state.committed.dirty).toBe(false);
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    original,
  );
  await session.close();
  target.remove();
});

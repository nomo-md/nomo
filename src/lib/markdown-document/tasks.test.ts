import { expect, it } from 'vitest';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownSessionController } from './controller';
import { searchMarkdownSnapshot, replaceMarkdownSnapshotMatches } from './tasks';

it('全文搜索和替换覆盖未挂载尾部、传输边界与中文/emoji，整次替换只撤销一次', async () => {
  const port = createMemoryMarkdownPort();
  const text = 'x'.repeat(65_534) + ' 中文😀目标 \r\n' + '未挂载尾部 中文😀目标 ';
  const session = new MarkdownSessionController(port, await port.openDocument({ text }));
  await session.withSnapshot(async (snapshot) => {
    const matches = await searchMarkdownSnapshot(port, snapshot, '中文😀目标', {
      signal: new AbortController().signal,
    });
    expect(matches).toHaveLength(2);
    await replaceMarkdownSnapshotMatches(
      session,
      snapshot,
      matches,
      '替换',
      session.state.viewGeneration,
    );
  });
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    text.split('中文😀目标').join('替换'),
  );
  await session.undo();
  expect(session.state.committed.dirty).toBe(false);
  await session.close();
});

it('旧快照的替换任务拒绝覆盖任务期间的新输入', async () => {
  const port = createMemoryMarkdownPort();
  const session = new MarkdownSessionController(
    port,
    await port.openDocument({ text: 'old tail' }),
  );
  await session.withSnapshot(async (snapshot) => {
    const generation = session.state.viewGeneration;
    const matches = await searchMarkdownSnapshot(port, snapshot, 'old', {
      signal: new AbortController().signal,
    });
    await session.applyEdits({
      viewGeneration: generation,
      historyGroupId: 'edit',
      edits: [{ fromByte: 0, toByte: 0, insertedText: 'new ' }],
    });
    await expect(
      replaceMarkdownSnapshotMatches(session, snapshot, matches, 'replacement', generation),
    ).rejects.toThrow('正文已经变化');
  });
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    'new old tail',
  );
  await session.close(true);
});

it('跨边界重复模式遵循全局非重叠匹配，全词模式检查相邻字符', async () => {
  const port = createMemoryMarkdownPort();
  const text = 'a'.repeat(140_003) + ' cat 猫cat cat猫 Cat ';
  const session = new MarkdownSessionController(port, await port.openDocument({ text }));
  await session.withSnapshot(async (snapshot) => {
    const matches = await searchMarkdownSnapshot(port, snapshot, 'aaa', {
      signal: new AbortController().signal,
    });
    expect(matches.length).toBe(Math.floor(140_003 / 3));
    expect(matches.every((match, index) => match.fromByte === index * 3)).toBe(true);
    const words = await searchMarkdownSnapshot(port, snapshot, 'cat', {
      wholeWord: true,
      signal: new AbortController().signal,
    });
    expect(words.map((match) => match.text)).toEqual(['cat', 'Cat']);
  });
  await session.close();
});

it('跨 CRLF 中点的连续命中合并为安全补丁，保留外侧换行并可完整撤销', async () => {
  const port = createMemoryMarkdownPort();
  const source = '\r\n\r\n\r\n';
  const session = new MarkdownSessionController(port, await port.openDocument({ text: source }));
  await session.withSnapshot(async (snapshot) => {
    const matches = await searchMarkdownSnapshot(port, snapshot, '\n\r', {
      signal: new AbortController().signal,
    });
    expect(matches).toHaveLength(2);
    await replaceMarkdownSnapshotMatches(
      session,
      snapshot,
      matches,
      'X',
      session.state.viewGeneration,
    );
  });
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    source.split('\n\r').join('X'),
  );
  await session.undo();
  expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
    source,
  );
  await session.close();
});

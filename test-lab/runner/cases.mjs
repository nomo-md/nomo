import assert from 'node:assert/strict';
import { readFile, writeFile, appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import { hashJson, hashFile, hashOutsideToc, saveJson, pause } from './common.mjs';
import { assertMarkdown, inlineResults, blockResults } from './oracles.mjs';

export const enhancementExpectations = { minimum_images: 'images', minimum_details: 'htmlDetails', image_references: 'images',
  code_blocks: 'codeBlocks', mermaid_blocks: 'diagrams', math_blocks: 'blockMath', inline_math: 'inlineMath', details: 'htmlDetails', table_images: 'images' };
export const supportedExpectations = [...Object.keys(enhancementExpectations), 'headings_in_body', 'exact_bytes'];

export function catalogue(dataset) {
  const corpus = dataset.samples.map(sample => {
    const category = sample.path.slice(0, 2);
    const suites = ['corpus'];
    if (['常规', '中压', '兼容', '异常'].includes(sample.level)) suites.push('standard');
    else suites.push('stress');
    if (sample.level === '兼容') suites.push('compat');
    else if (sample.level === '异常') suites.push('errors');
    else if (category === '01' || category === '02') suites.push('structure');
    else if (category === '03' || category === '04') suites.push('render');
    else suites.push('editing');
    return { id: `corpus.${hashJson(sample.path).slice(0, 10)}`, title: sample.path, kind: 'corpus', samples: [sample], suites,
      tags: [category, sample.level, ...(sample.deliberate ?? [])], expected: sample.expected ?? {}, purpose: sample.purpose };
  });
  const select = pattern => dataset.samples.find(e => pattern.test(e.path));
  const small = select(/短文编辑基线/) ?? dataset.samples[0];
  const four = dataset.samples.filter(e => /四标签_/.test(e.path));
  const comparison = dataset.samples.filter(e => /组合对照_/.test(e.path));
  const behavior = [
    ['smoke', '基础读写与接口错误处理', 'smoke', [small]],
    ['editing.commands', '选区、格式、粘贴与历史', 'editing', [small]],
    ['editing.blocks', '标题、引用、列表与任务编辑', 'editing', [small]],
    ['editing.table', '表格行列、对齐与撤销', 'editing', [small]],
    ['structure.outline', '大纲导航与过期修订拒绝', 'structure', [small]],
    ['editing.native', '真实键盘、快捷键与滚动', 'editing', [small]],
    ['interaction.semantic', '持续语义编辑与真实输入延迟', 'interaction', [small]],
    ['interaction.large-input', '大文档真实输入与正文完整性', 'interaction', [select(/05_.*05MiB/) ?? small]],
    ['interaction.scroll', '真实连续滚动与正文完整性', 'interaction', [select(/05_.*05MiB/) ?? small]],
    ['search.behavior', '搜索、分页、大小写、全词、替换与取消', 'search', [small]],
    ['lifecycle.tabs', '四标签切换、选区与阅读位置', 'lifecycle', four.length ? four : [small]],
    ['lifecycle.comparison', '组合、拆分与文档隔离', 'lifecycle', comparison.length ? comparison : [small]],
    ['persistence.restart', '正常退出后重新启动', 'persistence', [small]],
    ['persistence.recovery', '异常退出后的未保存内容恢复', 'persistence', [small]],
    ['persistence.external', '外部文件变更与重新加载', 'persistence', [small]],
    ['persistence.autosave', '自动保存到磁盘', 'persistence', [small]],
    ['structure.directory', '256 文件目录负载', 'structure', [...new Set([dataset.tree[0], dataset.tree[Math.floor(dataset.tree.length / 2)], dataset.tree.at(-1)].filter(Boolean))]],
    ['soak.lifecycle', '持续编辑与生命周期循环', 'soak', [small]],
    ['selftest.failure', '注入失败以验证报告与继续执行', 'selftest', [small]],
    ['selftest.after', '失败之后的独立用例', 'selftest', [small]],
    ['selftest.cleanup', '注入清理失败，确认停止后续用例', 'selftest-cleanup', [small]],
    ['selftest.cleanup-after', '清理失败后禁止执行的哨兵', 'selftest-cleanup', [small]],
  ].filter(e => e[3].length).map(([id, title, suite, samples]) => ({ id, title, kind: id, samples,
    suites: ['soak', 'selftest', 'selftest-cleanup', 'interaction'].includes(suite) ? [suite] : [suite, 'standard'], tags: [suite, ...(id === 'editing.native' || suite === 'interaction' || suite === 'soak' ? ['foreground'] : [])] }));
  const perfSamples = [select(/真正空白/), small, select(/富文本表格_80/), select(/^03_.*图片格式图鉴/),
    select(/七类Mermaid/), select(/05_.*05MiB/), select(/05_.*10MiB/), select(/06_.*01MiB/)].filter(Boolean);
  const performance = [...new Map(perfSamples.map(s => [s.path, s])).values()].map(sample => ({
    id: `performance.${hashJson(sample.path).slice(0, 10)}`, title: `性能：${sample.path}`, kind: 'performance', samples: [sample],
    suites: ['performance', 'standard'], tags: ['performance', sample.level] }));
  return [...behavior, ...corpus, ...performance];
}

export function selectCases(cases, config) {
  const suites = new Set(config.suites);
  const known = new Set(cases.flatMap(c => c.suites).concat(['diagnostics', 'all']));
  for (const suite of suites) if (!known.has(suite)) throw new Error(`未知测试集：${suite}`);
  if (suites.has('diagnostics') && !config.caseIds?.length) throw new Error('diagnostics 必须指定 --case，避免对全部样本采集堆快照');
  for (const id of config.caseIds ?? []) if (!cases.some(c => c.id === id)) throw new Error(`未知用例：${id}`);
  for (const tag of config.tags ?? []) if (!cases.some(c => c.tags.includes(tag))) throw new Error(`未知标签：${tag}`);
  const selected = cases.filter(c => (suites.has('all') || suites.has('diagnostics') || c.suites.some(s => suites.has(s))) &&
    (!config.caseIds?.length || config.caseIds.includes(c.id)) && (!config.tags?.length || config.tags.every(t => c.tags.includes(t))));
  if (!selected.length) throw new Error('筛选后没有用例；未启动程序');
  return selected;
}

export async function executeCase(test, host, root, config, result) {
  const call = (...args) => host.call(...args);
  const check = (name, actual, expected) => { assert.deepEqual(actual, expected, name); result.assertions.push(name); };
  const truth = (name, value) => check(name, Boolean(value), true);
  const digest = () => call('digest');
  const marker = 'nomo_test_marker_42';
  const scratch = join(root, 'scratch.md');
  const wait = (stage, timeoutMs = test.suites.includes('stress') ? config.stressTimeoutMs : config.actionTimeoutMs) => call('wait', { stage, timeoutMs }, timeoutMs + 2000);
  const open = async path => { const state = await call('open', { path }); await wait('indexed'); await wait('readable'); return state.activeTabId; };
  const edit = async (text = marker) => { await call('mode', { mode: 'source' }); await wait('editable'); await call('select', { fromByte: 0 }); await call('insert', { text }); };
  const roundTrip = async path => {
    const before = await digest();
    const tocBody = await hashOutsideToc(path, marker);
    await edit(); const changed = await digest(); truth('插入改变正文摘要', changed.sha256 !== before.sha256);
    await call('undo'); check('撤销恢复完整正文', (await digest()).sha256, before.sha256);
    await call('redo'); check('重做恢复完整正文', (await digest()).sha256, changed.sha256);
    await call('save'); const saved = await digest();
    if (tocBody.blocks) check('保存仅允许 TOC 派生区更新，其他字节保留', await hashOutsideToc(path), tocBody);
    else check('保存不改变正文', saved.sha256, changed.sha256);
    await call('close'); await open(path);
    check('保存重开完整正文', (await digest()).sha256, saved.sha256);
    return { before, changed };
  };
  const sample = test.samples[0];
  const nativeRound = async () => {
    result.requiredMetrics = ['inputP95Ms'];
    await call('mode', { mode: 'semantic' }); await wait('editable');
    const original = await digest();
    const opening = await call('readRange', { fromByte: 0, toByte: Math.min(4096, original.byteLength) });
    const paragraph = opening.text.indexOf('\n\n');
    // 使用合法 UTF-8 边界的正文起点，避免固定字节 12 落在中文标题字符或语法标记中。
    const insertionByte = Buffer.byteLength(opening.text.slice(0, paragraph >= 0 ? paragraph + 2 : 0));
    await call('reveal', { byte: insertionByte }); await wait('stable');
    const typed = 'NomoNativeabcdefghijklmnopqrstuvwxyz0123456789';
    await host.native({ text: typed });
    await wait('indexed'); await wait('stable');
    const range = await call('readRange', { fromByte: 0, toByte: Math.min(4096, original.byteLength + Buffer.byteLength(typed)) });
    const offset = range.text.indexOf(typed);
    truth('真实输入准确写入正文', offset >= 0);
    const from = Buffer.byteLength(range.text.slice(0, offset));
    await call('select', { fromByte: from, toByte: from + Buffer.byteLength(typed) }); await call('insert', { text: '' });
    const restored = await digest();
    if (restored.sha256 !== original.sha256) result.integrityFailure = { before: original, after: restored, insertionByte, markerFoundByte: from,
      beforePrefix: opening.text, afterPrefix: (await call('readRange', { fromByte: 0, toByte: Math.min(4096, restored.byteLength) })).text };
    check('删除本次输入后其他正文完整', restored.sha256, original.sha256);
    result.requiredMetrics = ['inputP95Ms'];
  };
  const scrollRound = async () => {
    await call('mode', { mode: 'semantic' }); await wait('stable');
    if (!host.cdp) throw new Error('滚动验证需要 CDP 正文锚点');
    await call('reveal', { byte: 0 }); await wait('stable');
    const before = await host.readingAnchors(), hash = await digest();
    for (let i = 0; i < 3; i++) await host.native({ keys: 'PAGEDOWN' });
    await wait('stable'); const after = await host.readingAnchors();
    truth('真实滚动改变正文位置', before[0]?.anchor && after[0]?.anchor &&
      (before[0].scrollTop !== after[0].scrollTop || JSON.stringify(before[0].anchor) !== JSON.stringify(after[0].anchor)));
    check('连续滚动不改变全文', (await digest()).sha256, hash.sha256);
  };
  if (test.kind.startsWith('interaction.')) {
    const path = test.kind === 'interaction.semantic' ? scratch : join(root, sample.path);
    if (test.kind === 'interaction.semantic') await writeFile(path, '# Workload\n\n' + 'Editable paragraph with stable text.\n\n'.repeat(200));
    await open(path); await wait('stable');
    result.measurements = [{ phase: 'initial', at: new Date().toISOString(), data: await host.capture() }];
    for (let i = 0; i < config.cycles; i++) {
      if (test.kind === 'interaction.scroll') await scrollRound(); else await nativeRound();
    }
    await wait('stable'); await pause(1000);
    result.measurements.push({ phase: 'idle', at: new Date().toISOString(), data: await host.capture() });
    result.workload = { kind: test.kind, cycles: config.cycles, native: true, bytes: (await digest()).byteLength };
    return;
  }
  if (test.kind === 'selftest.failure') { await open(scratch); throw new Error('INJECTED_FAILURE: intentional runner isolation check'); }
  if (test.kind === 'selftest.cleanup') {
    await open(scratch); const stop = host.stop.bind(host);
    host.stop = async () => { await stop(); throw new Error('INJECTED_CLEANUP_FAILURE: subsequent cases must not start'); };
    return;
  }
  if (test.kind === 'selftest.cleanup-after') throw new Error('CLEANUP_SENTINEL_EXECUTED: runner must have aborted');
  if (test.kind === 'selftest.after') { await open(scratch); truth('后续独立用例可执行', (await digest()).byteLength > 0); return; }
  if (test.kind === 'smoke') {
    await open(scratch);
    await assert.rejects(() => call('unsupported'), /未知测试方法/); result.steps.at(-1).expectedFailure = true; result.assertions.push('未知命令明确失败');
    await roundTrip(scratch); return;
  }
  if (test.kind === 'corpus') {
    const path = join(root, sample.path), diskBefore = await hashFile(path);
    await open(path); await call('mode', { mode: 'semantic' });
    let initial;
    try { initial = await wait('stable'); }
    catch (error) {
      if (!String(error).includes('UNSUPPORTED_SEMANTIC')) throw error;
      result.status = 'blocked'; result.reason = String(error);
      initial = await call('diagnostics');
      result.screenshot = await host.screenshot('semantic-restricted.png').catch(() => undefined);
      await call('mode', { mode: 'source' }); await wait('stable');
    }
    await wait('resources');
    if (sample.level !== '异常') {
      const images = (await call('diagnostics')).images;
      check('正常样本没有可见图片加载失败', images.failed + (images.errorPlaceholders ?? 0), 0);
    }
    const baseline = await digest();
    for (const mode of ['source', 'split', 'semantic']) {
      await call('mode', { mode }); await wait('readable');
      check(`${mode} 模式正文不丢失`, (await digest()).sha256, baseline.sha256);
    }
    const state = await call('diagnostics');
    check('索引终态', state.document.progress.state, 'ready');
    truth('正文长度与摘要一致', state.document.byteLength === baseline.byteLength);
    const enhancements = state.document.progress.enhancements;
    const expected = sample.expected ?? {};
    result.expectationChecks = [];
    for (const [fixtureKey, metric] of Object.entries(enhancementExpectations)) if (expected[fixtureKey]) {
      const observed = enhancements[metric], complete = state.document.progress.metricsComplete;
      if (observed < expected[fixtureKey] && !complete) {
        result.expectationChecks.push({ key: fixtureKey, expected: expected[fixtureKey], observed, status: 'unavailable', reason: '原生增强计数明确标记为不完整，不能据此判断完整语义数量。' });
        continue;
      }
      truth(`索引计数 ${fixtureKey}: ${observed} >= ${expected[fixtureKey]}`, observed >= expected[fixtureKey]);
      result.expectationChecks.push({ key: fixtureKey, expected: expected[fixtureKey], observed, status: 'passed' });
    }
    if (expected.exact_bytes) {
      check('精确体量与应用正文长度一致', baseline.byteLength, expected.exact_bytes);
      result.expectationChecks.push({ key: 'exact_bytes', expected: expected.exact_bytes, observed: baseline.byteLength, status: 'passed' });
    }
    const outline = await call('outline');
    truth('大纲查询有修订身份', outline.documentId === state.document.documentId && outline.revision === state.document.revision);
    if (expected.headings_in_body) {
      let count = outline.items.length, cursor = outline.next;
      while (cursor) { const page = await call('outline', { cursor }); count += page.items.length; cursor = page.next; }
      truth('标题索引完整', count >= expected.headings_in_body);
      result.expectationChecks.push({ key: 'headings_in_body', expected: expected.headings_in_body, observed: count, status: 'passed' });
    }
    const tail = Math.max(0, baseline.byteLength - 256);
    // 字节位置向前对齐到实际文档窗口，由原生端返回合法 UTF-8 边界。
    await call('reveal', { byte: tail }); await wait('readable');
    await call('reveal', { byte: 0 }); await wait('readable');
    if (/缺失图片/.test(sample.path)) {
      const images = (await call('diagnostics')).images;
      truth('预期缺图被观测', images.failed + (images.errorPlaceholders ?? 0) > 0);
    }
    result.initialState = initial;
    await roundTrip(path);
    if (sample.level === '兼容') {
      // 重新打开后没有跨进程历史，移除已知插入前缀，再真实保存以验证编码/换行。
      await call('mode', { mode: 'source' }); await wait('editable');
      await call('select', { fromByte: 0, toByte: Buffer.byteLength(marker) }); await call('insert', { text: '' }); await call('save');
      check('编码与换行字节级保留', await hashFile(path), diskBefore);
    }
    return;
  }
  if (test.kind === 'editing.commands') {
    await writeFile(scratch, '# Editing\n\nAlpha beta\n\n'); await open(scratch); await wait('editable');
    for (const type of ['toggleBold', 'toggleItalic', 'toggleCode', 'toggleStrikethrough', 'toggleUnderline', 'toggleHighlight']) {
      await call('select', { fromByte: 11, toByte: 16 }); const before = await digest();
      await call('command', { command: { type } });
      assertMarkdown((await call('read')).markdown, `# Editing\n\n${inlineResults[type]} beta\n\n`, `${type} 精确正文`); result.assertions.push(`${type} 精确正文与未选中文字保留`);
      const selection = (await call('diagnostics')).selection;
      truth(`${type} 选区仍可读取`, selection && Number.isFinite(selection.anchorByte) && Number.isFinite(selection.headByte));
      await call('undo'); check(`${type} 撤销`, (await digest()).sha256, before.sha256);
    }
    await call('select', { fromByte: 0 }); const before = await digest();
    await call('paste', { text: '- [ ] clipboard task\n', plain: false });
    assertMarkdown((await call('read')).markdown, '- [ ] clipboard task\n# Editing\n\nAlpha beta\n\n', '粘贴准确内容与位置'); result.assertions.push('粘贴准确内容与位置');
    await call('undo'); check('粘贴可撤销', (await digest()).sha256, before.sha256);
    await call('new'); const created = await wait('editable'); truth('新建文档获得编辑视图', Boolean(created.dom.activeEditor));
    return;
  }
  if (test.kind === 'editing.blocks') {
    await writeFile(scratch, '# Blocks\n\nAlpha beta\n\n'); await open(scratch); await wait('editable');
    for (const command of [{ type: 'setHeading', level: 2 }, ...['toggleBlockquote', 'toggleBulletList', 'toggleOrderedList', 'toggleTaskList'].map(type => ({ type }))]) {
      await call('select', { fromByte: 10, toByte: 15 }); const before = await digest();
      await call('command', { command });
      assertMarkdown((await call('read')).markdown, `# Blocks\n\n${blockResults[command.type]}\n\n`, `${command.type} 精确块语法`); result.assertions.push(`${command.type} 精确块语法与其他段落保留`);
      await call('undo'); check(`${command.type} 完整撤销`, (await digest()).sha256, before.sha256);
    }
    return;
  }
  if (test.kind === 'editing.table') {
    const original = '# Table\n\n| A | B |\n| --- | --- |\n| cell_x | cell_y |\n\n';
    await writeFile(scratch, original); await open(scratch); await wait('editable');
    const before = await digest();
    for (const command of ['addTableRowAfter', 'addTableColumnAfter', 'deleteTableRow', 'deleteTableColumn', 'setTableColumnAlignment'].map(type => ({ type, ...(type === 'setTableColumnAlignment' ? { align: 'center' } : {}) }))) {
      const text = (await call('read')).markdown;
      // 表格结构命令要求真实语义光标；全局源码选区只支持文本替换类命令。
      await call('reveal', { byte: Buffer.byteLength(text.slice(0, text.indexOf('cell_x'))) });
      await call('command', { command }); truth(`${command.type} 改变表格`, (await digest()).sha256 !== before.sha256);
      const after = (await call('read')).markdown;
      if (command.type === 'addTableRowAfter') check('插入一行', after.split('\n').filter(l => l.startsWith('|')).length, 4);
      if (command.type === 'addTableColumnAfter') truth('新列写入 Markdown', after.split('\n').filter(l => l.startsWith('|')).every(l => (l.match(/\|/g) ?? []).length === 4));
      if (command.type === 'deleteTableRow') truth('删除目标行且保留表格外正文', !after.includes('cell_x') && !after.includes('cell_y') && after.startsWith('# Table\n'));
      if (command.type === 'deleteTableColumn') truth('删除目标列且保留相邻列正文', !after.includes('cell_x') && after.includes('cell_y') && after.startsWith('# Table\n'));
      if (command.type === 'setTableColumnAlignment') truth('居中对齐写入分隔符', /:\s*-+\s*:/.test(after));
      await call('undo'); check(`${command.type} 完整撤销`, (await digest()).sha256, before.sha256);
    }
    return;
  }
  if (test.kind === 'structure.outline') {
    await writeFile(scratch, '# Start\n\n' + 'Paragraph.\n\n'.repeat(100) + '## Target\n\nTail.\n');
    await open(scratch); const outline = await call('outline');
    const target = outline.items.find(i => i.title === 'Target'); truth('大纲包含目标标题', target);
    await call('outlineNavigate', { byte: target.byte, revision: outline.revision }); await wait('stable');
    const state = await call('diagnostics'); truth('大纲导航移动阅读位置', state.readingByte > 0);
    await edit(); await assert.rejects(() => call('outlineNavigate', { byte: target.byte, revision: outline.revision }), /修订无效/);
    result.steps.at(-1).expectedFailure = true; result.assertions.push('旧修订大纲定位被拒绝'); return;
  }
  if (test.kind === 'editing.native') {
    await writeFile(scratch, '# Keyboard\n\n' + 'Line for scrolling.\n\n'.repeat(120)); await open(scratch); await wait('editable');
    await call('reveal', { byte: 17 }); await wait('stable');
    const editorBefore = await call('diagnostics'), editors = editorBefore.dom.semanticEditors;
    const before = await digest(); await host.native({ text: 'NativeTyping42' });
    await pause(300); const after = await digest(); truth('OS 键盘输入进入真实编辑器', before.sha256 !== after.sha256);
    const editorAfter = await call('diagnostics');
    result.observations = [{ kind: 'editor-reuse', before: editors, after: editorAfter.dom.semanticEditors,
      unchanged: JSON.stringify(editors) === JSON.stringify(editorAfter.dom.semanticEditors),
      beforeViews: editorBefore.views, afterViews: editorAfter.views,
      note: '虚拟窗口重分组可能替换视图；作为性能诊断记录，视觉闪烁仍需人工验收。' }];
    await host.native({ keys: 'CTRL+Z' }); await pause(300); check('真实撤销快捷键', (await digest()).sha256, before.sha256);
    await host.native({ keys: 'CTRL+Y' }); await pause(300); check('真实重做快捷键', (await digest()).sha256, after.sha256);
    await host.native({ keys: 'PAGEDOWN' }); await wait('stable');
    truth('滚动后文档仍可编辑', (await call('diagnostics')).presentation.phase === 'editable');
    await host.native({ keys: 'CTRL+S' }); await pause(1000); truth('保存快捷键落盘', (await readFile(scratch, 'utf8')).includes('NativeTyping42'));
    return;
  }
  if (test.kind === 'search.behavior') {
    await writeFile(scratch, '# Search\n\nAlpha alpha alphabet\n\n' + 'TOKEN_42\n\n'.repeat(300)); await open(scratch);
    check('忽略大小写匹配', (await call('search', { query: 'alpha' })).count, 3);
    check('大小写敏感匹配', (await call('search', { query: 'Alpha', caseSensitive: true })).count, 1);
    check('全词匹配', (await call('search', { query: 'alpha', wholeWord: true })).count, 2);
    const page = await call('search', { query: 'TOKEN_42', fromIndex: 256 });
    check('大量命中总数', page.count, 300); truth('有界末页', page.page.matches.some(m => m.index === 299) && page.page.matches.length <= 128);
    check('搜索取消', (await call('searchCancel', { query: 'TOKEN_42' })).cancelled, true);
    const before = await digest(); check('全部替换数量', (await call('replaceAll', { query: 'TOKEN_42', replacement: 'REPLACED' })).count, 300);
    check('旧文本无残留', (await call('search', { query: 'TOKEN_42' })).count, 0);
    check('新修订查询命中', (await call('search', { query: 'REPLACED' })).count, 300);
    await call('undo'); check('批量替换一次撤销', (await digest()).sha256, before.sha256); return;
  }
  if (test.kind === 'lifecycle.tabs' || test.kind === 'lifecycle.comparison') {
    const ids = []; for (const s of test.samples) ids.push(await open(join(root, s.path)));
    if (ids.length < 2) { result.status = 'blocked'; result.reason = '数据集缺少成对标签样本'; return; }
    if (test.kind.endsWith('comparison')) {
      await call('combine', { first: ids[0], second: ids[1] }); truth('生成组合标签', (await call('diagnostics')).workspace.some(w => w.kind === 'comparison'));
      await call('ungroup'); truth('拆分恢复独立标签', !(await call('diagnostics')).workspace.some(w => w.kind === 'comparison')); return;
    }
    if (!host.cdp) { result.status = 'blocked'; result.reason = '阅读位置的实际正文锚点测量需要 CDP'; return; }
    await call('switch', { tabId: ids[0] }); await call('mode', { mode: 'semantic' }); await wait('editable');
    await call('select', { fromByte: 0, toByte: 1 });
    await call('reveal', { byte: Math.floor((await digest()).byteLength / 2) }); await wait('stable');
    const anchor = await call('diagnostics'), selection = anchor.selection;
    const beforeAnchors = await host.readingAnchors();
    for (let i = 0; i < config.cycles; i++) for (const id of ids) { const s = await call('switch', { tabId: id }); check('标签激活正确', s.activeTabId, id); }
    await call('switch', { tabId: ids[0] }); await wait('stable'); check('切回保留选区', (await call('diagnostics')).selection, selection);
    const restored = await call('diagnostics');
    const afterAnchors = await host.readingAnchors();
    const beforeViewport = beforeAnchors[0], afterViewport = afterAnchors[0];
    result.readingPosition = { before: beforeViewport, after: afterViewport, requestedByte: anchor.readingByte, restoredByte: restored.readingByte };
    // 虚拟高度会在测量后修正；比较相同正文在视口中的位置，而不是绝对 scrollTop 或导航意图。
    truth('切回保留实际阅读位置（同一正文锚点，2 CSS px 容差）', beforeViewport?.anchor && afterViewport?.anchor &&
      beforeViewport.anchor.text === afterViewport.anchor.text && beforeViewport.anchor.node === afterViewport.anchor.node &&
      Math.abs(beforeViewport.anchor.top - afterViewport.anchor.top) <= 2);
    for (let i = ids.length - 1; i >= 0; i--) { await call('switch', { tabId: ids[i] }); await call('close'); }
    const closed = await call('diagnostics'); truth('文档运行时释放', closed.runtimeResources.documentRuntimes <= 1); return;
  }
  if (test.kind.startsWith('persistence.')) {
    await open(scratch);
    if (test.kind.endsWith('autosave')) {
      await call('autosave', { enabled: true }); await edit();
      const deadline = Date.now() + 15000;
      while (Date.now() < deadline && !(await readFile(scratch, 'utf8')).includes(marker)) await pause(100);
      truth('自动保存确实落盘', (await readFile(scratch, 'utf8')).includes(marker)); return;
    }
    if (test.kind.endsWith('external')) {
      await writeFile(scratch, '# External update\n\nexternal_marker\n'); await call('externalCheck');
      await call('reload'); check('重新加载外部变更', (await call('search', { query: 'external_marker' })).count, 1); return;
    }
    await edit(); const expected = await digest();
    const graceful = test.kind.endsWith('restart');
    if (graceful) await call('save');
    await call('checkpoint');
    if (graceful) { await call('exit'); await host.waitExit(); }
    await host.stop(); await host.start(); await call('adoptRestored'); await wait('indexed'); await wait('readable');
    check(graceful ? '正常重启恢复文档' : '崩溃恢复未保存正文', (await digest()).sha256, expected.sha256);
    await call('save'); return;
  }
  if (test.kind === 'structure.directory') {
    // 目录负载中的文件通过真实文档入口逐个验证；目录 UI 加载由 folder 接口验证。
    await call('folder', { path: join(root, '09_目录树256文件') });
    const state = await call('diagnostics'); truth('目录树根节点已加载', state.runtimeResources.folderRoots > 0);
    for (const entry of test.samples) {
      await open(join(root, entry.path)); truth(`跨目录文档可用：${entry.path}`, (await digest()).byteLength > 0);
    }
    return;
  }
  if (test.kind === 'performance' || test.kind === 'soak.lifecycle') {
    if (test.kind === 'soak.lifecycle') await writeFile(scratch, '# Scroll workload\n\n' + 'Scrolling paragraph with stable text.\n\n'.repeat(300));
    const path = join(root, sample.path), cycles = config.cycles;
    let id = await open(path); await wait('stable');
    result.measurements = [];
    const measure = async phase => {
      const value = { phase, at: new Date().toISOString(), data: await host.capture() };
      if (result.measurements.length >= 256) { result.measurements.splice(1, 1); result.evictedMeasurements = (result.evictedMeasurements ?? 0) + 1; }
      result.measurements.push(value); await appendFile(join(host.directory, 'measurements.jsonl'), JSON.stringify(value) + '\n');
      return value.data;
    };
    await measure('initial');
    const started = Date.now(), deadline = started + config.soakMinutes * 60000;
    let nextProgress = started + 60000;
    let i = 0;
    do {
      await call('mode', { mode: 'source' }); await wait('editable'); const before = await digest();
      await call('select', { fromByte: 0 }); await call('insert', { text: marker }); await call('undo');
      check('循环编辑撤销保持全文', (await digest()).sha256, before.sha256);
      const other = await open(scratch); await call('switch', { tabId: id }); await call('switch', { tabId: other }); await call('close');
      await call('switch', { tabId: id }); await call('save'); await call('close'); id = await open(path);
      if (test.kind === 'soak.lifecycle') {
        // 真实语义输入与大范围滚动穿插在生命周期循环内，避免长测只有源码 API 调用。
        if (i % 3 === 0) await nativeRound();
        if (i % 3 === 1) {
          await call('mode', { mode: 'semantic' }); await wait('editable');
          const semanticBefore = await digest(); await call('reveal', { byte: 12 });
          await call('insert', { text: 'SemanticWorkload42' });
          truth('语义编辑改变正文', (await digest()).sha256 !== semanticBefore.sha256);
          await call('undo'); check('语义编辑撤销完整', (await digest()).sha256, semanticBefore.sha256);
        }
        if (i % 3 === 2) {
          const large = await open(scratch);
          await scrollRound(); await call('close'); await call('switch', { tabId: id });
        }
      }
      if (i % 5 === 0) await measure(`cycle-${i}`);
      i++; result.cycles = i;
      if (Date.now() >= nextProgress) {
        const progress = { cycles: i, elapsedMs: Date.now() - started, at: new Date().toISOString() };
        await saveJson(join(host.directory, 'progress.json'), progress);
        console.log(`进度 ${test.id}：${(progress.elapsedMs / 60000).toFixed(1)} 分钟，${i} 次完整循环`);
        nextProgress = Date.now() + 60000;
      }
      if (test.kind === 'soak.lifecycle') await pause(500);
    } while (test.kind === 'soak.lifecycle' ? Date.now() < deadline : i < cycles);
    result.cycles = i; await wait('stable'); await pause(1000);
    const idle = await measure('idle');
    truth('循环结束文档运行时没有随次数累积', idle.runtimeResources.documentRuntimes <= 2);
    return;
  }
  throw new Error(`没有场景实现：${test.kind}`);
}

import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
const output = resolve(process.argv[2] ?? '.artifacts/structure-index-20261024');
const read = name => readFile(join(output, name), 'utf8').then(JSON.parse);
const [kernel, structure, environment, corpus, tests] = await Promise.all([
  read('kernel-report.classified.json'), read('structure-comparison.json'), read('environment.json'), read('corpus-manifest.json'), read('adapter-tests.json'),
]);
const candidate = 'pulldown-cmark';
const suites = kernel.suites.filter(s => s.engine === candidate);
const facts = structure.summaries.find(s => s.engine === candidate);
const criteria = [
  { name: 'CommonMark 与所选 GFM 扩展', passed: suites.length === 2 && suites.every(s => s.semanticPassed === s.total), evidence: 'kernel-report.classified.json' },
  { name: 'Nomo 块范围、标题、有效定义及跨读取页一致性', passed: facts.total >= 49 && facts.allComparedFactsEqual === facts.total, evidence: 'structure-comparison.json' },
  { name: '全文引用优先于片段重复定义', passed: kernel.globalReferenceProbes.every(p => p.adaptedOutputsEqual), evidence: 'kernel-report.json' },
  { name: '有界原文降级、转义、依赖与 slug 回归', passed: tests.passed === true, evidence: 'adapter-tests.json' },
  { name: '局部重扫范围收敛', passed: kernel.structureAdapters.incrementalRangeConvergence.filter(p => p.engine === candidate).every(p => p.blockRangesMatchFull), evidence: 'kernel-report.json' },
  { name: '取消后不发布', passed: kernel.structureAdapters.cancellation.filter(p => p.engine === candidate).every(p => p.rounds.every(r => !r.publishedAfterCancel)), evidence: 'kernel-report.json' },
];
const passed = criteria.every(c => c.passed);
await writeFile(join(output, 'gate.json'), JSON.stringify({ syntaxVersion: 'nomo-markdown-v1', decision: passed ? 'selected-for-native-service' : 'not-qualified', selectedEngine: passed ? candidate : null,
  version: passed ? '0.13.5' : null, criteria, scope: 'Parser selection only. Native service, integration and desktop acceptance are separate gates.' }, null, 2) + '\n');
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const report = `# 结构索引解析器选型验证

${passed ? '**选定 pulldown-cmark 0.13.5，进入原生服务阶段。**' : '**仍有选型门槛未通过。**'} 此结论只涵盖解析器选型，不代表结构索引迁移或桌面验收完成。

## 环境

- ${environment.baseline} / ${environment.branch}
- ${environment.os} / ${environment.cpu}
- ${environment.rust} / ${environment.node} / pnpm ${environment.pnpm}
- ${environment.generatedAt}；独立 Release 工程，两个候选及传递依赖由 Cargo.lock 固定。

## 语法与结构

| 内核 | 语料 | HTML 字节一致 | DOM 等价 |
|---|---|---:|---:|
${kernel.suites.map(s => `| ${s.engine} | ${s.corpus} | ${s.passed}/${s.total} | ${s.semanticPassed}/${s.total} |`).join('\n')}

DOM 比较保留文本、代码空白、属性和注释，只消除有明确规则的序列化差异。原始 expected/actual 均可回查。

${structure.summaries.map(s => `- ${s.engine}：${s.allComparedFactsEqual}/${s.total} 组块范围、类型、标题文本／源位置、有效定义及 1/3/17/16384 字节读取结果一致。`).join('\n')}

pulldown 的 Nomo 适配源码直接位于 src-tauri/src/text_document/structure_index/，实验工程引用同一份实现。没有把裸库默认选项当作 Nomo 契约。

新增的语义修正：上/下标包含行内代码时，代码中的 ^/~ 不再提前结束外层格式。原来这类输入会干扰后续内容；现在保留代码字面内容和其余行内文本。前端局部 tokenizer 同步修正。用户文件不主动改写。

## 门槛

${criteria.map(c => `- ${c.passed ? '通过' : '未通过'}：${c.name}（${c.evidence}）`).join('\n')}

全局引用实验同时处理 broken-link callback 和已在片段内解析的重复定义；普通 inline link 不覆盖。取消只能在 32 KiB 候选返回后观察，不声称库内部可抢占。

## 有界处理成本

| 内核 | 输入 | 字节 | 三轮中位 ms | 新增分配峰值 bytes |
|---|---|---:|---:|---:|
${kernel.probes.boundedFragmentBenchmarks.map(b => `| ${b.engine} | ${b.name} | ${b.inputBytes} | ${median(b.rounds.map(r => r.parseAndRenderMs)).toFixed(3)} | ${Math.max(...b.rounds.map(r => r.peakAdditionalAllocationBytes))} |`).join('\n')}

这是解析和 HTML 输出的独立测量，不是 Nomo 输入响应、RSS、私有提交或 WebView 内存收益。32 KiB 输入不等于 32 KiB 分配。巨型块保留有界原文降级；不能将降级区域未发现标题解释为全文没有标题。

## 后续验收

原生服务的分页、增量身份、预算、生命周期，以及正文、大纲、引用、目录接入和桌面性能须分别验收。选型探针中的范围收敛不代表生产增量引擎已完成。

复现：pwsh -NoProfile -File scripts/structure-index-evaluation/run.ps1

${corpus.sources.map(s => `- [${s.name}](${s.source})：${s.sha256}`).join('\n')}
`;
await writeFile(join(output, 'REPORT.md'), report);
console.log(`Parser gate: ${passed ? 'selected-for-native-service' : 'not-qualified'}`);

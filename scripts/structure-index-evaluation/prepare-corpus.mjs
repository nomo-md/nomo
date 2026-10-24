// 官方规范语料仅写入指定输出目录，不复制到产品包，也不改变现有测试。
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';

const output = resolve(process.argv[2] ?? '.artifacts/structure-index-20261024');
await mkdir(output, { recursive: true });
const sources = [
  ['commonmark-0.31.2.json', 'https://spec.commonmark.org/0.31.2/spec.json'],
  ['gfm-spec.txt', 'https://raw.githubusercontent.com/github/cmark-gfm/master/test/spec.txt'],
];
const manifest = [];
for (const [name, url] of sources) {
  const path = join(output, name);
  let bytes;
  try { bytes = await readFile(path); }
  catch (error) {
    if (error.code !== 'ENOENT') throw error;
    const response = await fetch(url);
    if (!response.ok) throw new Error(`${response.status} downloading ${url}`);
    bytes = Buffer.from(await response.arrayBuffer());
    await writeFile(path, bytes);
  }
  manifest.push({ name, source: url, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length });
}
const spec = await readFile(join(output, 'gfm-spec.txt'), 'utf8');
let section = '', state = 'prose', markdown = '', html = '';
const examples = [];
for (const line of spec.replace(/\r\n/g, '\n').split('\n')) {
  if (state === 'prose') {
    if (/^#{1,6} /.test(line)) section = line.replace(/^#+ /, '');
    if (/^`{32} example(?:\s|$)/.test(line)) { state = 'markdown'; markdown = ''; html = ''; }
  } else if (state === 'markdown' && line === '.') state = 'html';
  else if (state === 'html' && /^`{32}$/.test(line)) {
    examples.push({ example: examples.length + 1, section, markdown: markdown.replaceAll('→', '\t'), html: html.replaceAll('→', '\t') });
    state = 'prose';
  } else if (state === 'markdown') markdown += `${line}\n`;
  else html += `${line}\n`;
}
if (state !== 'prose' || examples.length < 600) throw new Error('Invalid or incomplete GFM corpus');
await writeFile(join(output, 'gfm-0.29.json'), JSON.stringify(examples, null, 2) + '\n');
await writeFile(join(output, 'corpus-manifest.json'), JSON.stringify({ sources: manifest, gfmExamples: examples.length }, null, 2) + '\n');
console.log(`Corpus prepared: ${output}; GFM examples: ${examples.length}`);

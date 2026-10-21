import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// 固定内容及本地资源；只生成到一个全新目录，绝不覆盖实测中已经编辑的文件。
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const destination = resolve(
  process.argv[2] ?? resolve(repository, '.artifacts/markdown-loading-fixtures'),
);
await mkdir(dirname(destination), { recursive: true });
await mkdir(destination);
await mkdir(resolve(destination, 'media'));
const files = new Map();
const add = (name, text) =>
  files.set(name, Buffer.isBuffer(text) ? text : Buffer.from(text, 'utf8'));
const ordinary =
  '# 首屏标题\r\n\r\n保留中文、emoji 😀、尾部空格。  \r\n\r\n' +
  '+ 列表第一项\n+ 列表第二项\n  + 嵌套条目\n\n> 引用段落\n\n' +
  '<details>\n<summary>折叠内容</summary>\n\n## 折叠中的标题\n\n可编辑正文。\n\n</details>\n\n' +
  '[文末引用][tail]\n\n<!-- toc -->\n- [保留打开前的目录](#old)\n<!-- /toc -->\n\n' +
  '# 首屏标题\n\n[tail]: <https://example.com> "原始引用标题"\n\n尾部标记 END-OF-DOCUMENT  \n\n\n';
add('00-empty.md', '');
add('01-ordinary.md', ordinary);
add('02-classmap.md', await readFile(resolve(repository, 'CLASSMAP.md')));
add(
  '03-large-prose.md',
  '# 开头\n\n' +
    Array.from(
      { length: 12_000 },
      (_, index) => `段落 ${index + 1}。固定中文与 English text，完整保存尾部。\n\n`,
    ).join('') +
    'END-OF-DOCUMENT\n',
);
add(
  '04-table-dense.md',
  Array.from(
    { length: 400 },
    (_, index) =>
      `## 表格 ${index + 1}\n\n| 左列 | 右列 |\n| :--- | ---: |\n| 中文 | English |\n| 未改  | 保留 |\n\n`,
  ).join('') + 'END-OF-DOCUMENT\n',
);
add('media/128x128.png', await readFile(resolve(repository, 'assets/128x128.png')));
add(
  '05-enhancement-dense.md',
  Array.from(
    { length: 48 },
    (_, index) =>
      `## 增强区域 ${index + 1}\n\n\`\`\`typescript\nconst index = ${index};\nconsole.log(index);\n\`\`\`\n\n` +
      '$$\nx^2 + y^2 = z^2\n$$\n\n```mermaid\ngraph LR\n  A[输入] --> B[保存]\n```\n\n' +
      '![固定本地图片](media/128x128.png)\n\n',
  ).join('') + 'END-OF-DOCUMENT\n',
);
add('06-giant-paragraph.md', '中文😀English '.repeat(80_000) + '\n\nEND-OF-DOCUMENT\n');
add(
  '07-giant-table.md',
  '| 序号 | 内容 |\n| --- | --- |\n' +
    Array.from({ length: 8_000 }, (_, i) => `| ${i} | 必须完整保留 |\n`).join('') +
    '\nEND-OF-DOCUMENT\n',
);
add(
  '08-giant-list.md',
  Array.from({ length: 8_000 }, (_, i) => `- 第 ${i} 项\n  - 子项与续行\n`).join('') +
    '\nEND-OF-DOCUMENT\n',
);
add(
  '09-giant-html.md',
  '<div>\n' + '<p>原文保留的 HTML 正文。</p>\n'.repeat(8_000) + '</div>\n\nEND-OF-DOCUMENT\n',
);
add(
  '10-giant-details.md',
  '<details>\n<summary>巨型折叠区域</summary>\n\n' +
    '段落。代码中的 `</details>` 不能结束容器。\n\n'.repeat(5_000) +
    '</details>\n\nEND-OF-DOCUMENT\n',
);
for (const size of [400_000, 500_000, 500_001]) {
  const unit = 'a'.repeat(998) + '\n\n';
  add(`bytes-${size}.md`, unit.repeat(Math.floor(size / 1000)) + 'a'.repeat(size % 1000));
}
for (const count of [1_600, 2_000, 2_001]) add(`units-${count}.md`, '段落。\n\n'.repeat(count));
for (const count of [100, 128, 129])
  add(`enhancements-${count}.md`, '```js\nconst a = 1;\n```\n\n'.repeat(count));
add(
  'encoding-utf8-bom.md',
  Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(ordinary)]),
);
add(
  'encoding-utf16-le.md',
  Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(ordinary, 'utf16le')]),
);
add(
  'encoding-utf16-be.md',
  Buffer.concat([Buffer.from([0xfe, 0xff]), Buffer.from(ordinary, 'utf16le').swap16()]),
);
add(
  'encoding-gbk.md',
  Buffer.concat([
    Buffer.from('# GBK\r\n\r\n'),
    Buffer.from([0xd6, 0xd0, 0xce, 0xc4]),
    Buffer.from('  \r\n\nEND-OF-DOCUMENT\n\n'),
  ]),
);
const manifest = [];
for (const [name, bytes] of files) {
  await writeFile(resolve(destination, name), bytes, { flag: 'wx' });
  manifest.push({
    name,
    bytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  });
}
await writeFile(
  resolve(destination, 'manifest.json'),
  JSON.stringify({ version: 1, files: manifest }, null, 2) + '\n',
  { flag: 'wx' },
);
console.log(`${manifest.length} fixed fixtures created: ${destination}`);

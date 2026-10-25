// 从 Web 版依赖的 @lucide/svelte 导出 GPUI 使用的图标：
// 1. 把各图标组件内的 iconNode 还原为 SVG，写入 assets/icons/；
// 2. 生成 src/ui/icon_assets.rs，用 include_bytes! 把 SVG 编进二进制。
// 用法（仓库根目录）：node src-gpui/scripts/export_icons.mjs
// 图标版权：Lucide，ISC 许可，见 assets/icons/LICENSE。

import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const crateDir = join(here, '..');
// 包的 exports 未暴露 package.json，直接按仓库 node_modules 布局定位（pnpm 会把它链接到这里）。
const lucideDir = join(crateDir, '..', 'node_modules', '@lucide', 'svelte', 'dist', 'icons');

// 与 Web 版工具栏、外壳保持同名；新增条目需同时在 src/ui/components.rs 的 Icon 枚举中登记。
const ICONS = [
  'heading-1', 'heading-2', 'heading-3', 'bold', 'italic', 'strikethrough', 'underline', 'highlighter',
  'code-xml', 'link', 'quote', 'list', 'list-ordered', 'square-check', 'braces', 'table-2', 'sigma', 'minus',
  'search', 'book-open-text', 'columns-2', 'list-tree', 'sun', 'moon', 'file-text', 'folder', 'folder-open',
  'chevron-right', 'chevron-down', 'plus', 'x', 'arrow-up', 'arrow-down', 'case-sensitive', 'replace',
  'replace-all', 'panel-left', 'zoom-in', 'zoom-out', 'copy', 'check', 'pilcrow', 'superscript', 'subscript',
  'info', 'square-pen', 'text-quote', 'history',
  // 设置窗口
  'settings-2', 'palette', 'file-image', 'chart-column', 'sliders-horizontal', 'keyboard', 'settings', 'rotate-ccw',
];

function toSvg(nodes) {
  const children = nodes
    .map(([tag, attrs]) => {
      const attributes = Object.entries(attrs)
        .filter(([name]) => name !== 'key')
        .map(([name, value]) => `${name}="${value}"`)
        .join(' ');
      return `  <${tag} ${attributes}/>`;
    })
    .join('\n');
  // GPUI 把 SVG 当作单色蒙版并以文字颜色着色，描边颜色只需不透明。
  return `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">\n${children}\n</svg>\n`;
}

const outDir = join(crateDir, 'assets', 'icons');
mkdirSync(outDir, { recursive: true });
for (const name of ICONS) {
  const source = readFileSync(join(lucideDir, `${name}.svelte`), 'utf8');
  const match = source.match(/const iconNode = (\[.*?\]);\s*\n/s);
  if (!match) throw new Error(`无法解析 ${name} 的 iconNode`);
  writeFileSync(join(outDir, `${name}.svg`), toSvg(JSON.parse(match[1])));
}
writeFileSync(
  join(outDir, 'LICENSE'),
  readFileSync(join(lucideDir, '..', '..', 'LICENSE'), 'utf8'),
);

const entries = readdirSync(outDir)
  .filter((file) => file.endsWith('.svg'))
  .sort()
  .map((file) => `        "icons/${file}" => Some(include_bytes!("../../assets/icons/${file}")),`)
  .join('\n');
writeFileSync(
  join(crateDir, 'src', 'ui', 'icon_assets.rs'),
  `//! 由 scripts/export_icons.mjs 生成，请勿手改。图标来自 Lucide（ISC 许可）。\n\n` +
    `pub fn icon_bytes(path: &str) -> Option<&'static [u8]> {\n    match path {\n${entries}\n        _ => None,\n    }\n}\n\n` +
    `pub const ICON_PATHS: &[&str] = &[\n${readdirSync(outDir).filter((f) => f.endsWith('.svg')).sort().map((f) => `    "icons/${f}",`).join('\n')}\n];\n`,
);
console.log(`exported ${ICONS.length} icons`);

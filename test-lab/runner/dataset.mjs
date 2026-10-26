import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { lab, json, saveJson, walk, hashFile, hashJson, within, exists } from './common.mjs';

export async function importDataset(source, name) {
  if (!/^[a-zA-Z0-9_-]+$/.test(name)) throw new Error('数据集名称只允许字母、数字、下划线和短横线');
  const target = join(lab, 'fixtures', name);
  if (await exists(target)) throw new Error(`数据集已存在，使用新名称导入：${target}`);
  await mkdir(target, { recursive: true });
  // 整目录复制保留相对链接与资源；不修改来源、不创建硬链接。
  await cp(source, target, { recursive: true, dereference: false, errorOnExist: true, force: false });
  const files = await walk(target);
  const records = [];
  for (const file of files) records.push({ path: relative(target, file).replaceAll('\\', '/'), sha256: await hashFile(file) });
  if (!await exists(join(target, 'manifest.json'))) {
    const markdown = [];
    for (const f of records.filter(f => /\.(md|markdown)$/i.test(f.path))) {
      const bytes = await readFile(within(target, f.path));
      markdown.push({ ...f, bytes: bytes.length, group: '样本', level: '常规', encoding: 'auto', purpose: '用户自定义文档' });
    }
    await saveJson(join(target, 'manifest.json'), { markdown, assets: records.filter(f => !/\.(md|markdown)$/i.test(f.path)), source });
  }
  await saveJson(join(target, 'import.json'), { source, importedAt: new Date().toISOString(), files: records });
  return loadDataset(name);
}

export async function loadDataset(name = 'main') {
  const root = within(join(lab, 'fixtures'), name);
  const manifest = await json(join(root, 'manifest.json'));
  const all = [...manifest.markdown, ...manifest.assets];
  for (const item of all) {
    const file = within(root, item.path);
    if (await hashFile(file) !== item.sha256) throw new Error(`样本校验失败：${item.path}`);
  }
  return { name, root, manifest, fingerprint: hashJson(all.map(e => [e.path, e.sha256])),
    samples: manifest.markdown.filter(e => e.group === '样本'),
    tree: manifest.markdown.filter(e => e.path.startsWith('09_')) };
}

export async function prepareFixtures(dataset, directory, samples, { tree = false } = {}) {
  const root = join(directory, 'fixtures'); await mkdir(root, { recursive: true });
  const selected = new Map((tree ? [...samples, ...dataset.tree] : samples).map(item => [item.path, item]));
  for (const item of selected.values()) {
    const target = within(root, item.path); await mkdir(join(target, '..'), { recursive: true });
    await cp(within(dataset.root, item.path), target, { force: false, errorOnExist: true });
  }
  // 清单是资源依赖的上界，不猜测目录名或正文编码。准备耗时不计作应用操作耗时。
  for (const asset of dataset.manifest.assets) {
    const target = within(root, asset.path); await mkdir(join(target, '..'), { recursive: true });
    await cp(within(dataset.root, asset.path), target, { force: false, errorOnExist: true });
  }
  await writeFile(join(root, 'scratch.md'), '# Test scratch\n\nAlpha alpha\n\n任务列表\n');
  return root;
}

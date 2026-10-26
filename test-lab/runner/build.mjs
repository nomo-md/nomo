import { cp, mkdir } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { lab, repo, walk, hashFile, hashJson, exists, json, saveJson, run, stamp } from './common.mjs';

export async function sourceFingerprint() {
  const roots = ['src', 'src-tauri/src', 'src-tauri/capabilities', 'src-tauri/icons', 'project.inlang'];
  const files = [];
  for (const root of roots) files.push(...await walk(join(repo, root)));
  files.push(...['package.json', 'pnpm-lock.yaml', 'vite.config.ts', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs',
    'src-tauri/tauri.conf.json', 'src-tauri/tauri.windows.conf.json', 'src-tauri/tauri.automation.conf.json', 'scripts/typography-runtime-plugin.ts',
    'scripts/run-tauri.mjs', 'test-lab/runner/build.mjs', 'sample.md', 'assets/128x128.png'].map(p => join(repo, p)));
  const hashes = [];
  for (const file of files.sort()) if (!file.includes('paraglide') && !file.endsWith('.test.ts')) hashes.push([relative(repo, file).replaceAll('\\', '/'), await hashFile(file)]);
  return hashJson(hashes);
}

export async function buildApp(configuration = 'release') {
  const fingerprint = await sourceFingerprint();
  const directory = join(lab, '.cache', `build-${stamp()}`); await mkdir(directory, { recursive: true });
  const config = join(directory, 'tauri.json');
  await saveJson(config, { productName: 'Nomo Automation', identifier: 'com.nomo.automation', build: {
    beforeBuildCommand: 'pnpm build --outDir test-lab/.cache/dist', frontendDist: '../test-lab/.cache/dist' }, bundle: { active: false } });
  console.log(`构建 ${configuration} 测试版，日志：${join(directory, 'build.log')}`);
  await run(process.execPath, ['scripts/run-tauri.mjs', 'build', '--no-bundle', '--features', 'automation', '--config', config,
    ...(configuration === 'debug' ? ['--debug'] : [])], { log: join(directory, 'build.log'), env: {
    ...process.env, VITE_NOMO_AUTOMATION: '1', VITE_NOMO_ACCEPTANCE: '0', CARGO_TARGET_DIR: join(lab, '.cache', 'target') } });
  if (fingerprint !== await sourceFingerprint()) throw new Error('构建期间源码发生变化，请重新构建');
  const built = join(lab, '.cache', 'target', configuration, 'nomo.exe');
  const sha256 = await hashFile(built), id = `${configuration}-${sha256.slice(0, 12)}`;
  const target = join(lab, 'apps', id); await mkdir(join(target, 'samples/assets'), { recursive: true });
  await cp(built, join(target, 'nomo.exe'));
  await cp(join(repo, 'sample.md'), join(target, 'samples/sample.md'));
  await cp(join(repo, 'assets/128x128.png'), join(target, 'samples/assets/128x128.png'));
  const manifest = { protocol: 2, id, configuration, sourceFingerprint: fingerprint, sha256,
    builtAt: new Date().toISOString(), binary: join(target, 'nomo.exe'),
    commit: await run('git', ['rev-parse', 'HEAD']), dirty: Boolean(await run('git', ['status', '--porcelain'])),
    resources: { 'samples/sample.md': await hashFile(join(target, 'samples/sample.md')), 'samples/assets/128x128.png': await hashFile(join(target, 'samples/assets/128x128.png')) } };
  await saveJson(join(target, 'manifest.json'), manifest);
  await saveJson(join(lab, 'apps', `latest-${configuration}.json`), manifest);
  return manifest;
}
export async function ensureBuild(configuration, { noBuild = false, build = false } = {}) {
  const latest = join(lab, 'apps', `latest-${configuration}.json`);
  const manifest = await exists(latest) ? await json(latest) : null;
  if (build || !manifest || manifest.sourceFingerprint !== await sourceFingerprint()) {
    if (noBuild) throw new Error('没有与当前源码一致的测试构建，请执行 build 或移除 --no-build');
    return buildApp(configuration);
  }
  if (await hashFile(manifest.binary) !== manifest.sha256) throw new Error('测试程序校验失败');
  for (const [file, hash] of Object.entries(manifest.resources)) if (await hashFile(join(lab, 'apps', manifest.id, file)) !== hash) throw new Error(`程序资源校验失败：${file}`);
  return manifest;
}

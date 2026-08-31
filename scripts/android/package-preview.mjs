import { createHash } from 'node:crypto';
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { createSearchFixture } from './create-search-fixture.mjs';

const sourceRoot = resolve('src-tauri/gen/android/app/build/outputs/apk');
function apkFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? apkFiles(path) : entry.name.endsWith('.apk') ? [path] : [];
  });
}
const head = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!process.env.PR_HEAD_SHA || head !== process.env.PR_HEAD_SHA) {
  throw new Error('APK checkout must match the exact PR head SHA');
}
const apks = apkFiles(sourceRoot).filter((path) => /debug/i.test(path));
if (apks.length !== 1) throw new Error(`Expected one ARM64 debug APK, got ${apks.length}`);
const aapt = join(process.env.ANDROID_HOME, 'build-tools', '36.0.0', 'aapt');
const badging = execFileSync(aapt, ['dump', 'badging', apks[0]], { encoding: 'utf8' });
if (
  !badging.includes("package: name='com.nomo.desktop.pr44'") ||
  !badging.includes("application-label:'Nomo PR44'") ||
  !badging.includes("native-code: 'arm64-v8a'")
) {
  throw new Error('Preview identity, label or ABI is not isolated as expected');
}
const output = resolve('.artifacts/android-preview');
mkdirSync(output, { recursive: true });
const name = `Nomo-PR44-${head.slice(0, 12)}-arm64-debug.apk`;
copyFileSync(apks[0], join(output, name));
const hash = createHash('sha256').update(readFileSync(apks[0])).digest('hex');
writeFileSync(join(output, 'SHA256SUMS.txt'), `${hash}  ${name}\n`);
writeFileSync(
  join(output, 'BUILD.txt'),
  `PR: https://github.com/nomo-md/nomo/pull/44\nCommit: ${head}\nApplication ID: com.nomo.desktop.pr44\nSigning: ephemeral CI debug key; no release credentials\nNot a release. Phone acceptance is still required.\n`,
);
copyFileSync('docs/android-pr44-checklist.zh-CN.md', join(output, 'CHECKLIST.md'));
copyFileSync('docs/android-pr44-validation.md', join(output, 'TECHNICAL-NOTES.md'));
const fixture = resolve('.artifacts/android-search-fixture');
createSearchFixture(fixture);
execFileSync('zip', ['-q', '-r', '-9', join(output, 'search-fixture.zip'), '.'], { cwd: fixture });

import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { mkdir, readdir, readFile, writeFile, stat } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { dirname, resolve, relative, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
export const lab = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const repo = resolve(lab, '..');
export const pause = ms => new Promise(resolvePause => setTimeout(resolvePause, ms));
export const stamp = () => new Date().toISOString().replace(/[:.]/g, '-');
export const json = async path => JSON.parse((await readFile(path, 'utf8')).replace(/^\uFEFF/, ''));
export async function saveJson(path, value) { await mkdir(dirname(path), { recursive: true }); await writeFile(path, JSON.stringify(value, null, 2) + '\n'); }
export async function exists(path) { try { await stat(path); return true; } catch (e) { if (e.code !== 'ENOENT') throw e; return false; } }
export function within(root, path) {
  const destination = resolve(root, path), rel = relative(resolve(root), destination);
  if (rel === '..' || rel.startsWith('..\\') || rel.startsWith('../') || isAbsolute(rel)) throw new Error(`路径超出目录：${path}`);
  return destination;
}
export async function hashFile(path) { const hash = createHash('sha256'); for await (const chunk of createReadStream(path)) hash.update(chunk); return hash.digest('hex'); }
const canonical = value => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
export const hashJson = value => createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
/** TOC 是保存时重建的派生内容；流式校验其外部字节，不能把整份正文变化都放行。 */
export async function hashOutsideToc(path, prefix = '') {
  let open = Buffer.from('<!-- toc -->'), close = Buffer.from('<!-- /toc -->');
  const hash = createHash('sha256');
  let pending = Buffer.alloc(0), inside = false, blocks = 0, initialized = false;
  for await (const chunk of createReadStream(path)) {
    let bytes = chunk;
    if (!initialized) {
      initialized = true;
      const little = chunk[0] === 0xff && chunk[1] === 0xfe, big = chunk[0] === 0xfe && chunk[1] === 0xff;
      const bom = little || big ? 2 : chunk.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf])) ? 3 : 0;
      const encode = text => {
        const encoded = Buffer.from(text, little || big ? 'utf16le' : 'utf8');
        return big ? encoded.swap16() : encoded;
      };
      open = encode('<!-- toc -->'); close = encode('<!-- /toc -->');
      hash.update(chunk.subarray(0, bom)).update(encode(prefix)); bytes = chunk.subarray(bom);
    }
    pending = Buffer.concat([pending, bytes]);
    while (pending.length) {
      const marker = inside ? close : open, at = pending.indexOf(marker);
      if (at >= 0) {
        if (!inside) hash.update(pending.subarray(0, at)).update(open).update(close);
        else blocks++;
        inside = !inside; pending = pending.subarray(at + marker.length);
      } else {
        const length = Math.max(0, pending.length - marker.length + 1);
        if (!inside) hash.update(pending.subarray(0, length));
        pending = pending.subarray(length); break;
      }
    }
  }
  if (!initialized) hash.update(prefix);
  if (inside) throw new Error('TOC 标记未闭合，不能使用派生内容豁免');
  hash.update(pending);
  return { sha256: hash.digest('hex'), blocks };
}
export async function walk(root) {
  const files = [];
  for (const e of await readdir(root, { withFileTypes: true })) {
    if (e.isSymbolicLink()) throw new Error(`不跟随样本或源码中的符号链接：${join(root, e.name)}`);
    if (e.isDirectory()) files.push(...await walk(join(root, e.name))); else if (e.isFile()) files.push(join(root, e.name));
  }
  return files.sort();
}
export function launch(executable, args, { log, env = process.env, cwd = repo } = {}) {
  const stream = log ? createWriteStream(log, { flags: 'a' }) : null;
  const child = spawn(executable, args, { env, cwd, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  for (const source of [child.stdout, child.stderr]) source.on('data', data => { if (stream) stream.write(data); else output = (output + data).slice(-16000); });
  child.finished = new Promise(resolveExit => {
    child.once('error', error => { child.launchError = error; });
    child.once('close', (code, signal) => { stream?.end(); resolveExit({ code, signal, output, error: child.launchError?.message }); });
  });
  return child;
}
export async function run(executable, args, options) {
  const child = launch(executable, args, options), result = await child.finished;
  if (result.code !== 0 || result.error) throw new Error(`${executable} 失败 (${result.code}): ${result.error ?? result.output ?? options?.log}`);
  return result.output.trim();
}
export function quantiles(values) {
  const a = values.filter(Number.isFinite).sort((x, y) => x - y);
  if (!a.length) return { n: 0, min: null, median: null, max: null, p95: null, p99: null };
  return { n: a.length, min: a[0], median: a.length % 2 ? a[(a.length - 1) / 2] : (a[a.length / 2 - 1] + a[a.length / 2]) / 2,
    max: a.at(-1), p95: a.length >= 20 ? a[Math.ceil(a.length * .95) - 1] : null, p99: a.length >= 100 ? a[Math.ceil(a.length * .99) - 1] : null };
}

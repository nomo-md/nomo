import { closeSync, mkdirSync, openSync, writeFileSync, writeSync } from 'node:fs';
import { join } from 'node:path';

/** Disposable acceptance inputs only; never reads or changes application/user documents. */
export function createSearchFixture(root) {
  mkdirSync(root, { recursive: true });
  for (let index = 0; index < 199; index += 1) {
    writeFileSync(
      join(root, `recent-${String(index).padStart(3, '0')}.md`),
      `# Recent ${index}\n\n中文目标：${index}\nUnicode: Ａé中😀XYZ\n`,
      { flag: 'wx' },
    );
  }
  const fd = openSync(join(root, 'large-100MiB.txt'), 'wx');
  const chunk = Buffer.from('test line\n'.repeat(6554).slice(0, 65536));
  try {
    for (let offset = 0; offset < 100 * 1024 * 1024; offset += chunk.length) {
      writeSync(fd, chunk, 0, chunk.length, offset);
    }
    writeSync(fd, Buffer.from('CROSS_BOUNDARY'), 0, 14, 65536 - 5);
    writeSync(fd, Buffer.from('NOMO_TAIL_HIT'), 0, 13, 100 * 1024 * 1024 - 13);
  } finally {
    closeSync(fd);
  }
}

// 仅将已知的序列化等价差异归类；保留原始 expected/actual，不能隐藏语法差异。
import { JSDOM } from 'jsdom';
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';

const block = new Set(['DIV', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'TABLE', 'THEAD', 'TBODY', 'TR', 'TD', 'TH',
  'P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'PRE', 'HR', 'SCRIPT']);
function canonical(html) {
  const fragment = JSDOM.fragment(html);
  function visit(node, verbatim = false) {
    if (node.nodeType === 3) {
      const text = node.data;
      if (!verbatim && !text.trim() && (!node.parentElement || ['UL', 'OL', 'TABLE', 'THEAD', 'TBODY', 'TR'].includes(node.parentElement.tagName)
        || block.has(node.previousSibling?.tagName) || block.has(node.nextSibling?.tagName))) return null;
      return ['text', verbatim ? text : text.replace(/[ \t\r\n]+/g, ' ')];
    }
    if (node.nodeType === 8) return ['comment', node.data];
    if (node.nodeType === 11) return [...node.childNodes].map(child => visit(child)).filter(Boolean);
    if (node.nodeType !== 1) return null;
    const tag = node.tagName.toLowerCase();
    if (tag === 'tbody' && !node.children.length && !node.textContent.trim()) return null;
    const attrs = [...node.attributes].map(({ name, value }) => [name, value]);
    const style = attrs.find(item => item[0] === 'style');
    if ((tag === 'td' || tag === 'th') && style && /^text-align:\s*(left|right|center);?$/.test(style[1])) {
      attrs.splice(attrs.indexOf(style), 1);
      attrs.push(['align', style[1].match(/left|right|center/)[0]]);
    }
    return [tag === 's' ? 'del' : tag, attrs.sort((a, b) => a[0].localeCompare(b[0])),
      [...node.childNodes].map(child => visit(child, verbatim || tag === 'pre' || tag === 'code')).filter(Boolean)];
  }
  return JSON.stringify(visit(fragment));
}

// 比较器不得把缺失标记、空格或代码空白归为等价。
assert.notEqual(canonical('<p>*£*bravo.</p>'), canonical('<p><em>£</em>bravo.</p>'));
assert.notEqual(canonical('<p>a b</p>'), canonical('<p>ab</p>'));
assert.notEqual(canonical('<pre><code>a\nb</code></pre>'), canonical('<pre><code>a b</code></pre>'));
assert.equal(canonical('<p>&quot;x&quot;</p>\n'), canonical('<p>"x"</p>\n'));
assert.equal(canonical('<table>\n<tr><td>x</td></tr>\n</table>'), canonical('<table><tbody><tr><td>x</td></tr></tbody></table>'));

const path = process.argv[2];
if (!path) throw new Error('Usage: node compare-html.mjs <kernel-report.json>');
const report = JSON.parse(await readFile(path, 'utf8'));
for (const suite of report.suites) {
  suite.serializationOnly = [];
  suite.semanticDifferences = [];
  for (const difference of suite.differences) {
    (canonical(difference.expected) === canonical(difference.actual) ? suite.serializationOnly : suite.semanticDifferences).push(difference);
  }
  suite.semanticPassed = suite.passed + suite.serializationOnly.length;
  console.log(`${suite.engine} ${suite.corpus}: ${suite.semanticPassed}/${suite.total} DOM-equivalent; ${suite.semanticDifferences.length} remaining`);
}
await writeFile(path.replace(/\.json$/, '.classified.json'), JSON.stringify(report, null, 2) + '\n');

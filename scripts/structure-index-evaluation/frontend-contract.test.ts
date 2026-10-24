// @vitest-environment jsdom
import { it, expect } from 'vitest';
import { readFileSync, writeFileSync } from 'node:fs';
import { createMarkdownTokenizer } from '../../src/lib/editor-core/markdownTokenizer';
const output = process.env.NOMO_STRUCTURE_EVALUATION_OUTPUT;
it.skipIf(!output)('前端局部 GFM 扩展与规范的链接、表格和删除线事实一致', () => {
  const examples = JSON.parse(readFileSync(`${output}/gfm-0.29.json`,'utf8')) as Array<{example:number;section:string;markdown:string;html:string}>;
  const tokenizer = createMarkdownTokenizer();
  const facts = (html:string) => {
    const root=document.createElement('div');root.innerHTML=html;
    return {links:[...root.querySelectorAll('a')].map(a=>[a.getAttribute('href'),a.textContent]),
      tables:[...root.querySelectorAll('table')].map(table=>[...table.querySelectorAll('tr')].map(tr=>[...tr.children].map(td=>td.textContent?.trim()))),
      deletion:[...root.querySelectorAll('del,s')].map(node=>node.textContent)};
  };
  const results=examples.filter(e=>['Tables (extension)','Strikethrough (extension)','Autolinks (extension)'].includes(e.section))
    .map(e=>({...e,expected:facts(e.html),actual:facts(tokenizer.render(e.markdown))}));
  writeFileSync(`${output}/frontend-contract.json`,JSON.stringify(results,null,2));
  for (const result of results) expect(result.actual,`${result.section} #${result.example}`).toEqual(result.expected);
});

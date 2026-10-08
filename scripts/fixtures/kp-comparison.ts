import { createDomTypography } from '../../src/lib/typography/dom';
import type { TypographyOptions } from '../../src/lib/typography/types';

// Public-domain work; transcription is kept separate from our authored stress samples.
// Source: Wikisource, https://zh.wikisource.org/zh-hans/拿来主义 (retrieved 2026-10-08).
const article = [
  '中国一向是所谓“闭关主义”，自己不去，别人也不许来。自从给枪炮打破了大门之后，又碰了一串钉子，到现在，成了什么都是“送去主义”了。别的且不说罢，单是学艺上的东西，近来就先送一批古董到巴黎去展览，但终“不知后事如何”；还有几位“大师”们捧著几张古画和新画，在欧洲各国一路的挂过去，叫作“发扬国光”。听说不远还要送梅兰芳博士到苏联去，以催进“象征主义”，此后是顺便到欧洲传道。我在这里不想讨论梅博士演艺和象征主义的关系，总之，活人替代了古董，我敢说，也可以算得显出一点进步了。',
  '但我们没有人根据了“礼尚往来”的仪节，说道：拿来！',
  '当然，能够只是送出去，也不算坏事情，一者见得丰富，二者见得大度。尼采就自诩过他是太阳，光热无穷，只是给与，不想取得。然而尼采究竟不是太阳，他发了疯。中国也不是，虽然有人说，掘起地下的煤来，就足够全世界几百年之用，但是，几百年之后呢？几百年之后，我们当然是化为魂灵，或上天堂，或落了地狱，但我们的子孙是在的，所以还应该给他们留下一点礼品。要不然，则当佳节大典之际，他们拿不出东西来，只好磕头贺喜，讨一点残羹冷炙做奖赏。这种奖赏，不要误解为“抛来”的东西，这是“抛给”的，说得冠冕些，可以称之为“送来”，我在这里不想举出实例。',
  '我在这里也并不想对于“送去”再说什么，否则太不“摩登”了。我只想鼓吹我们再吝啬一点，“送去”之外，还得“拿来”，是为“拿来主义”。',
  '但我们被“送来”的东西吓怕了。先有英国的鸦片，德国的废枪炮，后有法国的香粉，美国的电影，日本的印著“完全国货”的各种小东西。于是连清醒的青年们，也对于洋货发生了恐怖。其实，这正是因为那是“送来”的，而不是“拿来”的缘故。',
  '所以我们要运用脑髓，放出眼光，自己来拿！',
  '譬如罢，我们之中的一个穷青年，因为祖上的阴功（姑且让我这么说说罢），得了一所大宅子，且不问他是骗来的，抢来的，或合法继承的，或是做了女婿换来的。那么，怎么办呢？我想，首先是不管三七二十一，“拿来”！但是，如果反对这宅子的旧主人，怕给他的东西染污了，徘徊不敢走进门，是孱头；勃然大怒，放一把火烧光，算是保存自己的清白，则是昏蛋。不过因为原是羡慕这宅子的旧主人的，而这回接受一切，欣欣然的蹩进卧室，大吸剩下的鸦片，那当然更是废物。“拿来主义”者是全不这样的。',
  '他占有，挑选。看见鱼翅，并不就抛在路上以显其“平民化”，只要有养料，也和朋友们像萝卜白菜一样的吃掉，只不用它来宴大宾；看见鸦片，也不当众摔在毛厕里，以见其彻底革命，只送到药房里去，以供治病之用，却不弄“出售存膏，售完即止”的玄虚。只有烟枪和烟灯，虽然形式和印度，波斯，阿剌伯的烟具都不同，确可以算是一种国粹，倘使背著周游世界，一定会有人看，但我想，除了送一点进博物馆之外，其馀的是大可以毁掉的了。还有一群姨太太，也大可以请她们各自走散为是，要不然，“拿来主义”怕未免有些危机。',
  '总之，我们要拿来。我们要或使用，或存放，或毁灭。那么，主人是新主人，宅子也就会成为新宅子。然而首先要这人沉著，勇猛，有辨别，不自私。没有拿来的，人不能自成为新人，没有拿来的，文艺不能自成为新文艺。',
];
const mixed = [
  '在Nomo中读一段中文，遇到Knuth–Plass、Unicode和OpenType时，视觉间距就开始影响阅读。相同的18px字体与410px栏宽，会留下不同的候选断点；“这一行刚好放得下吗？”不能只看最后一个字，还要看前后几行的空白分布。这里的数字是测试内容，不是性能测量结果。',
  '排版编辑说：“请保留《中文排版》（修订版）里的原文，不要为了对齐而插入空格。”校对补充道：“引号后接句号，句号后接括号，究竟如何处理？”他停了一下……然后写下——标点可以调整位置，字形不能被挤扁；一个段落也不应该只剩孤零零的尾字。',
  'A good paragraph has a rhythm. The browser chooses its own line breaks, while this implementation considers the paragraph as a whole. Keep punctuation close to its words, compare the final short line, and try changing the column width. The discretionary spelling repre\u00adsentation includes a soft hyphen; the unit 10\u00a0kg contains a non-breaking space.',
  '繁體中文：「這是一段包含English與2026年數字的混排樣本。」請觀察書名號《測試》、括號（附註）、省略號……與破折號——是否留在合適的位置。emoji 👩‍👩‍👧‍👦 與組合字元a\u0301也應保持完整。這組內容是專為本頁編寫的測試文字，不屬於魯迅原文。',
];

const byId = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id)! as T;
const left = byId('kp-text');
const right = byId('native-text');
const configuration: TypographyOptions = { enabled: true, profile: 'zh-CN', hanging: false };
let paragraphs = article;
let controller: ReturnType<typeof createDomTypography> | undefined;
let version = 0;
let resizing: ReturnType<typeof setTimeout>;
const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

function mount() {
  controller?.destroy();
  paragraphs = byId<HTMLSelectElement>('sample').value === 'article' ? article : mixed;
  for (const root of [left, right]) {
    root.replaceChildren();
    paragraphs.forEach((text, index) => {
      const section = document.createElement('section');
      const note = document.createElement('div');
      note.className = 'paragraph-note';
      const label = document.createElement('span');
      label.textContent = `段 ${String(index + 1).padStart(2, '0')}`;
      const result = document.createElement('span');
      result.className = 'paragraph-result';
      note.append(label, result);
      const paragraph = document.createElement('p');
      paragraph.textContent = text;
      if (root === right) paragraph.style.textAlign = byId<HTMLSelectElement>('alignment').value;
      section.append(note, paragraph);
      root.append(section);
    });
  }
  if (paragraphs === article) {
    byId('article-title').textContent = '拿来主义';
    byId('source').innerHTML =
      '鲁迅 · 1934年6月4日 · 全文（公有领域） · <a href="https://zh.wikisource.org/zh-hans/%E6%8B%BF%E6%9D%A5%E4%B8%BB%E4%B9%89" target="_blank" rel="noopener noreferrer">维基文库原文 ↗</a>';
  } else {
    byId('article-title').textContent = '中英文混排压力样本';
    byId('source').textContent = '本页自编 · 英文、数字、禁则、NBSP、软连字符与字素边界';
  }
  controller = createDomTypography(left, configuration);
  void refresh();
}

/** Reads actual rendered line ends; it does not influence either layout. */
function inspectLines(paragraph: HTMLElement) {
  const lines: { top: number; right: number; bottom: number; end: number }[] = [];
  const walker = document.createTreeWalker(paragraph, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  const segmenter = new Intl.Segmenter('zh', { granularity: 'grapheme' });
  let node: Node | null;
  let offset = 0;
  let source = '';
  while ((node = walker.nextNode())) {
    if (node.parentElement?.closest('[data-kp-owned]')) continue;
    const text = node.textContent ?? '';
    source += text;
    for (const unit of segmenter.segment(text)) {
      range.setStart(node, unit.index);
      range.setEnd(node, unit.index + unit.segment.length);
      const rect = range.getBoundingClientRect();
      const end = offset + unit.index + unit.segment.length;
      if (!rect.height) continue;
      const previous = lines.at(-1);
      if (previous && Math.abs(previous.top - rect.top) < 3) {
        previous.right = rect.right;
        previous.bottom = Math.max(previous.bottom, rect.bottom);
        previous.end = end;
      } else lines.push({ top: rect.top, right: rect.right, bottom: rect.bottom, end });
    }
    offset += text.length;
  }
  return { lines, source };
}

function updateReadout() {
  const lp = [...left.querySelectorAll<HTMLElement>('p')];
  const rp = [...right.querySelectorAll<HTMLElement>('p')];
  let changed = 0,
    fallback = 0,
    preserved = true,
    kpLines = 0,
    nativeLines = 0;
  const overlays = [byId('kp-marks'), byId('native-marks')];
  overlays.forEach((overlay) => {
    overlay.replaceChildren();
    overlay.hidden = !byId<HTMLInputElement>('markers').checked;
  });
  lp.forEach((paragraph, index) => {
    const measurements = [inspectLines(paragraph), inspectLines(rp[index])];
    const different =
      JSON.stringify(measurements[0].lines.map((line) => line.end)) !==
      JSON.stringify(measurements[1].lines.map((line) => line.end));
    if (different) changed++;
    const isFallback = paragraph.dataset.kpLayout !== 'ready';
    if (isFallback) fallback++;
    preserved &&= measurements.every((measurement) => measurement.source === paragraphs[index]);
    kpLines += measurements[0].lines.length;
    nativeLines += measurements[1].lines.length;
    measurements.forEach((measurement, side) => {
      const element = side ? rp[index] : paragraph;
      const result = element.parentElement!.querySelector('.paragraph-result')!;
      result.textContent = `${measurement.lines.length} 行${!side && isFallback ? ` · 回退：${paragraph.dataset.kpReason}` : different ? ' · 断点不同' : ' · 断点相同'}`;
      result.className = `paragraph-result ${!side && isFallback ? 'fallback' : different ? 'different' : ''}`;
      const bounds = overlays[side].getBoundingClientRect();
      measurement.lines.forEach((line) => {
        const mark = document.createElement('i');
        mark.className = 'line-mark';
        mark.style.left = `${line.right - bounds.left + 3}px`;
        mark.style.top = `${line.bottom - bounds.top - 7}px`;
        overlays[side].append(mark);
      });
    });
  });
  const status = byId('status');
  status.classList.toggle('error', !!fallback || !preserved);
  status.textContent = `KP ${lp.length - fallback}/${lp.length} 段完成${fallback ? ` · ${fallback} 段回退` : ''} · ${preserved ? '两侧原文一致' : '文本保真检查失败'}`;
  byId('geometry').textContent =
    `实际栏宽 ${left.getBoundingClientRect().width.toFixed(1)} px · 字号 ${getComputedStyle(left).fontSize}`;
  byId('difference').textContent =
    `${changed}/${lp.length} 段断点不同 · KP ${kpLines} 行 / 浏览器 ${nativeLines} 行`;
}

async function refresh() {
  const token = ++version;
  byId('status').textContent = '正在重排…';
  byId('status').classList.remove('error');
  try {
    await document.fonts.ready;
    await frame();
    if (token !== version) return;
    controller?.invalidate();
    await controller?.flush();
    await frame();
    if (token === version) updateReadout();
  } catch (error) {
    if (token !== version) return;
    byId('status').textContent =
      `排版未完成：${error instanceof Error ? error.message : String(error)}`;
    byId('status').classList.add('error');
  }
}

byId('sample').addEventListener('change', mount);
for (const name of ['width', 'size'])
  byId(name).addEventListener('input', () => {
    const value = byId<HTMLInputElement>(name).value;
    document.documentElement.style.setProperty(
      name === 'width' ? '--column' : '--size',
      `${value}px`,
    );
    byId(`${name}-value`).textContent = `${value} px`;
    void refresh();
  });
byId('font').addEventListener('change', () => {
  document.documentElement.style.setProperty(
    '--body-font',
    byId<HTMLSelectElement>('font').value === 'serif'
      ? "'SimSun', 'Songti SC', serif"
      : "'Microsoft YaHei', 'PingFang SC', sans-serif",
  );
  void refresh();
});
byId('alignment').addEventListener('change', () => {
  const align = byId<HTMLSelectElement>('alignment').value;
  right.querySelectorAll<HTMLElement>('p').forEach((paragraph) => {
    paragraph.style.textAlign = align;
  });
  byId('native-caption').textContent =
    align === 'justify' ? '自动断行 · CSS 两端对齐 · 末行靠左' : '自动断行 · 浏览器默认左对齐';
  void refresh();
});
byId('hanging').addEventListener('change', () => {
  configuration.hanging = byId<HTMLInputElement>('hanging').checked;
  void refresh();
});
byId('markers').addEventListener('change', updateReadout);
window.addEventListener('resize', () => {
  clearTimeout(resizing);
  resizing = setTimeout(() => {
    void refresh();
  }, 180);
});
window.addEventListener('pagehide', () => controller?.destroy());
mount();

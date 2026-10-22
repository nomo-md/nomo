/** 缓存最终 SVG 字符串，不保留图形 DOM、渲染 Promise 或视图闭包。 */
export class MermaidSvgCache {
  private entries = new Map<string, { svg: string; token: string; bytes: number }>();
  private bytes = 0;
  private sequence = 0;
  constructor(readonly maxEntries = 16, readonly maxBytes = 8 * 1024 * 1024) {}
  get size() { return this.entries.size; }
  get estimatedBytes() { return this.bytes; }
  clear() { this.entries.clear(); this.bytes = 0; }
  get(key: string, id: string) {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.svg.split(entry.token).join(`${id}-`);
  }
  put(key: string, svg: string) {
    if ((key.length + svg.length) * 2 > this.maxBytes) return;
    const template = document.createElement('template');
    template.innerHTML = svg;
    if (!template.content.querySelector('svg')) return;
    // 每份 SVG 都有独立 ID；箭头、剪裁、样式和无障碍引用随之更新。
    let token = `nomo-cache-${++this.sequence}-`;
    while (svg.includes(token)) token += 'x';
    const ids = new Map<string, string>();
    for (const element of template.content.querySelectorAll('[id]')) {
      const id = element.id;
      if (!ids.has(id)) ids.set(id, `${token}${ids.size}`);
      element.id = ids.get(id)!;
    }
    const urls = (value: string) => value.replace(/url\(\s*(['"]?)([^)'"\s]*#)([^)'"\s]+)\1\s*\)/g,
      (match, quote: string, base: string, id: string) => ids.has(id) ? `url(${quote}${base}${ids.get(id)}${quote})` : match);
    for (const element of template.content.querySelectorAll('*')) {
      for (const attribute of [...element.attributes]) {
        let value = urls(attribute.value);
        if (attribute.localName === 'href' && value.startsWith('#') && ids.has(value.slice(1)))
          value = `#${ids.get(value.slice(1))}`;
        if (['aria-labelledby', 'aria-describedby'].includes(attribute.name))
          value = value.split(/\s+/).map(id => ids.get(id) ?? id).join(' ');
        if (value !== attribute.value) element.setAttributeNS(attribute.namespaceURI, attribute.name, value);
      }
      if (element.localName === 'style') {
        // 只改选择器中的 ID，不误改声明中的 #fff 等颜色值。
        element.textContent = urls(element.textContent ?? '').replace(/(^|})([^{}]+)\{/g,
          (_match, start: string, selectors: string) => start + selectors.replace(/#([\w-]+)/g,
            (match, id: string) => ids.has(id) ? `#${ids.get(id)}` : match) + '{');
      }
    }
    const result = template.innerHTML;
    const bytes = 128 + (key.length + result.length + token.length) * 2;
    const previous = this.entries.get(key);
    if (previous) { this.bytes -= previous.bytes; this.entries.delete(key); }
    if (bytes > this.maxBytes) return;
    this.entries.set(key, { svg: result, token, bytes }); this.bytes += bytes;
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes; this.entries.delete(oldest);
    }
  }
}

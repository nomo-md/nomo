import type MarkdownIt from 'markdown-it';
import type StateInline from 'markdown-it/lib/rules_inline/state_inline.mjs';
import { unescapeAll } from 'markdown-it/lib/common/utils.mjs';

/** 与原生 gfm.rs 的显式扩展规则一致；安全打开仍经过现有链接协议校验。 */
export function registerGfmAutolinks(markdown: MarkdownIt) {
  const cache = new WeakMap<StateInline, Array<{from:number;to:number;href:string}>>();
  const links = (state:StateInline) => {
    const cached=cache.get(state); if(cached) return cached;
    const found:Array<{from:number;to:number;href:string}>=[];
    for(const match of state.src.matchAll(/(?:https?:\/\/|ftp:\/\/|www\.)[^\s<]+|[\p{L}\p{N}.\-+_]+@[\p{L}\p{N}_-]+(?:\.[\p{L}\p{N}_-]+)+\.?/gu)) {
      const from=match.index!, candidate=match[0], email=!/^(?:https?:\/\/|ftp:\/\/|www\.)/.test(candidate);
      if(!email && from>0 && !/[\s*_~(]/u.test(state.src[from-1])) continue;
      let visible=candidate;
      if(email) { if(/[-_]$/.test(visible)) continue; visible=visible.replace(/\.$/,''); }
      else {
        visible=visible.replace(/[?!.,:*_~]+$/,'');
        let unmatched=(visible.match(/\)/g)?.length??0)-(visible.match(/\(/g)?.length??0);
        while(unmatched-->0 && visible.endsWith(')')) visible=visible.slice(0,-1);
        visible=visible.replace(/&[\p{L}\p{N}]+;$/u,'');
        const domain=visible.replace(/^[a-z]+:\/\//,'').split(/[/?#:]/)[0], segments=domain.split('.');
        if(segments.length<2 || segments.some(segment=>!segment || !/^[\p{L}\p{N}_-]+$/u.test(segment)) || segments.slice(-2).some(segment=>segment.includes('_'))) continue;
      }
      if(visible) found.push({from,to:from+visible.length,href:unescapeAll(email?`mailto:${visible}`:visible.startsWith('www.')?`http://${visible}`:visible)});
    }
    cache.set(state,found); return found;
  };
  const nextLink = (state: StateInline) => {
    const items=links(state); let from=0,to=items.length;
    while(from<to) {const mid=(from+to)>>>1;if(items[mid].from<state.pos)from=mid+1;else to=mid;}
    return items[from];
  };
  markdown.inline.ruler.before('text','nomo_gfm_autolink',(state,silent)=>{
    if((state as StateInline & {linkLevel:number}).linkLevel) return false;
    const link=nextLink(state);
    if(!link || link.from!==state.pos || link.to>state.posMax) return false;
    if(!silent) {
      const open=state.push('link_open','a',1);open.attrs=[['href',markdown.normalizeLink(link.href)]];
      open.markup='linkify';open.info='auto';
      const text=state.push('text','',0);text.content=unescapeAll(state.src.slice(link.from,link.to));
      text.meta={inlineSourceRange:{from:link.from,to:link.to,point:link.from}};
      state.pos=link.to;const close=state.push('link_close','a',-1);close.markup='linkify';close.info='auto';
    } else state.pos=link.to;
    return true;
  });
  // Markdown-it 的 text 规则先遇到扩展候选就让出，仍保留原有标点终止规则。
  markdown.inline.ruler.at('text',(state,silent)=>{
    const stop=(state as StateInline & {linkLevel:number}).linkLevel?state.posMax:Math.min(state.posMax,nextLink(state)?.from??state.posMax);
    let pos=state.pos;
    while(pos<stop && !/[\n!#$%&*+\-:<=>@[\\\]^_`{}~]/.test(state.src[pos])) pos++;
    if(pos===state.pos) return false;
    if(!silent) state.pending+=state.src.slice(state.pos,pos);
    state.pos=pos;return true;
  });
}

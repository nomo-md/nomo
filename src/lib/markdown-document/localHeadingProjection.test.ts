import { describe, it, expect } from 'vitest';
import { projectLocalHeadings } from './localHeadingProjection';
import type { StructureOutlineRow } from './structureProtocol';
const row = (id:string, byte:number, level:1|2=1):StructureOutlineRow => ({ id,byte,level,title:id,line:1,blockId:id,blockHeadingIndex:0,position:byte,collapsed:false,parentByte:null,hasChildren:false });
describe('当前窗口标题增删投影', () => {
  it('保留原生行身份并即时添加临时标题、删除旧标题', () => {
    const rows=[row('one',0),row('two',10),row('tail',100)];
    const projected=projectLocalHeadings(rows,[{fromByte:0,toByte:20,ownedIds:['one','two'],headings:[{id:'one',title:'changed',level:1},{id:'local-heading:1',title:'new',level:2}]}]);
    expect(projected.map(r=>r.title)).toEqual(['changed','new','tail']);
    expect(projected[0].blockId).toBe('one'); expect(projected[1].local).toBe(true);
    expect(rows.map(r=>r.id)).toEqual(['one','two','tail']);
  });
  it('支持空文档、末尾新标题及全部删除，不将屏外行混入中间分页', () => {
    const window={fromByte:10,toByte:30,ownedIds:[],headings:[{id:'local-heading:2',title:'new',level:1}]};
    expect(projectLocalHeadings([], [{...window,fromByte:0}])).toHaveLength(1);
    expect(projectLocalHeadings([row('old',0)], [window]).map(r=>r.title)).toEqual(['old','new']);
    expect(projectLocalHeadings([row('old',0)], [window],false)).toHaveLength(1);
    expect(projectLocalHeadings([row('old',0)], [{...window,fromByte:0,ownedIds:['old'],headings:[]}])).toEqual([]);
  });
  it('维持折叠状态，删除父项后允许子项即时显现', () => {
    const parent={...row('one',0),collapsed:true,hasChildren:true};
    const window={fromByte:0,toByte:30,ownedIds:['one','child'],headings:[{id:'one',title:'one',level:1},{id:'child',title:'child',level:2},{id:'local-heading:3',title:'new',level:2}]};
    expect(projectLocalHeadings([parent], [window])).toHaveLength(1);
    expect(projectLocalHeadings([parent], [{...window,headings:window.headings.slice(1)}]).map(r=>r.title)).toEqual(['child','new']);
  });
});

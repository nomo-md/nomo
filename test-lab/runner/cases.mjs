import assert from 'node:assert/strict';
import { readFile, writeFile, appendFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { hashJson, hashFile, hashOutsideToc, saveJson, pause } from './common.mjs';
import { assertMarkdown, inlineResults, blockResults } from './oracles.mjs';

export const supportedExpectations=['exact_bytes','headings_in_body'];
export function catalogue(dataset) {
  const select=pattern=>dataset.samples.find(s=>pattern.test(s.path));
  const small=select(/短文编辑基线/)??dataset.samples[0];
  const four=dataset.samples.filter(s=>/四标签_/.test(s.path));
  const corpus=dataset.samples.map(sample=>({id:`corpus.${hashJson(sample.path).slice(0,10)}`,title:sample.path,kind:'corpus',samples:[sample],
    suites:['corpus',...(['常规','中压','兼容','异常'].includes(sample.level)?['standard']:['stress'])],tags:[sample.level,sample.path.slice(0,2)],expected:sample.expected??{},purpose:sample.purpose}));
  const behavior=[['smoke','基础读写、沙箱及错误处理'],['editing.commands','行内格式与完整撤销'],['editing.blocks','标题列表引用'],
    ['editing.table','表格行列与对齐'],['structure.outline','大纲定位与过期修订'],['search.behavior','原生搜索与替换'],
    ['editing.native','系统键盘与快捷键'],['persistence.restart','保存与进程重启'],['persistence.autosave','自动保存'],
    ['selftest.failure','故障隔离注入'],['selftest.after','故障后的独立用例'],['selftest.cleanup','清理故障注入'],['selftest.cleanup-after','清理故障哨兵']]
    .map(([id,title])=>({id,title,kind:id,samples:[small],suites:id.startsWith('selftest.')?[id.includes('cleanup')?'selftest-cleanup':'selftest']:[id.split('.')[0],'standard'],tags:[id.split('.')[0]]}));
  const specs=[['blank',/真正空白/],['text',/05_.*05MiB/],['mixed',/06_.*01MiB/],['images',/图文瀑布流/],['diagrams',/七类Mermaid/],['code',/29种代码语言/]];
  const performance=specs.flatMap(([id,pattern])=>{const sample=select(pattern);return sample?[{id:`performance.${id}`,title:`性能：${sample.path}`,kind:'performance',samples:[sample],suites:['performance'],tags:['performance',id]}]:[];});
  const media=select(/图文瀑布流/);
  if(media)behavior.push({id:'resources.lifecycle',title:'共享媒体、后台标签与关闭回收',kind:'resources.lifecycle',samples:[media],suites:['resources','standard'],tags:['resources','lifecycle']});
  if(four.length===4)performance.push({id:'performance.four-tabs',title:'四标签切换、阅读位置与释放',kind:'performance',samples:four,suites:['performance'],tags:['performance','lifecycle']});
  performance.push({id:'soak.lifecycle',title:'持续编辑与打开关闭',kind:'soak.lifecycle',samples:[small],suites:['soak'],tags:['soak']});
  return [...behavior,...corpus,...performance];
}
export function selectCases(cases,config) {
  const suites=new Set(config.suites),known=new Set(cases.flatMap(c=>c.suites).concat(['diagnostics','all']));
  for(const suite of suites)if(!known.has(suite))throw new Error(`未知测试集：${suite}`);
  if(suites.has('diagnostics')&&!config.caseIds?.length)throw new Error('diagnostics 必须指定 --case');
  for(const id of config.caseIds??[])if(!cases.some(c=>c.id===id))throw new Error(`未知用例：${id}`);
  for(const tag of config.tags??[])if(!cases.some(c=>c.tags.includes(tag)))throw new Error(`未知标签：${tag}`);
  const selected=cases.filter(c=>(suites.has('all')||suites.has('diagnostics')||c.suites.some(s=>suites.has(s)))&&
    (!config.caseIds?.length||config.caseIds.includes(c.id))&&(!config.tags?.length||config.tags.every(t=>c.tags.includes(t))));
  if(!selected.length)throw new Error('筛选后没有用例；未启动程序');return selected;
}
export async function executeCase(test,host,root,config,result) {
  const call=(...args)=>host.call(...args),raw=(...args)=>host.raw(...args);
  const check=(name,a,b)=>{assert.deepEqual(a,b,name);result.assertions.push(name);};
  const truth=(name,value)=>check(name,Boolean(value),true);
  const wait=stage=>host.wait(stage,test.suites.includes('stress')?config.stressTimeoutMs:config.actionTimeoutMs);
  const digest=()=>call('digest'),read=async()=>(await call('read')).text;
  const scratch=join(root,'scratch.md'),path=join(root,test.samples[0].path);
  const open=async path=>(await call('open',{path})).activeTabId;
  const reset=async text=>{await writeFile(scratch,text);return open(scratch);};
  const initial=await wait('editable');await wait('indexed');
  const mark='nomo_test_marker_42';
  const replace=async(fromByte,toByte,text)=>{await call('select',{fromByte,toByte});await call('insert',{text});};
  const measure=async phase=>{
    const value={phase,at:new Date().toISOString(),data:await host.capture()};result.measurements??=[];
    if(result.measurements.length>=256)result.measurements.splice(1,1);
    result.measurements.push(value);await appendFile(join(host.directory,'measurements.jsonl'),JSON.stringify(value)+'\n');return value.data;
  };
  const roundTrip=async file=>{
    await call('mode',{mode:'source'});const before=await digest();
    const outside=await hashOutsideToc(file,mark);
    await replace(0,0,mark);const changed=await digest();truth('插入确实改变正文',before.sha256!==changed.sha256);
    await call('undo');check('撤销恢复完整正文',(await digest()).sha256,before.sha256);
    await call('redo');check('重做恢复完整正文',(await digest()).sha256,changed.sha256);
    await call('save');const saved=await digest();
    if(outside.blocks)check('保存仅更新派生目录',await hashOutsideToc(file),outside);
    else check('保存正文一致',saved.sha256,changed.sha256);
    await call('close');await open(file);check('重开全文一致',(await digest()).sha256,saved.sha256);
    return before;
  };
  const nativeInput=async mode=>{
    await call('mode',{mode});await wait('stable');const before=await digest();
    await call('reveal',{byte:0});await wait('stable');
    const text='NativeInputabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ';
    await host.native({text});await wait('editable');await wait('stable');
    const prefix=await call('readRange',{fromByte:0,toByte:text.length});
    check(`系统逐键 ${mode} 输入内容`,prefix.text,text);
    await replace(0,text.length,'');check(`移除 ${mode} 输入后全文未变`,(await digest()).sha256,before.sha256);
    result.requiredMetrics=['inputP95Ms'];
  };
  if(test.kind==='selftest.failure')throw new Error('INJECTED_EXPECTED_FAILURE');
  if(test.kind==='selftest.after') {truth('独立后续用例可读',initial.ready.readable);await pause(2000);return;}
  if(test.kind==='selftest.cleanup') {const stop=host.stop.bind(host);host.stop=async()=>{await stop();throw new Error('INJECTED_CLEANUP_FAILURE');};return;}
  if(test.kind==='selftest.cleanup-after')throw new Error('哨兵不应执行');
  if(test.kind==='smoke') {
    await roundTrip(path);
    await assert.rejects(()=>call('unknownMethod'),/不支持/);result.assertions.push('未知接口明确拒绝');
    await assert.rejects(()=>call('open',{path:join(root,'../profile/config.json')}),/fixtures/);result.assertions.push('拒绝跨 fixtures 访问');
    await assert.rejects(()=>call('readRange',{fromByte:0,toByte:100000}),/范围/);result.assertions.push('拒绝无界正文读取');
    await call('new');check('新建空白文档字节数',(await digest()).byteLength,0);
    await call('insert',{text:'# New\n\nHello native.\n'});await call('save',{path:join(root,'new.md')});
    check('新文档真实落盘',await readFile(join(root,'new.md'),'utf8'),'# New\n\nHello native.\n');return;
  }
  if(test.kind==='editing.commands'||test.kind==='editing.blocks') {
    const inline=test.kind.endsWith('commands');await reset(inline?'# Editing\n\nAlpha beta\n\n':'# Blocks\n\nAlpha beta\n\n');
    await call('mode',{mode:'semantic'});
    for(const [type,expected] of Object.entries(inline?inlineResults:blockResults)) {
      await call('select',{fromByte:inline?11:10,toByte:inline?16:15});const before=await digest();
      await call('command',{command:{type,...(type==='setHeading'?{level:2}:{})}});
      assertMarkdown(await read(),inline?`# Editing\n\n${expected} beta\n\n`:`# Blocks\n\n${expected}\n\n`,type);
      result.assertions.push(`${type} 精确正文及相邻内容`);
      await call('undo');check(`${type} 完整撤销`,(await digest()).sha256,before.sha256);
    }return;
  }
  if(test.kind==='editing.table') {
    const original='# Table\n\n| A | B |\n| --- | --- |\n| cell_x | cell_y |\n\n';await reset(original);await call('mode',{mode:'semantic'});
    const before=await digest();
    for(const type of ['addTableRowAfter','addTableColumnAfter','deleteTableRow','deleteTableColumn','setTableColumnAlignment']) {
      await call('reveal',{byte:original.indexOf('cell_x')});await call('command',{command:{type,align:'center'}});
      const after=await read();truth(`${type} 改变表格`,after!==original);
      if(type==='addTableRowAfter')check('插入恰好一行',after.split('\n').filter(l=>l.startsWith('|')).length,4);
      if(type==='addTableColumnAfter')truth('所有行增加一列',after.split('\n').filter(l=>l.startsWith('|')).every(l=>(l.match(/\|/g)??[]).length===4));
      if(type==='deleteTableRow')truth('删除指定行并保留正文',!after.includes('cell_x')&&!after.includes('cell_y')&&after.startsWith('# Table\n'));
      if(type==='deleteTableColumn')truth('删除指定列并保留相邻列',!after.includes('cell_x')&&after.includes('cell_y'));
      if(type==='setTableColumnAlignment')truth('居中标记落盘',/:\s*-+\s*:/.test(after));
      await call('undo');check('表格历史完整',(await digest()).sha256,before.sha256);
    }return;
  }
  if(test.kind==='search.behavior') {
    await reset('# Search\n\nAlpha alpha alphabet\n\n'+'TOKEN_42\n\n'.repeat(300));
    check('大小写不敏感',(await call('search',{query:'alpha'})).count,3);
    check('大小写敏感',(await call('search',{query:'Alpha',caseSensitive:true})).count,1);
    check('全词匹配',(await call('search',{query:'alpha',wholeWord:true})).count,2);
    const before=await digest();check('全文命中数',(await call('search',{query:'TOKEN_42'})).count,300);
    check('搜索取消',(await call('searchCancel')).cancelled,true);
    check('替换数量',(await call('replaceAll',{query:'TOKEN_42',replacement:'REPLACED'})).count,300);
    check('旧文本消失',(await call('search',{query:'TOKEN_42'})).count,0);
    check('新修订命中',(await call('search',{query:'REPLACED'})).count,300);
    await call('undo');check('批量替换一次撤销',(await digest()).sha256,before.sha256);return;
  }
  if(test.kind==='structure.outline') {
    await reset('# Start\n\n'+'Paragraph.\n\n'.repeat(100)+'## Target\n\nTail.\n');await wait('indexed');
    const outline=await call('outline'),target=outline.items.find(i=>i.title==='Target');truth('大纲包含目标',target);
    await call('outlineNavigate',{byte:target.byte,revision:outline.revision});await wait('stable');
    truth('真实阅读锚点已移动',(await raw('diagnostics')).readingByte>0);
    await call('mode',{mode:'source'});await replace(0,0,mark);
    await assert.rejects(()=>call('outlineNavigate',{byte:target.byte,revision:outline.revision}),/修订无效/);result.assertions.push('旧修订定位明确拒绝');return;
  }
  if(test.kind==='editing.native') {
    await reset('Editable paragraph.\n\n'+'Line for scrolling.\n\n'.repeat(100));await raw('telemetry',{reset:true});
    await nativeInput('semantic');await nativeInput('source');
    await call('select',{fromByte:0});const before=await digest();await host.native({text:'Shortcut42'});await wait('stable');const after=await digest();
    await host.native({keys:'CTRL+Z'});await wait('stable');check('真实撤销快捷键',(await digest()).sha256,before.sha256);
    await host.native({keys:'CTRL+Y'});await wait('stable');check('真实重做快捷键',(await digest()).sha256,after.sha256);
    await host.native({keys:'CTRL+S'});await host.wait('editable',15000,s=>!s.document.dirty);truth('真实保存快捷键落盘',(await readFile(scratch,'utf8')).startsWith('Shortcut42'));return;
  }
  if(test.kind==='persistence.restart') {
    await reset('# Restart\n\nOriginal.\n');await call('mode',{mode:'source'});await replace(0,0,mark);await call('save');const expected=await digest();
    while((await raw('diagnostics')).tabs.length)await call('close');await call('exit');await host.waitExit();await host.stop();await host.start(scratch);
    check('进程重启后全文保留',(await digest()).sha256,expected.sha256);return;
  }
  if(test.kind==='persistence.autosave') {
    await reset('# Autosave\n');await call('autosave',{enabled:true});await call('mode',{mode:'source'});await replace(0,0,mark);
    const deadline=Date.now()+15000;while(Date.now()<deadline&&!(await readFile(scratch,'utf8')).startsWith(mark))await pause(100);
    truth('自动保存真实落盘',(await readFile(scratch,'utf8')).startsWith(mark));await call('autosave',{enabled:false});return;
  }
  if(test.kind==='resources.lifecycle') {
    while((await raw('diagnostics')).tabs.length)await call('close');
    const content='$$x^2+y^2=z^2$$\n\n```mermaid\nflowchart LR\n A[Shared] --> B[Resource]\n```\n\n'+await readFile(path,'utf8');
    const a=join(dirname(path),'native-resource-a.md'),b=join(dirname(path),'native-resource-b.md');
    await writeFile(a,content);await writeFile(b,content);
    const first=await open(a);await wait('stable');await wait('resources');
    const firstResources=await measure('first-owner');
    truth('图片缓存已实际解码',firstResources.images.decodedBytes>0);
    truth('公式图表已实际栅格化',firstResources.embeds.decodedBytes>0);
    const second=await open(b);await wait('stable');await wait('resources');
    const before=await digest();await call('mode',{mode:'source'});
    await replace(0,0,mark);const changed=await digest();
    await call('undo');check('共享媒体文档撤销完整',(await digest()).sha256,before.sha256);
    await call('redo');check('共享媒体文档重做完整',(await digest()).sha256,changed.sha256);
    await call('undo');await call('save');
    for(const mode of ['split','semantic']){await call('mode',{mode});await wait('stable');}
    const anchorByte=Buffer.byteLength(content.slice(0,Math.max(0,content.indexOf('\n',200)+1)));
    await call('reveal',{byte:anchorByte});await wait('stable');const anchors=await host.readingAnchors();
    const shared=await wait('resources');
    const plain=await reset('# Background keeper\n');
    await call('switch',{tabId:first});await call('close');await call('switch',{tabId:plain});
    const hidden=await measure('remaining-hidden-owner');
    check('后台标签仍保有共享图片',hidden.images.decodedBytes,shared.images.decodedBytes);
    check('后台标签仍保有共享图表',hidden.embeds.decodedBytes,shared.embeds.decodedBytes);
    await call('switch',{tabId:second});await wait('stable');
    const restored=await host.readingAnchors();
    check('关闭其他标签不改变阅读锚点',restored.reading?.byte,anchors.reading?.byte);
    truth('关闭其他标签不改变阅读比例',Math.abs(restored.reading.fraction-anchors.reading.fraction)<.01);
    check('关闭其他标签不改变正文',(await digest()).sha256,before.sha256);
    await call('close');await call('switch',{tabId:plain});await wait('stable');await pause(1500);
    const released=await measure('last-media-owner-closed');
    check('最后媒体文档关闭后图片释放',released.images.decodedBytes,0);
    check('最后媒体文档关闭后图表释放',released.embeds.decodedBytes,0);
    check('最后媒体文档关闭后无旧条目回填',[released.images.entries,released.embeds.entries],[0,0]);
    await open(b);await wait('stable');const reopened=await wait('resources');
    truth('回收后重开图片仍可解码',reopened.images.decodedBytes>0);
    truth('回收后重开图表仍可渲染',reopened.embeds.decodedBytes>0);
    check('回收后重开保存内容完整',(await digest()).sha256,before.sha256);
    return;
  }
  if(test.kind==='corpus') {
    const diskBefore=await hashFile(path);await wait('stable');await wait('resources');
    const baseline=await digest();result.expectationChecks=[];
    if(test.expected.exact_bytes){check('精确正文体量',baseline.byteLength,test.expected.exact_bytes);result.expectationChecks.push({key:'exact_bytes',status:'passed'});}
    if(test.expected.headings_in_body){let page=await call('outline'),count=page.items.length;while(page.next){page=await call('outline',{cursor:page.next});count+=page.items.length;}truth('标题计数完整',count>=test.expected.headings_in_body);result.expectationChecks.push({key:'headings_in_body',status:'passed'});}
    for(const mode of ['source','split','semantic']){await call('mode',{mode});check('模式切换全文一致',(await digest()).sha256,baseline.sha256);}
    await roundTrip(path);
    if(test.samples[0].level==='兼容'){await call('mode',{mode:'source'});await replace(0,mark.length,'');await call('save');check('编码及换行逐字节保留',await hashFile(path),diskBefore);}
    return;
  }
  if(test.kind==='performance'||test.kind==='soak.lifecycle') {
    await wait('stable');const resources=await wait('resources');
    check('已请求图片没有加载失败',resources.images.failed,0);check('已请求公式图表没有渲染失败',resources.embeds.failed,0);
    await pause(2500);await measure('initial');await raw('telemetry',{reset:true});
    if(test.id==='performance.four-tabs') {
      result.notApplicableMetrics=['inputP95Ms'];
      const ids=[];for(const sample of test.samples)ids.push(await open(join(root,sample.path)));
      await call('switch',{tabId:ids[0]});await call('scroll',{pixels:1200});await wait('stable');const anchor=await host.readingAnchors();
      truth('已滚动至非零阅读位置',anchor.reading?.byte>0&&anchor.panes.some(p=>p.scrollTop>0));
      const hashes=[];for(const id of ids){await call('switch',{tabId:id});hashes.push((await digest()).sha256);}
      for(let i=0;i<config.cycles;i++){for(const id of ids)await call('switch',{tabId:id});if(i%5===0)await measure(`cycle-${i}`);}
      await call('switch',{tabId:ids[0]});await wait('stable');const restored=await host.readingAnchors();
      result.readingPosition={before:anchor,after:restored};check('同一正文阅读锚点',restored.reading?.byte,anchor.reading?.byte);
      truth('阅读锚点比例保留',Math.abs(restored.reading.fraction-anchor.reading.fraction)<.01);
      truth('阅读滚动位置保留',Math.abs(restored.panes[0].scrollTop-anchor.panes[0].scrollTop)<1);
      for(let i=0;i<ids.length;i++){await call('switch',{tabId:ids[i]});check('后台标签全文保留',(await digest()).sha256,hashes[i]);}
      for(const id of ids){await call('switch',{tabId:id});await call('close');}
      check('全部标签关闭后会话释放',(await raw('diagnostics')).runtimeResources.documentRuntimes,0);
      await open(path);result.cycles=config.cycles;
    }else{
      await nativeInput('source');let id=(await raw('diagnostics')).activeTabId;
      const baseline=(await digest()).sha256,deadline=Date.now()+config.soakMinutes*60000;let i=0;
      do{
        await call('mode',{mode:'source'});await replace(0,0,mark);await call('undo');check('循环编辑撤销完整',(await digest()).sha256,baseline);
        await call('mode',{mode:'semantic'});await wait('stable');
        await call('scroll',{pixels:600});await wait('stable');await call('scroll',{pixels:-600});await wait('stable');
        const other=await open(scratch);await call('switch',{tabId:id});await call('switch',{tabId:other});await call('close');await call('switch',{tabId:id});
        await call('close',{discard:true});id=await open(path);await wait('stable');
        if(i%5===0)await measure(`cycle-${i}`);i++;result.cycles=i;
        if(i%5===0)console.log(`进度 ${test.id}：${i} 轮`);
      }while(test.kind==='soak.lifecycle'?Date.now()<deadline:i<config.cycles);
      check('循环后完整正文',(await digest()).sha256,baseline);
    }
    await wait('stable');const finalResources=await wait('resources');
    check('循环后已请求图片没有加载失败',finalResources.images.failed,0);
    check('循环后已请求公式图表没有渲染失败',finalResources.embeds.failed,0);
    await pause(3000);const idle=await measure('idle');
    truth('原生会话数未随循环累积',idle.runtimeResources.documentRuntimes<=test.samples.length);
    check('全文摘要快照均已释放',idle.runtimeResources.snapshots,0);
    return;
  }
  throw new Error(`没有原生场景实现：${test.kind}`);
}

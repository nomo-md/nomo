import { mkdir, writeFile, rename, unlink, readFile, appendFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { randomBytes } from 'node:crypto';
import { launch, run, lab, pause, exists, saveJson } from './common.mjs';
import { stopOwnedProcess } from './process-lifecycle.mjs';

const pathKey = p => resolve(p.replace(/^\\\\\?\\/, '')).toLowerCase();
const mutations = new Set(['new','open','close','switch','mode','select','reveal','insert','command','undo','redo','save','outlineNavigate','scroll','replaceAll']);
export class TestHost {
  constructor(build,directory,config,record) { Object.assign(this,{build,directory,config,record});this.serial=0;this.boot=0; }
  async start(path) {
    this.boot++;this.exit=null;this.poisoned=false;this.stopTask=null;
    this.token=randomBytes(32).toString('hex');
    await mkdir(join(this.directory,'profile'),{recursive:true});
    for(const name of ['ready.json','request.json','response.json','sampler.stop']) await unlink(join(this.directory,name)).catch(e=>{if(e.code!=='ENOENT')throw e;});
    const configuration=join(this.directory,'profile/config.json');
    if(!await exists(configuration)) {
      const preferences={themeMode:'light',colorThemeId:'nomo-default',editorMode:'semantic',typographyEnabled:false,autoSaveEnabled:false,
        createSnapshotBeforeSave:false,openDefaultBehavior:'current-window',closeWindowBehavior:'exit',interfaceLanguage:'zh-CN'};
      await saveJson(configuration,{app:{settings:Object.fromEntries(Object.entries(preferences).map(([key,value])=>[key,{key,value_json:JSON.stringify(value),updated_at:0}]))}});
    }
    const started=performance.now();
    this.child=launch(this.build.binary,path?[path]:[],{log:join(this.directory,`app-${this.boot}.log`),windowsHide:false,env:{...process.env,
      NOMO_AUTOMATION_DIR:this.directory,NOMO_AUTOMATION_TOKEN:this.token,NOMO_DATA_DIR:join(this.directory,'profile'),
      NOMO_FRAME_PROFILE:join(this.directory,`frames-${this.boot}.json`),NOMO_OPEN_PROFILE:join(this.directory,`open-${this.boot}.json`),NOMO_FRAME_PROFILE_PAGES:'0'}});
    this.child.once('exit',(code,signal)=>{this.exit={code,signal};});
    this.child.once('error',error=>{this.exit={error:String(error)};});
    this.record.processes??=[];this.record.processes.push({pid:this.child.pid,boot:this.boot,startedAt:new Date().toISOString()});
    this.sampler=launch('pwsh',['-NoProfile','-File',join(lab,'runner/sample.ps1'),'-RootProcessId',String(this.child.pid),
      '-OutputPath',join(this.directory,`process-${this.boot}.jsonl`),'-StopPath',join(this.directory,'sampler.stop'),'-IntervalMs',String(this.config.sampleMs)],
      {log:join(this.directory,`sampler-${this.boot}.log`)});
    this.counters=launch('pwsh',['-NoProfile','-File',join(lab,'runner/counters.ps1'),'-RootProcessId',String(this.child.pid),
      '-OutputPath',join(this.directory,`process-${this.boot}.jsonl.counters.json`),'-StopPath',join(this.directory,'sampler.stop')],
      {log:join(this.directory,`counters-${this.boot}.log`)});
    const ready=await this.waitFile('ready.json',this.config.startupTimeoutMs);
    this.record.startupMs=performance.now()-started;
    if(ready.result?.protocol!==3||ready.result?.backend!=='gpui'||ready.result?.pid!==this.child.pid) throw new Error('原生测试协议或 PID 不匹配');
    this.record.environment=ready.result.environment;this.record.capabilities=ready.result.capabilities;
    const sampling=await this.waitFile(`process-${this.boot}.jsonl.ready`,this.config.startupTimeoutMs);
    if(sampling.pid!==this.child.pid) throw new Error('采样器身份不匹配');
    this.record.measurementStartedAt=new Date().toISOString();
    if(path) {
      await this.wait('editable',this.config.startupTimeoutMs,s=>s.document?.path&&pathKey(s.document.path)===pathKey(path));
      this.record.editableFromLaunchMs=performance.now()-started;
    }
    return ready;
  }
  async waitFile(name,timeout) {
    const deadline=Date.now()+timeout;
    while(Date.now()<deadline) {
      if(this.exit) throw new Error(`测试进程退出：${JSON.stringify(this.exit)}`);
      try{return JSON.parse(await readFile(join(this.directory,name),'utf8'));}
      catch(e){if(e.code!=='ENOENT')throw e;}
      await pause(10);
    }
    const error=new Error(`等待 ${name} 超时（${timeout} ms）`);error.code='TIMEOUT';this.poisoned=true;throw error;
  }
  async raw(method,params={},timeout=this.config.actionTimeoutMs) {
    if(this.poisoned) throw Object.assign(new Error('RPC 已超时，拒绝后续请求以免接受迟到响应'),{code:'TIMEOUT'});
    const id=`${this.boot}-${++this.serial}`,started=performance.now();
    const step={id,method,stage:params.stage,mode:params.mode,command:params.command?.type,startedAt:new Date().toISOString(),status:'running'};
    if(this.record.steps.length>=8192){this.record.steps.splice(0,4096);this.record.evictedSteps=(this.record.evictedSteps??0)+4096;}
    this.record.steps.push(step);
    try {
      await saveJson(join(this.directory,'last-step.json'),{...step,fromByte:params.fromByte,toByte:params.toByte,textBytes:typeof params.text==='string'?Buffer.byteLength(params.text):undefined});
      await writeFile(join(this.directory,'request.tmp'),JSON.stringify({protocol:3,token:this.token,id,method,params}));
      await rename(join(this.directory,'request.tmp'),join(this.directory,'request.json'));
      const response=await this.waitFile('response.json',timeout);await unlink(join(this.directory,'response.json'));
      if(response.id!==id){this.poisoned=true;throw new Error('响应 ID 不匹配');}
      step.appDurationMs=response.durationMs;
      if(!response.ok)throw new Error(response.error);
      step.status='passed';return response.result;
    } catch(error){step.status=error.code==='TIMEOUT'?'timeout':'failed';step.error=String(error);throw error;}
    finally{step.roundTripMs=performance.now()-started;await appendFile(join(this.directory,'steps.jsonl'),JSON.stringify(step)+'\n');}
  }
  async wait(stage,timeout=this.config.actionTimeoutMs,predicate=()=>true) {
    const deadline=Date.now()+timeout;let state,previous,stableSince=0;
    if(!['readable','editable','stable','indexed','resources','idle'].includes(stage))throw new Error(`未知等待阶段：${stage}`);
    while(Date.now()<deadline) {
      state=await this.raw('diagnostics',{},Math.max(1000,deadline-Date.now()));
      if(state.failure||state.panes?.some(p=>p.error||p.referencesFailed))throw new Error(`原生编辑失败：${JSON.stringify(state)}`);
      let ready=stage==='idle'?state.opening===0:state.ready?.[stage];
      if(stage==='indexed') {
        if(state.document){const progress=await this.raw('progress');state.progress=progress;if(progress.state==='failed')throw new Error(`索引失败：${progress.failure}`);ready=progress.state==='ready';}
      }
      if(stage==='resources')ready=state.ready?.stable&&state.images.pending===0&&state.embeds.pending===0;
      if(stage==='stable'||stage==='resources') {
        const key=JSON.stringify([state.document?.revision,state.reading,state.panes?.map(p=>[p.scrollTop,p.contentHeight,p.views])]);
        if(key!==previous){previous=key;stableSince=Date.now();}
        ready=ready&&Date.now()-stableSince>=120;
      }
      if(ready&&predicate(state))return state;
      await pause(40);
    }
    throw new Error(`等待 ${stage} 超时；最后状态 ${JSON.stringify(state)}`);
  }
  async call(method,params={},timeout=this.config.actionTimeoutMs) {
    if(method==='wait')return this.wait(params.stage,params.timeoutMs??timeout);
    if(method==='readingAnchors')return this.readingAnchors();
    if(method==='replaceAll'&&params.query!==undefined)await this.call('search',params,timeout);
    const before=mutations.has(method)?await this.raw('diagnostics'):null;
    const started=performance.now();
    const result=await this.raw(method,params,timeout);
    if(method==='search'||method==='searchCancel') {
      const state=await this.wait('editable',timeout,s=>!s.search.pending);
      if(state.search.error)throw new Error(state.search.error);
      return method==='searchCancel'?{cancelled:state.search.count===0}:state.search;
    }
    if(!mutations.has(method))return result;
    let state;
    if(method==='close')state=await this.wait('idle',timeout,s=>!s.tabs.some(t=>t.id===before.activeTabId));
    else state=await this.wait('editable',timeout,s=>{
      if(method==='open')return s.document?.path&&pathKey(s.document.path)===pathKey(params.path);
      if(method==='new')return s.activeTabId!==before.activeTabId&&s.document?.path===null;
      if(method==='switch')return s.activeTabId===params.tabId;
      if(method==='mode')return s.tabs.find(t=>t.id===s.activeTabId)?.mode.toLowerCase()===params.mode;
      if(method==='save')return !s.document.dirty&&!s.saving;
      return true;
    });
    await appendFile(join(this.directory,'steps.jsonl'),JSON.stringify({method:`settled.${method}`,startedAt:new Date().toISOString(),status:'passed',roundTripMs:performance.now()-started})+'\n');
    if(method==='replaceAll')return {count:before.search.count,state};
    return state;
  }
  async native(params) {
    await this.call('focus');await pause(100);
    const args=['-NoProfile','-File',join(lab,'runner/native.ps1'),'-Action','input','-RootProcessId',String(this.child.pid)];
    if(params.text)args.push('-Text',params.text);if(params.keys)args.push('-Keys',params.keys);
    return run('pwsh',args);
  }
  async screenshot(name='failure.png') {
    if(this.poisoned)throw new Error('RPC 已超时，截图由桌面观察者按测试 PID 获取');
    await this.raw('focus');
    await run('pwsh',['-NoProfile','-File',join(lab,'runner/native.ps1'),'-Action','screenshot','-RootProcessId',String(this.child.pid),'-OutputPath',join(this.directory,name)]);
    return name;
  }
  async capture(timeout=this.config.actionTimeoutMs) {
    const state=await this.raw('diagnostics',{},timeout);state.telemetry=await this.raw('telemetry',{},timeout);return state;
  }
  async readingAnchors() {
    const state=await this.raw('diagnostics');return {documentId:state.document?.documentId,revision:state.document?.revision,reading:state.reading,panes:state.panes};
  }
  async waitExit(timeout=30000) {
    const deadline=Date.now()+timeout;while(!this.exit&&Date.now()<deadline)await pause(25);
    if(!this.exit||this.exit.code!==0)throw new Error(`正常退出失败：${JSON.stringify(this.exit)}`);
  }
  stop(){return this.stopTask??=this.stopProcesses();}
  async stopProcesses() {
    const errors=[];
    this.record.measurementEndedAt=new Date().toISOString();
    await writeFile(join(this.directory,'sampler.stop'),'stop');
    // 只有当前运行器创建的子进程可终止；正常用例先走保存/关闭和应用退出流程。
    try{await stopOwnedProcess(this.child,{terminate:child=>run('taskkill',['/PID',String(child.pid),'/T','/F'])});}catch(e){errors.push(String(e));}
    for(const worker of [this.sampler,this.counters])try{
      const result=await stopOwnedProcess(worker,{graceMs:7000,terminate:child=>child.kill()});
      if(result?.code!==0)this.record.samplerWarning=`采样器异常退出：${result?.code}`;
    }catch(e){errors.push(String(e));}
    if(this.record.processes?.length)this.record.processes.at(-1).exit=this.exit;
    this.record.cleanupVerified=!errors.length;
    if(errors.length)throw Object.assign(new Error(errors.join('; ')),{code:'CLEANUP_FAILED'});
  }
}

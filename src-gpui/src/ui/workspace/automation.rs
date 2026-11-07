use super::*;
use crate::automation::{self, Config, Request, number, text};
use crate::nomo_mark::{commands::EditIntent, parse::InlineFormat, protocol::DocumentSelection};
use serde_json::{Value, json};

enum Reply { Value(Value), Background(Task<Result<Value,String>>), Exit }
impl Workspace {
    pub(super) fn start_automation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = automation::claim() else { return; };
        let bounds = window.viewport_size();
        let ready = json!({"ok":true,"result":{"protocol":automation::PROTOCOL,"backend":"gpui","pid":std::process::id(),
            "environment":{"width":f32::from(bounds.width),"height":f32::from(bounds.height),"scaleFactor":window.scale_factor(),
            "theme":format!("{:?}",window.appearance()),"gpui":"0.3.10","version":env!("CARGO_PKG_VERSION")},
            "capabilities":["open","new","close","switch","mode","select","reveal","insert","command","undo","redo","save",
                "readRange","digest","progress","outline","outlineNavigate","search","searchCancel","replaceAll","scroll","focus","telemetry","diagnostics","autosave","exit"]}});
        cx.spawn_in(window, async move |this,cx| {
            let writer = config.clone();
            if let Err(error) = cx.background_executor().spawn(async move {writer.write("ready.json",&ready)}).await {
                eprintln!("automation ready: {error}"); return;
            }
            loop {
                cx.background_executor().timer(Duration::from_millis(25)).await;
                if this.upgrade().is_none() { break; }
                let reader = config.clone();
                let request = match cx.background_executor().spawn(async move {reader.read()}).await {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(error) => { eprintln!("automation protocol: {error}"); break; }
                };
                let start = Instant::now();
                let response = this.update_in(cx, |workspace,window,cx|workspace.automation_dispatch(&config,&request,window,cx));
                let (result,exit) = match response {
                    Ok(Ok(Reply::Value(value))) => (Ok(value),false),
                    Ok(Ok(Reply::Background(task))) => (task.await,false),
                    Ok(Ok(Reply::Exit)) => (Ok(json!({"exiting":true})),true),
                    Ok(Err(error)) => (Err(error),false),
                    Err(error) => (Err(error.to_string()),false),
                };
                let value = match result {
                    Ok(result) => json!({"id":request.id,"ok":true,"durationMs":start.elapsed().as_secs_f64()*1000.0,"result":result}),
                    Err(error) => json!({"id":request.id,"ok":false,"durationMs":start.elapsed().as_secs_f64()*1000.0,"error":error}),
                };
                let writer = config.clone();
                if let Err(error) = cx.background_executor().spawn(async move {writer.write("response.json",&value)}).await {
                    eprintln!("automation response: {error}"); break;
                }
                if exit { let _=this.update_in(cx,|_,_,cx|cx.defer(crate::quit_workspaces)); break; }
            }
        }).detach();
    }
    fn automation_state(&mut self, window: &Window, cx: &mut Context<Self>) -> Value {
        let tabs: Vec<_> = self.tabs.iter().map(|tab|json!({"id":tab.key,"path":tab.document.read(cx).path(),
            "documentId":tab.document.read(cx).status().document_id,"mode":format!("{:?}",tab.mode),"dirty":tab.document.read(cx).is_dirty(cx)})).collect();
        let mut state = self.document().map(|document|document.update(cx,|host,cx|host.automation_state(cx))).unwrap_or(json!({}));
        state["tabs"]=json!(tabs);
        state["activeTabId"]=self.tab().map(|t|json!(t.key)).unwrap_or(Value::Null);
        state["opening"]=json!(self.opening.len());
        state["homeVisible"]=json!(self.home_visible);
        state["notice"]=self.failure_notice.as_ref().or(self.notice.as_ref()).map(|s|json!(s.as_ref())).unwrap_or(Value::Null);
        state["windowActive"]=json!(window.is_window_active());
        state["runtimeResources"]=self.services.as_ref().map(|m|m.automation_counts()).transpose().unwrap_or_else(|e|Some(json!({"error":e.to_string()}))).unwrap_or(json!({"documentRuntimes":0,"snapshots":0}));
        state["images"]=cx.try_global::<crate::images::ImageStore>().map(|s|s.automation_state()).unwrap_or(json!({"pending":0,"failed":0,"decodedBytes":0,"entries":0}));
        state["embeds"]=cx.try_global::<crate::embeds::EmbedStore>().map(|s|s.automation_state()).unwrap_or(json!({"pending":0,"failed":0,"decodedBytes":0,"entries":0}));
        state["workspace"]=json!(self.comparison.items.iter().map(|item| json!({"kind":if item.pair().is_some(){"comparison"}else{"single"},"keys":item.keys()})).collect::<Vec<_>>());
        state
    }
    fn automation_dispatch(&mut self, config: &Config, request: &Request, window: &mut Window, cx: &mut Context<Self>) -> Result<Reply,String> {
        let params=&request.params;
        let method=request.method.as_str();
        match method {
            "diagnostics"=>return Ok(Reply::Value(self.automation_state(window,cx))),
            "telemetry"=>return Ok(Reply::Value(crate::frame_profile::snapshot(params["reset"].as_bool().unwrap_or(false)))),
            "open"|"folder"=>{
                let path=config.path(Path::new(text(params,"path")?),false)?;
                self.open_path(&path,window,cx);
                return Ok(Reply::Value(json!({"accepted":true})));
            }
            "new"=>{self.new_file(window,cx);return Ok(Reply::Value(json!({"accepted":true})));}
            "switch"=>{
                let id=number(params,"tabId")?;
                let index=self.tabs.iter().position(|tab|tab.key==id).ok_or("标签不存在")?;
                self.activate(index,window,cx);
                return Ok(Reply::Value(json!({"accepted":true})));
            }
            "mode"=>{
                let mode=match text(params,"mode")? {"source"=>EditorMode::Source,"semantic"=>EditorMode::Semantic,"split"=>EditorMode::Split,_=>return Err("无效模式".into())};
                self.set_mode(mode,window,cx);
                return Ok(Reply::Value(json!({"accepted":true})));
            }
            "exit"=>{
                if !self.opening.is_empty() || self.tabs.iter().any(|tab|tab.document.read(cx).is_dirty(cx) || tab.document.read(cx).ready(cx).is_err()) {
                    return Err("退出前须保存或关闭未完成的测试文档".into());
                }
                return Ok(Reply::Exit);
            }
            "autosave"=>{
                Settings::set(cx,"autoSaveEnabled",json!(params["enabled"].as_bool().ok_or("缺少 enabled")?));
                return Ok(Reply::Value(json!({"accepted":true})));
            }
            _=>{}
        }
        let document=self.document().ok_or("没有活动文档")?;
        if matches!(method,"digest"|"read"|"readRange"|"progress"|"outline") {
            let port=document.read(cx).port();
            let method=method.to_string();let params=params.clone();
            return Ok(Reply::Background(cx.background_executor().spawn(async move {automation::read_document(port,method,params)})));
        }
        if let Some(revision)=params["revision"].as_u64() {if revision!=document.read(cx).status().revision {return Err("修订无效".into());}}
        match method {
            "focus"=>{window.activate_window();document.update(cx,|host,cx|host.automation_focus(window,cx));}
            "close"=>{
                let discard=params["discard"].as_bool().unwrap_or(false);
                if document.read(cx).is_dirty(cx) && !discard {return Err("测试文档尚未保存；需显式 discard".into());}
                document.read(cx).ready(cx).map_err(|e|e.to_string())?;
                if discard {self.remove_tab(&document,true,window,cx);}else{self.close_tab(self.active,window,cx);}
            }
            "save"=>{
                let path=if let Some(path)=params["path"].as_str() {config.path(Path::new(path),true)?}
                    else {config.path(&document.read(cx).path().ok_or("新建文档须提供保存路径")?,false)?};
                document.read(cx).ready(cx).map_err(|e|e.to_string())?;
                self.save_path(document,path,params["path"].is_null(),false,window,cx);
            }
            "select"|"reveal"=>{
                let from=if method=="reveal" {number(params,"byte")?}else{number(params,"fromByte")?};
                let to=params["toByte"].as_u64().unwrap_or(from);
                document.update(cx,|host,cx|host.automation_select(DocumentSelection{anchor_byte:from,head_byte:to},cx))?;
            }
            "outlineNavigate"=>{
                let byte=number(params,"byte")?;let revision=number(params,"revision")?;
                document.update(cx,|host,cx|host.reveal_outline_heading(byte,revision,cx));
            }
            "insert"=>{
                let value=text(params,"text")?.to_string();
                document.read(cx).ready(cx).map_err(|e|e.to_string())?;
                document.update(cx,|host,cx|host.command(EditIntent::Replace(value),cx));
            }
            "undo"|"redo"=>{
                document.read(cx).ready(cx).map_err(|e|e.to_string())?;
                let status=document.read(cx).status();
                if (method=="undo" && !status.can_undo)||(method=="redo" && !status.can_redo){return Err("历史操作不可用".into());}
                document.update(cx,|host,cx|host.undo_redo(method=="redo",cx));
            }
            "command"=>{
                let command=&params["command"];
                let kind=text(command,"type")?;
                if kind.contains("Table") {
                    document.update(cx,|host,cx|host.automation_table(kind,command["align"].as_str().unwrap_or("left"),cx))?;
                } else {
                    let intent=match kind {
                        "toggleBold"=>EditIntent::ToggleInline(InlineFormat::Strong),
                        "toggleItalic"=>EditIntent::ToggleInline(InlineFormat::Emphasis),
                        "toggleCode"=>EditIntent::ToggleInline(InlineFormat::Code),
                        "toggleStrikethrough"=>EditIntent::ToggleInline(InlineFormat::Strike),
                        "toggleUnderline"=>EditIntent::ToggleInline(InlineFormat::Underline),
                        "toggleHighlight"=>EditIntent::ToggleInline(InlineFormat::Highlight),
                        "setHeading"=>{let level=number(command,"level")?;if level>6{return Err("标题级别无效".into());}EditIntent::Heading(level as u8)}
                        "toggleBlockquote"=>EditIntent::LinePrefix("> ".into()),
                        "toggleBulletList"=>EditIntent::LinePrefix("- ".into()),
                        "toggleOrderedList"=>EditIntent::LinePrefix("1. ".into()),
                        "toggleTaskList"=>EditIntent::LinePrefix("- [ ] ".into()),
                        "newline"=>EditIntent::Newline,"backspace"=>EditIntent::Backspace,"delete"=>EditIntent::Delete,
                        _=>return Err(format!("不支持的原生命令：{kind}")),
                    };
                    document.read(cx).ready(cx).map_err(|e|e.to_string())?;
                    document.update(cx,|host,cx|host.command(intent,cx));
                }
            }
            "search"=>{
                let query=text(params,"query")?.to_string();
                document.update(cx,|host,cx|host.set_search(query,params["caseSensitive"].as_bool().unwrap_or(false),params["wholeWord"].as_bool().unwrap_or(false),cx));
            }
            "searchCancel"=>document.update(cx,|host,cx|host.set_search(String::new(),false,false,cx)),
            "replaceAll"=>{
                let replacement=text(params,"replacement")?.to_string();
                if document.read(cx).search().pending{return Err("搜索尚未完成".into());}
                document.update(cx,|host,cx|host.replace_all(replacement,cx));
            }
            "scroll"=>{
                let pixels=params["pixels"].as_f64().ok_or("缺少滚动距离")?;
                document.update(cx,|host,cx|host.automation_scroll(pixels as f32,cx))?;
            }
            _=>return Err(format!("不支持的原生接口：{method}")),
        }
        Ok(Reply::Value(json!({"accepted":true})))
    }
}

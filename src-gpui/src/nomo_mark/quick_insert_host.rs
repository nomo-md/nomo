//! 文档级快捷插入浮层及选择器。只保存插入目标和短期请求，关闭标签即取消。
use super::*;
use gpui::App;
use crate::{image_import::{self,ImageSettings,ImportCancellation}, ui::{editor_popovers::{EditorPopover,EditorPopoverEvent},emoji_picker::{EmojiPicker,EmojiPickerEvent},quick_insert::{QuickInsertPanel,PanelAction},popup::PopupAnchor,toolbar_i18n::ToolbarLocale}};
use crate::nomo_mark::{quick_insert::{self,QuickInsertAction as A},image_properties::ImageProperties};
use std::{ops::Range,path::PathBuf,rc::Rc,sync::atomic::{AtomicU64,Ordering}};

#[derive(Clone)]
struct QuickTarget {edit:EditTarget,range:Range<usize>,trigger:String,pane:u64}
enum Picker {
    Link {target:QuickTarget,view:Entity<EditorPopover>,draft:crate::nomo_mark::link_edit::LinkDraft,_subscription:Subscription},
    Emoji {target:QuickTarget,view:Entity<EmojiPicker>,_subscription:Subscription},
}
impl Picker {fn target(&self)->&QuickTarget {match self {Self::Link{target,..}|Self::Emoji{target,..}=>target}}}
#[derive(Clone)]
struct ImportRequest {id:u64,target:QuickTarget,cancel:ImportCancellation,settings:ImageSettings,path:Option<PathBuf>}
#[derive(Default)]
pub(super) struct QuickInsertUi {
    pub action:Option<A>,
    picker:Option<Picker>,
    import:Option<ImportRequest>,
    save_pending:Option<(u64,Vec<PathBuf>,PathBuf)>,
    focus:Option<(gpui::EntityId,Subscription)>,
    activation:Option<Subscription>,
}
impl QuickInsertUi {pub fn cancel(&mut self) {if let Some(request)=self.import.take(){request.cancel.cancel();}self.action=None;self.picker=None;self.save_pending=None;self.focus=None;self.activation=None;}}
impl Drop for QuickInsertUi {fn drop(&mut self){self.cancel();}}

impl DocumentHost {
    fn quick_target(&self,cx:&App)->Result<QuickTarget,ControlError> {
        let edit=self.edit_target(cx)?;
        let view=self.control.read(cx);
        let session=view.quick_insert.as_ref().ok_or(ControlError::Stale)?;
        let settings=cx.global::<crate::settings::Settings>();
        if !settings.bool("quickInsertEnabled") || settings.string("quickInsertTrigger")!=session.trigger || !session.valid(&view.host.parsed,view.host.selection) || !edit.is_semantic(){return Err(ControlError::Stale);}
        Ok(QuickTarget{edit,range:session.range.clone(),trigger:session.trigger.clone(),pane:self.panes[self.active_pane].id})
    }
    fn quick_current(&self,target:&QuickTarget,cx:&App)->bool {
        if self.hidden || self.closed || self.panes[self.active_pane].id!=target.pane{return false;}
        let view=self.control.read(cx);
        view.host.parsed.identity==target.edit.identity && view.host.selection==target.edit.selection
            && view.effective_mode()==target.edit.mode && view.host.parsed.source.get(target.range.clone())==Some(target.trigger.as_str())
    }
    fn quick_validate(&self,target:&QuickTarget,cx:&App)->Result<(),ControlError> {
        if !self.quick_current(target,cx){return Err(ControlError::Stale);}
        self.validate_target(&target.edit,cx)
    }
    fn import_current(&self,request:&ImportRequest,cx:&App)->bool {
        self.quick_current(&request.target,cx) && request.path.as_ref().is_none_or(|path|self.path().as_ref()==Some(path))
    }
    fn dismiss_quick(&mut self,cx:&mut Context<Self>) {self.control.update(cx,|view,cx|view.dismiss_quick_insert(cx));}
    fn quick_error(&mut self,error:ControlError,cx:&mut Context<Self>) {
        let message=ToolbarLocale::current(cx).error(&error);
        self.control.update(cx,|view,cx|view.quick_insert_error(message,cx));
    }
    pub(super) fn process_quick_insert(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        if self.hidden || self.closed {self.quick.cancel();return;}
        let control_id=self.control.entity_id();
        let menu_open=self.control.read(cx).quick_insert.is_some();
        if !menu_open {self.quick.focus=None;self.quick.activation=None;}
        else if self.quick.focus.as_ref().map(|entry|entry.0)!=Some(control_id) {
            let focus=self.control.focus_handle(cx);
            let subscription=cx.on_blur(&focus,window,|host,_,cx|host.dismiss_quick(cx));
            self.quick.focus=Some((control_id,subscription));
        }
        if menu_open && self.quick.activation.is_none() {
            self.quick.activation=Some(cx.observe_window_activation(window,|host,window,cx|{
                if !window.is_window_active(){host.dismiss_quick(cx);}
            }));
        }
        let stale=self.quick.picker.as_ref().is_some_and(|picker|!self.quick_current(picker.target(),cx))
            || self.quick.import.as_ref().is_some_and(|request|!self.import_current(request,cx));
        if stale {self.quick.cancel();self.report(ControlError::Stale,cx);window.focus(&self.control.focus_handle(cx),cx);}
        if self.control.read(cx).host.pending_count()==0 {
            if let Some(action)=self.quick.action.take(){self.activate_quick_insert(action,window,cx);}
        }
        if self.quick.save_pending.is_some() && !self.saving && self.toc_ready_for_save(cx) {
            let (id,paths,path)=self.quick.save_pending.take().unwrap();
            self.save_for_quick_image(id,paths,path,window,cx);
        }
    }
    pub(super) fn quick_insert_overlay(&mut self,cx:&mut Context<Self>)->Option<gpui::AnyElement> {
        if let Some(picker)=&self.quick.picker {return Some(gpui::deferred(match picker {Picker::Link{view,..}=>view.clone().into_any_element(),Picker::Emoji{view,..}=>view.clone().into_any_element()}).with_priority(2).into_any_element());}
        let view=self.control.read(cx);
        let session=view.quick_insert_snapshot()?;
        let settings=cx.global::<crate::settings::Settings>();
        if !settings.bool("quickInsertEnabled") || settings.string("quickInsertTrigger")!=session.trigger || view.effective_mode()!=ViewMode::Semantic || !session.valid(&view.host.parsed,view.host.selection) {
            self.dismiss_quick(cx);return None;
        }
        let Some(anchor)=view.quick_insert_anchor() else {
            let pane=&self.panes[self.active_pane];
            // 新排版可能已完成，但父容器还沿用旧高度并等待滚动显露光标。
            if view.quick_insert_layout_ready() && view.measured_height().is_some()
                && !pane.reveal_selection && !pane.viewport.dirty && !pane.viewport.loading
            {self.dismiss_quick(cx);}
            return None;
        };
        let host=cx.entity().downgrade();
        let on_action=Rc::new(move|action,window:&mut Window,cx:&mut App|{let _=host.update(cx,|host,cx|match action {
            PanelAction::Dismiss=>host.dismiss_quick(cx),
            PanelAction::Step(category,forward)=>host.control.update(cx,|view,cx|view.quick_insert_step(Some(category),forward,cx)),
            PanelAction::Choose(category)=>{
                let action=host.control.read(cx).quick_insert.as_ref().map(|session|quick_insert::CATEGORIES[category].variants[session.variants[category]].1);
                if let Some(action)=action{host.activate_quick_insert(action,window,cx);}
            }
        });});
        Some(QuickInsertPanel{session,anchor,on_action}.render(cx))
    }
    fn activate_quick_insert(&mut self,action:A,window:&mut Window,cx:&mut Context<Self>) {
        if self.control.read(cx).host.pending_count()>0 {self.quick.action=Some(action);cx.notify();return;}
        let target=match self.quick_target(cx){Ok(target)=>target,Err(error)=>{self.quick_error(error,cx);return;}};
        let result=match action {
            A::Link=>return self.open_quick_link(target,window,cx),
            A::Emoji=>return self.open_quick_emoji(target,window,cx),
            A::Image=>return self.choose_quick_images(target,window,cx),
            A::Math(inline)|A::Comment(inline)=>{
                use crate::nomo_mark::widgets::ObjectKind;
                let kind=match action {A::Math(true)=>ObjectKind::InsertInlineMath,A::Math(false)=>ObjectKind::InsertBlockMath,A::Comment(true)=>ObjectKind::InsertInlineComment,_=>ObjectKind::InsertBlockComment};
                let _=inline;
                self.control.update(cx,|view,cx|view.begin_quick_object(target.range,kind,window,cx))
            }
            _=>{
                let result=quick_insert::apply(&self.control.read(cx).host.parsed,target.range,action);
                result.and_then(|result|self.control.update(cx,|view,cx|view.apply_host_edit(result,cx)))
            }
        };
        match result {Ok(())=>{self.dismiss_quick(cx);if !matches!(action,A::Math(_)|A::Comment(_)){window.focus(&self.control.focus_handle(cx),cx);}},Err(error)=>self.quick_error(error,cx)}
    }
    fn open_quick_link(&mut self,target:QuickTarget,window:&mut Window,cx:&mut Context<Self>) {
        let draft=match crate::nomo_mark::link_edit::LinkDraft::insertion(&self.control.read(cx).host.parsed,target.range.clone()) {Ok(draft)=>draft,Err(error)=>{self.quick_error(error,cx);return;}};
        let Some(anchor)=self.control.read(cx).quick_insert_anchor() else{return;};
        let view=cx.new(|cx|{let mut view=EditorPopover::new_link(String::new(),String::new(),false,cx);view.set_popup_anchor(PopupAnchor::Bounds(anchor));view});
        let subscription=cx.subscribe_in(&view,window,|host,view,event,window,cx| {
            if matches!(event,EditorPopoverEvent::Dismiss){host.quick.picker=None;window.focus(&host.control.focus_handle(cx),cx);cx.notify();return;}
            let Some(Picker::Link{target,draft,..})=host.quick.picker.as_ref() else{return;};
            let result=host.quick_validate(target,cx).and_then(|_|match event {EditorPopoverEvent::ApplyLink{text,href}=>draft.edit(&host.control.read(cx).host.parsed,text,Some(href)),_=>Err(ControlError::InvalidRange)});
            let result=result.and_then(|result|host.control.update(cx,|control,cx|control.apply_host_edit(result,cx)));
            match result {Ok(())=>{host.quick.picker=None;window.focus(&host.control.focus_handle(cx),cx);},Err(error)=>view.update(cx,|view,cx|view.set_error(Some(ToolbarLocale::current(cx).error(&error).into()),cx))}
            cx.notify();
        });
        self.dismiss_quick(cx);window.focus(&view.focus_handle(cx),cx);
        self.quick.picker=Some(Picker::Link{target,view,draft,_subscription:subscription});cx.notify();
    }
    fn open_quick_emoji(&mut self,target:QuickTarget,window:&mut Window,cx:&mut Context<Self>) {
        let Some(anchor)=self.control.read(cx).quick_insert_anchor() else{return;};
        let view=cx.new(|cx|EmojiPicker::new(PopupAnchor::Bounds(anchor),cx));
        let subscription=cx.subscribe_in(&view,window,|host,_,event,window,cx|{
            if let EmojiPickerEvent::Choose(value)=event {
                let result=host.quick.picker.as_ref().ok_or(ControlError::Stale).and_then(|picker|{
                    let target=picker.target();host.quick_validate(target,cx)?;
                    let parsed=&host.control.read(cx).host.parsed;let base=parsed.identity.from_byte;
                    crate::nomo_mark::commands::apply_semantic(parsed,DocumentSelection{anchor_byte:base+target.range.start as u64,head_byte:base+target.range.end as u64},EditIntent::Replace(value.clone()))
                });
                if let Err(error)=result.and_then(|result|host.control.update(cx,|view,cx|view.apply_host_edit(result,cx))){host.report(error,cx);}
            }
            host.quick.picker=None;window.focus(&host.control.focus_handle(cx),cx);cx.notify();
        });
        self.dismiss_quick(cx);window.focus(&view.focus_handle(cx),cx);
        self.quick.picker=Some(Picker::Emoji{target,view,_subscription:subscription});cx.notify();
    }
    fn choose_quick_images(&mut self,target:QuickTarget,window:&mut Window,cx:&mut Context<Self>) {
        let settings=ImageSettings::read(&cx.global::<crate::settings::Settings>().value("imageHandlingSettings"));
        if !matches!(settings.strategy.as_str(),"copy-current-folder"|"copy-assets"|"copy-document-assets") {self.quick_error(ControlError::Unsupported("请在图片设置中切换为本地复制策略；上传配置已保留".into()),cx);return;}
        static NEXT:AtomicU64=AtomicU64::new(1);
        let id=NEXT.fetch_add(1,Ordering::Relaxed);
        self.quick.cancel();self.quick.import=Some(ImportRequest{id,target,cancel:Default::default(),settings,path:self.path()});self.dismiss_quick(cx);
        let response=cx.prompt_for_paths(gpui::PathPromptOptions{files:true,directories:false,multiple:true,prompt:Some(ToolbarLocale::current(cx).text("quick_choose_images").into())});
        cx.spawn_in(window,async move|this,cx|{
            let paths=response.await;
            let _=this.update_in(cx,|host,window,cx|{
                if !host.quick.import.as_ref().is_some_and(|r|r.id==id){return;}
                match paths {Ok(Ok(Some(paths))) if !paths.is_empty()=>host.quick_image_paths(id,paths,window,cx),Ok(Ok(_))=>host.quick.cancel(),_=>{host.quick.cancel();host.report(ControlError::HostFailure("无法打开图片选择窗口".into()),cx);}}
                cx.notify();
            });
        }).detach();
    }
    fn quick_image_paths(&mut self,id:u64,paths:Vec<PathBuf>,window:&mut Window,cx:&mut Context<Self>) {
        let Some(request)=self.quick.import.as_ref().filter(|r|r.id==id)else{return;};
        if !self.import_current(request,cx){self.quick.cancel();self.report(ControlError::Stale,cx);return;}
        if self.path().is_some(){self.start_quick_image_import(id,paths,window,cx);return;}
        let response=cx.prompt_for_new_path(&std::env::current_dir().unwrap_or_default(),Some("未命名.md"));
        cx.spawn_in(window,async move|this,cx|{
            let path=response.await;
            let _=this.update_in(cx,|host,_,cx|{
                if !host.quick.import.as_ref().is_some_and(|r|r.id==id){return;}
                match path {Ok(Ok(Some(path)))=>host.quick.save_pending=Some((id,paths,path)),Ok(Ok(None))=>host.quick.cancel(),_=>{host.quick.cancel();host.report(ControlError::HostFailure("无法选择文档保存路径".into()),cx);}}
                cx.notify();
            });
        }).detach();
    }
    fn save_for_quick_image(&mut self,id:u64,paths:Vec<PathBuf>,path:PathBuf,window:&mut Window,cx:&mut Context<Self>) {
        let Some(request)=self.quick.import.as_ref().filter(|r|r.id==id)else{return;};
        if !self.import_current(request,cx){self.quick.cancel();self.report(ControlError::Stale,cx);return;}
        let (port,snapshot)=match self.begin_save(cx){Ok(value)=>value,Err(error)=>{self.quick.cancel();self.report(error,cx);return;}};
        let backup=cx.global::<crate::settings::Settings>().snapshot_before_save;
        let task=cx.background_executor().spawn(async move{port.save_captured(snapshot,Some(path),backup)});
        cx.spawn_in(window,async move|this,cx|{
            let result=task.await;
            let _=this.update_in(cx,|host,window,cx|{
                let success=host.finish_save(result,cx);
                if success{cx.emit(DocumentEvent::Saved);if !host.hidden{window.set_window_title(&format!("{} — Nomo",host.title()));}}
                if host.quick.import.as_ref().is_some_and(|r|r.id==id) {
                    if success{host.start_quick_image_import(id,paths,window,cx);}else{host.quick.cancel();}
                }
            });
        }).detach();
    }
    fn start_quick_image_import(&mut self,id:u64,paths:Vec<PathBuf>,window:&mut Window,cx:&mut Context<Self>) {
        let Some(mut request)=self.quick.import.as_ref().filter(|r|r.id==id).cloned()else{return;};
        if !self.import_current(&request,cx){self.quick.cancel();self.report(ControlError::Stale,cx);return;}
        let Some(path)=self.path()else{self.quick.cancel();return;};
        request.path=Some(path.clone());self.quick.import=Some(request.clone());
        let worker=request.clone();
        let task=cx.background_executor().spawn(async move{image_import::import(paths,&path,&worker.settings,&worker.cancel)});
        cx.spawn_in(window,async move|this,cx|{
            let result=task.await;
            let _=this.update_in(cx,|host,window,cx|{
                if !host.quick.import.as_ref().is_some_and(|r|r.id==id){return;}
                if !host.import_current(&request,cx){host.quick.cancel();host.report(ControlError::Unsupported("插入目标或文档路径已改变，原文已保留；已复制的图片保留在所选资源目录".into()),cx);return;}
                let result=result.map_err(ControlError::HostFailure).and_then(|batch|{
                    if batch.images.is_empty(){return Err(ControlError::HostFailure(format!("图片导入失败（{} 张）：{}",batch.failed,batch.errors.join("；"))));}
                    let view=host.control.read(cx);let parsed=&view.host.parsed;let range=request.target.range.clone();let base=parsed.identity.from_byte;
                    let eol=crate::nomo_mark::commands::line_ending(&parsed.source,range.start);
                    let line_start=crate::nomo_mark::commands::line_start(&parsed.source,range.start);
                    let after=&parsed.source[range.end..];let line_after=after.split(['\r','\n']).next().unwrap_or("");
                    let inline=!parsed.source[line_start..range.start].trim().is_empty() || !line_after.trim().is_empty()
                        || parsed.nodes.iter().any(|node|matches!(node.kind,"heading"|"cell")&&node.source.contains(&range.start));
                    let markdown=inline || (request.settings.width.is_empty() && request.settings.align=="none");
                    let snippets=batch.images.iter().map(|image|ImageProperties{src:image.src.clone(),alt:image.alt.clone(),title:String::new(),width:request.settings.width.clone(),align:if request.settings.align=="none"{String::new()}else{request.settings.align.clone()}}.write("",markdown,eol)).collect::<Result<Vec<_>,_>>()?;
                    let text=snippets.join(if inline{" "}else{eol});
                    let mut result=crate::nomo_mark::commands::apply_semantic(parsed,DocumentSelection{anchor_byte:base+range.start as u64,head_byte:base+range.end as u64},EditIntent::Replace(text))?;
                    if !inline {quick_insert::continue_after_block(parsed,&mut result)?;}
                    host.control.update(cx,|view,cx|view.apply_host_edit(result,cx))?;
                    let message=format!("已插入 {} 张图片，{} 张失败{}",batch.images.len(),batch.failed,if batch.errors.is_empty(){String::new()}else{format!("：{}",batch.errors.join("；"))});
                    host.message=message.clone();cx.emit(DocumentEvent::Notice(message));Ok(())
                });
                host.quick.cancel();
                if let Err(error)=result{host.report(error,cx);}
                window.focus(&host.control.focus_handle(cx),cx);cx.notify();
            });
        }).detach();
    }
}

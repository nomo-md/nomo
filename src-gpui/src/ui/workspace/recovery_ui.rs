//! 恢复目录属于应用；主页不自动提示，取消不会删除资料或丢失再次进入的入口。
use super::*;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
use super::application_dialogs::OpenSource;
use crate::text_document::markdown_session::RecoveryCandidate;
use crate::ui::dialogs::{SecondaryAction as A, SecondaryDialog, SecondaryKind, Tone};
use std::collections::{HashMap,HashSet};

#[derive(Default)]
struct RecoveryCatalog {
    loading: bool,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    candidates: Vec<RecoveryCandidate>,
    offered: HashSet<String>,
    busy: HashMap<String,gpui::WeakEntity<Workspace>>,
    error: Option<String>,
    warning: Option<String>,
}
impl gpui::Global for RecoveryCatalog {}
impl Drop for RecoveryCatalog {
    fn drop(&mut self) {self.cancel.store(true,std::sync::atomic::Ordering::Release);}
}

fn dialog(title: UiMessage, message: UiMessage, actions: Vec<A>, cancel: usize) -> SecondaryDialog {
    SecondaryDialog {kind:SecondaryKind::Recovery,title,message,actions,cancel,detail:None,notes:None,remember:None,leading:Vec::new()}
}

impl Workspace {
    pub(super) fn start_recovery_catalog(&self, cx: &mut Context<Self>) {
        if cx.try_global::<RecoveryCatalog>().is_some() { return; }
        let Some(manager)=self.services.clone() else {return;};
        let mut catalog=RecoveryCatalog::default();
        catalog.loading=true;
        cx.set_global(catalog);
        let cancel=cx.global::<RecoveryCatalog>().cancel.clone();
        let task=cx.background_executor().spawn(async move {manager.recovery_candidates(&cancel)});
        // 不引用首个窗口；关闭首窗后其他窗口仍能访问恢复资料。
        cx.spawn(async move |_,cx| {
            let result=task.await;
            let _=cx.update(|cx| {
                let catalog=cx.global_mut::<RecoveryCatalog>();
                catalog.loading=false;
                match result {Ok((candidates,warning))=>{catalog.candidates=candidates;catalog.warning=warning;},Err(error)=>catalog.error=Some(error.to_string())}
            });
        }).detach();
    }

    pub(super) fn offer_document_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.home_visible || !window.is_window_active() || !self.open_requests.pending.is_empty() {return false;}
        let Some(document)=self.document() else {return false;};
        let Some(path)=document.read(cx).path() else {return false;};
        if document.read(cx).ready(cx).is_err() {return false;}
        let Some(catalog)=cx.try_global::<RecoveryCatalog>() else {return false;};
        let candidate=catalog.candidates.iter().find(|candidate| candidate.verified && candidate.path.as_ref()==Some(&path)
            && !catalog.offered.contains(&candidate.document_id) && !catalog.busy.contains_key(&candidate.document_id)).cloned();
        if let Some(candidate)=candidate {self.choose_recovery(candidate,window,cx);true} else {false}
    }

    pub(super) fn show_recovery_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() || self.application_dialogs.busy {return;}
        if !self.leave_mini(window,cx) {return;}
        self.refresh_recovery_catalog(cx);
        let Some(catalog)=cx.try_global::<RecoveryCatalog>() else {return;};
        if catalog.loading {self.notice=Some(ToolbarLocale::current(cx).app_text("recovery.loading").into());cx.notify();return;}
        if let Some(error)=&catalog.error {self.failure_notice=Some(ToolbarLocale::current(cx).app_format("recovery.failed", &[("error", &error.to_string())]).into());cx.notify();return;}
        let available=catalog.candidates.iter().filter(|candidate|!catalog.busy.contains_key(&candidate.document_id)).count();
        let candidates=catalog.candidates.iter().filter(|candidate|!catalog.busy.contains_key(&candidate.document_id)).take(32).cloned().collect::<Vec<_>>();
        if candidates.is_empty() {
            if let Some(warning)=&catalog.warning {self.failure_notice=Some(ToolbarLocale::current(cx).app_format("recovery.legacyWarning", &[("error", &warning.to_string())]).into());}
            else {self.notice=Some(ToolbarLocale::current(cx).app_text("recovery.empty").into());}
            cx.notify();return;
        }
        let mut actions=candidates.iter().map(|candidate|A::new(candidate.label(),IconName::FileText,Tone::Normal)).collect::<Vec<_>>();
        actions.push(A::new(UiMessage::new("recovery.later"),IconName::Close,Tone::Normal));
        let mut spec=dialog(UiMessage::new("recovery.title"),UiMessage::new("recovery.choose").into(),actions,candidates.len());
        if available>candidates.len() {spec.message=UiMessage::new("recovery.chooseMore").arg("shown",candidates.len()).arg("total",available);}
        if let Some(warning)=&catalog.warning {spec.detail=Some(UiMessage::new("recovery.legacyWarning").arg("error",warning));}
        spec.kind=SecondaryKind::Folder;
        spec.leading=candidates.iter().map(|candidate|candidate.path.as_ref().map(|path|UiMessage::raw(super::explorer_actions::display_tree_path(path))).unwrap_or_else(||UiMessage::new("recovery.unsavedPath"))).collect();
        self.application_dialogs.busy=true;
        let response=dialogs::choose(spec,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                if let Some(candidate)=candidates.get(choice.index) {workspace.choose_recovery(candidate.clone(),window,cx);}
                workspace.drain_open_requests(window,cx);
            });
        }).detach();
    }

    fn choose_recovery(&mut self,candidate:RecoveryCandidate,window:&mut Window,cx:&mut Context<Self>) {
        if let Some(path)=&candidate.path {
            let current=window.window_handle();
            for handle in cx.windows().into_iter().filter_map(|handle|handle.downcast::<Workspace>()) {
                if gpui::AnyWindowHandle::from(handle)==current {continue;}
                let found=handle.update(cx,|workspace,window,cx| {
                    if !workspace.tabs.iter().any(|tab|tab.document.read(cx).path().as_ref()==Some(path)) {return false;}
                    workspace.reveal_window(window,cx);
                    workspace.choose_recovery_here(candidate.clone(),window,cx);
                    true
                }).unwrap_or(false);
                if found {return;}
            }
        }
        self.choose_recovery_here(candidate,window,cx);
    }

    fn choose_recovery_here(&mut self,candidate:RecoveryCandidate,window:&mut Window,cx:&mut Context<Self>) {
        if window.has_active_prompt() || self.application_dialogs.busy {return;}
        if !self.opening.is_empty() {self.notice=Some(ToolbarLocale::current(cx).app_text("recovery.opening").into());cx.notify();return;}
        if !self.leave_mini(window,cx) {return;}
        let owner=cx.entity().downgrade();
        let catalog=cx.global_mut::<RecoveryCatalog>();
        if catalog.busy.contains_key(&candidate.document_id) || !catalog.candidates.iter().any(|entry|entry.document_id==candidate.document_id) {return;}
        catalog.busy.insert(candidate.document_id.clone(),owner);
        catalog.offered.insert(candidate.document_id.clone());
        let named=candidate.path.is_some();
        let message=UiMessage::new(if !candidate.verified {"recovery.unverified"} else if candidate.conflict {"recovery.conflict"} else {"recovery.found"}).arg("name",candidate.label());
        let spec=dialog(UiMessage::new("recovery.title"),message,vec![
            A::new(UiMessage::new("recovery.saveAs"),IconName::Save,Tone::Accent),A::new(UiMessage::new("recovery.restore"),IconName::FileText,Tone::Primary),
            A::new(if named {UiMessage::new("recovery.useDisk")}else{UiMessage::new("recovery.discard")},IconName::Trash2,if named {Tone::Normal}else{Tone::DangerSoft}),
            A::new(UiMessage::new("recovery.later"),IconName::Close,Tone::Normal),
        ],3);
        self.application_dialogs.busy=true;
        let response=dialogs::choose(spec,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                cx.global_mut::<RecoveryCatalog>().busy.remove(&candidate.document_id);
                match choice.index {
                    0|1=>{
                        let copy=choice.index==0;
                        let previous=(!copy).then(||workspace.tabs.iter().find(|tab|candidate.path.is_some()&&tab.document.read(cx).path()==candidate.path).map(|tab|tab.document.clone())).flatten();
                        if previous.as_ref().is_some_and(|document|document.read(cx).is_dirty(cx)) {
                            workspace.failure_notice=Some(ToolbarLocale::current(cx).app_text("recovery.currentDirty").into());cx.notify();return;
                        }
                        let revision=previous.as_ref().map_or(0,|document|document.read(cx).status().revision);
                        let path=if copy {None}else{candidate.path.clone()};

                        let generation=workspace.next_open_intent();
                        workspace.open_document_from(path,false,None,OpenSource::Recovery(candidate,copy,previous,revision),generation,window,cx);
                    }
                    2 if named=>{
                        let path=candidate.path.clone().expect("named recovery");
                        let previous=workspace.tabs.iter().find(|tab|tab.document.read(cx).path().as_ref()==Some(&path)).map(|tab|tab.document.clone());
                        if !path.is_file() || previous.as_ref().is_some_and(|document|document.read(cx).is_dirty(cx)) {
                            workspace.failure_notice=Some(ToolbarLocale::current(cx).app_text("recovery.diskUnavailable").into());cx.notify();return;
                        }
                        let revision=previous.as_ref().map_or(0,|document|document.read(cx).status().revision);

                        let generation=workspace.next_open_intent();
                        workspace.open_document_from(Some(path),false,None,OpenSource::DiskRecovery(candidate,previous,revision),generation,window,cx);
                    }
                    2=>workspace.confirm_discard_recovery(candidate,window,cx),
                    _=>workspace.drain_open_requests(window,cx),
                }
            });
        }).detach();
    }

    fn confirm_discard_recovery(&mut self,candidate:RecoveryCandidate,window:&mut Window,cx:&mut Context<Self>) {
        let Some(manager)=self.services.clone() else {return;};
        let owner=cx.entity().downgrade();
        cx.global_mut::<RecoveryCatalog>().busy.insert(candidate.document_id.clone(),owner);
        self.application_dialogs.busy=true;
        let message=UiMessage::new(if candidate.is_legacy() {"recovery.discardLegacy"} else {"recovery.discardPrompt"}).arg("name",candidate.label());
        let response=dialogs::choose(dialog(UiMessage::new("recovery.discardTitle"),message,vec![A::new(UiMessage::new("recovery.keep"),IconName::Close,Tone::Normal),A::new(UiMessage::new("recovery.discard"),IconName::Trash2,Tone::Danger)],0),window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let result=if choice.index==1 {let candidate=candidate.clone();Some(cx.background_executor().spawn(async move{manager.dismiss_recovery(&candidate)}).await)}else{None};
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                cx.global_mut::<RecoveryCatalog>().busy.remove(&candidate.document_id);
                match result {Some(Ok(()))=>cx.global_mut::<RecoveryCatalog>().candidates.retain(|entry|entry.document_id!=candidate.document_id),Some(Err(error))=>workspace.failure_notice=Some(ToolbarLocale::current(cx).app_format("recovery.cleanupFailed", &[("error", &error.to_string())]).into()),None=>{}}
                workspace.drain_open_requests(window,cx);cx.notify();
            });
        }).detach();
    }

    fn refresh_recovery_catalog(&self,cx:&mut Context<Self>) {
        if cx.try_global::<RecoveryCatalog>().is_none() {return;}
        let catalog=cx.global_mut::<RecoveryCatalog>();
        catalog.busy.retain(|_,owner|owner.upgrade().is_some());
        if let Some(manager)=&self.services {for candidate in &mut catalog.candidates {manager.refresh_recovery_location(candidate);}}
    }

    pub(super) fn layout_recovery_candidate(&self,id:&str,cx:&App)->Result<Option<RecoveryCandidate>,String> {
        let catalog=cx.try_global::<RecoveryCatalog>().ok_or(ToolbarLocale::current(cx).app_text("recovery.notReady"))?;
        if catalog.loading { return Err(ToolbarLocale::current(cx).app_text("recovery.waitLayout").into()); }
        if let Some(error)=&catalog.error {return Err(error.clone());}
        if catalog.busy.contains_key(id) {return Err(ToolbarLocale::current(cx).app_text("recovery.busy").into());}
        Ok(catalog.candidates.iter().find(|candidate|candidate.document_id==id).cloned())
    }
    pub(super) fn begin_recovery_attempt(&mut self,source:&OpenSource,cx:&mut Context<Self>) -> bool {
        let candidate=match source {OpenSource::Layout(_,Some(candidate))|OpenSource::Recovery(candidate,_,_,_)|OpenSource::DiskRecovery(candidate,_,_)=>candidate,_=>return true};
        let owner=cx.entity().downgrade();
        let catalog=cx.global_mut::<RecoveryCatalog>();
        catalog.busy.retain(|_,owner|owner.upgrade().is_some());
        // 输入收尾可能跨越其他窗口的恢复操作，真正开始前再次核对归属。
        if catalog.busy.contains_key(&candidate.document_id) || !catalog.candidates.iter().any(|entry|entry.document_id==candidate.document_id) {
            self.notice=Some(ToolbarLocale::current(cx).app_text("recovery.handled").into());
            cx.notify();
            return false;
        }
        catalog.busy.insert(candidate.document_id.clone(),owner);
        true
    }

    pub(super) fn finish_recovery_attempt(&mut self,source:&OpenSource,accepted:bool,cx:&mut Context<Self>) {
        let candidate=match source {OpenSource::Layout(_,Some(candidate))|OpenSource::Recovery(candidate,_,_,_)|OpenSource::DiskRecovery(candidate,_,_)=>candidate,_=>return};
        let catalog=cx.global_mut::<RecoveryCatalog>();
        catalog.busy.remove(&candidate.document_id);
        if accepted {catalog.candidates.retain(|entry|entry.document_id!=candidate.document_id);}
        else if let Some(manager)=&self.services {if let Some(entry)=catalog.candidates.iter_mut().find(|entry|entry.document_id==candidate.document_id) {manager.refresh_recovery_location(entry);}}
    }
}

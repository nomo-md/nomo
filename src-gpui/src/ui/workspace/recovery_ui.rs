//! 恢复目录属于应用；主页不自动提示，取消不会删除资料或丢失再次进入的入口。
use super::*;
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
}
impl gpui::Global for RecoveryCatalog {}
impl Drop for RecoveryCatalog {
    fn drop(&mut self) {self.cancel.store(true,std::sync::atomic::Ordering::Release);}
}

fn dialog(title: &str, message: String, actions: Vec<A>, cancel: usize) -> SecondaryDialog {
    SecondaryDialog {kind:SecondaryKind::Recovery,title:title.into(),message,actions,cancel,detail:None,notes:None,remember:None,leading:Vec::new()}
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
                match result {Ok(candidates)=>catalog.candidates=candidates,Err(error)=>catalog.error=Some(error.to_string())}
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
        if catalog.loading {self.notice=Some("正在检查恢复资料，请稍后重试。".into());cx.notify();return;}
        if let Some(error)=&catalog.error {self.failure_notice=Some(format!("恢复资料检查失败：{error}").into());cx.notify();return;}
        let candidates=catalog.candidates.iter().filter(|candidate|!catalog.busy.contains_key(&candidate.document_id)).cloned().collect::<Vec<_>>();
        if candidates.is_empty() {self.notice=Some("没有待恢复的未保存文档。".into());cx.notify();return;}
        let mut actions=candidates.iter().map(|candidate|A::new(candidate.label(),IconName::FileText,Tone::Normal)).collect::<Vec<_>>();
        actions.push(A::new("稍后处理",IconName::Close,Tone::Normal));
        let mut spec=dialog("恢复未保存文档","选择要处理的草稿；稍后处理会保留恢复资料。".into(),actions,candidates.len());
        spec.kind=SecondaryKind::Folder;
        spec.leading=candidates.iter().map(|candidate|candidate.path.as_ref().map(|path|super::explorer_actions::display_tree_path(path)).unwrap_or_else(||"尚未保存到文件".into())).collect();
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
        if !self.opening.is_empty() {self.notice=Some("文档仍在打开，请稍后从恢复列表重试。".into());cx.notify();return;}
        if !self.leave_mini(window,cx) {return;}
        let owner=cx.entity().downgrade();
        let catalog=cx.global_mut::<RecoveryCatalog>();
        if catalog.busy.contains_key(&candidate.document_id) || !catalog.candidates.iter().any(|entry|entry.document_id==candidate.document_id) {return;}
        catalog.busy.insert(candidate.document_id.clone(),owner);
        catalog.offered.insert(candidate.document_id.clone());
        let named=candidate.path.is_some();
        let message=if !candidate.verified {format!("{}\n保存状态尚未核实，可恢复后检查内容，原资料会保留到处理完成。",candidate.label())} else if candidate.conflict {format!("{}\n磁盘文件也发生了变化，请选择要保留的内容。",candidate.label())}
            else {format!("{}\n发现尚未保存的内容。稍后处理会保留草稿，可从文件菜单再次进入。",candidate.label())};
        let spec=dialog("恢复未保存文档",message,vec![
            A::new("另存草稿",IconName::Save,Tone::Accent),A::new("恢复草稿",IconName::FileText,Tone::Primary),
            A::new(if named {"使用磁盘"}else{"放弃草稿"},IconName::Trash2,if named {Tone::Normal}else{Tone::DangerSoft}),
            A::new("稍后处理",IconName::Close,Tone::Normal),
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
                            workspace.failure_notice=Some("当前文档已有未保存修改，请先保存当前文档，或选择另存草稿。".into());cx.notify();return;
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
                            workspace.failure_notice=Some("磁盘文件不可用或当前文档已有修改，恢复资料仍保留。".into());cx.notify();return;
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
        let response=dialogs::choose(dialog("放弃未保存草稿",format!("将删除“{}”的恢复资料，此操作无法撤销。",candidate.label()),vec![A::new("保留草稿",IconName::Close,Tone::Normal),A::new("放弃草稿",IconName::Trash2,Tone::Danger)],0),window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let result=if choice.index==1 {let candidate=candidate.clone();Some(cx.background_executor().spawn(async move{manager.dismiss_recovery(&candidate)}).await)}else{None};
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                cx.global_mut::<RecoveryCatalog>().busy.remove(&candidate.document_id);
                match result {Some(Ok(()))=>cx.global_mut::<RecoveryCatalog>().candidates.retain(|entry|entry.document_id!=candidate.document_id),Some(Err(error))=>workspace.failure_notice=Some(format!("清理草稿失败：{error}").into()),None=>{}}
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

    pub(super) fn begin_recovery_attempt(&mut self,source:&OpenSource,cx:&mut Context<Self>) -> bool {
        let candidate=match source {OpenSource::Recovery(candidate,_,_,_)|OpenSource::DiskRecovery(candidate,_,_)=>candidate,_=>return true};
        let owner=cx.entity().downgrade();
        let catalog=cx.global_mut::<RecoveryCatalog>();
        catalog.busy.retain(|_,owner|owner.upgrade().is_some());
        // 输入收尾可能跨越其他窗口的恢复操作，真正开始前再次核对归属。
        if catalog.busy.contains_key(&candidate.document_id) || !catalog.candidates.iter().any(|entry|entry.document_id==candidate.document_id) {
            self.notice=Some("此草稿正在其他窗口处理或已恢复，请重新查看恢复列表。".into());
            cx.notify();
            return false;
        }
        catalog.busy.insert(candidate.document_id.clone(),owner);
        true
    }

    pub(super) fn finish_recovery_attempt(&mut self,source:&OpenSource,accepted:bool,cx:&mut Context<Self>) {
        let candidate=match source {OpenSource::Recovery(candidate,_,_,_)|OpenSource::DiskRecovery(candidate,_,_)=>candidate,_=>return};
        let catalog=cx.global_mut::<RecoveryCatalog>();
        catalog.busy.remove(&candidate.document_id);
        if accepted {catalog.candidates.retain(|entry|entry.document_id!=candidate.document_id);}
        else if let Some(manager)=&self.services {if let Some(entry)=catalog.candidates.iter_mut().find(|entry|entry.document_id==candidate.document_id) {manager.refresh_recovery_location(entry);}}
    }
}

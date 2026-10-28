//! 外部变更、恢复、打开位置及关闭选择；只保存身份和弱生命周期任务，不保存正文。
use super::*;
use crate::text_document::markdown_session::{ExternalChange, RecoveryCandidate};
use crate::ui::{dialogs::{SecondaryDialog, SecondaryKind, SecondaryAction as A, Tone}, toolbar_i18n::ToolbarLocale};
use std::collections::{HashMap, VecDeque};

#[derive(Clone)]
pub(super) enum OpenSource { File, Reload(Entity<DocumentHost>,u64), Recovery(RecoveryCandidate,bool), DiskRecovery(RecoveryCandidate,Option<Entity<DocumentHost>>,u64) }
#[derive(Default)]
pub(super) struct ApplicationDialogs {
    pub ready: bool,
    pub close_approved: bool,
    tray: Option<crate::ui::tray::TrayIcon>,
    watch: Option<Task<()>>,
    probe: Option<Task<()>>,
    checking: bool,
    generation: u64,
    busy: bool,
    ignored: HashMap<String,ExternalChange>,
    recoveries: VecDeque<RecoveryCandidate>,
    opening: VecDeque<PathBuf>,
}
#[derive(Default)]
struct RecoveryStarted;
impl gpui::Global for RecoveryStarted {}

fn spec(kind:SecondaryKind,title:String,message:String,actions:Vec<A>,cancel:usize)->SecondaryDialog {
    SecondaryDialog {kind,title,message,detail:None,notes:None,actions,cancel,remember:None,leading:Vec::new()}
}
fn display(path:&Path)->String { super::explorer_actions::display_tree_path(path) }
pub(super) fn clear_external_after_save(workspace:&mut Workspace,document:&Entity<DocumentHost>,cx:&App){workspace.application_dialogs.ignored.remove(&document.read(cx).status().document_id);workspace.application_dialogs.generation=workspace.application_dialogs.generation.wrapping_add(1);}
pub(super) fn mark_recovered_change(workspace:&mut Workspace,document:&Entity<DocumentHost>,source:&OpenSource,cx:&App){
    if let OpenSource::Recovery(candidate,_)=source {if let Some(change)=&candidate.change {workspace.application_dialogs.ignored.insert(document.read(cx).status().document_id.clone(),change.clone());}}
}

impl Workspace {
    pub(crate) fn reveal_window(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        if let Some(tray)=&self.application_dialogs.tray{tray.restore();}
        window.activate_window();
        if let Some(document)=self.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}
    }
    pub(super) fn start_application_dialogs(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        self.application_dialogs.ready=true;
        if cx.try_global::<RecoveryStarted>().is_none() {
            cx.set_global(RecoveryStarted);
            if let Some(manager)=self.services.clone() {
                let task=cx.background_executor().spawn(async move {manager.recovery_candidates()});
                cx.spawn_in(window,async move |this,cx| {
                    let result=task.await;
                    let _=this.update_in(cx,|workspace,_,cx| {match result {Ok(candidates)=>workspace.application_dialogs.recoveries=candidates.into(),Err(error)=>workspace.failure_notice=Some(format!("恢复资料检查失败：{error}").into())};cx.notify();});
                }).detach();
            }
        }
        self.application_dialogs.watch=Some(cx.spawn_in(window,async move |this,cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1200)).await;
                if this.update_in(cx,|workspace,window,cx| {
                    if workspace.application_dialogs.busy || window.has_active_prompt() || workspace.closing_window || !workspace.opening.is_empty() {return;}
                    if let Some(path)=workspace.application_dialogs.opening.pop_front() {workspace.route_open_path(path,window,cx);return;}
                    if let Some(candidate)=workspace.application_dialogs.recoveries.pop_front() {workspace.choose_recovery(candidate,window,cx);return;}
                    if crate::ui::updates::present_pending(window,cx){return;}
                    if window.is_window_active() {workspace.check_external_change(window,cx);}
                }).is_err(){break;}
            }
        }));
        crate::ui::updates::startup(window,cx);
    }

    pub(super) fn route_open_path(&mut self,path:PathBuf,window:&mut Window,cx:&mut Context<Self>) {
        if self.application_dialogs.busy || window.has_active_prompt() {
            if self.application_dialogs.opening.len()<64 {self.application_dialogs.opening.push_back(path);}
            else {self.failure_notice=Some("打开请求过多，请等待当前选择完成。".into());cx.notify();}
            return;
        }
        let path=path.canonicalize().unwrap_or(path);
        let current=window.window_handle();
        let mut directory_windows=Vec::new();
        for handle in cx.windows().into_iter().filter_map(|handle|handle.downcast::<Workspace>()) {
            if gpui::AnyWindowHandle::from(handle)==current {continue;}
            let match_result=handle.update(cx,|workspace,window,cx| {
                if let Some(index)=workspace.tabs.iter().position(|tab|tab.document.read(cx).path().as_ref()==Some(&path)) {workspace.activate(index,window,cx);workspace.reveal_window(window,cx);return (true,None);}
                (false,workspace.tree.as_ref().filter(|tree|path.starts_with(&tree.root)).map(|tree|tree.root.clone()))
            });
            if let Ok((opened,root))=match_result {if opened{return;}if let Some(root)=root {directory_windows.push((handle,root));}}
        }
        let reusable=self.tabs.is_empty() && self.opening.is_empty();
        let belongs=self.tree.as_ref().is_some_and(|tree|path.starts_with(&tree.root));
        let already=self.tabs.iter().any(|tab|tab.document.read(cx).path().as_ref()==Some(&path));
        let preference=cx.global::<Settings>().string("openDefaultBehavior");
        if reusable || belongs || already || preference=="current-window" {self.open_path_with_preview(&path,false,window,cx);return;}
        if preference=="new-window" {crate::open_workspace(vec![path],cx);return;}
        let locale=ToolbarLocale::current(cx);
        let name=path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        let mut actions=directory_windows.iter().map(|_|A::new(locale.text("openInDirectoryWindow"),IconName::FolderOpen,Tone::Normal)).collect::<Vec<_>>();
        let current_index=actions.len();
        actions.extend([A::new(locale.text("openInCurrentWindow"),IconName::FileText,Tone::Normal),A::new(locale.text("openInNewWindow"),IconName::ExternalLink,Tone::Primary),A::new(locale.text("cancel"),IconName::Close,Tone::Normal)]);
        let mut dialog=spec(SecondaryKind::Folder,locale.text("folderOpenQuestion").replace("{folderName}",&name),display(&path),actions,current_index+2);
        dialog.remember=Some(locale.text("rememberDefaultWindowChoice").into());
        dialog.leading=directory_windows.iter().map(|(_,root)|display(root)).collect();
        self.application_dialogs.busy=true;
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                if choice.index<current_index {
                    if let Some((handle,_))=directory_windows.get(choice.index) {let _=handle.update(cx,|target,window,cx|{target.open_path_with_preview(&path,false,window,cx);target.reveal_window(window,cx);});}
                } else if choice.index==current_index {if choice.remember {Settings::set(cx,"openDefaultBehavior",serde_json::json!("current-window"));}workspace.open_path_with_preview(&path,false,window,cx);}
                else if choice.index==current_index+1 {if choice.remember {Settings::set(cx,"openDefaultBehavior",serde_json::json!("new-window"));}crate::open_workspace(vec![path],cx);}
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn check_external_change(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        if self.application_dialogs.checking || window.has_active_prompt() || self.application_dialogs.busy {return;}
        let Some(document)=self.document() else {return;};
        if document.read(cx).ready(cx).is_err() || document.read(cx).path().is_none() {return;}
        let port=document.read(cx).port();
        self.application_dialogs.ignored.retain(|id,change|self.tabs.iter().any(|tab|{let host=tab.document.read(cx);host.status().document_id==*id&&host.path().as_ref()==Some(&change.path)}));
        self.application_dialogs.checking=true;
        let generation=self.application_dialogs.generation;
        let task=cx.background_executor().spawn(async move {port.manager.external_change(&port.owner,&port.document_id)});
        self.application_dialogs.probe=Some(cx.spawn_in(window,async move |this,cx| {
            let result=task.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.checking=false;
                if generation!=workspace.application_dialogs.generation || workspace.document().as_ref()!=Some(&document) || window.has_active_prompt() || document.read(cx).ready(cx).is_err() {return;}
                match result {
                    Ok(Some(change)) if document.read(cx).status().revision==change.revision && document.read(cx).path().as_ref()==Some(&change.path) => {
                        let id=document.read(cx).status().document_id.clone();
                        if workspace.application_dialogs.ignored.get(&id).map(|change|&change.token)!=Some(&change.token) {
                            if workspace.mini.active() {
                                workspace.application_dialogs.ignored.insert(id,change);
                                cx.notify();
                            } else {workspace.choose_external(document,change,window,cx);}
                        }
                    }
                    Err(error)=>{workspace.failure_notice=Some(format!("检查外部文件变化失败：{error}").into());cx.notify();},
                    _=>{}
                }
            });
        }));
    }

    fn choose_external(&mut self,document:Entity<DocumentHost>,change:ExternalChange,window:&mut Window,cx:&mut Context<Self>) {
        if !self.leave_mini(window,cx) {return;}
        self.cancel_close(cx);
        let locale=ToolbarLocale::current(cx);
        let deleted=change.deleted;
        let labels=if deleted {["saveAs","recreateOriginalFile","closeAndDiscard","keepTemporarily"]} else {["reloadExternalShort","saveAs","overwriteExternalShort","ignoreExternalChange"]};
        let actions=labels.into_iter().enumerate().map(|(index,key)|A::new(locale.text(key),if deleted && index==2 {IconName::Trash2} else if index==3 {if deleted{IconName::Clock3}else{IconName::Close}} else if key=="saveAs" {IconName::Save} else if !deleted&&index==0 {IconName::Download}else{IconName::Upload},if index==3 {Tone::Quiet}else if deleted && index==2 {Tone::DangerSoft}else if (!deleted && index==2)||(deleted && index==1) {Tone::Danger}else if key=="saveAs" {Tone::TextAccent}else{Tone::Accent})).collect();
        let message=locale.text(if deleted {"externalFileDeleted"}else if document.read(cx).is_dirty(cx){"externalFileModifiedDirty"}else{"externalFileModifiedClean"}).to_owned();
        let dialog=spec(SecondaryKind::External,locale.text("externalFileChanged").into(),message,actions,3);
        self.application_dialogs.busy=true;
        self.pending_saves.retain(|pending|pending.document!=document);
        if let Some(tab)=self.tabs.iter_mut().find(|tab|tab.document==document){tab.autosave=None;}
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                if !workspace.tabs.iter().any(|tab|tab.document==document) || document.read(cx).path().as_ref()!=Some(&change.path) {return;}
                if document.read(cx).status().revision!=change.revision {workspace.failure_notice=Some("文档已有新修改，请重新处理外部变化。".into());cx.notify();return;}
                let id=document.read(cx).status().document_id.clone();
                // 忽略只记住本次文件身份；普通保存仍由原服务拒绝覆盖未确认的外部内容。
                workspace.application_dialogs.ignored.insert(id,change.clone());
                match (deleted,choice.index) {
                    (false,0)=>workspace.open_document_from(Some(change.path.clone()),false,None,OpenSource::Reload(document,change.revision),window,cx),
                    (false,1)|(true,0)=>workspace.save_document(&document,true,false,window,cx),
                    (false,2)|(true,1)=>workspace.overwrite_external(document,change,window,cx),
                    (true,2)=>workspace.remove_tab(&document,true,window,cx),
                    _=>{},
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn external_save_blocked(&self,document:&Entity<DocumentHost>,cx:&App)->bool {let host=document.read(cx);self.application_dialogs.ignored.get(&host.status().document_id).is_some_and(|change|host.path().as_ref()==Some(&change.path))}
    pub(super) fn resolve_external_before_save(&mut self,document:&Entity<DocumentHost>,window:&mut Window,cx:&mut Context<Self>)->bool {
        let Some(mut change)=self.application_dialogs.ignored.get(&document.read(cx).status().document_id).cloned() else{return false;};
        if document.read(cx).path().as_ref()!=Some(&change.path){self.application_dialogs.ignored.remove(&document.read(cx).status().document_id);return false;}
        change.revision=document.read(cx).status().revision;
        self.choose_external(document.clone(),change,window,cx);true
    }
    fn overwrite_external(&mut self,document:Entity<DocumentHost>,change:ExternalChange,window:&mut Window,cx:&mut Context<Self>) {
        let (port,snapshot)=match document.update(cx,|host,cx|host.begin_save(cx)) {Ok(value)=>value,Err(error)=>{self.failure_notice=Some(error.to_string().into());cx.notify();return;}};
        let before=cx.global::<Settings>().snapshot_before_save;
        let task=cx.background_executor().spawn(async move {port.save_external_captured(snapshot,&change,before)});
        cx.spawn_in(window,async move |this,cx| {let result=task.await;let _=this.update_in(cx,|workspace,_,cx| {document.update(cx,|host,cx|host.finish_save(result,cx));clear_external_after_save(workspace,&document,cx);workspace.refresh_tree(cx);cx.notify();});}).detach();
    }

    fn choose_recovery(&mut self,candidate:RecoveryCandidate,window:&mut Window,cx:&mut Context<Self>) {
        let locale=ToolbarLocale::current(cx);
        let name=candidate.path.as_ref().and_then(|path|path.file_name()).map(|name|name.to_string_lossy().into_owned()).unwrap_or_else(||"未命名".into());
        let dialog=spec(SecondaryKind::Recovery,locale.text(if candidate.conflict {"startupDraftConflictTitle"}else{"nativeRecoveryTitle"}).into(),locale.text(if candidate.conflict {"startupDraftConflictMessage"}else{"nativeRecoveryMessage"}).replace("{fileName}",&name),vec![A::new(locale.text("startupDraftConflictSaveDraftAs"),IconName::Save,Tone::Accent),A::new(locale.text("startupDraftConflictRestoreDraft"),IconName::AlertTriangle,Tone::Warning),A::new(locale.text(if candidate.path.is_some(){"startupDraftConflictUseDisk"}else{"cancel"}),IconName::Close,Tone::Normal)],3);
        self.application_dialogs.busy=true;
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {let choice=response.await;let _=this.update_in(cx,|workspace,window,cx| {
            workspace.application_dialogs.busy=false;
            match choice.index {
                0|1=>{let path=candidate.path.clone();workspace.open_document_from(path,false,None,OpenSource::Recovery(candidate,choice.index==0),window,cx);},
                2=>{if let Some(path)=candidate.path.clone() {
                    if !path.exists(){workspace.failure_notice=Some("磁盘文件已不存在，恢复资料仍保留。".into());}
                    else {
                        let previous=workspace.tabs.iter().find(|tab|tab.document.read(cx).path().as_ref()==Some(&path)).map(|tab|tab.document.clone());
                        let revision=previous.as_ref().map(|document|document.read(cx).status().revision).unwrap_or(0);
                        if previous.as_ref().is_some_and(|document|document.read(cx).is_dirty(cx)){workspace.failure_notice=Some("当前文档已有新修改，草稿和当前内容均保留。".into());}
                        else{workspace.open_document_from(Some(path),false,None,OpenSource::DiskRecovery(candidate,previous,revision),window,cx);}
                    }
                }},
                _=>{},
            }
            cx.notify();
        });}).detach();
    }

    pub(super) fn choose_close_behavior(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        if self.application_dialogs.tray.as_ref().is_some_and(|tray|tray.exit_requested()) {cx.defer(crate::quit_workspaces);return;}
        let preference=cx.global::<Settings>().string("closeWindowBehavior");
        if preference=="close-window" {self.application_dialogs.close_approved=true;if self.request_close_window(window,cx){window.remove_window();}return;}
        if preference=="close-to-tray" && cfg!(windows) {self.hide_to_tray(window,cx);return;}
        let locale=ToolbarLocale::current(cx);
        let mut actions=vec![A::new(locale.text("closeWindow"),IconName::Close,Tone::Normal)];
        if cfg!(windows) {actions.push(A::new(locale.text("closeToTray"),IconName::Minus,Tone::Primary));}
        let cancel=actions.len();
        let mut dialog=spec(SecondaryKind::Close,locale.text("closeWindowChoiceTitle").into(),locale.text("closeWindowChoiceMessage").into(),actions,cancel);
        dialog.remember=Some(locale.text("rememberCloseWindowChoice").into());
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {let choice=response.await;let _=this.update_in(cx,|workspace,window,cx| {
            if choice.index==0 {if choice.remember{Settings::set(cx,"closeWindowBehavior",serde_json::json!("close-window"));}workspace.application_dialogs.close_approved=true;if workspace.request_close_window(window,cx){window.remove_window();}}
            else if choice.index==1 && cfg!(windows) {if workspace.hide_to_tray(window,cx) && choice.remember{Settings::set(cx,"closeWindowBehavior",serde_json::json!("close-to-tray"));}}
        });}).detach();
    }
    fn hide_to_tray(&mut self,window:&mut Window,cx:&mut Context<Self>)->bool {
        if let Some(document)=self.document(){if let Err(error)=document.update(cx,|host,cx|host.set_active(false,cx)){self.failure_notice=Some(error.to_string().into());cx.notify();return false;}}
        let result=if let Some(tray)=&self.application_dialogs.tray {tray.hide_again();Ok(())}else{crate::ui::tray::TrayIcon::hide(window).map(|tray|self.application_dialogs.tray=Some(tray))};
        if let Err(error)=result {self.failure_notice=Some(error.into());if let Some(document)=self.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}cx.notify();return false;}
        true
    }
}

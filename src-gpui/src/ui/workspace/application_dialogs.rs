//! 外部变更、恢复、打开位置及关闭选择；只保存身份和弱生命周期任务，不保存正文。
use super::*;
use crate::text_document::markdown_session::{ExternalChange, RecoveryCandidate};
use crate::ui::{dialogs::{SecondaryDialog, SecondaryKind, SecondaryAction as A, Tone}, toolbar_i18n::{ToolbarLocale, UiMessage}};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone)]
pub(super) enum OpenSource { File, Layout(u64,Option<RecoveryCandidate>), Reload(Entity<DocumentHost>,u64), Recovery(RecoveryCandidate,bool,Option<Entity<DocumentHost>>,u64), DiskRecovery(RecoveryCandidate,Option<Entity<DocumentHost>>,u64) }
#[derive(Default)]
pub(super) struct ApplicationDialogs {
    pub close_approved: bool,
    tray: Option<crate::ui::tray::TrayIcon>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    tray_start: Option<Task<()>>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    tray_events: Option<Task<()>>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    tray_visibility: Option<Task<()>>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    tray_generation: u64,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    tray_hidden: bool,
    watch: Option<Task<()>>,
    probe: Option<Task<()>>,
    checking: bool,
    /// 只记录最近一次失败的身份摘要，避免每 1.2 秒的检查反复弹出相同错误。
    probe_failure: Option<u64>,
    generation: u64,
    pub(super) busy: bool,
    pub(super) ignored: HashMap<String,ExternalChange>,
    pub(super) mini_deferred: std::collections::HashSet<String>,
}


fn spec(kind:SecondaryKind,title:UiMessage,message:UiMessage,actions:Vec<A>,cancel:usize)->SecondaryDialog {
    SecondaryDialog {kind,title,message,detail:None,notes:None,actions,cancel,remember:None,leading:Vec::new()}
}
fn display(path:&Path)->String { super::explorer_actions::display_tree_path(path) }
pub(super) fn clear_external_after_save(workspace:&mut Workspace,document:&Entity<DocumentHost>,cx:&App){
    let id=&document.read(cx).status().document_id;
    workspace.application_dialogs.ignored.remove(id);
    workspace.application_dialogs.mini_deferred.remove(id);
    workspace.application_dialogs.generation=workspace.application_dialogs.generation.wrapping_add(1);
}
pub(super) fn mark_recovered_change(workspace:&mut Workspace,document:&Entity<DocumentHost>,source:&OpenSource,cx:&App){
    if let OpenSource::Recovery(candidate,false,_,_) | OpenSource::Layout(_,Some(candidate))=source {if let Some(change)=&candidate.change {workspace.application_dialogs.ignored.insert(document.read(cx).status().document_id.clone(),change.clone());}}
}

pub(crate) fn refresh_interface_locales(cx: &mut App) {
    let locale = ToolbarLocale::current(cx);
    for handle in cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
        let _ = handle.update(cx, |workspace, window, cx| {
            if let Some(document) = workspace.document() { window.set_window_title(&format!("{} — Nomo", document.read(cx).display_title(cx))); }
            if let Some(tray) = &mut workspace.application_dialogs.tray {
                if let Err(error) = tray.set_locale(locale, cx) { workspace.failure_notice = Some(tray_error(error, cx)); cx.notify(); }
            }
        });
    }
}

fn tray_error(error: impl std::fmt::Display, cx: &App) -> SharedString {
    ToolbarLocale::current(cx).app_format("tray.operationFailed", &[("error", &error.to_string())]).into()
}

impl Workspace {
    pub(crate) fn reveal_window(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        if let Some(tray)=&self.application_dialogs.tray{tray.restore();}
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.application_dialogs.tray_generation = self.application_dialogs.tray_generation.wrapping_add(1);
            self.application_dialogs.tray_start = None;
            // macOS 还需要恢复用户手动最小化的窗口，并激活应用；操作在主线程任务中合并。
            if self.application_dialogs.tray_hidden || cfg!(target_os = "macos") {
                self.application_dialogs.tray_visibility = None;
                match crate::ui::tray::restore_window(window,cx) {
                    Ok(task) => self.application_dialogs.tray_visibility = task,
                    Err(error) => {
                        self.failure_notice = Some(tray_error(error, cx));
                        cx.notify();
                        return;
                    }
                }
            }
            self.application_dialogs.tray_hidden = false;
        }
        #[cfg(not(target_os = "macos"))]
        window.activate_window();
        if let Some(document)=self.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}
    }
    pub(super) fn start_application_dialogs(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        self.start_recovery_catalog(cx);
        self.application_dialogs.watch=Some(cx.spawn_in(window,async move |this,cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1200)).await;
                if this.update_in(cx,|workspace,window,cx| {
                    if workspace.application_dialogs.busy || window.has_active_prompt() || workspace.closing_window || workspace.comparison.closing.is_some() || !workspace.opening.is_empty() {return;}
                    workspace.drain_open_requests(window,cx);
                    if workspace.application_dialogs.busy || window.has_active_prompt() || !workspace.opening.is_empty() || workspace.pending_document_transition.is_some() {return;}
                    if workspace.offer_document_recovery(window,cx) {return;}
                    if crate::ui::updates::present_pending(window,cx){return;}
                    if window.is_window_active() {workspace.check_external_change(window,cx);}
                }).is_err(){break;}
            }
        }));
        crate::ui::updates::startup(window,cx);
    }

    pub(super) fn route_open_request(&mut self,mut request:OpenRequest,window:&mut Window,cx:&mut Context<Self>) {
        let Some(path)=request.path.take() else {self.open_request_here(request,window,cx);return;};
        let path=path.canonicalize().unwrap_or(path);
        request.path=Some(path.clone());
        if self.reuse_open_document(&request,window,cx) {return;}
        let current=window.window_handle();
        let mut directory_windows=Vec::new();
        for handle in cx.windows().into_iter().filter_map(|handle|handle.downcast::<Workspace>()) {
            if gpui::AnyWindowHandle::from(handle)==current {continue;}
            if let Ok(Some(root))=handle.update(cx,|workspace,_,_|workspace.tree.as_ref().filter(|tree|path.starts_with(&tree.root)).map(|tree|tree.root.clone())) {directory_windows.push((handle,root));}
        }
        let reusable=self.tabs.is_empty() && self.opening.is_empty();
        let belongs=self.tree.as_ref().is_some_and(|tree|path.starts_with(&tree.root));
        let already=self.saved_path_key(&path).is_some() || self.tabs.iter().any(|tab|tab.document.read(cx).path().as_ref()==Some(&path));
        let preference=cx.global::<Settings>().string("openDefaultBehavior");
        if !request.choose_window || reusable || belongs || already || preference=="current-window" {self.open_request_here(request,window,cx);return;}
        if preference=="new-window" {crate::open_workspace(vec![path],cx);return;}
        let name=path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        let mut actions=directory_windows.iter().map(|_|A::new(UiMessage::toolbar("openInDirectoryWindow"),IconName::FolderOpen,Tone::Normal)).collect::<Vec<_>>();
        let current_index=actions.len();
        actions.extend([A::new(UiMessage::toolbar("openInCurrentWindow"),IconName::FileText,Tone::Normal),A::new(UiMessage::toolbar("openInNewWindow"),IconName::ExternalLink,Tone::Primary),A::new(UiMessage::toolbar("cancel"),IconName::Close,Tone::Normal)]);
        let mut dialog=spec(SecondaryKind::Folder,UiMessage::toolbar("folderOpenQuestion").arg("folderName", &name),UiMessage::raw(display(&path)),actions,current_index+2);
        dialog.remember=Some(UiMessage::toolbar("rememberDefaultWindowChoice"));
        dialog.leading=directory_windows.iter().map(|(_,root)|UiMessage::raw(display(root))).collect();
        self.application_dialogs.busy=true;
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                if choice.index<current_index {
                    if let Some((handle,_))=directory_windows.get(choice.index) {let _=handle.update(cx,|target,window,cx|{
                        target.enqueue_open(Some(path),false,None,false,cx);
                        target.drain_open_requests(window,cx);target.reveal_window(window,cx);
                    });}
                } else if choice.index==current_index {
                    if choice.remember {Settings::set(cx,"openDefaultBehavior",serde_json::json!("current-window"));}
                    if !workspace.reuse_open_document(&request,window,cx) {workspace.open_requests.pending.push_front(OpenRequest {choose_window:false,..request});}
                } else if choice.index==current_index+1 {
                    if choice.remember {Settings::set(cx,"openDefaultBehavior",serde_json::json!("new-window"));}
                    if !workspace.reuse_open_document(&request,window,cx) {crate::open_workspace(vec![path],cx);}
                }
                workspace.drain_open_requests(window,cx);
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
        self.application_dialogs.mini_deferred.retain(|id| self.application_dialogs.ignored.contains_key(id));
        self.application_dialogs.checking=true;
        let generation=self.application_dialogs.generation;
        let task=cx.background_executor().spawn(async move {port.manager.external_change(&port.owner,&port.document_id)});
        self.application_dialogs.probe=Some(cx.spawn_in(window,async move |this,cx| {
            let result=task.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.checking=false;
                if generation!=workspace.application_dialogs.generation || workspace.document().as_ref()!=Some(&document) || window.has_active_prompt() || workspace.application_dialogs.busy || !workspace.opening.is_empty() || workspace.pending_document_transition.is_some() || document.read(cx).ready(cx).is_err() {return;}
                if result.is_ok() { workspace.application_dialogs.probe_failure = None; }
                match result {
                    Ok(Some(change)) if document.read(cx).status().revision==change.revision && document.read(cx).path().as_ref()==Some(&change.path) => {
                        let id=document.read(cx).status().document_id.clone();
                        // 必须先消费延迟标记；磁盘令牌改变时也不能被 || 短路留到下一轮。
                        let deferred = !workspace.mini.active() && workspace.application_dialogs.mini_deferred.remove(&id);
                        if workspace.application_dialogs.ignored.get(&id).map(|change|&change.token)!=Some(&change.token) || deferred {
                            if workspace.mini.active() {
                                workspace.application_dialogs.mini_deferred.insert(id.clone());
                                workspace.application_dialogs.ignored.insert(id,change);
                                cx.notify();
                            } else if !workspace.apply_external_preference(&document, &change, window, cx) {workspace.choose_external(document,change,window,cx);}
                        }
                    }
                    Err(error)=>{
                        let message = crate::ui::toolbar_i18n::UiMessage::new("dialog.externalCheckFailed").arg("error", error);
                        let mut hash = DefaultHasher::new();
                        (document.entity_id(), document.read(cx).path(), &message).hash(&mut hash);
                        let key = hash.finish();
                        if workspace.application_dialogs.probe_failure != Some(key) {
                            workspace.application_dialogs.probe_failure = Some(key);
                            workspace.failure_notice = Some(message.render(ToolbarLocale::current(cx)).into());
                            cx.notify();
                        }
                    },
                    _=>{}
                }
            });
        }));
    }

    /// 原版仅在没有本地修改时自动处理；忽略仍保留保存冲突，覆盖仍验证检测到的磁盘身份。
    fn apply_external_preference(&mut self, document: &Entity<DocumentHost>, change: &ExternalChange, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if change.deleted || document.read(cx).is_dirty(cx) { return false; }
        let preference = cx.global::<Settings>().string("externalFileChangeBehavior");
        if !matches!(preference.as_str(), "reload-external" | "overwrite-external" | "ignore") { return false; }
        self.pending_saves.retain(|pending| pending.document != *document);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.document == *document) { tab.autosave = None; }
        self.application_dialogs.ignored.insert(document.read(cx).status().document_id.clone(), change.clone());
        match preference.as_str() {
            "reload-external" => {
                let generation = self.next_open_intent();
                self.open_document_from(Some(change.path.clone()), false, None, OpenSource::Reload(document.clone(), change.revision), generation, window, cx);
            }
            "overwrite-external" => self.overwrite_external(document.clone(), change.clone(), window, cx),
            _ => self.notice = Some(ToolbarLocale::current(cx).text("externalChangeKeptCurrent").to_owned().into()),
        }
        cx.notify();
        true
    }

    fn choose_external(&mut self,document:Entity<DocumentHost>,change:ExternalChange,window:&mut Window,cx:&mut Context<Self>) {
        if !self.leave_mini(window,cx) {return;}
        self.cancel_close(cx);
        let deleted=change.deleted;
        let labels=if deleted {["saveAs","recreateOriginalFile","closeAndDiscard","keepTemporarily"]} else {["reloadExternalShort","saveAs","overwriteExternalShort","ignoreExternalChange"]};
        let actions=labels.into_iter().enumerate().map(|(index,key)|A::new(UiMessage::toolbar(key),if deleted && index==2 {IconName::Trash2} else if index==3 {if deleted{IconName::Clock3}else{IconName::Close}} else if key=="saveAs" {IconName::Save} else if !deleted&&index==0 {IconName::Download}else{IconName::Upload},if index==3 {Tone::Quiet}else if deleted && index==2 {Tone::DangerSoft}else if (!deleted && index==2)||(deleted && index==1) {Tone::Danger}else if key=="saveAs" {Tone::TextAccent}else{Tone::Accent})).collect();
        let message=UiMessage::toolbar(if deleted {"externalFileDeleted"}else if document.read(cx).is_dirty(cx){"externalFileModifiedDirty"}else{"externalFileModifiedClean"});
        let dialog=spec(SecondaryKind::External,UiMessage::toolbar("externalFileChanged"),message,actions,3);
        self.application_dialogs.busy=true;
        self.pending_saves.retain(|pending|pending.document!=document);
        if let Some(tab)=self.tabs.iter_mut().find(|tab|tab.document==document){tab.autosave=None;}
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.application_dialogs.busy=false;
                if !workspace.tabs.iter().any(|tab|tab.document==document) || document.read(cx).path().as_ref()!=Some(&change.path) {return;}
                if document.read(cx).status().revision!=change.revision {workspace.failure_notice=Some(ToolbarLocale::current(cx).app_text("dialog.externalChanged").into());cx.notify();return;}
                let id=document.read(cx).status().document_id.clone();
                // 忽略只记住本次文件身份；普通保存仍由原服务拒绝覆盖未确认的外部内容。
                workspace.application_dialogs.ignored.insert(id,change.clone());
                match (deleted,choice.index) {
                    (false,0)=>{let generation=workspace.next_open_intent();workspace.open_document_from(Some(change.path.clone()),false,None,OpenSource::Reload(document,change.revision),generation,window,cx);},
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
        cx.spawn_in(window,async move |this,cx| {let result=task.await;let _=this.update_in(cx,|workspace,_,cx| {if document.update(cx,|host,cx|host.finish_save(result,cx)) {clear_external_after_save(workspace,&document,cx);workspace.refresh_tree(cx);}cx.notify();});}).detach();
    }

    pub(super) fn choose_close_behavior(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        #[cfg(windows)]
        if self.application_dialogs.tray.as_ref().is_some_and(|tray|tray.exit_requested()) {cx.defer(crate::quit_workspaces);return;}
        let preference=cx.global::<Settings>().string("closeWindowBehavior");
        if preference=="close-window" {self.application_dialogs.close_approved=true;if self.request_close_window(window,cx){window.remove_window();}return;}
        if preference=="close-to-tray" && crate::ui::tray::supported(window) {self.begin_hide_to_tray(false,window,cx);return;}
        let mut actions=vec![A::new(UiMessage::toolbar("closeWindow"),IconName::Close,Tone::Normal)];
        if crate::ui::tray::supported(window) {actions.push(A::new(UiMessage::toolbar("closeToTray"),IconName::Minus,Tone::Primary));}
        let cancel=actions.len();
        let mut dialog=spec(SecondaryKind::Close,UiMessage::toolbar("closeWindowChoiceTitle"),UiMessage::toolbar("closeWindowChoiceMessage"),actions,cancel);
        if preference=="close-to-tray" && let Some(reason)=crate::ui::tray::unavailable_reason_key(window) {
            // 旧配置可能来自 X11；说明限制后仍可选择正常关闭，不让关闭按钮失效。
            dialog.detail=Some(UiMessage::new(reason));
        }
        dialog.remember=Some(UiMessage::toolbar("rememberCloseWindowChoice"));
        let response=dialogs::choose(dialog,window,cx);
        cx.spawn_in(window,async move |this,cx| {let choice=response.await;let _=this.update_in(cx,|workspace,window,cx| {
            if choice.index==0 {if choice.remember{Settings::set(cx,"closeWindowBehavior",serde_json::json!("close-window"));}workspace.application_dialogs.close_approved=true;if workspace.request_close_window(window,cx){window.remove_window();}}
            else if choice.index==1 && crate::ui::tray::supported(window) {workspace.begin_hide_to_tray(choice.remember,window,cx);}
        });}).detach();
    }
    fn begin_hide_to_tray(&mut self,remember:bool,window:&mut Window,cx:&mut Context<Self>) {
        if let Some(reason) = crate::ui::tray::unavailable_reason_key(window) {
            self.failure_notice = Some(ToolbarLocale::current(cx).app_text(reason).into());
            cx.notify();
            return;
        }
        #[cfg(windows)]
        if self.hide_to_tray(window,cx) && remember {Settings::set(cx,"closeWindowBehavior",serde_json::json!("close-to-tray"));}
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.start_native_tray(remember,window,cx);
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let _ = remember;
    }

    #[cfg(windows)]
    fn hide_to_tray(&mut self,window:&mut Window,cx:&mut Context<Self>)->bool {
        if let Some(document)=self.document(){if let Err(error)=document.update(cx,|host,cx|host.set_active(false,cx)){self.failure_notice=Some(error.to_string().into());cx.notify();return false;}}
        let result=if let Some(tray)=&self.application_dialogs.tray {tray.hide_again();Ok(())}else{crate::ui::tray::TrayIcon::hide(window, ToolbarLocale::current(cx)).map(|tray|self.application_dialogs.tray=Some(tray))};
        if let Err(error)=result {self.failure_notice=Some(tray_error(error, cx));if let Some(document)=self.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}cx.notify();return false;}
        true
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn start_native_tray(&mut self, remember:bool, window:&mut Window, cx:&mut Context<Self>) {
        if self.application_dialogs.tray_start.is_some() { return; }
        self.application_dialogs.tray_generation = self.application_dialogs.tray_generation.wrapping_add(1);
        let generation = self.application_dialogs.tray_generation;
        if self.application_dialogs.tray.is_some() {
            self.hide_native_tray(generation,remember,window,cx);
            return;
        }
        let task = match crate::ui::tray::TrayIcon::start(window,cx) {
            Ok(task) => task,
            Err(error) => { self.failure_notice=Some(tray_error(error, cx));cx.notify();return; }
        };
        let document = self.document().map(|document| (document.entity_id(), document.read(cx).status().revision));
        let home = self.home_visible;
        self.application_dialogs.tray_start=Some(cx.spawn_in(window,async move |this,cx| {
            let result=task.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                if generation != workspace.application_dialogs.tray_generation { return; }
                workspace.application_dialogs.tray_start=None;
                let current = workspace.document().map(|document| (document.entity_id(), document.read(cx).status().revision));
                if current != document || workspace.home_visible != home {
                    workspace.notice=Some(ToolbarLocale::current(cx).app_text("tray.documentChanged").into());
                    cx.notify();return;
                }
                match result {
                    Ok(mut tray) => {
                        if let Err(error) = tray.set_locale(ToolbarLocale::current(cx), cx) { workspace.failure_notice = Some(tray_error(error, cx)); }
                        let events=tray.events();
                        workspace.application_dialogs.tray=Some(tray);
                        workspace.application_dialogs.tray_events=Some(cx.spawn_in(window,async move |this,cx| {
                            loop {
                                let event=events.next().await;
                                let keep=this.update_in(cx,|workspace,window,cx| {
                                    use crate::ui::tray::TrayEvent;
                                    match event {
                                        TrayEvent::Restore => workspace.reveal_window(window,cx),
                                        TrayEvent::Quit => {
                                            workspace.reveal_window(window,cx);
                                            if !workspace.application_dialogs.tray_hidden {cx.defer(crate::quit_workspaces);}
                                        }
                                        TrayEvent::Unavailable => {
                                            if workspace.application_dialogs.tray_hidden { workspace.reveal_window(window,cx); }
                                            workspace.application_dialogs.tray=None;
                                            workspace.application_dialogs.tray_events=None;
                                            if !workspace.application_dialogs.tray_hidden {
                                                workspace.failure_notice=Some(ToolbarLocale::current(cx).app_text("tray.hostUnavailable").into());
                                            }
                                            cx.notify();return false;
                                        }
                                    }
                                    true
                                }).unwrap_or(false);
                                if !keep { break; }
                            }
                        }));
                        workspace.hide_native_tray(generation,remember,window,cx);
                    }
                    Err(error) => {workspace.failure_notice=Some(tray_error(error, cx));cx.notify();}
                }
            });
        }));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn hide_native_tray(&mut self, generation:u64, remember:bool, window:&mut Window, cx:&mut Context<Self>) {
        if generation != self.application_dialogs.tray_generation { return; }
        if window.has_active_prompt() || self.closing_window || self.application_dialogs.close_approved
            || self.application_dialogs.busy || self.mini.active() || !self.opening.is_empty()
            || !self.open_requests.pending.is_empty() || !self.pending_saves.is_empty() {
            self.notice=Some(ToolbarLocale::current(cx).app_text("tray.operationBusy").into());cx.notify();return;
        }
        if let Some(document)=self.document() {
            if !self.prepare_document_transition(&document,true,window,cx,move |workspace,window,cx| {
                workspace.hide_native_tray(generation,remember,window,cx);
            }) { return; }
            if let Err(error)=document.update(cx,|host,cx|host.set_active(false,cx)) {
                self.failure_notice=Some(error.to_string().into());cx.notify();return;
            }
        }
        self.application_dialogs.tray_visibility=None;
        let result=self.application_dialogs.tray.as_ref().ok_or_else(||ToolbarLocale::current(cx).app_text("tray.invalid").to_owned())
            .and_then(|tray|tray.hide_again(window,cx));
        match result {
            Ok(task) => {
                self.application_dialogs.tray_visibility=task;
                self.application_dialogs.tray_hidden=true;
                if remember {Settings::set(cx,"closeWindowBehavior",serde_json::json!("close-to-tray"));}
            }
            Err(error) => {
                // 即使平台在 unmap 后的回滚失败，也保留可重试恢复的身份状态。
                self.application_dialogs.tray_hidden=true;
                match crate::ui::tray::restore_window(window,cx) {
                    Ok(task) => {
                        self.application_dialogs.tray_visibility=task;
                        self.application_dialogs.tray_hidden=false;
                        #[cfg(not(target_os = "macos"))]
                        window.activate_window();
                        if let Some(document)=self.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}
                        self.failure_notice=Some(tray_error(error, cx));
                    }
                    Err(restore) => {
                        eprintln!("关闭到托盘失败，恢复窗口也失败：{restore}");
                        self.failure_notice=Some(ToolbarLocale::current(cx).app_format("tray.restoreFailed", &[("error", &error), ("restore", &restore)]).into());
                    }
                }
                cx.notify();
            }
        }
    }
}

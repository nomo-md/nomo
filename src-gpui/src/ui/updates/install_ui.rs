use super::*;
use super::installer::{Candidate, DownloadCancel, Verified};
use crate::ui::chrome::ChromeUpdateState;

struct Transfer { owner: AnyWindowHandle, cancel: Arc<DownloadCancel>, percent: u8 }
impl Drop for Transfer { fn drop(&mut self) { self.cancel.cancel(); } }
#[derive(Default)]
pub(super) struct InstallState { transfer: Option<Transfer>, ready: Option<Verified> }
impl InstallState {
    pub(super) fn owner_closed(&mut self, windows: &[AnyWindowHandle]) {
        if let Some(t) = &mut self.transfer && !windows.contains(&t.owner) {
            // 关闭设置窗口不丢失整个应用的下载；退出所有窗口才取消。
            if let Some(owner) = windows.first() { t.owner = *owner; } else { t.cancel.cancel(); }
        }
    }
}
pub(super) fn stage(cx: &App) -> Option<ChromeUpdateState> {
    let install = &cx.try_global::<Updates>()?.install;
    if let Some(t) = &install.transfer { return Some(ChromeUpdateState::Downloading(t.percent)); }
    install.ready.as_ref().map(|_| ChromeUpdateState::Downloaded)
}

pub(super) fn start(release: &Release, window: &mut Window, cx: &mut App) {
    if handle_action(window, cx) || busy(cx) { return; }
    let candidate = match Candidate::from_release(release) {
        Ok(candidate) => candidate,
        Err(e) => { show_error(e, window, cx); return; }
    };
    let owner = window.window_handle();
    let cancel = Arc::new(DownloadCancel::default());
    cx.global_mut::<Updates>().install.transfer = Some(Transfer { owner, cancel: cancel.clone(), percent: 0 });
    cx.global_mut::<Updates>().notice = None;
    // 最多 33 个百分比通知；不使用空闲轮询或为每个网络块刷新界面。
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let work = cx.background_executor().spawn(async move {
        installer::download(candidate, &cancel, |p| { let _ = tx.send(p); })
    });
    let task = cx.spawn(async move |cx| {
        while let Some(p) = rx.recv().await {
            cx.update(|cx| {
                if let Some(t) = &mut cx.global_mut::<Updates>().install.transfer { t.percent = p; }
                cx.refresh_windows();
            });
        }
        let result = work.await;
        cx.update(|cx| {
            let Some(t) = cx.global_mut::<Updates>().install.transfer.take() else { return; };
            cx.refresh_windows();
            if t.cancel.cancelled() { return; }
            let pending = match result {
                Ok(verified) => { cx.global_mut::<Updates>().install.ready = Some(verified); Pending::Install }
                Err(e) => Pending::Failure(e),
            };
            cx.global_mut::<Updates>().pending = Some((t.owner, pending));
            let _ = t.owner.update(cx, |_,window,cx| { present_pending(window,cx); });
        });
    });
    cx.global_mut::<Updates>().task = Some(task);
    cx.refresh_windows();
}

pub(super) fn handle_action(window: &mut Window, cx: &mut App) -> bool {
    let Some(stage) = stage(cx) else { return false; };
    if window.has_active_prompt() { return true; }
    if let ChromeUpdateState::Downloading(percent) = stage {
        let Some(cancel) = cx.global::<Updates>().install.transfer.as_ref().map(|t| t.cancel.clone()) else { return true; };
        let dialog = SecondaryDialog { kind: SecondaryKind::Update, title: UiMessage::toolbar("nativeUpdateDownload"),
            message: UiMessage::new("updates.downloadProgress").arg("percent",percent), detail: None, notes: None, cancel: 0, remember: None, leading: vec![],
            actions: vec![A::new(UiMessage::toolbar("close"),IconName::Close,Tone::Normal), A::new(UiMessage::toolbar("nativeUpdateCancel"),IconName::Close,Tone::Warning)] };
        let reply = dialogs::choose(dialog,window,cx);
        window.to_async(cx).spawn(async move |_| { if reply.await.index == 1 { cancel.cancel(); } }).detach();
    } else { confirm(window, cx); }
    true
}

pub(super) fn confirm(window: &mut Window, cx: &mut App) {
    let Some(ready) = &cx.global::<Updates>().install.ready else { return; };
    let dialog = SecondaryDialog { kind: SecondaryKind::Update, title: UiMessage::toolbar("nativeUpdateReady").arg("version",&ready.version),
        message: UiMessage::toolbar("nativeUpdateRestartHint"), detail: None, notes: None, cancel: 0, remember: None, leading: vec![],
        actions: vec![A::new(UiMessage::toolbar("softwareUpdateLater"),IconName::Close,Tone::Normal), A::new(UiMessage::toolbar("nativeUpdateInstall"),IconName::RefreshCw,Tone::Primary)] };
    let reply = dialogs::choose(dialog,window,cx);
    window.to_async(cx).spawn(async move |context| {
        if reply.await.index == 1 { let _ = context.update(|_,cx| cx.defer(crate::quit_to_install)); }
    }).detach();
}

pub(crate) fn launch_after_close(cx: &mut App) -> Result<(),String> {
    let ready = cx.try_global::<Updates>().and_then(|s| s.install.ready.as_ref()).ok_or_else(|| ToolbarLocale::current(cx).app_text("installer.noVerified").to_owned())?;
    ready.launch().map_err(|message| message.render(ToolbarLocale::current(cx)))?;
    Settings::set(cx,"nativeReleaseNotesVersion",serde_json::json!(env!("CARGO_PKG_VERSION")));
    Ok(())
}

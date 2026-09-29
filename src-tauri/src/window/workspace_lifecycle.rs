//! 启动快照与多窗口退出事务。普通自动保存不改变下次启动目标。
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

const STARTUP_KEY: &str = "startupWorkspace";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STATE: OnceLock<Mutex<Lifecycle>> = OnceLock::new();

#[derive(Default)]
struct Lifecycle {
    closing: HashMap<String, (String, Vec<ClosingSession>)>,
    exit: Option<ExitRequest>,
}
struct ExitRequest {
    id: u64,
    pending: HashSet<String>,
    participants: HashSet<String>,
    target: Option<String>,
    snapshot: Option<String>,
    waiting_settings: bool,
    committing: bool,
    sessions: Vec<ClosingSession>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClosingSession {
    session_id: String,
    discard_changes: bool,
}

// 仅在窗口实际销毁或退出已提交时释放会话，取消时保留完整编辑能力。
fn release_sessions<R: Runtime>(app: &AppHandle<R>, sessions: Vec<ClosingSession>) {
    if let Some(manager) = app.try_state::<crate::text_document::DocumentSessionManager>() {
        for session in sessions {
            if let Err(error) = manager.close_session(&session.session_id, session.discard_changes)
            {
                crate::app_logger::error("Workspace", &format!("关闭分段会话失败：{error:?}"));
            }
        }
    }
}
pub(crate) fn committed_exit<R: Runtime>(app: &AppHandle<R>) {
    let sessions = {
        let mut state = state().lock().unwrap();
        state
            .exit
            .as_mut()
            .filter(|exit| exit.committing)
            .map(|exit| std::mem::take(&mut exit.sessions))
            .unwrap_or_default()
    };
    release_sessions(app, sessions);
}
fn state() -> &'static Mutex<Lifecycle> {
    STATE.get_or_init(Default::default)
}
pub(crate) fn is_exiting() -> bool {
    state().lock().unwrap().exit.is_some()
}

pub(crate) async fn wait_until_idle() {
    while is_exiting() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn snapshot<R: Runtime>(app: &AppHandle<R>, label: &str) -> Result<String, String> {
    crate::config::commands::get_setting_value(app, &format!("workspaceTabs:{label}"))?
        .ok_or_else(|| format!("窗口工作区尚未保存：{label}"))
}
fn save_startup<R: Runtime>(app: &AppHandle<R>, value: String) -> Result<(), String> {
    crate::config::with_manager(app, |manager| {
        manager.update(|config| {
            config.app.settings.insert(
                STARTUP_KEY.into(),
                crate::models::SettingRecord {
                    key: STARTUP_KEY.into(),
                    value_json: value,
                    updated_at: crate::config::now_ts(),
                },
            );
        })
    })
}
pub(crate) fn prepare_close<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    sessions: Vec<ClosingSession>,
) -> Result<(), String> {
    if !super::external_open::is_document_window_label(label) {
        return Ok(());
    }
    if is_exiting() {
        return Err("正在等待所有窗口完成退出".into());
    }
    let value = snapshot(app, label)?;
    state()
        .lock()
        .unwrap()
        .closing
        .insert(label.into(), (value, sessions));
    Ok(())
}
pub(crate) fn forget_close(label: &str) {
    state().lock().unwrap().closing.remove(label);
}

pub(crate) fn destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let (closing, unexpected) = {
        let mut state = state().lock().unwrap();
        if state.exit.as_ref().is_some_and(|exit| exit.committing) {
            return;
        }
        let unexpected = state.exit.as_ref().is_some_and(|exit| {
            super::external_open::is_document_window_label(label)
                && exit.participants.contains(label)
        });
        (state.closing.remove(label), unexpected)
    };
    if unexpected {
        cancel(app, "exitWindowLost");
    }
    if let Some((value, sessions)) = closing {
        release_sessions(app, sessions);
        if let Err(error) = save_startup(app, value) {
            crate::app_logger::error("Workspace", &error);
            let _ = app.emit("nomo://workspace-error", error);
        }
    }
}

pub(crate) fn begin<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let pending: HashSet<_> = app
        .webview_windows()
        .into_keys()
        .filter(|label| super::external_open::is_document_window_label(label))
        .collect();
    let target = super::tray::last_active_document_window_label()
        .filter(|label| pending.contains(label))
        .or_else(|| {
            let mut labels: Vec<_> = pending.iter().cloned().collect();
            labels.sort();
            labels.into_iter().next()
        });
    let recipients: Vec<_> = pending.iter().cloned().collect();
    let id = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    {
        let mut state = state().lock().unwrap();
        if state.exit.is_some() {
            return Ok(());
        }
        state.exit = Some(ExitRequest {
            id,
            participants: pending.clone(),
            pending,
            target,
            snapshot: None,
            waiting_settings: false,
            committing: false,
            sessions: Vec::new(),
        });
    }
    for label in recipients {
        if let Err(error) = app.emit_to(label, "nomo://request-exit-app", id) {
            cancel(app, "exitSaveFailed");
            return Err(error.to_string());
        }
    }
    let timeout_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        cancel_request(&timeout_app, Some(id), "exitTimedOut");
    });
    complete_if_ready(app)
}

#[tauri::command]
pub(crate) fn acknowledge_workspace_exit(
    app: AppHandle,
    window: WebviewWindow,
    request_id: u64,
    approved: bool,
    sessions: Option<Vec<ClosingSession>>,
) -> Result<(), String> {
    {
        let state = state().lock().unwrap();
        if !state
            .exit
            .as_ref()
            .is_some_and(|exit| exit.id == request_id && exit.pending.contains(window.label()))
        {
            return Ok(());
        }
    }
    if !approved {
        cancel_request(&app, Some(request_id), "exitCancelled");
        return Ok(());
    }
    let value = match snapshot(&app, window.label()) {
        Ok(value) => value,
        Err(error) => {
            cancel_request(&app, Some(request_id), "exitSaveFailed");
            return Err(error);
        }
    };
    {
        let mut state = state().lock().unwrap();
        let Some(exit) = state.exit.as_mut().filter(|exit| exit.id == request_id) else {
            return Ok(());
        };
        if !exit.pending.remove(window.label()) {
            return Ok(());
        }
        exit.sessions.extend(sessions.unwrap_or_default());
        if exit.target.as_deref() == Some(window.label()) {
            exit.snapshot = Some(value);
        }
    }
    complete_if_ready(&app)
}

fn complete_if_ready<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let mut state = state().lock().unwrap();
        let Some(exit) = state.exit.as_mut() else {
            return Ok(());
        };
        if !exit.pending.is_empty() || exit.waiting_settings {
            return Ok(());
        }
        exit.waiting_settings = true;
    }
    match super::commands::request_app_exit_after_settings(app) {
        Ok(true) => Ok(()),
        Ok(false) => finish(app),
        Err(error) => {
            cancel(app, "exitSaveFailed");
            Err(error)
        }
    }
}
pub(crate) fn finish<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let mut state = state().lock().unwrap();
    let Some(exit) = state.exit.as_mut() else {
        return Ok(());
    };
    if !exit.pending.is_empty() || !exit.waiting_settings {
        return Ok(());
    }
    let alive: HashSet<_> = app
        .webview_windows()
        .into_keys()
        .filter(|label| super::external_open::is_document_window_label(label))
        .collect();
    if alive != exit.participants {
        drop(state);
        cancel(app, "exitWindowLost");
        return Err("退出期间窗口列表发生变化".into());
    }
    if let Some(value) = &exit.snapshot {
        if let Err(error) = save_startup(app, value.clone()) {
            drop(state);
            cancel(app, "exitSaveFailed");
            return Err(error);
        }
    }
    // 保持退出状态直到进程结束，避免窗口销毁事件覆盖选定快照。
    exit.committing = true;
    drop(state);
    app.exit(0);
    Ok(())
}
pub(crate) fn cancel<R: Runtime>(app: &AppHandle<R>, reason: &str) {
    cancel_request(app, None, reason);
}
fn cancel_request<R: Runtime>(app: &AppHandle<R>, id: Option<u64>, reason: &str) {
    let cancelled = {
        let mut state = state().lock().unwrap();
        if state
            .exit
            .as_ref()
            .is_some_and(|exit| !exit.committing && (id.is_none() || id == Some(exit.id)))
        {
            state.exit.take().map(|exit| exit.id)
        } else {
            None
        }
    };
    if let Some(id) = cancelled {
        super::commands::take_deferred_settings_action();
        let _ = app.emit(
            "nomo://exit-cancelled",
            serde_json::json!({ "requestId": id, "reason": reason }),
        );
    }
}

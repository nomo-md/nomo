//! Markdown 项的跨窗口移交。正文先在目标暂存，磁盘元数据和文件归属提交成功后才移除源项。
use crate::models::{SettingInput, SettingRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow, WebviewWindowBuilder};

const JOURNAL_PREFIX: &str = "tabTransfer:";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STATE: OnceLock<Mutex<TransferState>> = OnceLock::new();

#[derive(Default)]
struct TransferState {
    stamps: HashMap<String, WindowStamp>,
    transfers: HashMap<String, TransferSnapshot>,
}

#[derive(Clone, Default)]
struct WindowStamp {
    epoch: u64,
    workspace_revision: u64,
    targets_revision: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWriteStamp {
    ownership_epoch: u64,
    revision: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum DropPlacement {
    Insert {
        #[serde(rename = "insertionIndex")]
        insertion_index: usize,
    },
    Combine {
        #[serde(rename = "targetItemId")]
        target_item_id: String,
    },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferOpenTargets {
    pub(crate) folder_path: Option<String>,
    pub(crate) file_paths: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct ScreenPoint {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferInput {
    item: Value,
    tabs: Vec<Value>,
    #[serde(default)]
    positions: Value,
    source_workspace: Value,
    source_open_targets: TransferOpenTargets,
    target_window_label: Option<String>,
    placement: DropPlacement,
    screen_position: Option<ScreenPoint>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferSnapshot {
    pub(crate) token: String,
    pub(crate) status: String,
    pub(crate) source_window_label: String,
    pub(crate) target_window_label: String,
    source_epoch: u64,
    target_epoch: u64,
    new_window: bool,
    item: Value,
    tabs: Vec<Value>,
    positions: Value,
    source_workspace: Value,
    #[serde(default)]
    source_before_workspace: Value,
    source_open_targets: TransferOpenTargets,
    placement: DropPlacement,
    screen_position: Option<ScreenPoint>,
    target_workspace: Option<Value>,
    #[serde(default)]
    target_before_workspace: Option<Value>,
    target_open_targets: Option<TransferOpenTargets>,
    created_at: i64,
}

fn state() -> &'static Mutex<TransferState> {
    STATE.get_or_init(Default::default)
}

fn document_window(label: &str) -> Result<(), String> {
    if !super::external_open::is_document_window_label(label)
        || super::state::is_markdown_mini_mode_window(label)
    {
        return Err("只有普通文档窗口支持标签移交".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_window_workspace_stamp(
    window: WebviewWindow,
) -> Result<WorkspaceWriteStamp, String> {
    if !super::external_open::is_document_window_label(window.label()) {
        return Err("只有文档窗口可以读取工作区序号".into());
    }
    let mut state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
    let stamp = state.stamps.entry(window.label().into()).or_default();
    Ok(WorkspaceWriteStamp {
        ownership_epoch: stamp.epoch,
        revision: stamp.workspace_revision.max(stamp.targets_revision),
    })
}

/// 通用设置入口必须在同一锁内筛选并写入，避免检查 epoch 后发生移交提交。
pub(crate) fn with_guarded_settings(
    inputs: Vec<SettingInput>,
    write: impl FnOnce(Vec<SettingInput>) -> Result<(), String>,
) -> Result<(), String> {
    let mut state = state().lock().map_err(|_| "锁定工作区归属失败")?;
    let mut accepted = Vec::new();
    let mut next_stamps = state.stamps.clone();
    for input in inputs {
        let Some(workspace_key) = input.key.strip_prefix("workspaceTabs:") else {
            accepted.push(input);
            continue;
        };
        let window_key = super::external_open::is_document_window_label(workspace_key);
        if !window_key && !workspace_key.starts_with("folder:") {
            accepted.push(input);
            continue;
        }
        let value: Value = match serde_json::from_str(&input.value_json) {
            Ok(value) => value,
            Err(error) if window_key => return Err(format!("工作区快照无效：{error}")),
            Err(_) => Value::Null,
        };
        let label = if window_key {
            workspace_key
        } else if let Some(writer) = value.get("writerWindowLabel").and_then(Value::as_str) {
            if !super::external_open::is_document_window_label(writer) {
                return Err("文件夹工作区写入窗口无效".into());
            }
            writer
        } else {
            // 旧迁移快照没有 writer，继续沿用原来的文件夹恢复行为。
            accepted.push(input);
            continue;
        };
        // 同一次批量写入的窗口项和文件夹项共享一个 stamp，均以提交前版本核对。
        let current = state.stamps.get(label).cloned().unwrap_or_default();
        let epoch = value
            .get("ownershipEpoch")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let revision = value.get("revision").and_then(Value::as_u64).unwrap_or(0);
        if epoch != current.epoch
            || (current.workspace_revision > 0 && revision <= current.workspace_revision)
        {
            continue;
        }
        let next = next_stamps.entry(label.into()).or_default();
        next.workspace_revision = next.workspace_revision.max(revision);
        accepted.push(input);
    }
    if accepted.is_empty() {
        return Ok(());
    }
    write(accepted)?;
    state.stamps = next_stamps;
    Ok(())
}

pub(crate) fn with_open_targets_guard(
    label: &str,
    epoch: u64,
    revision: u64,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let mut state = state().lock().map_err(|_| "锁定文件归属失败")?;
    let current = state.stamps.entry(label.into()).or_default();
    if epoch != current.epoch
        || (current.targets_revision > 0 && revision <= current.targets_revision)
    {
        return Ok(());
    }
    write()?;
    current.targets_revision = revision;
    Ok(())
}

fn item_members(item: &Value) -> Result<Vec<&str>, String> {
    match item.get("kind").and_then(Value::as_str) {
        Some("single") => Ok(vec![item
            .get("tabId")
            .and_then(Value::as_str)
            .ok_or("普通项缺少文档 ID")?]),
        Some("comparison") => {
            let left = item
                .get("leftTabId")
                .and_then(Value::as_str)
                .ok_or("组合缺少左文档")?;
            let right = item
                .get("rightTabId")
                .and_then(Value::as_str)
                .ok_or("组合缺少右文档")?;
            if left == right {
                return Err("组合不能包含重复文档".into());
            }
            Ok(vec![left, right])
        }
        _ => Err("无法识别工作区项".into()),
    }
}

fn workspace_tabs(workspace: &Value) -> Result<&Vec<Value>, String> {
    if workspace.get("version").and_then(Value::as_u64) != Some(4) {
        return Err("移交需要 v4 工作区".into());
    }
    workspace
        .get("tabs")
        .and_then(Value::as_array)
        .ok_or("工作区缺少文档列表".into())
}

fn validate_workspace(workspace: &Value) -> Result<(), String> {
    let tabs = workspace_tabs(workspace)?;
    let ids: HashSet<_> = tabs
        .iter()
        .filter_map(|tab| tab.get("id").and_then(Value::as_str))
        .collect();
    if ids.len() != tabs.len() {
        return Err("工作区文档 ID 无效或重复".into());
    }
    let items = workspace
        .get("items")
        .and_then(Value::as_array)
        .ok_or("工作区缺少外层项")?;
    let mut assigned = HashSet::new();
    let mut item_ids = HashSet::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or("工作区项缺少 ID")?;
        if !item_ids.insert(id) {
            return Err("工作区项 ID 重复".into());
        }
        for member in item_members(item)? {
            if !ids.contains(member) || !assigned.insert(member) {
                return Err("文档归属无效或重复".into());
            }
            if item.get("kind").and_then(Value::as_str) == Some("comparison") {
                let tab = tabs
                    .iter()
                    .find(|tab| tab.get("id").and_then(Value::as_str) == Some(member))
                    .unwrap();
                if tab.get("documentKind").and_then(Value::as_str) != Some("markdown") {
                    return Err("组合只允许 Markdown".into());
                }
            }
        }
    }
    if assigned != ids {
        return Err("工作区存在未分配文档".into());
    }
    Ok(())
}

fn validate_open_targets(workspace: &Value, targets: &TransferOpenTargets) -> Result<(), String> {
    let normalize = super::open_targets::normalize_target_path;
    let actual: HashSet<_> = workspace_tabs(workspace)?
        .iter()
        .filter_map(|tab| {
            tab.get("nativePath")
                .and_then(Value::as_str)
                .and_then(normalize)
        })
        .collect();
    let registered: HashSet<_> = targets
        .file_paths
        .iter()
        .filter_map(|path| normalize(path))
        .collect();
    if actual != registered {
        return Err("工作区与文件归属快照不一致".into());
    }
    let folder = workspace
        .get("currentFolderPath")
        .and_then(Value::as_str)
        .and_then(normalize);
    if folder != targets.folder_path.as_deref().and_then(normalize) {
        return Err("工作区与文件夹归属快照不一致".into());
    }
    Ok(())
}

fn validate_incoming_paths(incoming: &[Value], existing: &[Value]) -> Result<(), String> {
    let normalize = super::open_targets::normalize_target_path;
    let existing_paths: HashSet<_> = existing
        .iter()
        .filter_map(|tab| {
            tab.get("nativePath")
                .and_then(Value::as_str)
                .and_then(normalize)
        })
        .collect();
    let mut incoming_paths = HashSet::new();
    for tab in incoming {
        let Some(path) = tab
            .get("nativePath")
            .and_then(Value::as_str)
            .and_then(normalize)
        else {
            continue;
        };
        if !incoming_paths.insert(path.clone()) {
            return Err("迁移成员指向同一个文件，不能重复接收".into());
        }
        if existing_paths.contains(&path) {
            return Err("目标窗口已经打开迁移文件，不能创建重复编辑基线".into());
        }
    }
    Ok(())
}

fn put_setting(config: &mut crate::config::AppConfig, key: String, value: Value) {
    if let Some(label) = key.strip_prefix("workspaceTabs:") {
        config
            .workspace
            .settings
            .insert(label.into(), value.clone());
    }
    config.app.settings.insert(
        key.clone(),
        SettingRecord {
            key,
            value_json: value.to_string(),
            updated_at: crate::config::now_ts(),
        },
    );
}

fn put_workspace_snapshot(config: &mut crate::config::AppConfig, label: &str, workspace: Value) {
    if let Some(folder) = workspace
        .get("currentFolderPath")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    {
        put_setting(
            config,
            format!("workspaceTabs:folder:{folder}"),
            workspace.clone(),
        );
    }
    put_setting(config, format!("workspaceTabs:{label}"), workspace);
}

fn persist<R: Runtime>(app: &AppHandle<R>, transfer: &TransferSnapshot) -> Result<(), String> {
    let journal = serde_json::to_value(transfer).map_err(|error| error.to_string())?;
    crate::config::with_manager(app, |manager| {
        manager.update(|config| {
            put_setting(
                config,
                format!("{JOURNAL_PREFIX}{}", transfer.token),
                journal,
            );
        })
    })
}

fn emit_participants<R: Runtime>(app: &AppHandle<R>, event: &str, transfer: &TransferSnapshot) {
    for label in [&transfer.source_window_label, &transfer.target_window_label] {
        if let Err(error) = app.emit_to(label.clone(), event, transfer.clone()) {
            crate::app_logger::warn("TabTransfer", &format!("移交事件发送失败：{error}"));
        }
    }
}

#[tauri::command]
pub(crate) async fn prepare_tab_transfer(
    app: AppHandle,
    window: WebviewWindow,
    input: TransferInput,
) -> Result<TransferSnapshot, String> {
    document_window(window.label())?;
    if super::workspace_lifecycle::is_exiting() {
        return Err("退出期间不能移交标签".into());
    }
    let members = item_members(&input.item)?;
    let incoming: HashSet<_> = input
        .tabs
        .iter()
        .filter_map(|tab| tab.get("id").and_then(Value::as_str))
        .collect();
    if input.tabs.len() != members.len() || members.iter().any(|member| !incoming.contains(member))
    {
        return Err("移交快照成员不完整".into());
    }
    for tab in &input.tabs {
        if tab.get("documentKind").and_then(Value::as_str) != Some("markdown")
            || !tab.get("markdown").is_some_and(Value::is_string)
            || !tab.get("savedMarkdown").is_some_and(Value::is_string)
        {
            return Err("移交仅支持完整 Markdown 快照".into());
        }
    }
    validate_incoming_paths(&input.tabs, &[])?;
    validate_workspace(&input.source_workspace)?;
    validate_open_targets(&input.source_workspace, &input.source_open_targets)?;
    if workspace_tabs(&input.source_workspace)?
        .iter()
        .any(|tab| incoming.contains(tab.get("id").and_then(Value::as_str).unwrap_or("")))
    {
        return Err("源工作区仍然包含迁移文档".into());
    }
    if matches!(&input.placement, DropPlacement::Combine { .. }) && members.len() != 1 {
        return Err("组合不可再次嵌套".into());
    }
    let new_window = input.target_window_label.is_none();
    let source_before_workspace: Value = serde_json::from_str(
        &crate::config::commands::get_setting_value(
            &app,
            &format!("workspaceTabs:{}", window.label()),
        )?
        .ok_or("源工作区尚未保存")?,
    )
    .map_err(|error| format!("源工作区无效：{error}"))?;
    validate_workspace(&source_before_workspace)?;
    let before_ids: HashSet<_> = workspace_tabs(&source_before_workspace)?
        .iter()
        .filter_map(|tab| tab.get("id").and_then(Value::as_str))
        .collect();
    let after_ids: HashSet<_> = workspace_tabs(&input.source_workspace)?
        .iter()
        .filter_map(|tab| tab.get("id").and_then(Value::as_str))
        .collect();
    if !incoming.is_subset(&before_ids)
        || before_ids
            .difference(&incoming)
            .copied()
            .collect::<HashSet<_>>()
            != after_ids
    {
        return Err("源工作区变更超出移交文档".into());
    }
    let target = match &input.target_window_label {
        Some(label) => {
            document_window(label)?;
            if label == window.label() || app.get_webview_window(label).is_none() {
                return Err("目标窗口无效".into());
            }
            if !super::tab_drag::window_ready(label) {
                return Err("目标窗口尚未就绪".into());
            }
            label.clone()
        }
        None => app
            .state::<super::open_targets::OpenTargetRegistry>()
            .next_window_label(&app)?,
    };
    let target_before_workspace = if new_window {
        None
    } else {
        let value =
            crate::config::commands::get_setting_value(&app, &format!("workspaceTabs:{target}"))?;
        if let Some(value) = value {
            let workspace: Value =
                serde_json::from_str(&value).map_err(|error| format!("目标工作区无效：{error}"))?;
            validate_workspace(&workspace)?;
            if workspace_tabs(&workspace)?
                .iter()
                .any(|tab| incoming.contains(tab.get("id").and_then(Value::as_str).unwrap_or("")))
            {
                return Err("目标工作区已有同 ID 文档".into());
            }
            Some(workspace)
        } else {
            None
        }
    };
    let token = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("读取移交时间失败：{error}"))?
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1
    );
    let snapshot = {
        let mut state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        if state.transfers.values().any(|transfer| {
            (transfer.status == "prepared" || transfer.status == "target-ready")
                && [
                    transfer.source_window_label.as_str(),
                    transfer.target_window_label.as_str(),
                ]
                .iter()
                .any(|label| *label == window.label() || *label == target)
        }) {
            return Err("窗口已有进行中的标签移交".into());
        }
        let source_epoch = state.stamps.entry(window.label().into()).or_default().epoch;
        let target_epoch = state.stamps.entry(target.clone()).or_default().epoch;
        if input
            .source_workspace
            .get("ownershipEpoch")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != source_epoch
        {
            return Err("源工作区快照已过期".into());
        }
        let snapshot = TransferSnapshot {
            token: token.clone(),
            status: "prepared".into(),
            source_window_label: window.label().into(),
            target_window_label: target.clone(),
            source_epoch,
            target_epoch,
            new_window,
            item: input.item,
            tabs: input.tabs,
            positions: input.positions,
            source_workspace: input.source_workspace,
            source_open_targets: input.source_open_targets,
            source_before_workspace,
            placement: input.placement,
            screen_position: input.screen_position,
            target_workspace: None,
            target_before_workspace,
            target_open_targets: None,
            created_at: crate::config::now_ts(),
        };
        persist(&app, &snapshot)?;
        state.transfers.insert(token.clone(), snapshot.clone());
        snapshot
    };
    if new_window {
        if let Err(error) = create_transfer_window(&app, &snapshot) {
            let _ = cancel_internal(&app, &token);
            return Err(error);
        }
    } else if let Err(error) =
        app.emit_to(&target, "nomo://tab-transfer-prepared", snapshot.clone())
    {
        let _ = cancel_internal(&app, &token);
        return Err(format!("发送标签快照失败：{error}"));
    }
    let timeout_app = app.clone();
    let timeout_token = token;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if let Err(error) = cancel_internal(&timeout_app, &timeout_token) {
            crate::app_logger::warn("TabTransfer", &format!("取消超时移交失败：{error}"));
        }
    });
    Ok(snapshot)
}

fn create_transfer_window(app: &AppHandle, snapshot: &TransferSnapshot) -> Result<(), String> {
    let mut builder = WebviewWindowBuilder::new(
        app,
        &snapshot.target_window_label,
        tauri::WebviewUrl::App("/".into()),
    )
    .title("Nomo")
    .inner_size(1180.0, 760.0)
    .min_inner_size(920.0, 640.0)
    .decorations(false)
    .visible(false)
    .shadow(true);
    if let Some(point) = &snapshot.screen_position {
        // 移动到目标显示器后再应用物理坐标，避免把 DOM 逻辑坐标误当屏幕坐标。
        builder = builder.position(point.x as f64, point.y as f64);
    }
    let target = builder
        .build()
        .map_err(|error| format!("创建标签窗口失败：{error}"))?;
    if let Some(point) = &snapshot.screen_position {
        target
            .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                point.x - 80,
                point.y - 24,
            )))
            .map_err(|error| format!("设置标签窗口位置失败：{error}"))?;
    }
    Ok(())
}

fn transfer_for_caller(
    state: &TransferState,
    token: &str,
    caller: &str,
) -> Result<TransferSnapshot, String> {
    let transfer = state
        .transfers
        .get(token)
        .cloned()
        .ok_or("标签移交记录不存在")?;
    if caller != transfer.source_window_label && caller != transfer.target_window_label {
        return Err("当前窗口无权访问该移交".into());
    }
    Ok(transfer)
}

#[tauri::command]
pub(crate) fn get_tab_transfer_bootstrap(
    window: WebviewWindow,
) -> Result<Option<TransferSnapshot>, String> {
    document_window(window.label())?;
    let state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
    Ok(state
        .transfers
        .values()
        .find(|transfer| {
            transfer.target_window_label == window.label()
                && transfer.new_window
                && (transfer.status == "prepared" || transfer.status == "target-ready")
        })
        .cloned())
}

#[tauri::command]
pub(crate) fn target_ready_tab_transfer(
    app: AppHandle,
    window: WebviewWindow,
    token: String,
    target_workspace: Value,
    target_open_targets: TransferOpenTargets,
) -> Result<TransferSnapshot, String> {
    validate_workspace(&target_workspace)?;
    validate_open_targets(&target_workspace, &target_open_targets)?;
    let snapshot = {
        let mut state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        let mut transfer = transfer_for_caller(&state, &token, window.label())?;
        if transfer.target_window_label != window.label() {
            return Err("只有目标窗口可以确认接收".into());
        }
        if transfer.status != "prepared" {
            return Ok(transfer);
        }
        if !transfer.new_window {
            let before = crate::config::commands::get_setting_value(
                &app,
                &format!("workspaceTabs:{}", transfer.target_window_label),
            )?
            .ok_or("目标原工作区尚未保存")?;
            let before: Value = serde_json::from_str(&before)
                .map_err(|error| format!("目标原工作区无效：{error}"))?;
            validate_workspace(&before)?;
            transfer.target_before_workspace = Some(before);
        }
        let tabs = workspace_tabs(&target_workspace)?;
        let mut expected_ids: HashSet<&str> = transfer
            .tabs
            .iter()
            .filter_map(|tab| tab.get("id").and_then(Value::as_str))
            .collect();
        if let Some(before) = &transfer.target_before_workspace {
            for tab in workspace_tabs(before)? {
                if let Some(id) = tab.get("id").and_then(Value::as_str) {
                    expected_ids.insert(id);
                }
            }
            if before.get("currentFolderPath") != target_workspace.get("currentFolderPath") {
                return Err("移交不能改变已有目标的文件夹上下文".into());
            }
        }
        let actual_ids: HashSet<_> = tabs
            .iter()
            .filter_map(|tab| tab.get("id").and_then(Value::as_str))
            .collect();
        if expected_ids != actual_ids {
            return Err("目标工作区包含额外或缺失的文档".into());
        }
        let mut migrated_metadata = Vec::new();
        for tab in &transfer.tabs {
            let incoming = tab
                .get("id")
                .and_then(Value::as_str)
                .ok_or("移交文档 ID 无效")?;
            let target_tab = tabs
                .iter()
                .find(|tab| {
                    tab.get("id").and_then(Value::as_str) == Some(incoming)
                        && tab.get("documentKind").and_then(Value::as_str) == Some("markdown")
                })
                .ok_or("目标工作区未完整接收移交文档")?;
            let normalize = super::open_targets::normalize_target_path;
            if tab
                .get("nativePath")
                .and_then(Value::as_str)
                .and_then(normalize)
                != target_tab
                    .get("nativePath")
                    .and_then(Value::as_str)
                    .and_then(normalize)
            {
                return Err("目标工作区改变了迁移文档的文件路径".into());
            }
            migrated_metadata.push(target_tab.clone());
        }
        let existing_metadata: Vec<_> = tabs
            .iter()
            .filter(|tab| {
                !transfer
                    .tabs
                    .iter()
                    .any(|incoming| incoming.get("id") == tab.get("id"))
            })
            .cloned()
            .collect();
        validate_incoming_paths(&migrated_metadata, &existing_metadata)?;
        if target_workspace
            .get("ownershipEpoch")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != transfer.target_epoch
        {
            return Err("目标工作区快照已过期".into());
        }
        let target_items = target_workspace
            .get("items")
            .and_then(Value::as_array)
            .ok_or("目标工作区项缺失")?;
        let members = item_members(&transfer.item)?;
        match &transfer.placement {
            DropPlacement::Combine { target_item_id } => {
                let before = transfer
                    .target_before_workspace
                    .as_ref()
                    .ok_or("新窗口不能与既有标签组合")?;
                let before_target = before
                    .get("items")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items.iter().find(|item| {
                            item.get("id").and_then(Value::as_str) == Some(target_item_id)
                        })
                    })
                    .ok_or("目标标签已不存在")?;
                if before_target.get("kind").and_then(Value::as_str) != Some("single") {
                    return Err("不能嵌套组合".into());
                }
                let before_members = item_members(before_target)?;
                if !target_items.iter().any(|item| {
                    item.get("kind").and_then(Value::as_str) == Some("comparison")
                        && item.get("leftTabId").and_then(Value::as_str) == Some(before_members[0])
                        && item.get("rightTabId").and_then(Value::as_str) == Some(members[0])
                }) {
                    return Err("目标未按左右顺序组成双文档标签".into());
                }
            }
            DropPlacement::Insert { insertion_index } => {
                let index = (*insertion_index).min(target_items.len().saturating_sub(1));
                let item = target_items.get(index).ok_or("目标缺少迁入工作区项")?;
                if item_members(item)? != members {
                    return Err("目标插入位置或组合成员错误".into());
                }
            }
        }
        transfer.target_workspace = Some(target_workspace);
        transfer.target_open_targets = Some(target_open_targets);
        transfer.status = "target-ready".into();
        persist(&app, &transfer)?;
        state.transfers.insert(token, transfer.clone());
        transfer
    };
    emit_participants(&app, "nomo://tab-transfer-ready", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub(crate) fn commit_tab_transfer(
    app: AppHandle,
    window: WebviewWindow,
    token: String,
) -> Result<TransferSnapshot, String> {
    let snapshot = {
        let mut state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        let mut transfer = transfer_for_caller(&state, &token, window.label())?;
        if transfer.source_window_label != window.label() {
            return Err("只有源窗口可以提交移交".into());
        }
        if transfer.status == "committed" {
            drop(state);
            emit_participants(&app, "nomo://tab-transfer-committed", &transfer);
            return Ok(transfer);
        }
        if transfer.status != "target-ready" {
            return Err("目标尚未准备好接收".into());
        }
        if app
            .get_webview_window(&transfer.source_window_label)
            .is_none()
            || app
                .get_webview_window(&transfer.target_window_label)
                .is_none()
        {
            return Err("移交窗口已关闭".into());
        }
        for (label, epoch) in [
            (&transfer.source_window_label, transfer.source_epoch),
            (&transfer.target_window_label, transfer.target_epoch),
        ] {
            if state
                .stamps
                .get(label)
                .map(|stamp| stamp.epoch)
                .unwrap_or(0)
                != epoch
            {
                return Err("移交工作区归属已变化".into());
            }
        }
        transfer.source_epoch += 1;
        transfer.target_epoch += 1;
        transfer.source_workspace["ownershipEpoch"] = json!(transfer.source_epoch);
        transfer.source_workspace["revision"] = json!(0);
        transfer.source_workspace["writerWindowLabel"] = json!(transfer.source_window_label);
        let mut target_workspace = transfer.target_workspace.clone().ok_or("目标快照缺失")?;
        target_workspace["ownershipEpoch"] = json!(transfer.target_epoch);
        target_workspace["revision"] = json!(0);
        target_workspace["writerWindowLabel"] = json!(transfer.target_window_label);
        transfer.target_workspace = Some(target_workspace.clone());
        transfer.status = "committed".into();
        let target_targets = transfer
            .target_open_targets
            .clone()
            .ok_or("目标文件归属缺失")?;
        let mut source_targets = transfer.source_open_targets.clone();
        if workspace_tabs(&transfer.source_workspace)?.is_empty() {
            // 空源窗即将关闭，不再接收指向原文件夹的外部打开请求；恢复快照仍保留目录。
            source_targets.folder_path = None;
        }
        let journal = serde_json::to_value(&transfer).map_err(|error| error.to_string())?;
        app.state::<super::open_targets::OpenTargetRegistry>()
            .commit_transfer(
                &transfer.source_window_label,
                &transfer.target_window_label,
                &source_targets,
                &target_targets,
                || {
                    crate::config::with_manager(&app, |manager| {
                        manager.update(|config| {
                            put_workspace_snapshot(
                                config,
                                &transfer.source_window_label,
                                transfer.source_workspace.clone(),
                            );
                            put_workspace_snapshot(
                                config,
                                &transfer.target_window_label,
                                target_workspace.clone(),
                            );
                            put_setting(
                                config,
                                format!("{JOURNAL_PREFIX}{}", transfer.token),
                                journal,
                            );
                            // 程序崩溃仍可由一份启动工作区恢复完整迁移文档。
                            put_setting(config, "startupWorkspace".into(), target_workspace);
                        })
                    })
                },
            )?;
        for (label, epoch) in [
            (&transfer.source_window_label, transfer.source_epoch),
            (&transfer.target_window_label, transfer.target_epoch),
        ] {
            state.stamps.insert(
                label.clone(),
                WindowStamp {
                    epoch,
                    ..Default::default()
                },
            );
        }
        state.transfers.insert(token, transfer.clone());
        transfer
    };
    emit_participants(&app, "nomo://tab-transfer-committed", &snapshot);
    if let Some(target) = app.get_webview_window(&snapshot.target_window_label) {
        let _ = target.set_focus();
    }
    Ok(snapshot)
}

fn cancel_internal<R: Runtime>(
    app: &AppHandle<R>,
    token: &str,
) -> Result<TransferSnapshot, String> {
    let (snapshot, persistence_result) = {
        let mut state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        let mut transfer = state
            .transfers
            .get(token)
            .cloned()
            .ok_or("标签移交记录不存在")?;
        if transfer.status == "committed" || transfer.status == "cancelled" {
            return Ok(transfer);
        }
        transfer.status = "cancelled".into();
        let persistence_result = persist(app, &transfer);
        state.transfers.insert(token.into(), transfer.clone());
        (transfer, persistence_result)
    };
    emit_participants(app, "nomo://tab-transfer-cancelled", &snapshot);
    if snapshot.new_window {
        if let Some(target) = app.get_webview_window(&snapshot.target_window_label) {
            // 新窗口的内容尚未提交；直接销毁不触发文档确认或覆盖启动工作区。
            super::workspace_lifecycle::forget_close(&snapshot.target_window_label);
            let _ = target.destroy();
        }
    }
    // 磁盘失败不能把源窗口永久锁在 pending；内存已回滚，旧 journal 留供重启恢复。
    if let Err(error) = persistence_result {
        let _ = app.emit("nomo://workspace-error", error.clone());
        return Err(error);
    }
    Ok(snapshot)
}

#[tauri::command]
pub(crate) fn cancel_tab_transfer(
    app: AppHandle,
    window: WebviewWindow,
    token: String,
) -> Result<TransferSnapshot, String> {
    {
        let state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        transfer_for_caller(&state, &token, window.label())?;
    }
    cancel_internal(&app, &token)
}

#[tauri::command]
pub(crate) fn query_tab_transfer(
    window: WebviewWindow,
    token: String,
) -> Result<TransferSnapshot, String> {
    let state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
    transfer_for_caller(&state, &token, window.label())
}

#[tauri::command]
pub(crate) async fn close_empty_transferred_window(
    window: WebviewWindow,
    token: String,
) -> Result<(), String> {
    {
        let state = state().lock().map_err(|_| "锁定标签移交状态失败")?;
        let transfer = transfer_for_caller(&state, &token, window.label())?;
        if transfer.status != "committed"
            || transfer.source_window_label != window.label()
            || !workspace_tabs(&transfer.source_workspace)?.is_empty()
        {
            return Err("仅允许关闭已移空的源窗口".into());
        }
    }
    let label = window.label().to_string();
    let app = window.app_handle().clone();
    super::workspace_lifecycle::forget_close(&label);
    super::commands::close_transferred_empty_window(&window)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if app.get_webview_window(&label).is_none() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("等待移空窗口关闭超时，请检查偏好设置保存状态".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// 重启时 pending 仍归源，committed 的目标快照已落盘；只恢复一份启动工作区。
pub(crate) fn recover_journals<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    crate::config::with_manager(app, |manager| {
        let config = manager.get_config()?;
        let mut pending: Vec<_> = config
            .app
            .settings
            .iter()
            .filter(|(key, _)| key.starts_with(JOURNAL_PREFIX))
            .filter_map(|(_, value)| {
                serde_json::from_str::<TransferSnapshot>(&value.value_json).ok()
            })
            .filter(|transfer| transfer.status == "prepared" || transfer.status == "target-ready")
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        pending.sort_by(|left, right| {
            (left.created_at, &left.token).cmp(&(right.created_at, &right.token))
        });
        manager.update(|config| {
            for mut transfer in pending {
                // 源快照尚未移除迁移文档；将正文保留在恢复记录和草稿中。
                transfer.status = "cancelled".into();
                let latest_source = config
                    .app
                    .settings
                    .get(&format!("workspaceTabs:{}", transfer.source_window_label))
                    .and_then(|record| serde_json::from_str::<Value>(&record.value_json).ok());
                let recovery = match latest_source {
                    Some(workspace)
                        if workspace
                            .get("ownershipEpoch")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            != transfer.source_epoch =>
                    {
                        None
                    }
                    Some(workspace) if validate_workspace(&workspace).is_ok() => Some(workspace),
                    _ => validate_workspace(&transfer.source_before_workspace)
                        .is_ok()
                        .then(|| transfer.source_before_workspace.clone()),
                };
                if let Some(mut recovered) = recovery {
                    recovered["ownershipEpoch"] = json!(0);
                    recovered["revision"] = json!(0);
                    put_setting(config, "startupWorkspace".into(), recovered);
                }
                if let Ok(value) = serde_json::to_value(&transfer) {
                    put_setting(config, format!("{JOURNAL_PREFIX}{}", transfer.token), value);
                }
            }
        })
    })
}

pub(crate) fn forget_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let pending: Vec<_> = state()
        .lock()
        .map(|state| {
            state
                .transfers
                .values()
                .filter(|transfer| {
                    (transfer.status == "prepared" || transfer.status == "target-ready")
                        && (transfer.source_window_label == label
                            || transfer.target_window_label == label)
                })
                .map(|transfer| transfer.token.clone())
                .collect()
        })
        .unwrap_or_default();
    for token in pending {
        let _ = cancel_internal(app, &token);
    }
}

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

const GITHUB_LATEST_RELEASE_API: &str = "https://api.github.com/repos/nomo-md/nomo/releases/latest";
const GITHUB_PROXY_PREFIX: &str = "https://gh-proxy.com/";
const UPDATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UPDATE_SMALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const UPDATE_DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(20);
const CHECKSUMS_ASSET_NAME: &str = "checksums.md5";
const DOWNLOAD_PROGRESS_EVENT: &str = "nomo://software-update-download-progress";
const UPDATE_STATE_EVENT: &str = "nomo://software-update-state";
const CACHED_UPDATE_INFO_FILE: &str = "update-info.json";
const INSTALLED_RELEASE_NOTES_AVAILABLE_EVENT: &str = "nomo://installed-release-notes-available";
const INSTALLED_RELEASE_NOTES_HISTORY_KEY: &str = "installedReleaseNotesHistory";
const CURRENT_RELEASE_NOTES: &str = include_str!(concat!(
    "../../.github/release-notes/v",
    env!("CARGO_PKG_VERSION"),
    ".md"
));

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SoftwareUpdateInstallationKind {
    Installer,
    Portable,
    Store,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SoftwareUpdateAssetKind {
    WindowsInstaller,
    WindowsPortable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SoftwareUpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
    Installing,
    Managed,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SoftwareUpdateCandidate {
    pub(crate) version: String,
    pub(crate) date: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) asset_kind: SoftwareUpdateAssetKind,
    pub(crate) asset_name: String,
    pub(crate) asset_size: Option<u64>,
    pub(crate) download_url: String,
    pub(crate) md5: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SoftwareUpdateCheckPayload {
    pub(crate) supported: bool,
    pub(crate) available: bool,
    pub(crate) current_version: String,
    pub(crate) installation_kind: SoftwareUpdateInstallationKind,
    pub(crate) version: Option<String>,
    pub(crate) date: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) candidate: Option<SoftwareUpdateCandidate>,
    pub(crate) store_product_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadedSoftwareUpdate {
    pub(crate) version: String,
    pub(crate) asset_name: String,
    pub(crate) file_path: String,
    pub(crate) md5: String,
    pub(crate) downloaded_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SoftwareUpdateDownloadProgress {
    request_id: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SoftwareUpdateSnapshot {
    status: SoftwareUpdateStatus,
    current_version: String,
    installation_kind: SoftwareUpdateInstallationKind,
    version: Option<String>,
    date: Option<String>,
    body: Option<String>,
    candidate: Option<SoftwareUpdateCandidate>,
    downloaded_update: Option<DownloadedSoftwareUpdate>,
    progress: Option<SoftwareUpdateDownloadProgress>,
    error: Option<String>,
    notice_window_label: Option<String>,
    store_product_id: Option<String>,
}

impl Default for SoftwareUpdateSnapshot {
    fn default() -> Self {
        Self {
            status: SoftwareUpdateStatus::Idle,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            installation_kind: SoftwareUpdateInstallationKind::Unsupported,
            version: None,
            date: None,
            body: None,
            candidate: None,
            downloaded_update: None,
            progress: None,
            error: None,
            notice_window_label: None,
            store_product_id: None,
        }
    }
}

#[derive(Default)]
struct SoftwareUpdateRuntimeState {
    snapshot: SoftwareUpdateSnapshot,
    check_in_progress: bool,
}

static SOFTWARE_UPDATE_STATE: OnceLock<Mutex<SoftwareUpdateRuntimeState>> = OnceLock::new();

/// 当前运行版本的离线更新日志，不包含下载或安装状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledReleaseNotes {
    version: String,
    body: String,
}

/// 放在既有 settings 容器中，旧版本重写配置时也能保留升级记录。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
struct ReleaseNotesHistory {
    highest_version: Option<String>,
    pending_version: Option<String>,
}

fn read_release_notes_history(
    config: &crate::config::AppConfig,
) -> Result<ReleaseNotesHistory, String> {
    match config.app.settings.get(INSTALLED_RELEASE_NOTES_HISTORY_KEY) {
        Some(record) => serde_json::from_str(&record.value_json)
            .map_err(|error| format!("读取更新日志版本记录失败：{error}")),
        None => Ok(ReleaseNotesHistory::default()),
    }
}

fn save_release_notes_history(
    manager: &crate::config::ConfigManager,
    history: &ReleaseNotesHistory,
) -> Result<(), String> {
    let record = crate::models::SettingRecord {
        key: INSTALLED_RELEASE_NOTES_HISTORY_KEY.to_string(),
        value_json: serde_json::to_string(history)
            .map_err(|error| format!("序列化更新日志版本记录失败：{error}"))?,
        updated_at: crate::config::now_ts(),
    };
    manager.update(|config| {
        config.app.settings.insert(record.key.clone(), record);
    })
}

#[derive(Default)]
struct InstalledReleaseNotesRuntime {
    initialized: bool,
    owner_window_label: Option<String>,
}

static INSTALLED_RELEASE_NOTES_STATE: OnceLock<Mutex<InstalledReleaseNotesRuntime>> =
    OnceLock::new();

fn installed_release_notes_state() -> &'static Mutex<InstalledReleaseNotesRuntime> {
    INSTALLED_RELEASE_NOTES_STATE
        .get_or_init(|| Mutex::new(InstalledReleaseNotesRuntime::default()))
}

/// 必须在本次启动写入窗口、工作区等记录前调用，避免把全新安装识别为已有用户。
pub(crate) fn initialize_installed_release_notes<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let mut runtime = installed_release_notes_state()
        .lock()
        .map_err(|error| format!("初始化更新日志状态失败：{error}"))?;
    crate::config::with_manager(app, |manager| {
        let config = manager.get_config()?;
        let previous_history = read_release_notes_history(&config)?;
        let mut history = previous_history.clone();
        let current_version = env!("CARGO_PKG_VERSION");
        let should_prompt = match history.highest_version.as_deref() {
            Some(previous) => is_release_newer(previous, current_version)?,
            None => has_existing_usage(&config),
        };
        if history.highest_version.is_none() || should_prompt {
            history.highest_version = Some(current_version.to_string());
            history.pending_version = should_prompt.then(|| current_version.to_string());
        }
        if history != previous_history {
            save_release_notes_history(manager, &history)?;
        }
        Ok(())
    })?;
    runtime.initialized = true;
    Ok(())
}

fn has_existing_usage(config: &crate::config::AppConfig) -> bool {
    !config.recent.entries.is_empty()
        || !config.workspace.settings.is_empty()
        || !config.snapshots.documents.is_empty()
        || !config.editor.settings.is_empty()
        || config.app.settings.keys().any(|key| {
            // 待打开目标和窗口几何不代表用户已使用过应用。
            key != INSTALLED_RELEASE_NOTES_HISTORY_KEY
                && !key.starts_with("pending")
                && !key.starts_with("windowState:")
        })
}

/// 原子领取待展示日志；release=true 用于前端卸载时释放尚未确认的领取权。
#[tauri::command]
pub(crate) fn claim_installed_release_notes<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    release: Option<bool>,
) -> Result<Option<InstalledReleaseNotes>, String> {
    if !crate::window::external_open::is_document_window_label(window.label()) {
        return Ok(None);
    }
    if release.unwrap_or(false) {
        release_installed_release_notes(&app, window.label());
        return Ok(None);
    }
    let mut runtime = installed_release_notes_state()
        .lock()
        .map_err(|error| format!("领取更新日志失败：{error}"))?;
    if !runtime.initialized {
        return Err("更新日志版本记录初始化失败，本次启动不展示。".to_string());
    }
    if runtime.owner_window_label.is_some() {
        return Ok(None);
    }
    let pending_version = crate::config::with_manager(&app, |manager| {
        Ok(read_release_notes_history(&manager.get_config()?)?.pending_version)
    })?;
    let current_version = env!("CARGO_PKG_VERSION");
    // 降级时保留较高版本记录，不能展示与当前运行版本不符的日志。
    if pending_version.as_deref() != Some(current_version) {
        return Ok(None);
    }
    runtime.owner_window_label = Some(window.label().to_string());
    Ok(Some(InstalledReleaseNotes {
        version: current_version.to_string(),
        body: CURRENT_RELEASE_NOTES.to_string(),
    }))
}

/// 仅在所属窗口实际挂载弹窗后确认；磁盘提交失败时保留待展示记录。
#[tauri::command]
pub(crate) fn confirm_installed_release_notes_shown<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    version: String,
) -> Result<(), String> {
    let mut runtime = installed_release_notes_state()
        .lock()
        .map_err(|error| format!("确认更新日志展示失败：{error}"))?;
    if version != env!("CARGO_PKG_VERSION")
        || runtime.owner_window_label.as_deref() != Some(window.label())
    {
        return Err("当前窗口未领取此版本更新日志。".to_string());
    }
    crate::config::with_manager(&app, |manager| {
        let mut history = read_release_notes_history(&manager.get_config()?)?;
        if history.pending_version.as_deref() != Some(version.as_str()) {
            return Err("此版本更新日志已不在待展示记录中。".to_string());
        }
        history.pending_version = None;
        save_release_notes_history(manager, &history)
    })?;
    runtime.owner_window_label = None;
    Ok(())
}

pub(crate) fn release_installed_release_notes<R: Runtime>(app: &AppHandle<R>, window_label: &str) {
    let released = match installed_release_notes_state().lock() {
        Ok(mut runtime) if runtime.owner_window_label.as_deref() == Some(window_label) => {
            runtime.owner_window_label = None;
            true
        }
        Ok(_) => false,
        Err(error) => {
            crate::app_logger::warn("Update", &format!("释放更新日志领取权失败：{error}"));
            false
        }
    };
    if released {
        if let Err(error) = app.emit(INSTALLED_RELEASE_NOTES_AVAILABLE_EVENT, ()) {
            crate::app_logger::warn("Update", &format!("通知更新日志可领取失败：{error}"));
        }
    }
}

struct SoftwareUpdateCheckGuard;

impl Drop for SoftwareUpdateCheckGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = shared_state().lock() {
            state.check_in_progress = false;
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: Option<String>,
    created_at: Option<String>,
    body: Option<String>,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct GitHubReleaseAsset {
    name: String,
    size: Option<u64>,
    browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoftwareUpdateSource {
    GitHub,
    GhProxy,
}

impl SoftwareUpdateSource {
    fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub 直连",
            Self::GhProxy => "gh-proxy",
        }
    }
}

#[derive(Debug)]
struct SoftwareUpdateRequestFailure {
    message: String,
    retryable: bool,
}

impl SoftwareUpdateRequestFailure {
    fn from_reqwest(context: &str, error: reqwest::Error) -> Self {
        let retryable = error
            .status()
            .map(is_retryable_update_status)
            .unwrap_or_else(|| !error.is_builder());
        Self {
            message: format!("{context}：{error}"),
            retryable,
        }
    }

    fn terminal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

struct DownloadAttemptOutcome {
    downloaded_bytes: u64,
    actual_md5: String,
}

#[tauri::command]
pub(crate) fn get_cached_software_update<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<DownloadedSoftwareUpdate>, String> {
    if current_installation_kind()? == SoftwareUpdateInstallationKind::Store {
        return Ok(None);
    }
    let update_dir = software_update_cache_dir(&app)?;
    let info = read_cached_update_info(&update_dir)?;

    // 检查缓存信息对应的安装包文件是否仍然存在
    match &info {
        Some(cached) => {
            let path = PathBuf::from(&cached.file_path);
            if !path.is_file() {
                crate::app_logger::info("Update", "缓存信息存在但安装包文件已丢失，忽略");
                return Ok(None);
            }
            crate::app_logger::info("Update", &format!("发现已下载的更新：{}", cached.version));
        }
        None => {}
    }

    Ok(info)
}

#[tauri::command]
pub(crate) async fn is_windows_installer_installation() -> Result<bool, String> {
    is_current_windows_installer_installation()
}

#[tauri::command]
pub(crate) fn get_software_update_state<R: Runtime>(
    app: AppHandle<R>,
) -> Result<SoftwareUpdateSnapshot, String> {
    let installation_kind = current_installation_kind()?;
    let current_version = env!("CARGO_PKG_VERSION");
    let cached_update = if installation_kind == SoftwareUpdateInstallationKind::Installer {
        get_cached_software_update(app.clone())?
            .filter(|cached| is_release_newer(current_version, &cached.version).unwrap_or(false))
    } else {
        None
    };

    update_shared_state(&app, |state| {
        state.installation_kind = installation_kind;
        state.store_product_id = if installation_kind == SoftwareUpdateInstallationKind::Store {
            crate::windows_package::store_product_id()
        } else {
            None
        };
        if installation_kind == SoftwareUpdateInstallationKind::Store {
            state.status = SoftwareUpdateStatus::Managed;
            state.current_version = current_version.to_string();
            state.version = Some(current_version.to_string());
            state.body = Some(CURRENT_RELEASE_NOTES.to_string());
            state.candidate = None;
            state.downloaded_update = None;
            state.progress = None;
            state.error = None;
        } else if let Some(cached) = cached_update {
            state.version = Some(cached.version.clone());
            state.downloaded_update = Some(cached);
            state.status = SoftwareUpdateStatus::Downloaded;
        } else if installation_kind == SoftwareUpdateInstallationKind::Unsupported
            && state.status == SoftwareUpdateStatus::Idle
        {
            state.status = SoftwareUpdateStatus::Unsupported;
        }
    })
}

#[tauri::command]
pub(crate) async fn check_software_update<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    startup: Option<bool>,
) -> Result<SoftwareUpdateCheckPayload, String> {
    let installation_kind = current_installation_kind()?;
    let previous_snapshot = {
        let mut state = shared_state()
            .lock()
            .map_err(|error| format!("读取软件更新状态失败：{error}"))?;
        if state.check_in_progress {
            return Err("软件更新检查正在进行中。".to_string());
        }
        state.check_in_progress = true;
        state.snapshot.clone()
    };
    let _check_guard = SoftwareUpdateCheckGuard;

    let is_startup = startup.unwrap_or(false);
    let notice_window_label = if is_startup {
        crate::window::tray::last_active_document_window_label()
            .or_else(|| Some(window.label().to_string()))
    } else {
        None
    };
    update_shared_state(&app, |state| {
        state.status = SoftwareUpdateStatus::Checking;
        state.installation_kind = installation_kind;
        state.error = None;
        state.progress = None;
        state.notice_window_label = notice_window_label.clone();
    })?;

    let result = perform_software_update_check(installation_kind).await;
    let final_result = match result {
        Ok(payload) => {
            let cached = if payload.available
                && payload.installation_kind == SoftwareUpdateInstallationKind::Installer
            {
                get_cached_software_update(app.clone())
                    .ok()
                    .flatten()
                    .filter(|cached| Some(&cached.version) == payload.version.as_ref())
            } else {
                None
            };
            update_shared_state(&app, |state| {
                state.current_version = payload.current_version.clone();
                state.installation_kind = payload.installation_kind;
                state.version = payload.version.clone();
                state.date = payload.date.clone();
                state.body = payload.body.clone();
                state.candidate = payload.candidate.clone();
                state.store_product_id = payload.store_product_id.clone();
                state.downloaded_update = cached.clone();
                state.status = if payload.installation_kind == SoftwareUpdateInstallationKind::Store
                {
                    SoftwareUpdateStatus::Managed
                } else if !payload.supported {
                    SoftwareUpdateStatus::Unsupported
                } else if cached.is_some() {
                    SoftwareUpdateStatus::Downloaded
                } else if payload.available {
                    SoftwareUpdateStatus::Available
                } else {
                    SoftwareUpdateStatus::UpToDate
                };
                state.error = None;
            })?;
            Ok(payload)
        }
        Err(error) => {
            if is_startup {
                update_shared_state(&app, |state| {
                    *state = previous_snapshot.clone();
                })?;
            } else {
                update_shared_state(&app, |state| {
                    state.status = SoftwareUpdateStatus::Error;
                    state.error = Some(error.clone());
                })?;
            }
            Err(error)
        }
    };

    final_result
}

async fn perform_software_update_check(
    installation_kind: SoftwareUpdateInstallationKind,
) -> Result<SoftwareUpdateCheckPayload, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    crate::app_logger::info(
        "Update",
        &format!("开始检查软件更新，当前版本：{current_version}"),
    );

    let timer = std::time::Instant::now();

    // Microsoft Store 包由 Store 管理更新，不访问 GitHub，也不生成下载候选。
    if installation_kind == SoftwareUpdateInstallationKind::Store {
        crate::app_logger::info("Update", "当前为 Microsoft Store 包，跳过 GitHub 更新检查");
        return Ok(SoftwareUpdateCheckPayload {
            supported: true,
            available: false,
            current_version: current_version.clone(),
            installation_kind,
            version: Some(current_version),
            date: None,
            body: Some(CURRENT_RELEASE_NOTES.to_string()),
            candidate: None,
            store_product_id: crate::windows_package::store_product_id(),
        });
    }

    // 步骤1：开发环境和非 Windows 平台不发起远程更新检查
    if installation_kind == SoftwareUpdateInstallationKind::Unsupported {
        crate::app_logger::info("Update", "当前环境不支持软件更新，跳过远程检查");
        return Ok(SoftwareUpdateCheckPayload {
            supported: false,
            available: false,
            current_version,
            installation_kind,
            version: None,
            date: None,
            body: None,
            candidate: None,
            store_product_id: None,
        });
    }

    // 步骤2：优先直连 GitHub，网络失败时通过 gh-proxy 请求 Release
    let client = release_http_client()?;
    let (release, release_source) = fetch_latest_github_release(&client).await?;

    // 步骤3：比较版本号
    let release_version = normalize_release_version(&release.tag_name)?;
    let date = release.published_at.clone().or(release.created_at.clone());
    crate::app_logger::info(
        "Update",
        &format!("版本比较：当前 {current_version}，远端 {release_version}"),
    );
    if !is_release_newer(&current_version, &release_version)? {
        crate::app_logger::info("Update", "当前已是最新版本，无需更新");
        return Ok(SoftwareUpdateCheckPayload {
            supported: true,
            available: false,
            current_version,
            installation_kind,
            version: Some(release_version),
            date,
            body: release.body,
            candidate: None,
            store_product_id: None,
        });
    }

    // 步骤4：根据当前安装形态选择安装包或免安装 zip
    crate::app_logger::info(
        "Update",
        &format!("发现新版本 {release_version}，正在查找对应资产"),
    );
    let (asset_kind, update_asset) = match installation_kind {
        SoftwareUpdateInstallationKind::Installer => (
            SoftwareUpdateAssetKind::WindowsInstaller,
            select_windows_installer_asset(&release.assets, &release_version),
        ),
        SoftwareUpdateInstallationKind::Portable => (
            SoftwareUpdateAssetKind::WindowsPortable,
            select_windows_portable_asset(&release.assets, &release_version),
        ),
        SoftwareUpdateInstallationKind::Store => unreachable!(),
        SoftwareUpdateInstallationKind::Unsupported => unreachable!(),
    };
    let update_asset = update_asset.ok_or_else(|| {
        let expected_name = expected_asset_name(asset_kind, &release_version);
        crate::app_logger::error("Update", &format!("缺少更新资产：{expected_name}"));
        format!("GitHub Release 缺少 Windows 更新资产：{expected_name}")
    })?;
    crate::app_logger::info(
        "Update",
        &format!(
            "找到更新资产：{}（{} bytes）",
            update_asset.name,
            update_asset.size.unwrap_or(0)
        ),
    );

    let checksums_asset =
        find_asset_by_name(&release.assets, CHECKSUMS_ASSET_NAME).ok_or_else(|| {
            crate::app_logger::error("Update", "缺少 checksums.md5 校验清单");
            "GitHub Release 缺少 MD5 校验清单 checksums.md5".to_string()
        })?;

    // 步骤5：下载校验清单并匹配 MD5
    let checksums = fetch_github_checksums(
        &client,
        &checksums_asset.browser_download_url,
        release_source,
    )
    .await?;

    let expected_md5 = find_md5_for_file(&checksums, &update_asset.name).ok_or_else(|| {
        crate::app_logger::error(
            "Update",
            &format!("校验清单中未找到 {} 的 MD5", update_asset.name),
        );
        format!("MD5 校验清单缺少更新资产条目：{}", update_asset.name)
    })?;
    crate::app_logger::info("Update", &format!("MD5 校验通过：{}", &expected_md5[..8]));

    let candidate = SoftwareUpdateCandidate {
        version: release_version.clone(),
        date: date.clone(),
        body: release.body.clone(),
        asset_kind,
        asset_name: update_asset.name.clone(),
        asset_size: update_asset.size,
        download_url: update_asset.browser_download_url.clone(),
        md5: expected_md5,
    };

    crate::app_logger::info(
        "Update",
        &format!(
            "更新检查完成，新版本 {release_version} 可用，总耗时：{:?}",
            timer.elapsed()
        ),
    );

    Ok(SoftwareUpdateCheckPayload {
        supported: true,
        available: true,
        current_version,
        installation_kind,
        version: Some(release_version),
        date,
        body: release.body,
        candidate: Some(candidate),
        store_product_id: None,
    })
}

#[tauri::command]
pub(crate) async fn download_software_update<R: Runtime>(
    app: AppHandle<R>,
    candidate: SoftwareUpdateCandidate,
    request_id: String,
) -> Result<DownloadedSoftwareUpdate, String> {
    update_shared_state(&app, |state| {
        state.status = SoftwareUpdateStatus::Downloading;
        state.version = Some(candidate.version.clone());
        state.candidate = Some(candidate.clone());
        state.progress = Some(SoftwareUpdateDownloadProgress {
            request_id: request_id.clone(),
            downloaded_bytes: 0,
            total_bytes: candidate.asset_size,
            percent: Some(0),
        });
        state.error = None;
    })?;

    let result = download_software_update_inner(app.clone(), candidate, request_id).await;
    match &result {
        Ok(downloaded) => {
            update_shared_state(&app, |state| {
                state.status = SoftwareUpdateStatus::Downloaded;
                state.downloaded_update = Some(downloaded.clone());
                state.progress = None;
                state.error = None;
            })?;
        }
        Err(error) => {
            update_shared_state(&app, |state| {
                state.status = SoftwareUpdateStatus::Error;
                state.progress = None;
                state.error = Some(error.clone());
            })?;
        }
    }
    result
}

async fn download_software_update_inner<R: Runtime>(
    app: AppHandle<R>,
    candidate: SoftwareUpdateCandidate,
    request_id: String,
) -> Result<DownloadedSoftwareUpdate, String> {
    crate::app_logger::info(
        "Update",
        &format!(
            "开始下载更新包：{}（{} bytes）",
            candidate.asset_name,
            candidate.asset_size.unwrap_or(0)
        ),
    );
    let timer = std::time::Instant::now();

    if candidate.asset_kind != SoftwareUpdateAssetKind::WindowsInstaller
        || current_installation_kind()? != SoftwareUpdateInstallationKind::Installer
    {
        crate::app_logger::error("Update", "非安装版环境，拒绝下载更新");
        return Err("当前环境不支持自动更新：仅 Windows 安装版支持应用内更新。".to_string());
    }
    validate_md5(&candidate.md5)?;

    let update_dir = software_update_cache_dir(&app)?;
    fs::create_dir_all(&update_dir).map_err(|error| format!("创建更新缓存目录失败：{error}"))?;
    let target_path = update_dir.join(&candidate.asset_name);
    let temp_path = update_dir.join(format!("{}.download", candidate.asset_name));
    let _ = fs::remove_file(&temp_path);
    crate::app_logger::info(
        "Update",
        &format!("下载目标路径：{}", target_path.display()),
    );

    let client = release_http_client()?;
    let direct_result = download_software_update_from_source(
        &app,
        &client,
        &candidate,
        &request_id,
        &temp_path,
        SoftwareUpdateSource::GitHub,
    )
    .await;
    let outcome = match direct_result {
        Ok(outcome) => outcome,
        Err(direct_error) if direct_error.retryable => {
            crate::app_logger::warn(
                "Update",
                &format!(
                    "GitHub 直连下载失败，将通过 gh-proxy 重试：{}",
                    direct_error.message
                ),
            );
            let _ = fs::remove_file(&temp_path);
            emit_download_progress(&app, &request_id, 0, candidate.asset_size);

            match download_software_update_from_source(
                &app,
                &client,
                &candidate,
                &request_id,
                &temp_path,
                SoftwareUpdateSource::GhProxy,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(proxy_error) => {
                    let _ = fs::remove_file(&temp_path);
                    let error = format!(
                        "下载更新安装包失败：GitHub 直连失败（{}）；gh-proxy 失败（{}）",
                        direct_error.message, proxy_error.message
                    );
                    crate::app_logger::error("Update", &error);
                    return Err(error);
                }
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            crate::app_logger::error("Update", &error.message);
            return Err(error.message);
        }
    };
    let downloaded_bytes = outcome.downloaded_bytes;
    let actual_md5 = outcome.actual_md5;
    crate::app_logger::info("Update", "MD5 校验通过");

    fs::rename(&temp_path, &target_path).map_err(|error| format!("保存更新安装包失败：{error}"))?;
    crate::app_logger::info(
        "Update",
        &format!(
            "更新包已保存：{}，总耗时：{:?}",
            target_path.display(),
            timer.elapsed()
        ),
    );

    let downloaded = DownloadedSoftwareUpdate {
        version: candidate.version,
        asset_name: candidate.asset_name,
        file_path: target_path.to_string_lossy().to_string(),
        md5: actual_md5,
        downloaded_bytes,
    };

    // 持久化下载信息，以便重新打开设置页时能识别已下载的更新
    save_cached_update_info(&update_dir, &downloaded)?;

    Ok(downloaded)
}

#[tauri::command]
pub(crate) fn install_software_update<R: Runtime>(
    app: AppHandle<R>,
    downloaded_update: DownloadedSoftwareUpdate,
) -> Result<(), String> {
    update_shared_state(&app, |state| {
        state.status = SoftwareUpdateStatus::Installing;
        state.error = None;
    })?;
    let result = install_software_update_inner(app.clone(), downloaded_update);
    if let Err(error) = &result {
        update_shared_state(&app, |state| {
            state.status = SoftwareUpdateStatus::Error;
            state.error = Some(error.clone());
        })?;
    }
    result
}

fn install_software_update_inner<R: Runtime>(
    app: AppHandle<R>,
    downloaded_update: DownloadedSoftwareUpdate,
) -> Result<(), String> {
    crate::app_logger::info(
        "Update",
        &format!("开始安装更新：{}", downloaded_update.version),
    );

    if !is_current_windows_installer_installation()? {
        crate::app_logger::error("Update", "非安装版环境，拒绝安装更新");
        return Err("当前环境不支持自动更新：仅 Windows 安装版支持应用内更新。".to_string());
    }

    let update_dir = software_update_cache_dir(&app)?;
    let installer_path = validate_downloaded_installer_path(
        &PathBuf::from(&downloaded_update.file_path),
        &update_dir,
        &downloaded_update.asset_name,
    )?;
    crate::app_logger::info(
        "Update",
        &format!("安装包路径验证通过：{}", installer_path.display()),
    );

    let actual_md5 = calculate_file_md5(&installer_path)?;
    if !actual_md5.eq_ignore_ascii_case(&downloaded_update.md5) {
        crate::app_logger::error(
            "Update",
            &format!(
                "安装前 MD5 校验失败：期望 {}，实际 {}",
                downloaded_update.md5, actual_md5
            ),
        );
        return Err(format!(
            "更新包校验失败：Release 记录的 MD5 为 {}，实际安装文件 MD5 为 {}。",
            downloaded_update.md5, actual_md5
        ));
    }
    crate::app_logger::info("Update", "安装前 MD5 校验通过，正在启动安装器");

    // 安装启动前清除缓存信息，避免用户取消安装后仍显示"已下载"
    remove_cached_update_info(&update_dir);

    launch_windows_installer_and_exit(&app, &installer_path)
}

#[cfg(target_os = "windows")]
fn is_current_windows_installer_installation() -> Result<bool, String> {
    let exe_path = std::env::current_exe()
        .map_err(|error| format!("读取 Nomo 可执行文件路径失败：{error}"))?;
    is_windows_installer_installation_for_path(&exe_path)
}

#[cfg(not(target_os = "windows"))]
fn is_current_windows_installer_installation() -> Result<bool, String> {
    Ok(false)
}

#[cfg(all(target_os = "windows", debug_assertions))]
fn current_installation_kind() -> Result<SoftwareUpdateInstallationKind, String> {
    if crate::windows_package::is_packaged() {
        Ok(SoftwareUpdateInstallationKind::Store)
    } else {
        Ok(SoftwareUpdateInstallationKind::Unsupported)
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn current_installation_kind() -> Result<SoftwareUpdateInstallationKind, String> {
    if crate::windows_package::is_packaged() {
        Ok(SoftwareUpdateInstallationKind::Store)
    } else if is_current_windows_installer_installation()? {
        Ok(SoftwareUpdateInstallationKind::Installer)
    } else {
        Ok(SoftwareUpdateInstallationKind::Portable)
    }
}

#[cfg(not(target_os = "windows"))]
fn current_installation_kind() -> Result<SoftwareUpdateInstallationKind, String> {
    Ok(SoftwareUpdateInstallationKind::Unsupported)
}

#[cfg(target_os = "windows")]
fn is_windows_installer_installation_for_path(exe_path: &Path) -> Result<bool, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::info(
        "Update",
        &format!("开始查询注册表安装位置，可执行文件：{}", exe_path.display()),
    );

    let hkcu_location = query_install_location("HKCU")?;
    crate::app_logger::info(
        "Update",
        &format!(
            "HKCU 安装位置：{:?}，耗时：{:?}",
            hkcu_location,
            timer.elapsed()
        ),
    );

    let hkcu_elapsed = timer.elapsed();
    let hklm_location = query_install_location("HKLM")?;
    crate::app_logger::info(
        "Update",
        &format!(
            "HKLM 安装位置：{:?}，耗时：{:?}",
            hklm_location,
            timer.elapsed() - hkcu_elapsed
        ),
    );

    let install_locations = [hkcu_location, hklm_location];

    for location in install_locations.into_iter().flatten() {
        if executable_belongs_to_install_location(exe_path, &location) {
            crate::app_logger::info("Update", "检测到安装版环境");
            return Ok(true);
        }
    }

    crate::app_logger::info(
        "Update",
        &format!("未检测到安装版环境，总耗时：{:?}", timer.elapsed()),
    );
    Ok(false)
}

#[cfg(target_os = "windows")]
fn query_install_location(root: &str) -> Result<Option<String>, String> {
    query_reg_value(
        root,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Nomo",
        "InstallLocation",
    )
}

#[cfg(target_os = "windows")]
pub(crate) fn legacy_windows_installer_exists() -> Result<bool, String> {
    for root in ["HKCU", "HKLM"] {
        if let Some(location) = query_install_location(root)? {
            let install_dir = PathBuf::from(location.trim().trim_matches('"'));
            if install_dir.join("uninstall.exe").is_file() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(target_os = "windows")]
fn executable_belongs_to_install_location(exe_path: &Path, location: &str) -> bool {
    let install_dir = PathBuf::from(location.trim().trim_matches('"'));
    if install_dir.as_os_str().is_empty() {
        return false;
    }

    let normalized_exe = normalize_path(exe_path);
    let normalized_install_dir = normalize_path(&install_dir);
    let uninstall_exe_exists = install_dir.join("uninstall.exe").exists();

    uninstall_exe_exists && normalized_exe.starts_with(&normalized_install_dir)
}

#[cfg(target_os = "windows")]
fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .trim_matches('"')
        .to_ascii_lowercase()
}

#[cfg(target_os = "windows")]
fn query_reg_value(root: &str, key: &str, value: &str) -> Result<Option<String>, String> {
    use std::os::windows::process::CommandExt;
    // 禁止为 reg.exe 创建可见控制台窗口，避免执行时弹出黑框
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let reg_path = format!("{root}\\{key}");
    crate::app_logger::debug("Update", &format!("查询注册表：{reg_path} /v {value}"));
    let timer = std::time::Instant::now();

    let output = std::process::Command::new("reg.exe")
        .args(["query", &reg_path, "/v", value])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| {
            crate::app_logger::error("Update", &format!("调用 reg.exe 失败：{error}"));
            format!("调用 reg.exe 失败：{error}")
        })?;

    let elapsed = timer.elapsed();
    if !output.status.success() {
        crate::app_logger::debug(
            "Update",
            &format!("注册表键不存在：{reg_path}，耗时：{elapsed:?}"),
        );
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = parse_reg_value(&stdout, value);
    crate::app_logger::debug(
        "Update",
        &format!(
            "注册表查询完成：{reg_path}，结果：{:?}，耗时：{elapsed:?}",
            result
        ),
    );
    Ok(result)
}

#[cfg(target_os = "windows")]
fn parse_reg_value(output: &str, value: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with(value) {
            return None;
        }

        let mut parts = trimmed.split_whitespace();
        let _name = parts.next()?;
        let _kind = parts.next()?;
        let data = parts.collect::<Vec<_>>().join(" ");
        if data.is_empty() {
            None
        } else {
            Some(data)
        }
    })
}

async fn fetch_latest_github_release(
    client: &reqwest::Client,
) -> Result<(GitHubRelease, SoftwareUpdateSource), String> {
    crate::app_logger::info("Update", "正在通过 GitHub 直连请求最新 Release");
    let direct_timer = std::time::Instant::now();
    match fetch_latest_github_release_from_source(client, SoftwareUpdateSource::GitHub).await {
        Ok(release) => {
            crate::app_logger::info(
                "Update",
                &format!(
                    "GitHub 直连 Release 请求完成，耗时：{:?}，tag：{}",
                    direct_timer.elapsed(),
                    release.tag_name
                ),
            );
            Ok((release, SoftwareUpdateSource::GitHub))
        }
        Err(error) if error.retryable => {
            crate::app_logger::warn(
                "Update",
                &format!(
                    "GitHub 直连 Release 请求失败，将通过 gh-proxy 重试：{}",
                    error.message
                ),
            );
            let proxy_timer = std::time::Instant::now();
            match fetch_latest_github_release_from_source(client, SoftwareUpdateSource::GhProxy)
                .await
            {
                Ok(release) => {
                    crate::app_logger::info(
                        "Update",
                        &format!(
                            "gh-proxy Release 请求完成，耗时：{:?}，tag：{}",
                            proxy_timer.elapsed(),
                            release.tag_name
                        ),
                    );
                    Ok((release, SoftwareUpdateSource::GhProxy))
                }
                Err(proxy_error) => {
                    let message = format!(
                        "检查 GitHub Release 更新失败：GitHub 直连失败（{}）；gh-proxy 失败（{}）",
                        error.message, proxy_error.message
                    );
                    crate::app_logger::error("Update", &message);
                    Err(message)
                }
            }
        }
        Err(error) => {
            let message = format!("检查 GitHub Release 更新失败：{}", error.message);
            crate::app_logger::error("Update", &message);
            Err(message)
        }
    }
}

async fn fetch_latest_github_release_from_source(
    client: &reqwest::Client,
    source: SoftwareUpdateSource,
) -> Result<GitHubRelease, SoftwareUpdateRequestFailure> {
    let response = send_small_update_request(
        client,
        GITHUB_LATEST_RELEASE_API,
        source,
        "请求 GitHub Release 失败",
    )
    .await?;
    response.json::<GitHubRelease>().await.map_err(|error| {
        SoftwareUpdateRequestFailure::from_reqwest("解析 GitHub Release 更新信息失败", error)
    })
}

async fn fetch_github_checksums(
    client: &reqwest::Client,
    original_url: &str,
    release_source: SoftwareUpdateSource,
) -> Result<String, String> {
    crate::app_logger::info(
        "Update",
        &format!("正在通过 {}下载 MD5 校验清单", release_source.label()),
    );
    let request_timer = std::time::Instant::now();
    match fetch_github_checksums_from_source(client, original_url, release_source).await {
        Ok(checksums) => {
            crate::app_logger::info(
                "Update",
                &format!(
                    "{}校验清单下载完成，耗时：{:?}",
                    release_source.label(),
                    request_timer.elapsed()
                ),
            );
            Ok(checksums)
        }
        Err(error) if release_source == SoftwareUpdateSource::GitHub && error.retryable => {
            crate::app_logger::warn(
                "Update",
                &format!(
                    "GitHub 直连下载校验清单失败，将通过 gh-proxy 重试：{}",
                    error.message
                ),
            );
            let proxy_timer = std::time::Instant::now();
            match fetch_github_checksums_from_source(
                client,
                original_url,
                SoftwareUpdateSource::GhProxy,
            )
            .await
            {
                Ok(checksums) => {
                    crate::app_logger::info(
                        "Update",
                        &format!(
                            "gh-proxy 校验清单下载完成，耗时：{:?}",
                            proxy_timer.elapsed()
                        ),
                    );
                    Ok(checksums)
                }
                Err(proxy_error) => {
                    let message = format!(
                        "下载 MD5 校验清单失败：GitHub 直连失败（{}）；gh-proxy 失败（{}）",
                        error.message, proxy_error.message
                    );
                    crate::app_logger::error("Update", &message);
                    Err(message)
                }
            }
        }
        Err(error) => {
            let message = format!(
                "通过 {}下载 MD5 校验清单失败：{}",
                release_source.label(),
                error.message
            );
            crate::app_logger::error("Update", &message);
            Err(message)
        }
    }
}

async fn fetch_github_checksums_from_source(
    client: &reqwest::Client,
    original_url: &str,
    source: SoftwareUpdateSource,
) -> Result<String, SoftwareUpdateRequestFailure> {
    let response =
        send_small_update_request(client, original_url, source, "下载 MD5 校验清单失败").await?;
    response
        .text()
        .await
        .map_err(|error| SoftwareUpdateRequestFailure::from_reqwest("读取 MD5 校验清单失败", error))
}

async fn send_small_update_request(
    client: &reqwest::Client,
    original_url: &str,
    source: SoftwareUpdateSource,
    context: &str,
) -> Result<reqwest::Response, SoftwareUpdateRequestFailure> {
    let request_url = update_request_url(original_url, source)?;
    client
        .get(request_url)
        .timeout(UPDATE_SMALL_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| SoftwareUpdateRequestFailure::from_reqwest(context, error))?
        .error_for_status()
        .map_err(|error| SoftwareUpdateRequestFailure::from_reqwest(context, error))
}

async fn download_software_update_from_source<R: Runtime>(
    app: &AppHandle<R>,
    client: &reqwest::Client,
    candidate: &SoftwareUpdateCandidate,
    request_id: &str,
    temp_path: &Path,
    source: SoftwareUpdateSource,
) -> Result<DownloadAttemptOutcome, SoftwareUpdateRequestFailure> {
    let request_url = update_request_url(&candidate.download_url, source)?;
    crate::app_logger::info(
        "Update",
        &format!("正在通过 {}下载更新安装包", source.label()),
    );
    let timer = std::time::Instant::now();
    let mut response = client
        .get(request_url)
        .send()
        .await
        .map_err(|error| {
            SoftwareUpdateRequestFailure::from_reqwest("发起更新安装包下载请求失败", error)
        })?
        .error_for_status()
        .map_err(|error| {
            SoftwareUpdateRequestFailure::from_reqwest("更新安装包下载接口返回异常", error)
        })?;
    let total_bytes = response.content_length().or(candidate.asset_size);
    crate::app_logger::info(
        "Update",
        &format!(
            "{}下载连接建立成功，总大小：{:?} bytes",
            source.label(),
            total_bytes
        ),
    );

    let mut file = File::create(temp_path).map_err(|error| {
        SoftwareUpdateRequestFailure::terminal(format!("创建更新安装包缓存失败：{error}"))
    })?;
    let mut context = md5::Context::new();
    let mut downloaded_bytes = 0_u64;
    emit_download_progress(app, request_id, downloaded_bytes, total_bytes);

    while let Some(chunk) = response.chunk().await.map_err(|error| {
        SoftwareUpdateRequestFailure::from_reqwest("读取更新安装包下载内容失败", error)
    })? {
        file.write_all(&chunk).map_err(|error| {
            SoftwareUpdateRequestFailure::terminal(format!("写入更新安装包缓存失败：{error}"))
        })?;
        context.consume(&chunk);
        downloaded_bytes += chunk.len() as u64;
        emit_download_progress(app, request_id, downloaded_bytes, total_bytes);
    }
    file.flush().map_err(|error| {
        SoftwareUpdateRequestFailure::terminal(format!("刷新更新安装包缓存失败：{error}"))
    })?;

    let actual_md5 = format!("{:x}", context.compute());
    if !actual_md5.eq_ignore_ascii_case(&candidate.md5) {
        return Err(SoftwareUpdateRequestFailure::terminal(format!(
            "更新包校验失败：Release 记录的 MD5 为 {}，实际下载文件 MD5 为 {}。",
            candidate.md5, actual_md5
        )));
    }
    crate::app_logger::info(
        "Update",
        &format!(
            "{}下载完成：{downloaded_bytes} bytes，耗时：{:?}",
            source.label(),
            timer.elapsed()
        ),
    );

    Ok(DownloadAttemptOutcome {
        downloaded_bytes,
        actual_md5,
    })
}

fn update_request_url(
    original_url: &str,
    source: SoftwareUpdateSource,
) -> Result<String, SoftwareUpdateRequestFailure> {
    match source {
        SoftwareUpdateSource::GitHub => Ok(original_url.to_string()),
        SoftwareUpdateSource::GhProxy => github_proxy_url(original_url).ok_or_else(|| {
            SoftwareUpdateRequestFailure::terminal(
                "该更新地址不属于允许通过 gh-proxy 访问的 Nomo GitHub 地址。",
            )
        }),
    }
}

fn github_proxy_url(original_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(original_url).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    // 仓库迁移后 Release 返回新地址，同时兼容旧地址和已有更新缓存。
    let allowed = (url.host_str() == Some("api.github.com")
        && matches!(
            url.path(),
            "/repos/nomo-md/nomo/releases/latest" | "/repos/LIXianSenQwQ/nomo/releases/latest"
        ))
        || (url.host_str() == Some("github.com")
            && (url.path().starts_with("/nomo-md/nomo/releases/download/")
                || url
                    .path()
                    .starts_with("/LIXianSenQwQ/nomo/releases/download/")));
    allowed.then(|| format!("{GITHUB_PROXY_PREFIX}{original_url}"))
}

fn is_retryable_update_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn release_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Nomo software updater")
        .connect_timeout(UPDATE_CONNECT_TIMEOUT)
        .read_timeout(UPDATE_DOWNLOAD_READ_TIMEOUT)
        .build()
        .map_err(|error| format!("创建更新请求客户端失败：{error}"))
}

fn normalize_release_version(tag_name: &str) -> Result<String, String> {
    let version = tag_name.trim().trim_start_matches('v').trim();
    if version.is_empty() {
        return Err("GitHub Release 缺少有效版本号。".to_string());
    }
    Ok(version.to_string())
}

fn is_release_newer(current_version: &str, release_version: &str) -> Result<bool, String> {
    let current = semver::Version::parse(current_version)
        .map_err(|error| format!("解析当前版本号失败：{error}"))?;
    let release = semver::Version::parse(release_version)
        .map_err(|error| format!("解析 Release 版本号失败：{error}"))?;
    Ok(release > current)
}

fn select_windows_installer_asset<'a>(
    assets: &'a [GitHubReleaseAsset],
    version: &str,
) -> Option<&'a GitHubReleaseAsset> {
    let expected_name = format!("Nomo_{version}_x64-setup.exe");
    assets.iter().find(|asset| asset.name == expected_name)
}

fn select_windows_portable_asset<'a>(
    assets: &'a [GitHubReleaseAsset],
    version: &str,
) -> Option<&'a GitHubReleaseAsset> {
    let expected_name = format!("Nomo_{version}_x64.zip");
    assets.iter().find(|asset| asset.name == expected_name)
}

fn expected_asset_name(kind: SoftwareUpdateAssetKind, version: &str) -> String {
    match kind {
        SoftwareUpdateAssetKind::WindowsInstaller => {
            format!("Nomo_{version}_x64-setup.exe")
        }
        SoftwareUpdateAssetKind::WindowsPortable => format!("Nomo_{version}_x64.zip"),
    }
}

fn find_asset_by_name<'a>(
    assets: &'a [GitHubReleaseAsset],
    name: &str,
) -> Option<&'a GitHubReleaseAsset> {
    assets.iter().find(|asset| asset.name == name)
}

fn find_md5_for_file(checksums: &str, file_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let md5 = parts.next()?.trim();
        let rest = parts.next()?.trim();
        if !is_valid_md5(md5) || rest != file_name {
            return None;
        }

        Some(md5.to_ascii_lowercase())
    })
}

fn validate_md5(md5: &str) -> Result<(), String> {
    if is_valid_md5(md5) {
        Ok(())
    } else {
        Err("Release 记录的 MD5 格式无效。".to_string())
    }
}

fn is_valid_md5(md5: &str) -> bool {
    md5.len() == 32 && md5.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn software_update_cache_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("获取更新缓存目录失败：{error}"))?
        .join("updates"))
}

fn save_cached_update_info(
    update_dir: &Path,
    downloaded: &DownloadedSoftwareUpdate,
) -> Result<(), String> {
    let json_path = update_dir.join(CACHED_UPDATE_INFO_FILE);
    let json = serde_json::to_string(downloaded)
        .map_err(|error| format!("序列化更新缓存信息失败：{error}"))?;
    fs::write(&json_path, json).map_err(|error| format!("写入更新缓存信息失败：{error}"))?;
    crate::app_logger::debug(
        "Update",
        &format!("更新缓存信息已保存：{}", json_path.display()),
    );
    Ok(())
}

fn read_cached_update_info(update_dir: &Path) -> Result<Option<DownloadedSoftwareUpdate>, String> {
    let json_path = update_dir.join(CACHED_UPDATE_INFO_FILE);
    if !json_path.is_file() {
        return Ok(None);
    }
    let json =
        fs::read_to_string(&json_path).map_err(|error| format!("读取更新缓存信息失败：{error}"))?;
    let info: DownloadedSoftwareUpdate =
        serde_json::from_str(&json).map_err(|error| format!("解析更新缓存信息失败：{error}"))?;
    Ok(Some(info))
}

fn remove_cached_update_info(update_dir: &Path) {
    let json_path = update_dir.join(CACHED_UPDATE_INFO_FILE);
    if json_path.is_file() {
        let _ = fs::remove_file(&json_path);
    }
}

fn emit_download_progress<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let percent = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((downloaded_bytes.saturating_mul(100) / total).min(100)) as u8);
    let progress = SoftwareUpdateDownloadProgress {
        request_id: request_id.to_string(),
        downloaded_bytes,
        total_bytes,
        percent,
    };
    let _ = app.emit(DOWNLOAD_PROGRESS_EVENT, progress.clone());
    let _ = update_shared_state(app, |state| {
        state.status = SoftwareUpdateStatus::Downloading;
        state.progress = Some(progress);
    });
}

fn shared_state() -> &'static Mutex<SoftwareUpdateRuntimeState> {
    SOFTWARE_UPDATE_STATE.get_or_init(|| Mutex::new(SoftwareUpdateRuntimeState::default()))
}

fn update_shared_state<R: Runtime>(
    app: &AppHandle<R>,
    updater: impl FnOnce(&mut SoftwareUpdateSnapshot),
) -> Result<SoftwareUpdateSnapshot, String> {
    let snapshot = {
        let mut runtime_state = shared_state()
            .lock()
            .map_err(|error| format!("更新软件状态失败：{error}"))?;
        updater(&mut runtime_state.snapshot);
        runtime_state.snapshot.clone()
    };
    let _ = app.emit(UPDATE_STATE_EVENT, snapshot.clone());
    Ok(snapshot)
}

fn calculate_file_md5(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("读取更新安装包失败：{error}"))?;
    Ok(format!("{:x}", md5::compute(bytes)))
}

fn validate_downloaded_installer_path(
    file_path: &Path,
    update_dir: &Path,
    asset_name: &str,
) -> Result<PathBuf, String> {
    let canonical_file = file_path
        .canonicalize()
        .map_err(|error| format!("读取更新安装包路径失败：{error}"))?;
    let canonical_dir = update_dir
        .canonicalize()
        .map_err(|error| format!("读取更新缓存目录失败：{error}"))?;
    let file_name = canonical_file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "更新安装包文件名无效。".to_string())?;

    if !canonical_file.starts_with(&canonical_dir) || file_name != asset_name {
        return Err("更新安装包路径无效。".to_string());
    }

    Ok(canonical_file)
}

#[cfg(target_os = "windows")]
fn launch_windows_installer_and_exit<R: Runtime>(
    app: &AppHandle<R>,
    installer_path: &Path,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW},
    };

    fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let file = wide_null(installer_path.as_os_str());
    let operation = wide_null(std::ffi::OsStr::new("open"));
    let parameters = wide_null(std::ffi::OsStr::new("/P /R"));

    let result = unsafe {
        ShellExecuteW(
            0 as HWND,
            operation.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        )
    };
    if result as isize <= 32 {
        return Err(format!(
            "启动更新安装器失败，系统错误码：{}",
            result as isize
        ));
    }

    app.exit(0);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn launch_windows_installer_and_exit<R: Runtime>(
    _app: &AppHandle<R>,
    _installer_path: &Path,
) -> Result<(), String> {
    Err("当前平台不支持 Windows 安装包更新。".to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        find_md5_for_file, is_release_newer, perform_software_update_check,
        select_windows_installer_asset, GitHubReleaseAsset, SoftwareUpdateInstallationKind,
    };

    fn asset(name: &str) -> GitHubReleaseAsset {
        GitHubReleaseAsset {
            name: name.to_string(),
            size: Some(100),
            browser_download_url: format!("https://example.test/{name}"),
        }
    }

    #[test]
    fn parses_checksums_md5_with_exact_file_name() {
        let checksums = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  Nomo_0.1.4_x64.zip
BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB  Nomo_0.1.4_x64-setup.exe
cccccccccccccccccccccccccccccccc  Nomo_0.1.4_x64-setup.exe.sig";

        assert_eq!(
            find_md5_for_file(checksums, "Nomo_0.1.4_x64-setup.exe").as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn ignores_invalid_or_missing_checksum_rows() {
        let checksums = "\
not-md5  Nomo_0.1.4_x64-setup.exe
dddddddddddddddddddddddddddddddd
eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  Nomo_0.1.4_x64.zip";

        assert_eq!(
            find_md5_for_file(checksums, "Nomo_0.1.4_x64-setup.exe"),
            None
        );
    }

    #[test]
    fn supports_asset_names_with_spaces() {
        let checksums = "ffffffffffffffffffffffffffffffff  Nomo Setup 0.1.4 x64.exe";

        assert_eq!(
            find_md5_for_file(checksums, "Nomo Setup 0.1.4 x64.exe").as_deref(),
            Some("ffffffffffffffffffffffffffffffff")
        );
    }

    #[test]
    fn selects_only_expected_windows_installer_asset() {
        let assets = vec![
            asset("Nomo_0.1.4_x64.zip"),
            asset("Nomo_0.1.4_x64-setup.exe.sig"),
            asset("Nomo_0.1.4_aarch64.dmg"),
            asset("Nomo_0.1.4_x64-setup.exe"),
        ];

        assert_eq!(
            select_windows_installer_asset(&assets, "0.1.4").map(|asset| asset.name.as_str()),
            Some("Nomo_0.1.4_x64-setup.exe")
        );
    }

    #[test]
    fn compares_semantic_versions() {
        assert!(is_release_newer("0.1.3", "0.1.4").unwrap());
        assert!(!is_release_newer("0.1.4", "0.1.4").unwrap());
        assert!(!is_release_newer("0.1.5", "0.1.4").unwrap());
    }

    #[test]
    fn store_update_check_never_creates_a_github_candidate() {
        let payload = tauri::async_runtime::block_on(perform_software_update_check(
            SoftwareUpdateInstallationKind::Store,
        ))
        .unwrap();

        assert!(payload.supported);
        assert!(!payload.available);
        assert_eq!(
            payload.installation_kind,
            SoftwareUpdateInstallationKind::Store
        );
        assert!(payload.candidate.is_none());
        assert!(payload.body.is_some());
    }

    #[cfg(target_os = "windows")]
    mod windows_tests {
        use super::super::parse_reg_value;

        #[test]
        fn parses_install_location_with_spaces() {
            let output = r#"
HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo
    InstallLocation    REG_SZ    "C:\Users\Qing Yu\AppData\Local\Programs\Nomo"
"#;

            assert_eq!(
                parse_reg_value(output, "InstallLocation"),
                Some(r#""C:\Users\Qing Yu\AppData\Local\Programs\Nomo""#.to_string())
            );
        }
    }
}

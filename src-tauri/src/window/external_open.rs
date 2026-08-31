use crate::models::SettingInput;
use serde::Serialize;
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
#[cfg(any(target_os = "macos", test))]
use tauri::Url;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const OPEN_DOCUMENT_EVENT: &str = "nomo://open-document";
const OPEN_FOLDER_EVENT: &str = "nomo://open-folder";
const PENDING_EXTERNAL_OPEN_PREFIX: &str = "pendingExternalOpen:";
const PENDING_EXTERNAL_FOLDER_PREFIX: &str = "pendingFolder:";

static EARLY_EXTERNAL_OPEN_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
struct ExternalOpenPayload {
    #[serde(rename = "windowLabel")]
    window_label: String,
    paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ExternalFolderOpenPayload {
    #[serde(rename = "windowLabel")]
    window_label: String,
    folder_path: String,
}

#[derive(Debug, Default)]
pub(crate) struct ExternalOpenTargets {
    pub(crate) markdown_paths: Vec<String>,
    pub(crate) folder_paths: Vec<String>,
}

#[allow(dead_code)]
pub(crate) fn collect_markdown_paths_from_startup_args() -> Vec<String> {
    collect_external_open_targets_from_startup_args().markdown_paths
}

#[allow(dead_code)]
pub(crate) fn collect_markdown_paths_from_args(
    args: Vec<String>,
    cwd: Option<PathBuf>,
) -> Vec<String> {
    collect_external_open_targets_from_args(args, cwd).markdown_paths
}

pub(crate) fn collect_external_open_targets_from_startup_args() -> ExternalOpenTargets {
    let args = env::args().collect::<Vec<_>>();
    let cwd = env::current_dir().ok();
    collect_external_open_targets_from_args(args, cwd)
}

pub(crate) fn collect_external_open_targets_from_args(
    args: Vec<String>,
    cwd: Option<PathBuf>,
) -> ExternalOpenTargets {
    let exe_path = env::current_exe().ok();
    let mut seen = HashSet::new();
    let mut targets = ExternalOpenTargets::default();

    for arg in args {
        if arg.starts_with('-') {
            continue;
        }

        let candidate = PathBuf::from(&arg);
        let absolute = if candidate.is_absolute() {
            candidate
        } else if let Some(cwd) = cwd.as_ref() {
            cwd.join(candidate)
        } else {
            candidate
        };

        if is_supported_external_file(&absolute, exe_path.as_deref()) {
            let normalized = normalize_path(&absolute);
            if seen.insert(normalized.clone()) {
                targets.markdown_paths.push(normalized);
            }
        } else if is_supported_external_folder(&absolute, exe_path.as_deref()) {
            let normalized = normalize_path(&absolute);
            if seen.insert(normalized.clone()) {
                targets.folder_paths.push(normalized);
            }
        }
    }

    targets
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn collect_markdown_paths_from_urls(urls: Vec<Url>) -> Vec<String> {
    let exe_path = env::current_exe().ok();
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    for url in urls {
        if url.scheme() != "file" {
            continue;
        }
        let Ok(path) = url.to_file_path() else {
            continue;
        };
        if is_supported_external_file(&path, exe_path.as_deref()) {
            let normalized = normalize_path(&path);
            if seen.insert(normalized.clone()) {
                paths.push(normalized);
            }
        }
    }

    paths
}

pub(crate) fn queue_early_external_open(paths: Vec<String>) -> Result<(), String> {
    let mut pending = EARLY_EXTERNAL_OPEN_PATHS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "锁定 setup 前外部打开队列失败".to_string())?;
    for path in paths {
        if !pending.contains(&path) {
            pending.push(path);
        }
    }
    Ok(())
}

pub(crate) fn take_early_external_open_paths() -> Result<Vec<String>, String> {
    let mut pending = EARLY_EXTERNAL_OPEN_PATHS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "锁定 setup 前外部打开队列失败".to_string())?;
    Ok(std::mem::take(&mut *pending))
}

pub(crate) fn route_external_open(app: &AppHandle, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }

    let Some(window) = target_document_window(app) else {
        persist_pending_external_open(app, "main", &paths)?;
        return Ok(());
    };

    let label = window.label().to_string();
    let _ = persist_pending_external_open(app, &label, &paths);

    // 这里只转交请求；前端确定使用当前窗口或需要询问时再激活，避免新窗口模式唤起旧窗口。
    window
        .emit(
            OPEN_DOCUMENT_EVENT,
            ExternalOpenPayload {
                window_label: label.clone(),
                paths: paths.clone(),
            },
        )
        .map_err(|error| format!("发送外部打开文件事件失败：{error}"))?;
    Ok(())
}

pub(crate) fn route_external_open_targets(
    app: &AppHandle,
    targets: ExternalOpenTargets,
) -> Result<(), String> {
    route_external_open(app, targets.markdown_paths)?;
    if let Some(folder_path) = targets.folder_paths.into_iter().next() {
        route_external_folder_open(app, folder_path)?;
    }
    Ok(())
}

pub(crate) fn route_external_folder_open(
    app: &AppHandle,
    folder_path: String,
) -> Result<(), String> {
    if folder_path.is_empty() {
        return Ok(());
    }

    let Some(window) = target_document_window(app) else {
        persist_pending_external_folder_open(app, "main", &folder_path)?;
        return Ok(());
    };

    let label = window.label().to_string();
    let _ = persist_pending_external_folder_open(app, &label, &folder_path);

    // 恢复窗口显示并强制到前台（处理托盘/最小化/后台等各种状态）
    window
        .show()
        .map_err(|error| format!("显示外部打开目标窗口失败：{error}"))?;
    window
        .unminimize()
        .map_err(|error| format!("还原外部打开目标窗口失败：{error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("聚焦外部打开目标窗口失败：{error}"))?;
    crate::window::os::bring_window_to_front(&window);

    // 延迟补一次激活，确保在 WebView 初始化或异步场景下也能成功
    let window_for_focus = window.clone();
    tauri::async_runtime::spawn(async move {
        std::thread::sleep(std::time::Duration::from_millis(120));
        let _ = window_for_focus.show();
        let _ = window_for_focus.unminimize();
        let _ = window_for_focus.set_focus();
        crate::window::os::bring_window_to_front(&window_for_focus);
    });

    window
        .emit(
            OPEN_FOLDER_EVENT,
            ExternalFolderOpenPayload {
                window_label: label.clone(),
                folder_path: folder_path.clone(),
            },
        )
        .map_err(|error| format!("发送外部打开文件夹事件失败：{error}"))?;
    crate::window::tray::set_tray_active(app, true);
    Ok(())
}

pub(crate) fn persist_pending_external_open(
    app: &AppHandle,
    label: &str,
    paths: &[String],
) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }

    let key = pending_external_open_key(label);
    crate::config::commands::update_app_setting(
        app.clone(),
        SettingInput {
            key,
            value_json: serde_json::to_string(paths)
                .map_err(|error| format!("序列化待外部打开路径失败：{error}"))?,
        },
    )
}

pub(crate) fn persist_pending_external_folder_open(
    app: &AppHandle,
    label: &str,
    folder_path: &str,
) -> Result<(), String> {
    if folder_path.is_empty() {
        return Ok(());
    }

    let key = pending_external_folder_key(label);
    crate::config::commands::update_app_setting(
        app.clone(),
        SettingInput {
            key,
            value_json: serde_json::to_string(folder_path)
                .map_err(|error| format!("序列化待外部打开文件夹失败：{error}"))?,
        },
    )
}

pub(crate) fn pending_external_open_key(label: &str) -> String {
    format!("{PENDING_EXTERNAL_OPEN_PREFIX}{label}")
}

pub(crate) fn pending_external_folder_key(label: &str) -> String {
    format!("{PENDING_EXTERNAL_FOLDER_PREFIX}{label}")
}

pub(crate) fn is_document_window_label(label: &str) -> bool {
    label == "main" || (label.starts_with("window-") && label != "window-settings")
}

fn target_document_window(app: &AppHandle) -> Option<WebviewWindow> {
    for (label, window) in app.webview_windows() {
        if is_document_window_label(&label) && window.is_focused().unwrap_or(false) {
            return Some(window);
        }
    }

    app.get_webview_window("main")
}

fn is_supported_external_file(path: &Path, exe_path: Option<&Path>) -> bool {
    if let Some(exe_path) = exe_path {
        if same_path(path, exe_path) {
            return false;
        }
    }

    if !has_supported_extension(path) || !path.is_file() {
        return false;
    }

    true
}

fn is_supported_external_folder(path: &Path, exe_path: Option<&Path>) -> bool {
    if let Some(exe_path) = exe_path {
        if same_path(path, exe_path) {
            return false;
        }
    }

    path.is_dir()
}

fn has_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            matches!(extension.as_str(), "md" | "markdown" | "txt" | "json")
        })
        .unwrap_or(false)
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn normalize_path(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let path_str = canonical.to_string_lossy();
    // Windows 上 canonicalize() 会产生 \\?\ 前缀的扩展长度路径，需要去除以友好显示
    if path_str.starts_with(r"\\?\") {
        let without_prefix = &path_str[4..];
        if without_prefix.starts_with(r"UNC\") {
            // 网络路径: \\?\UNC\server\share → \\server\share
            format!(r"\\{}", &without_prefix[4..])
        } else {
            without_prefix.to_string()
        }
    } else {
        path_str.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn queues_setup_time_external_open_paths_without_duplicates() {
        queue_early_external_open(vec![
            "/tmp/first.md".to_string(),
            "/tmp/first.md".to_string(),
        ])
        .unwrap();
        queue_early_external_open(vec!["/tmp/second.md".to_string()]).unwrap();

        assert_eq!(
            take_early_external_open_paths().unwrap(),
            vec!["/tmp/first.md".to_string(), "/tmp/second.md".to_string()]
        );
        assert!(take_early_external_open_paths().unwrap().is_empty());
    }

    #[test]
    fn collects_existing_markdown_paths_from_args() {
        let dir = env::temp_dir().join(format!("nomo-external-open-{}", crate::config::now_ts()));
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("测试 文件.md");
        let ignored = dir.join("ignored.exe");
        fs::write(&md, "# ok").unwrap();
        fs::write(&ignored, "no").unwrap();

        let paths = collect_markdown_paths_from_args(
            vec![
                "Nomo.exe".to_string(),
                md.to_string_lossy().to_string(),
                ignored.to_string_lossy().to_string(),
            ],
            None,
        );

        assert_eq!(paths, vec![normalize_path(&md)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn accepts_shell_extension_activation_marker_before_paths() {
        let dir =
            env::temp_dir().join(format!("nomo-shell-activation-{}", crate::config::now_ts()));
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("shell file.md");
        fs::write(&md, "# shell").unwrap();

        let targets = collect_external_open_targets_from_args(
            vec![
                "Nomo.exe".to_string(),
                "--nomo-shell-open".to_string(),
                md.to_string_lossy().to_string(),
            ],
            None,
        );

        assert_eq!(targets.markdown_paths, vec![normalize_path(&md)]);
        assert!(targets.folder_paths.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collects_json_paths_from_args() {
        let dir = env::temp_dir().join(format!(
            "nomo-external-open-json-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).unwrap();
        let json = dir.join("large-data.json");
        fs::write(&json, "{}").unwrap();

        let paths = collect_markdown_paths_from_args(
            vec!["Nomo.exe".to_string(), json.to_string_lossy().to_string()],
            None,
        );

        assert_eq!(paths, vec![normalize_path(&json)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collects_txt_paths_from_args() {
        let dir = env::temp_dir().join(format!(
            "nomo-external-open-txt-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("large-data.txt");
        fs::write(&txt, "plain text").unwrap();

        let paths = collect_markdown_paths_from_args(
            vec!["Nomo.exe".to_string(), txt.to_string_lossy().to_string()],
            None,
        );

        assert_eq!(paths, vec![normalize_path(&txt)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_relative_args_with_cwd() {
        let dir = env::temp_dir().join(format!(
            "nomo-external-open-relative-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("demo.markdown");
        fs::write(&md, "# ok").unwrap();

        let paths = collect_markdown_paths_from_args(
            vec!["Nomo.exe".to_string(), "demo.markdown".to_string()],
            Some(dir.clone()),
        );

        assert_eq!(paths, vec![normalize_path(&md)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collects_existing_folder_paths_from_args() {
        let dir = env::temp_dir().join(format!(
            "nomo-external-open-folder-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("demo.md");
        fs::write(&md, "# ok").unwrap();

        let targets = collect_external_open_targets_from_args(
            vec![
                "Nomo.exe".to_string(),
                dir.to_string_lossy().to_string(),
                md.to_string_lossy().to_string(),
            ],
            None,
        );

        assert_eq!(targets.folder_paths, vec![normalize_path(&dir)]);
        assert_eq!(targets.markdown_paths, vec![normalize_path(&md)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collects_file_urls() {
        let dir = env::temp_dir().join(format!(
            "nomo-external-open-url-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("demo.md");
        fs::write(&md, "# ok").unwrap();
        let url = Url::from_file_path(&md).unwrap();

        let paths = collect_markdown_paths_from_urls(vec![url]);

        assert_eq!(paths, vec![normalize_path(&md)]);
        let _ = fs::remove_dir_all(dir);
    }
}

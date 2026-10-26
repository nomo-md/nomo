//! 测试专用文件 RPC。没有监听端口；仅接受独立测试应用的主窗口。
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) fn profile_root() -> Result<PathBuf, String> {
    let root =
        PathBuf::from(std::env::var_os("NOMO_AUTOMATION_DIR").ok_or("缺少 NOMO_AUTOMATION_DIR")?);
    if !root.is_absolute() || !root.is_dir() {
        return Err("测试运行目录无效".into());
    }
    Ok(root.join("profile"))
}

pub(crate) fn configure(context: &mut tauri::Context<tauri::Wry>) -> Result<(), String> {
    if context.config().identifier != "com.nomo.automation" {
        return Err("测试构建必须使用独立应用标识".into());
    }
    let root = profile_root()?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    for window in &mut context.config_mut().app.windows {
        window.data_directory = Some(root.join("webview").join(&window.label));
        let mut args = window.additional_browser_args.clone().unwrap_or_default();
        if let Ok(port) = std::env::var("NOMO_AUTOMATION_CDP_PORT") {
            let port: u16 = port.parse().map_err(|_| "无效调试端口")?;
            args.push_str(&format!(
                " --remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
            ));
        }
        window.additional_browser_args = Some(args);
    }
    Ok(())
}

/// 快照流式摘要只在原生端计算，避免大文档反复跨 IPC 传输。
#[tauri::command]
pub(crate) async fn automation_digest(
    window: tauri::WebviewWindow,
    document_id: String,
) -> Result<Value, String> {
    if window.app_handle().config().identifier != "com.nomo.automation" || window.label() != "main"
    {
        return Err("非测试主窗口".into());
    }
    let manager = window
        .state::<crate::text_document::markdown_session::MarkdownDocumentSessionManager>()
        .inner()
        .clone();
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        let snapshot = manager.capture(&owner, &document_id).map_err(|e| e.message)?;
        let result = (|| {
            let mut hash = Sha256::new();
            let mut offset = 0;
            while offset < snapshot.byte_length {
                let part = manager.read_snapshot(&owner, &snapshot.snapshot_id, offset, (offset + 65536).min(snapshot.byte_length)).map_err(|e| e.message)?;
                if part.to_byte <= offset { return Err("快照读取未前进".to_string()); }
                hash.update(part.text.as_bytes()); offset = part.to_byte;
            }
            Ok(serde_json::json!({"sha256": format!("{:x}", hash.finalize()), "byteLength": snapshot.byte_length, "revision": snapshot.revision}))
        })();
        let released = manager.release(&owner, &snapshot.snapshot_id).map_err(|e| e.message);
        match (result, released) { (Ok(value), Ok(())) => Ok(value), (Err(error), _) | (_, Err(error)) => Err(error) }
    }).await.map_err(|e| e.to_string())?
}

fn write_response(root: &Path, response: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(response).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("测试响应超过 1 MiB".into());
    }
    let name = if response["id"] == "ready" {
        "ready.json"
    } else {
        "response.json"
    };
    let pending = root.join(format!("{name}.tmp"));
    fs::write(&pending, bytes).map_err(|e| e.to_string())?;
    fs::rename(pending, root.join(name)).map_err(|e| e.to_string())
}

fn validate_open(root: &Path, request: &Value) -> Result<(), String> {
    let fixture = root
        .join("fixtures")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let target = PathBuf::from(request["params"]["path"].as_str().ok_or("缺少文件路径")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !target.starts_with(fixture) {
        return Err("只能打开本次测试的 fixture".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn automation_exchange(
    window: tauri::WebviewWindow,
    response: Option<Value>,
) -> Result<Option<Value>, String> {
    if window.label() != "main" || window.app_handle().config().identifier != "com.nomo.automation"
    {
        return Err("自动化接口仅允许独立测试应用的主窗口调用".into());
    }
    let root = PathBuf::from(std::env::var_os("NOMO_AUTOMATION_DIR").ok_or("缺少测试目录")?);
    if !root.is_absolute() || !root.is_dir() {
        return Err("测试目录必须是已存在的绝对路径".into());
    }
    if let Some(mut response) = response {
        if response["id"] == "ready" {
            response["result"]["pid"] = std::process::id().into();
            response["result"]["profile"] = profile_root()?.to_string_lossy().to_string().into();
        }
        write_response(&root, &response)?;
    }
    let path = root.join("request.json");
    let metadata = match fs::metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.len() > MAX_BYTES {
        return Err("测试请求超过 1 MiB".into());
    }
    let request: Value = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::remove_file(path).map_err(|e| e.to_string())?;
    // 文件打开限定在本次脚本生成的 fixtures 目录内，包括解析符号链接后的边界。
    if request["method"] == "open" || request["method"] == "folder" {
        if let Err(error) = validate_open(&root, &request) {
            write_response(
                &root,
                &serde_json::json!({ "id": request["id"], "ok": false, "error": error }),
            )?;
            return Ok(None);
        }
    }
    Ok(Some(request))
}

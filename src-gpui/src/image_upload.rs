//! 显式选择上传策略后才调用 PicGo；超时、取消和失败都不生成正文事务。
use super::{ImageSettings, ImportCancellation};
use std::{fs, io::Read, path::Path, process::{Command, Stdio}, time::{Duration, Instant}};

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "无法创建上传请求".into())
}
fn server_url(settings: &ImageSettings) -> Result<url::Url, String> {
    let url = url::Url::parse(settings.server.trim()).map_err(|_| "PicGo Server 地址无效")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() { return Err("PicGo Server 只支持 HTTP 或 HTTPS".into()); }
    Ok(url)
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "无法创建 PicGo 请求".into())
}
fn image_url(text: &str) -> Option<String> {
    let url = url::Url::parse(text.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.into())
}
async fn response_text(mut response: reqwest::Response) -> Result<String, String> {
    if !response.status().is_success() { return Err(format!("PicGo Server 返回 HTTP {}", response.status().as_u16())); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取 PicGo 响应失败")? {
        if bytes.len() + chunk.len() > 256 * 1024 { return Err("PicGo 响应超过 256 KiB 预算".into()); }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "PicGo 响应编码无效".into())
}
pub(super) fn upload(path: &Path, settings: &ImageSettings, cancel: &ImportCancellation) -> Result<String, String> {
    cancel.check()?;
    match settings.provider.as_str() {
        "picgo" => {
            let url = server_url(settings)?;
            let client = client()?;
            let path = path.canonicalize().map_err(|_| "上传图片路径不可用")?;
            let text = runtime()?.block_on(async {
                let request = async {
                    let response = client.post(url).json(&serde_json::json!({"list": [path.to_string_lossy()]}))
                        .send().await.map_err(|_| "连接 PicGo Server 失败或请求超时".to_string())?;
                    response_text(response).await
                };
                tokio::pin!(request);
                loop {
                    if let Ok(result) = tokio::time::timeout(Duration::from_millis(100), request.as_mut()).await { break result; }
                    cancel.check()?;
                }
            })?;
            let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| "PicGo Server 返回了无效 JSON")?;
            if value.get("success").and_then(|v| v.as_bool()) != Some(true) { return Err("PicGo Server 上传失败".into()); }
            value.get("result").and_then(|v| v.as_array()).and_then(|v| v.first()).and_then(|v| v.as_str())
                .and_then(image_url).ok_or("PicGo Server 未返回有效图片 URL".into())
        }
        "picgo-core" => {
            let mut command = core_command(&settings.command)?;
            command.arg("upload").arg(path);
            if !settings.config.trim().is_empty() { command.arg("--config").arg(settings.config.trim()); }
            let output = run_core(command, cancel)?;
            output.split_whitespace().find_map(|part| image_url(part.trim_matches(['"', '\'', ',', '[', ']'])))
                .ok_or("PicGo-Core 未返回有效图片 URL".into())
        }
        _ => Err("图片上传方式无效".into()),
    }
}
fn core_command(value: &str) -> Result<Command, String> {
    let mut parts = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for ch in value.trim().chars() {
        match ch {
            '"' => quoted = !quoted,
            ch if ch.is_whitespace() && !quoted => if !word.is_empty() { parts.push(std::mem::take(&mut word)); },
            ch => word.push(ch),
        }
    }
    if quoted { return Err("PicGo-Core 命令的引号未闭合".into()); }
    if !word.is_empty() { parts.push(word); }
    let executable = parts.first().ok_or("PicGo-Core 命令不能为空")?;
    #[cfg(target_os = "windows")]
    let executable = {
        let path = std::path::PathBuf::from(executable);
        if path.extension().is_none() {
            let bases = if path.components().count() > 1 { vec![path.clone()] } else {
                std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).map(|dir| dir.join(&path)).collect()).unwrap_or_default()
            };
            bases.iter().flat_map(|base| ["exe", "cmd", "bat"].map(|ext| base.with_extension(ext)))
                .find(|candidate| candidate.is_file()).unwrap_or(path)
        } else { path }
    };
    let mut command = Command::new(executable);
    command.args(&parts[1..]).stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    Ok(command)
}
#[cfg(target_os = "windows")]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(target_os = "windows")]
impl ProcessJob {
    fn attach(child: &mut std::process::Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() { let _ = child.kill(); let _ = child.wait(); return Err("无法创建 PicGo 进程组".into()); }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(handle, JobObjectExtendedLimitInformation, &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32) == 0 || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0 {
                let _ = child.kill(); let _ = child.wait();
                return Err("无法隔离 PicGo 进程组".into());
            }
            Ok(job)
        }
    }
}
#[cfg(target_os = "windows")]
impl Drop for ProcessJob {
    fn drop(&mut self) { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); } }
}
fn run_core(mut command: Command, cancel: &ImportCancellation) -> Result<String, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path = std::env::temp_dir().join(format!("nomo-picgo-{}-{stamp}.log", std::process::id()));
    let file = fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).map_err(|_| "无法创建 PicGo 临时输出")?;
    struct OutputFile(std::path::PathBuf);
    impl Drop for OutputFile { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
    let _cleanup = OutputFile(path.clone());
    command.stdout(Stdio::from(file.try_clone().map_err(|_| "无法连接 PicGo 输出")?));
    command.stderr(Stdio::from(file));
    let mut child = command.spawn().map_err(|_| "无法启动 PicGo-Core，请检查命令及安装路径")?;
    drop(command);
    #[cfg(target_os = "windows")]
    let job = ProcessJob::attach(&mut child)?;
    let started = Instant::now();
    loop {
        if cancel.cancelled() || started.elapsed() > Duration::from_secs(30) || fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
            let _ = child.kill(); let _ = child.wait();
            return Err(if cancel.cancelled() { "图片上传已取消" } else { "PicGo-Core 超时或输出超过预算" }.into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() { return Err(format!("PicGo-Core 上传失败，退出码：{status}")); }
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => { let _ = child.kill(); let _ = child.wait(); return Err("读取 PicGo-Core 状态失败".into()); }
        }
    }
    #[cfg(target_os = "windows")]
    drop(job);
    let mut bytes = Vec::new();
    fs::File::open(&path).map_err(|_| "无法读取 PicGo 输出")?.take(256 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "无法读取 PicGo 输出")?;
    if bytes.len() > 256 * 1024 { return Err("PicGo-Core 输出超过预算".into()); }
    Ok(String::from_utf8_lossy(&bytes).into())
}
pub(crate) fn test_connection(settings: ImageSettings, cancel: ImportCancellation) -> Result<(), String> {
    if settings.provider == "picgo-core" {
        let mut command = core_command(&settings.command)?;
        command.arg("--version");
        run_core(command, &cancel).map(|_| ())
    } else {
        let url = server_url(&settings)?;
        runtime()?.block_on(async {
            let request = client()?.get(url).send();
            tokio::pin!(request);
            let response = loop {
                cancel.check()?;
                if let Ok(result) = tokio::time::timeout(Duration::from_millis(100), request.as_mut()).await { break result; }
            };
            let status = response.map_err(|_| "无法连接 PicGo Server")?.status();
            if status.is_success() || matches!(status.as_u16(), 404 | 405) { Ok(()) }
            else { Err(format!("PicGo Server 返回 HTTP {}", status.as_u16())) }
        })
    }
}

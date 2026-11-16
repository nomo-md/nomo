//! 用户主动重置：退出后独占数据目录，再清除专属目录；普通升级不删除资料。
use std::{fs::{self, File, OpenOptions}, path::{Path, PathBuf}, time::{Duration, Instant}};

const CLEAR: &str = "--clear-app-data";
const AFTER_EXIT: &str = "--clear-app-data-after-exit";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope { All, Settings, Usage }

impl Scope {
    fn marker(self) -> &'static [u8] {
        match self {
            Self::All => b"Nomo user-confirmed reset v1\n",
            Self::Settings => b"Nomo user-confirmed reset v2: settings\n",
            Self::Usage => b"Nomo user-confirmed reset v2: usage\n",
        }
    }
    fn command(self, wait: bool) -> &'static str {
        match (self, wait) {
            (Self::All, false) => CLEAR,
            (Self::All, true) => AFTER_EXIT,
            (Self::Settings, false) => "--reset-settings",
            (Self::Settings, true) => "--reset-settings-after-exit",
            (Self::Usage, false) => "--clear-usage-data",
            (Self::Usage, true) => "--clear-usage-data-after-exit",
        }
    }
}

fn pending_scope(root: &Path) -> Result<Option<Scope>, String> {
    use std::io::Read;
    let path = sibling(root, "reset-pending");
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || is_link(&meta) => return Err("清理标记不是普通文件".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        _ => {},
    }
    let mut bytes = Vec::new();
    File::open(path).map_err(|e| e.to_string())?.take(128).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    [Scope::All, Scope::Settings, Scope::Usage].into_iter().find(|scope| scope.marker() == bytes)
        .map(Some).ok_or_else(|| "无法识别清理范围，未清除任何资料".into())
}

fn root() -> Result<PathBuf, String> {
    let path = crate::settings::config_path().ok_or("无法确定应用数据目录")?;
    let root = path.parent().ok_or("应用数据目录无效")?;
    let root = if root.is_absolute() { root.to_owned() } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(root)
    };
    let parent = root.parent().ok_or("应用数据目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    Ok(parent.canonicalize().map_err(|e| e.to_string())?.join(root.file_name().ok_or("应用数据目录无效")?))
}

fn sibling(root: &Path, suffix: &str) -> PathBuf {
    root.with_file_name(format!(".{}-{suffix}", root.file_name().unwrap().to_string_lossy()))
}

fn lease(root: &Path) -> Result<File, String> {
    let path = sibling(root, "data.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_file() || is_link(&meta) { return Err("数据锁不是普通文件，未执行清理".into()); }
    }
    OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(path).map_err(|e| e.to_string())
}

/// 锁位于数据目录外，删除数据时不能删除锁本身，否则其他进程可能绕过独占锁。
pub(crate) fn start() -> Result<File, String> {
    let root = root()?;
    if let Some(scope) = pending_scope(&root)? { clear(false, scope)?; }
    let lease = lease(&root)?;
    lease.try_lock_shared().map_err(|_| "Nomo 正在清除应用数据，请稍后重新打开。".to_owned())?;
    // 与退出后清理进程竞争时不能加载旧数据。
    if sibling(&root, "reset-pending").exists() {
        return Err("Nomo 正在退出并清除应用数据，请稍后重新打开。".into());
    }
    Ok(lease)
}

fn targets(root: &Path, scope: Scope) -> Result<Vec<PathBuf>, String> {
    // NOMO_DATA_DIR 可以指向任意目录，不能将任意自定义目录当作应用专属目录递归删除。
    if root.file_name() != Some(std::ffi::OsStr::new("com.nomo.desktop")) {
        return Err("完全清除仅支持名为 com.nomo.desktop 的专属数据目录。自定义数据目录请自行备份和管理。".into());
    }
    let mut targets = vec![root.to_owned()];
    if scope != Scope::Settings && std::env::var_os("NOMO_DATA_DIR").is_none() {
        #[cfg(windows)]
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            targets.push(PathBuf::from(local).join("com.nomo.desktop"));
        }
        #[cfg(target_os = "macos")]
        if let Some(home) = std::env::var_os("HOME") {
            targets.push(PathBuf::from(home).join("Library/Caches/com.nomo.desktop"));
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))) {
            targets.push(cache.join("com.nomo.desktop"));
        }
    }
    let executable = std::env::current_exe().and_then(|path| path.canonicalize()).map_err(|e| e.to_string())?;
    for target in &targets {
        if !target.is_absolute() { return Err("应用数据目录必须是绝对路径".into()); }
        match fs::symlink_metadata(target) {
            Ok(meta) => {
                if !meta.is_dir() || is_link(&meta) { return Err(format!("拒绝清除非普通目录：{}", target.display())); }
                let canonical = target.canonicalize().map_err(|e| e.to_string())?;
                let expected = target.parent().unwrap().canonicalize().map_err(|e| e.to_string())?.join("com.nomo.desktop");
                if canonical != expected || executable.starts_with(&canonical) {
                    return Err(format!("应用数据目录身份不安全，未清除：{}", target.display()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
    }
    targets.dedup();
    Ok(targets)
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { meta.is_symlink() }
}

pub(crate) fn validate(scope: Scope) -> Result<(), String> {
    let root = root()?;
    targets(&root, scope)?;
    separated_config(&root, scope).map(|_| ())
}

pub(crate) fn launch_after_close(scope: Scope) -> Result<(), String> {
    let root = root()?;
    targets(&root, scope)?;
    // 已知无法分离的配置在退出前报告，不留下阻止正常启动的清理标记。
    separated_config(&root, scope)?;
    if pending_scope(&root)?.is_some() { return Err("上次清理尚未完成，请重新打开 Nomo 后重试。".into()); }
    let pending = sibling(&root, "reset-pending");
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::file_system::write_file_atomically(&pending, scope.marker())?;
    let mut command = std::process::Command::new(executable);
    command.arg(scope.command(true));
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    if let Err(error) = command.spawn() {
        let _ = fs::remove_file(pending);
        return Err(error.to_string());
    }
    Ok(())
}

/// 在平台、配置、恢复目录及单实例监听器初始化之前处理，清理进程不创建应用窗口。
pub(crate) fn run_command() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (scope, wait) = [Scope::All, Scope::Settings, Scope::Usage].into_iter()
        .flat_map(|scope| [false, true].map(|wait| (scope, wait)))
        .find(|(scope, wait)| args.first().is_some_and(|arg| arg == scope.command(*wait)))?;
    let result = if args.len() == 1 { clear(wait, scope) } else { Err("清理命令不接受目录或额外参数".into()) };
    match result {
        Ok(()) => Some(0),
        Err(error) => { show_error(&format!("清理未完成：{error}")); Some(1) }
    }
}

fn clear(wait: bool, scope: Scope) -> Result<(), String> {
    let root = root()?;
    let lease = lease(&root)?;
    let deadline = Instant::now() + Duration::from_secs(if wait { 60 } else { 0 });
    loop {
        match lease.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(format!("请先退出所有 Nomo 窗口和托盘进程，再重试。{error}")),
        }
    }
    #[cfg(windows)]
    if std::env::var_os("NOMO_DATA_DIR").is_none() {
        // 旧 Web 版共用默认目录，尚无原生锁；显式隔离目录则由数据锁和会话锁保护。
        loop {
            match ensure_no_other_process() {
                Ok(()) => break,
                Err(_) if wait && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => return Err(error),
            }
        }
    }
    if pending_scope(&root)?.is_some_and(|pending| pending != scope) {
        return Err("上次清理尚未完成，不能更改清理范围。".into());
    }
    let targets = targets(&root, scope)?;
    // 先读取并校验混合配置，不能为了清除记录而误删偏好，或反过来丢失草稿。
    let config = separated_config(&root, scope)?;
    // 兼容尚未使用数据目录共享锁的原生版本；其会话锁在所有平台都有效。
    let _sessions = lock_sessions(&root)?;
    let pending = sibling(&root, "reset-pending");
    crate::file_system::write_file_atomically(&pending, scope.marker())?;
    if let Some(config) = config {
        crate::file_system::write_file_atomically(&root.join("config.json"), &config)?;
    }
    for target in targets {
        let result = if scope == Scope::Settings {
            fs::remove_file(target.join("gpui-window-placement-v1.json"))
        } else if scope == Scope::Usage && target == root {
            clear_usage_files(&target)
        } else {
            fs::remove_dir_all(&target)
        };
        match result {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("{}：{e}。下次启动将重试清理。", target.display())),
        }
    }
    fs::remove_file(pending).map_err(|e| e.to_string())?;
    Ok(())
}

fn usage_setting(key: &str) -> bool {
    matches!(key, "workspaceTabs" | "readingPositions" | "nativeReleaseNotesVersion" | "softwareUpdateDismissedVersion")
        || ["workspaceTabs:", "pendingFolder:", "pendingExternalOpen:"].iter().any(|prefix| key.starts_with(prefix))
}

fn separated_config(root: &Path, scope: Scope) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    if scope == Scope::All { return Ok(None); }
    let path = root.join("config.json");
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || is_link(&meta) => return Err("配置不是普通文件，未清理".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        _ => {},
    }
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path).map_err(|e| e.to_string())?.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT { return Err("配置超过 16 MiB，无法安全分离设置与使用数据。".into()); }
    let mut config: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("配置无法读取，未清理：{e}"))?;
    let object = config.as_object_mut().ok_or("配置不是对象，未清理")?;
    // 旧版 app.settings 混有窗口和工作区记录，其余键属于偏好（含未知旧偏好）。
    if let Some(app) = object.get_mut("app") {
        let app = app.as_object_mut().ok_or("配置 app 不是对象，未清理")?;
        if let Some(settings) = app.get_mut("settings") {
            let settings = settings.as_object_mut().ok_or("配置 app.settings 不是对象，未清理")?;
            settings.retain(|key, _| usage_setting(key) == (scope == Scope::Settings));
        }
    }
    if scope == Scope::Settings {
        object.remove("editor");
    } else {
        for key in ["recent", "workspace", "snapshots"] { object.remove(key); }
    }
    if let Some(window) = object.get_mut("window") {
        let window = window.as_object_mut().ok_or("配置 window 不是对象，未清理")?;
        if let Some(settings) = window.get_mut("settings") {
            let settings = settings.as_object_mut().ok_or("配置 window.settings 不是对象，未清理")?;
            settings.retain(|key, _| usage_setting(key) == (scope == Scope::Settings));
        }
    }
    serde_json::to_vec(&config).map(Some).map_err(|e| e.to_string())
}

fn clear_usage_files(root: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if matches!(entry.file_name().to_str(), Some("config.json" | "themes" | "gpui-window-placement-v1.json")) { continue; }
        let meta = fs::symlink_metadata(entry.path())?;
        // remove_dir_all 不跟随目录链接；保留的设置和主题不会被移走再写回。
        if meta.is_dir() { fs::remove_dir_all(entry.path())?; }
        else { fs::remove_file(entry.path())?; }
    }
    Ok(())
}

fn lock_sessions(root: &Path) -> Result<Vec<File>, String> {
    let directory = root.join("gpui-sessions");
    if let Ok(meta) = fs::symlink_metadata(&directory) {
        if is_link(&meta) { return Err("会话目录不是普通目录，未执行清理".into()); }
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut leases = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= 4096 { return Err("会话目录过多，请先备份并手动检查".into()); }
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if is_link(&meta) { return Err("会话目录包含链接，未执行清理".into()); }
        if !meta.is_dir() { continue; }
        let path = entry.path().join(".active");
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || is_link(&meta) => return Err("会话锁不是普通文件，未执行清理".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
            _ => {},
        }
        let file = OpenOptions::new().read(true).write(true).open(path).map_err(|e| e.to_string())?;
        file.try_lock().map_err(|_| "仍有 Nomo 会话正在使用数据，请退出所有版本后重试。".to_owned())?;
        leases.push(file);
    }
    Ok(leases)
}

#[cfg(windows)]
fn ensure_no_other_process() -> Result<(), String> {
    use windows_sys::Win32::{Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE}, System::Diagnostics::ToolHelp::*};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error().to_string()); }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry);
        while more != 0 {
            let end = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            if entry.th32ProcessID != std::process::id() && String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case("nomo.exe") {
                found = true; break;
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        let error = GetLastError();
        CloseHandle(snapshot);
        if found { return Err("请先退出其他版本或其他安装目录中的 Nomo（包括托盘），再重试。".into()); }
        if error != ERROR_NO_MORE_FILES { return Err(std::io::Error::from_raw_os_error(error as i32).to_string()); }
    }
    Ok(())
}

pub(crate) fn show_error(error: &str) {
    #[cfg(windows)]
    crate::open_instance::show_error(error);
    #[cfg(not(windows))]
    eprintln!("{error}");
}

use chrono::Local;
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

const MAX_LOG_FILE_BYTES: u64 = 5 * 1024 * 1024;

struct LoggerState {
    enabled: bool,
}

static LOGGER_STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();
static APP_IDENTIFIER: OnceLock<String> = OnceLock::new();

pub(crate) fn init(identifier: &str) {
    // 启动阶段根据配置提前开启日志，确保后续渲染模式等关键启动日志能落盘
    let _ = APP_IDENTIFIER.set(identifier.to_string());
    set_enabled(crate::config::is_developer_mode(identifier));
    info("App", "日志系统初始化");
}

#[tauri::command]
pub(crate) fn set_logger_enabled(enabled: bool) {
    set_enabled(enabled);
}

#[tauri::command]
pub(crate) fn get_logger_enabled() -> bool {
    is_enabled()
}

#[tauri::command]
pub(crate) fn log_message(level: String, tag: String, message: String) {
    write(&level, &tag, &message);
}

pub(crate) fn set_enabled(enabled: bool) {
    if let Ok(mut state) = logger_state().lock() {
        state.enabled = enabled;
    }
    if enabled {
        write_force("INFO", "Logger", "日志输出已开启");
    } else {
        // write_force("INFO", "Logger", "日志输出已关闭");
    }
}

pub(crate) fn is_enabled() -> bool {
    logger_state()
        .lock()
        .map(|state| state.enabled)
        .unwrap_or(true)
}

pub(crate) fn debug(tag: &str, message: &str) {
    write("DEBUG", tag, message);
}

pub(crate) fn info(tag: &str, message: &str) {
    write("INFO", tag, message);
}

pub(crate) fn warn(tag: &str, message: &str) {
    write("WARN", tag, message);
}

pub(crate) fn error(tag: &str, message: &str) {
    write("ERROR", tag, message);
}

pub(crate) fn perf(tag: &str, operation: &str, elapsed: std::time::Duration) {
    write(
        "PERF",
        tag,
        &format!("{operation} 耗时 {:.2}ms", elapsed.as_secs_f64() * 1000.0),
    );
}

fn write(level: &str, tag: &str, message: &str) {
    if !is_enabled() {
        return;
    }
    write_force(level, tag, message);
}

fn write_force(level: &str, tag: &str, message: &str) {
    let line = format_log_line(level, tag, message);
    eprintln!("{line}");
    if let Err(error) = append_to_log_file(&line) {
        eprintln!(
            "{}",
            format_log_line("WARN", "Logger", &format!("写入日志文件失败：{error}"))
        );
    }
}

fn logger_state() -> &'static Mutex<LoggerState> {
    LOGGER_STATE.get_or_init(|| Mutex::new(LoggerState { enabled: false }))
}

fn format_log_line(level: &str, tag: &str, message: &str) -> String {
    #[cfg(target_os = "android")]
    let message = redact_android_message(message);
    format!(
        "[{}][{}][{}] {}",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        level.to_uppercase(),
        tag,
        message
    )
}

#[cfg(any(target_os = "android", test))]
fn redact_android_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    let first_private = [
        "https://",
        "http://",
        "content://",
        "file://",
        "data:",
        "mailto:",
        "/data/",
        "/storage/",
        "/sdcard/",
        "/mnt/",
    ]
    .iter()
    .filter_map(|marker| lower.find(marker))
    .min();
    match first_private {
        Some(index) => format!("{}[redacted]", &message[..index]),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod mobile_privacy_tests {
    use super::redact_android_message;

    #[test]
    fn mobile_logs_redact_shared_urls_and_paths_but_keep_safe_diagnostics() {
        for private in [
            "https://example.com/?token=secret",
            "content://provider/private.md",
            "/data/user/0/app/files/private name.md",
            "/storage/emulated/0/private name.md",
            "data:text/plain,shared%20body",
        ] {
            assert_eq!(
                redact_android_message(&format!("打开文档：{private}")),
                "打开文档：[redacted]"
            );
        }
        assert_eq!(
            redact_android_message("导入失败，请检查读取权限"),
            "导入失败，请检查读取权限"
        );
    }
}

fn append_to_log_file(line: &str) -> Result<(), String> {
    let log_path = current_log_path()?;
    rotate_log_file_if_needed(&log_path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("打开日志文件失败：{error}"))?;
    writeln!(file, "{line}").map_err(|error| format!("追加日志失败：{error}"))
}

fn current_log_path() -> Result<PathBuf, String> {
    let current_dir =
        std::env::current_dir().map_err(|error| format!("定位当前目录失败：{error}"))?;

    // dev 模式下当前目录为 src-tauri/，日志放到项目根目录避免触发 Cargo 重建
    let logs_dir = if current_dir.file_name() == Some(OsStr::new("src-tauri")) {
        current_dir.join("../logs")
    } else {
        // 生产环境：日志应输出到应用数据目录（安装目录），而非运行目录
        let identifier = APP_IDENTIFIER
            .get()
            .ok_or_else(|| "应用标识符未初始化".to_string())?;
        let app_data_dir = crate::config::resolve_app_data_dir(identifier)
            .ok_or_else(|| "无法定位应用数据目录".to_string())?;
        app_data_dir.join("logs")
    };

    fs::create_dir_all(&logs_dir).map_err(|error| format!("创建日志目录失败：{error}"))?;
    Ok(logs_dir.join(format!("{}.log", Local::now().format("%Y-%m-%d"))))
}

fn rotate_log_file_if_needed(log_path: &PathBuf) -> Result<(), String> {
    let Ok(metadata) = fs::metadata(log_path) else {
        return Ok(());
    };
    if metadata.len() < MAX_LOG_FILE_BYTES {
        return Ok(());
    }

    let rotated_path = log_path.with_file_name(format!(
        "{}-{}.log",
        Local::now().format("%Y-%m-%d"),
        Local::now().format("%H%M%S")
    ));
    fs::rename(log_path, rotated_path).map_err(|error| format!("轮转日志文件失败：{error}"))
}

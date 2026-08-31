use std::{
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
};

use jni::{
    objects::{JObject, JString, JValue},
    JNIEnv,
};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Url};
use tauri_plugin_dialog::DialogExt;

const OPEN_DOCUMENT_EVENT: &str = "nomo://open-document";
const TARGET_WINDOW_LABEL: &str = "main";
const PENDING_EXTERNAL_OPEN_KEY: &str = "pendingExternalOpen:main";
const COPY_BUFFER_BYTES: usize = 64 * 1024;

static PENDING_URLS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static IMPORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize)]
struct ExternalOpenPayload {
    #[serde(rename = "windowLabel")]
    window_label: String,
    paths: Vec<String>,
}

/// 挂在 `App::run` 回调上：消费框架转出的 `RunEvent::Opened`（微信等外部应用
/// 通过 ACTION_VIEW / ACTION_SEND 打开文件时由 tao/wry 自动触发），并把
/// content:// 等 URI 导入为本地文件后转发给前端。
pub(crate) fn handle_run_event(app: &AppHandle, event: &RunEvent) {
    match event {
        RunEvent::Opened { urls } => {
            let url_strings: Vec<String> =
                urls.iter().map(|url| url.as_str().to_string()).collect();
            crate::app_logger::info(
                "ExternalOpen",
                &format!("收到 Android 打开事件：count={}", url_strings.len()),
            );
            if app.try_state::<crate::config::ConfigManager>().is_none() {
                queue_pending(url_strings);
                crate::app_logger::info("ExternalOpen", "setup 尚未完成，暂存待处理 URL");
            } else {
                dispatch_urls(app, url_strings);
            }
        }
        _ => flush_pending(app),
    }
}

fn queue_pending(urls: Vec<String>) {
    if let Ok(mut pending) = PENDING_URLS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        for url in urls {
            if !pending.contains(&url) {
                pending.push(url);
            }
        }
    }
}

fn take_pending() -> Vec<String> {
    PENDING_URLS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default()
}

fn flush_pending(app: &AppHandle) {
    let has_pending = PENDING_URLS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|pending| !pending.is_empty())
        .unwrap_or(false);
    if !has_pending || app.try_state::<crate::config::ConfigManager>().is_none() {
        return;
    }
    dispatch_urls(app, take_pending());
}

fn dispatch_urls(app: &AppHandle, urls: Vec<String>) {
    let mut deferred: Vec<Url> = Vec::new();

    for url_string in &urls {
        match Url::parse(url_string) {
            Ok(url) => match url.scheme() {
                "file" => match url.to_file_path() {
                    Ok(path) => {
                        if has_supported_path_extension(&path) {
                            deferred.push(url);
                        } else {
                            crate::app_logger::info("ExternalOpen", "忽略不支持的文件类型");
                            report_import_error(app);
                        }
                    }
                    Err(_) => {
                        crate::app_logger::warn("ExternalOpen", "file:// URL 无法转换为本地路径");
                        report_import_error(app);
                    }
                },
                "content" | "data" | "http" | "https" | "mailto" => deferred.push(url),
                _ => {
                    crate::app_logger::info("ExternalOpen", "不支持的打开请求");
                    report_import_error(app);
                }
            },
            Err(_) => {
                crate::app_logger::warn("ExternalOpen", "无法解析打开请求");
                report_import_error(app);
            }
        }
    }

    if deferred.is_empty() {
        return;
    }

    // 仅在 WebView 线程取得 JVM/Activity 的全局引用；慢 provider 的读取在附加的后台线程执行。
    let Some(window) = app.get_webview_window(TARGET_WINDOW_LABEL) else {
        queue_pending(deferred.iter().map(|url| url.to_string()).collect());
        return;
    };
    let deferred_strings: Vec<String> = deferred.iter().map(|url| url.to_string()).collect();
    let app_handle = app.clone();
    let Ok(incoming_dir) = app
        .path()
        .app_data_dir()
        .map(|root| root.join(crate::mobile_imports::INCOMING_DIR))
    else {
        report_import_error(app);
        return;
    };
    let import_result = window.as_ref().with_webview(move |platform_webview| {
        platform_webview.jni_handle().exec(move |env, activity, _| {
            let runtime = env
                .get_java_vm()
                .and_then(|vm| env.new_global_ref(activity).map(|activity| (vm, activity)));
            let Ok((vm, activity)) = runtime else {
                report_import_error(&app_handle);
                return;
            };
            tauri::async_runtime::spawn_blocking(move || {
                let Ok(_serial) = IMPORT_LOCK.lock() else {
                    report_import_error(&app_handle);
                    return;
                };
                let Ok(mut env) = vm.attach_current_thread() else {
                    report_import_error(&app_handle);
                    return;
                };
                let mut imported: Vec<String> = Vec::new();
                for url in deferred {
                    match with_exception_guard(&mut env, |env| {
                        import_url(env, activity.as_obj(), &url, &incoming_dir)
                    }) {
                        Ok(Some(path)) => imported.push(path),
                        Ok(None) => {
                            crate::app_logger::info("ExternalOpen", "跳过不支持的导入内容");
                            report_import_error(&app_handle);
                        }
                        Err(_error) => {
                            crate::app_logger::error(
                                "ExternalOpen",
                                "导入失败，请检查内容格式、读取权限及剩余空间",
                            );
                            report_import_error(&app_handle);
                        }
                    }
                }
                if !imported.is_empty() {
                    route_paths(&app_handle, imported);
                }
            });
        })
    });
    if let Err(error) = import_result {
        crate::app_logger::error(
            "ExternalOpen",
            &format!("JNI 任务投递失败，回退等待重试：{error}"),
        );
        queue_pending(deferred_strings);
    }
}

fn import_url(
    env: &mut JNIEnv,
    activity: &JObject,
    url: &Url,
    incoming_dir: &Path,
) -> Result<Option<String>, String> {
    match url.scheme() {
        "file" => {
            let source = url.to_file_path().map_err(|_| "文件路径无效")?;
            crate::mobile_imports::copy_file(incoming_dir, &source)
                .map(|path| Some(path.to_string_lossy().into_owned()))
                .map_err(|_| "文件导入失败".into())
        }
        "content" => {
            let resolver = env
                .call_method(
                    activity,
                    "getContentResolver",
                    "()Landroid/content/ContentResolver;",
                    &[],
                )
                .and_then(|value| value.l())
                .map_err(|error| format!("获取 ContentResolver 失败：{error}"))?;
            import_content_uri(env, resolver, url.as_str(), &incoming_dir)
        }
        "data" => import_data_url(url, &incoming_dir).map(Some),
        "http" | "https" | "mailto" => {
            write_shared_text(url, url.as_str(), &incoming_dir).map(Some)
        }
        _ => Ok(None),
    }
}

fn import_content_uri(
    env: &mut JNIEnv,
    resolver: JObject,
    uri_string: &str,
    incoming_dir: &Path,
) -> Result<Option<String>, String> {
    let null_object = JObject::null();
    let uri_jstring = env
        .new_string(uri_string)
        .map_err(|error| format!("创建 URI 字符串失败：{error}"))?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri_jstring)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("解析 URI 失败：{error}"))?;
    if uri.is_null() {
        return Ok(None);
    }

    let display_name = with_exception_guard(env, |env| {
        Ok(query_display_name(env, &resolver, &uri, &null_object))
    })
    .ok()
    .flatten()
    .or_else(|| last_path_segment(env, &uri));
    let Some(display_name) = display_name else {
        crate::app_logger::info("ExternalOpen", "URI 缺少文件名");
        return Ok(None);
    };
    if !has_supported_name_extension(&display_name) {
        crate::app_logger::info("ExternalOpen", "忽略不支持的文件类型");
        return Ok(None);
    }

    let mut staged = crate::mobile_imports::StagedImport::create(incoming_dir, &display_name)
        .map_err(|_| "无法创建导入临时文件".to_string())?;
    let input_stream = env
        .call_method(
            &resolver,
            "openInputStream",
            "(Landroid/net/Uri;)Ljava/io/InputStream;",
            &[JValue::Object(&uri)],
        )
        .and_then(|value| value.l())
        .map_err(|error| format!("打开输入流失败（可能没有读取权限）：{error}"))?;
    if input_stream.is_null() {
        return Err("openInputStream 返回空".to_string());
    }

    let copy_result = copy_stream(env, &input_stream, &mut staged.file);

    // 无论拷贝成败都关闭流，失败时清理半成品文件
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    let _ = env.call_method(&input_stream, "close", "()V", &[]);
    copy_result?;
    let target_path = staged
        .publish()
        .map_err(|_| "导入文件发布失败".to_string())?;

    crate::app_logger::info("ExternalOpen", "文件导入成功");
    Ok(Some(target_path.to_string_lossy().into_owned()))
}

fn import_data_url(url: &Url, incoming_dir: &Path) -> Result<String, String> {
    // data:[<mediatype>][;base64],<data>，此处只处理微信分享文本场景
    if url.path().contains(";base64,") {
        return Err("暂不支持 base64 data URL".to_string());
    }
    let (media, encoded) = url
        .path()
        .split_once(',')
        .ok_or_else(|| "data URL 缺少数据段".to_string())?;
    if media.split(';').next() != Some("text/plain") {
        return Err("分享文本类型不受支持".to_string());
    }
    let decoded = percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| "分享文本不是有效的 UTF-8".to_string())?;
    if decoded.trim().is_empty() {
        return Err("分享的文本内容为空".to_string());
    }

    write_shared_text(url, &decoded, incoming_dir)
}

fn write_shared_text(url: &Url, text: &str, incoming_dir: &Path) -> Result<String, String> {
    let target_path = crate::mobile_imports::write_shared(incoming_dir, url.as_str(), text)
        .map_err(|_| "写入分享文本失败".to_string())?;
    crate::app_logger::info("ExternalOpen", "分享文本导入成功");
    Ok(target_path.to_string_lossy().into_owned())
}

fn query_display_name(
    env: &mut JNIEnv,
    resolver: &JObject,
    uri: &JObject,
    null_object: &JObject,
) -> Option<String> {
    let projection_key = env.new_string("_display_name").ok()?;
    let projection = env
        .new_object_array(1, "java/lang/String", projection_key)
        .ok()?;
    let cursor = env
        .call_method(
            resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::Object(uri),
                JValue::Object(&projection),
                JValue::Object(null_object),
                JValue::Object(null_object),
                JValue::Object(null_object),
            ],
        )
        .and_then(|value| value.l())
        .ok()?;
    if cursor.is_null() {
        return None;
    }
    let moved = env
        .call_method(&cursor, "moveToFirst", "()Z", &[])
        .and_then(|value| value.z())
        .unwrap_or(false);
    let name = if moved {
        env.call_method(
            &cursor,
            "getString",
            "(I)Ljava/lang/String;",
            &[JValue::Int(0)],
        )
        .and_then(|value| value.l())
        .ok()
        .filter(|value| !value.is_null())
        .and_then(|value| jstring_to_string(env, value))
    } else {
        None
    };
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    let _ = env.call_method(&cursor, "close", "()V", &[]);
    name.filter(|name| !name.trim().is_empty())
}

fn last_path_segment(env: &mut JNIEnv, uri: &JObject) -> Option<String> {
    env.call_method(uri, "getLastPathSegment", "()Ljava/lang/String;", &[])
        .and_then(|value| value.l())
        .ok()
        .filter(|value| !value.is_null())
        .and_then(|value| jstring_to_string(env, value))
        .filter(|segment| !segment.trim().is_empty())
}

fn copy_stream(
    env: &mut JNIEnv,
    input: &JObject,
    output: &mut std::fs::File,
) -> Result<(), String> {
    let buffer = env
        .new_byte_array(COPY_BUFFER_BYTES as jni::sys::jsize)
        .map_err(|_| "分配读取缓冲失败")?;
    loop {
        let read = env
            .call_method(input, "read", "([B)I", &[JValue::Object(&buffer)])
            .and_then(|value| value.i())
            .map_err(|_| "读取分享文件失败")?;
        if read < 0 {
            break;
        }
        if read == 0 || read as usize > COPY_BUFFER_BYTES {
            return Err("分享文件返回无效读取长度".into());
        }
        let bytes = env
            .convert_byte_array(&buffer)
            .map_err(|_| "读取缓冲转换失败")?;
        output
            .write_all(&bytes[..read as usize])
            .map_err(|_| "写入导入文件失败")?;
    }
    Ok(())
}

// 执行 JNI 任务后清理可能挂起的 Java 异常，避免污染同一线程上的后续调用
fn with_exception_guard<T>(
    env: &mut JNIEnv,
    task: impl FnOnce(&mut JNIEnv) -> Result<T, String>,
) -> Result<T, String> {
    let result = task(env);
    if env.exception_check().unwrap_or(false) {
        // Java exception messages may contain content URIs or shared text.
        let _ = env.exception_clear();
    }
    result
}

fn report_import_error(app: &AppHandle) {
    app.dialog()
        .message("无法导入文档。请确认内容非空、格式受支持，且 Nomo 有读取权限和足够存储空间。")
        .title("Nomo")
        .show(|_| {});
}

fn jstring_to_string(env: &mut JNIEnv, value: JObject) -> Option<String> {
    let jstring = JString::from(value);
    let text = env.get_string(&jstring).ok()?;
    Some(text.to_string_lossy().into_owned())
}

fn has_supported_path_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(is_supported_extension)
        .unwrap_or(false)
}

fn has_supported_name_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(is_supported_extension)
        .unwrap_or(false)
}

fn is_supported_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "markdown" | "txt" | "json"
    )
}

/// 把已就绪的本地路径交给前端：先持久化到设置（冷启动时前端尚未开始监听事件，
/// 由启动恢复逻辑兜底消费），再广播打开事件并尝试唤起主窗口。
fn route_paths(app: &AppHandle, mut paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    paths.sort();
    paths.dedup();

    let persist_result = serde_json::to_string(&paths)
        .map_err(|error| error.to_string())
        .and_then(|value_json| {
            crate::config::commands::update_app_setting(
                app.clone(),
                crate::models::SettingInput {
                    key: PENDING_EXTERNAL_OPEN_KEY.to_string(),
                    value_json,
                },
            )
        });
    if persist_result.is_err() {
        crate::app_logger::error("ExternalOpen", "持久化待打开记录失败");
    }

    if let Some(window) = app.get_webview_window(TARGET_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Err(error) = app.emit(
        OPEN_DOCUMENT_EVENT,
        ExternalOpenPayload {
            window_label: TARGET_WINDOW_LABEL.to_string(),
            paths,
        },
    ) {
        crate::app_logger::error("ExternalOpen", &format!("发送打开文档事件失败：{error}"));
    }
}

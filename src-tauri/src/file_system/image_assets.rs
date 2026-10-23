use crate::models::{
    DesktopActionPayload, ImageAssetInput, ImageAssetPayload, ImageDeleteInput, ImageDeletePayload,
    ImageResolveInput, ImageResolvePayload, ImageUploadPayload, PicgoConnectionTestInput,
    PicgoCoreUploadInput, PicgoServerUploadInput,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Command,
};

#[tauri::command]
pub(crate) fn import_image_asset(input: ImageAssetInput) -> Result<ImageAssetPayload, String> {
    let document_path = Path::new(&input.document_path);
    let document_dir = document_path
        .parent()
        .ok_or_else(|| "请先保存 Markdown 文件，再插入本地图片".to_string())?;
    if input.bytes.is_empty() {
        return Err("图片内容为空，无法导入".to_string());
    }

    let document_stem = document_stem(&input.document_file_name, document_path);
    let target = build_image_asset_target(
        document_dir,
        &document_stem,
        &input.strategy,
        &input.file_name,
    )?;
    if !target.directory.exists() {
        fs::create_dir_all(&target.directory)
            .map_err(|error| format!("创建图片资源目录失败：{error}"))?;
    }

    if let Some(existing_file_name) =
        find_existing_image_with_same_fingerprint(&target.directory, &input.bytes)
    {
        let absolute_path = target.directory.join(&existing_file_name);
        return Ok(ImageAssetPayload {
            markdown_src: format!("{}{}", target.markdown_prefix, existing_file_name),
            absolute_path: absolute_path.to_string_lossy().to_string(),
            reused: true,
        });
    }

    fs::write(&target.absolute_path, &input.bytes)
        .map_err(|error| format!("写入图片资源失败：{error}"))?;

    Ok(ImageAssetPayload {
        markdown_src: target.markdown_src,
        absolute_path: target.absolute_path.to_string_lossy().to_string(),
        reused: false,
    })
}

#[tauri::command]
pub(crate) fn resolve_image_asset(input: ImageResolveInput) -> ImageResolvePayload {
    let src = input.src.trim().to_string();
    if is_remote_image_src(&src) {
        return ImageResolvePayload {
            src: src.clone(),
            display_src: src,
            exists: true,
            absolute_path: None,
            error: None,
        };
    }

    let local_src = decode_percent_encoded_path(&src);
    let absolute_path = if Path::new(&local_src).is_absolute() {
        PathBuf::from(&local_src)
    } else if let Some(document_path) = input.document_path {
        let parent = Path::new(&document_path).parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            parent.join(
                local_src
                    .trim_start_matches("./")
                    .replace('/', std::path::MAIN_SEPARATOR_STR),
            )
        } else {
            PathBuf::from(&local_src)
        }
    } else {
        PathBuf::from(&local_src)
    };

    // Tauri 的 asset 协议拒绝含 `..` 的路径；先解析为真实绝对路径，保留 Markdown 原始引用。
    let absolute_path = absolute_path.canonicalize().unwrap_or(absolute_path);
    let exists = absolute_path.is_file();
    ImageResolvePayload {
        src,
        display_src: absolute_path.to_string_lossy().to_string(),
        exists,
        absolute_path: Some(absolute_path.to_string_lossy().to_string()),
        error: if exists {
            None
        } else {
            Some("图片文件不存在".to_string())
        },
    }
}

#[tauri::command]
pub(crate) fn delete_image_asset(input: ImageDeleteInput) -> ImageDeletePayload {
    let src = input.src.trim().to_string();
    let local_src = decode_percent_encoded_path(&src);
    if src.is_empty() || is_remote_image_src(&src) || Path::new(&local_src).is_absolute() {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: true,
            error: None,
        };
    }

    let Some(document_path) = input.document_path.as_deref() else {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: true,
            error: Some("缺少文档路径，无法删除图片文件".to_string()),
        };
    };

    let Some(document_dir) = Path::new(document_path).parent() else {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: true,
            error: Some("缺少文档目录，无法删除图片文件".to_string()),
        };
    };

    let absolute_path = document_dir.join(
        local_src
            .trim_start_matches("./")
            .replace('/', std::path::MAIN_SEPARATOR_STR),
    );

    if !is_supported_image_file_path(&absolute_path) {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: true,
            error: None,
        };
    }

    if !absolute_path.exists() {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: false,
            error: None,
        };
    }

    if let Err(error) = ensure_path_stays_in_document_dir(document_dir, &absolute_path) {
        return ImageDeletePayload {
            src,
            removed: false,
            skipped: true,
            error: Some(error),
        };
    }

    match fs::remove_file(&absolute_path) {
        Ok(()) => ImageDeletePayload {
            src,
            removed: true,
            skipped: false,
            error: None,
        },
        Err(error) => ImageDeletePayload {
            src,
            removed: false,
            skipped: false,
            error: Some(format!("删除图片文件失败：{error}")),
        },
    }
}

#[tauri::command]
pub(crate) fn upload_image_via_picgo_core(
    input: PicgoCoreUploadInput,
) -> Result<ImageUploadPayload, String> {
    if input.bytes.is_empty() {
        return Err("图片内容为空，无法上传".to_string());
    }

    let temp_path = write_temp_image_file(&input.file_name, &input.bytes)?;
    let command = input.command.trim();
    if command.is_empty() {
        cleanup_temp_file(&temp_path);
        return Err("PicGo-Core 命令不能为空".to_string());
    }

    let mut process = create_picgo_core_command(command);
    process
        .arg("upload")
        .arg(temp_path.to_string_lossy().to_string());
    if let Some(config_path) = input
        .config_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        process.arg("--config").arg(config_path);
    }

    let output = process
        .output()
        .map_err(|error| format!("调用 PicGo-Core 失败：{error}"));
    cleanup_temp_file(&temp_path);
    let output = output?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("PicGo-Core 上传失败，退出码：{}", output.status)
        } else {
            format!("PicGo-Core 上传失败：{stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let url = extract_first_url(&format!("{stdout}\n{stderr}"))
        .ok_or_else(|| "PicGo-Core 上传成功但未返回图片 URL".to_string())?;
    Ok(ImageUploadPayload { url })
}

#[tauri::command]
pub(crate) fn upload_image_via_picgo_server(
    input: PicgoServerUploadInput,
) -> Result<ImageUploadPayload, String> {
    if input.bytes.is_empty() {
        return Err("图片内容为空，无法上传".to_string());
    }

    let server_url = input.server_url.trim();
    if server_url.is_empty() {
        return Err("PicGo Server 地址不能为空".to_string());
    }

    let temp_path = write_temp_image_file(&input.file_name, &input.bytes)?;
    let body = serde_json::json!({
        "list": [temp_path.to_string_lossy().to_string()]
    })
    .to_string();
    let response = post_json_to_picgo_server(server_url, &body);
    cleanup_temp_file(&temp_path);
    let response = response?;

    parse_picgo_server_url(&response)
        .map(|url| ImageUploadPayload { url })
        .ok_or_else(|| "PicGo Server 未返回图片 URL".to_string())
}

#[tauri::command]
pub(crate) fn test_picgo_connection(
    input: PicgoConnectionTestInput,
) -> Result<DesktopActionPayload, String> {
    match input.provider.as_str() {
        "picgo" => {
            let server_url = input
                .server_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "PicGo Server 地址不能为空".to_string())?;
            let endpoint = parse_http_endpoint(server_url)?;
            TcpStream::connect((&endpoint.host[..], endpoint.port))
                .map_err(|error| format!("连接 PicGo Server 失败：{error}"))?;
            Ok(DesktopActionPayload {
                ok: true,
                message: "PicGo Server 可以连接".to_string(),
            })
        }
        "picgo-core" => {
            let command = input
                .command
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "PicGo-Core 命令不能为空".to_string())?;
            let output = create_picgo_core_command(command)
                .arg("--version")
                .output()
                .map_err(|error| format!("调用 PicGo-Core 失败：{error}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    format!("PicGo-Core 测试失败，退出码：{}", output.status)
                } else {
                    format!("PicGo-Core 测试失败：{stderr}")
                });
            }
            Ok(DesktopActionPayload {
                ok: true,
                message: "PicGo-Core 命令可以调用".to_string(),
            })
        }
        _ => Err("未知图片上传方式".to_string()),
    }
}

fn create_picgo_core_command(command: &str) -> Command {
    let mut parts = split_command_line(command);
    let executable = parts
        .first()
        .cloned()
        .unwrap_or_else(|| command.trim().to_string());
    let mut process = Command::new(executable);
    for part in parts.drain(1..) {
        process.arg(part);
    }
    process
}

fn split_command_line(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in command.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

struct ImageAssetTarget {
    directory: PathBuf,
    absolute_path: PathBuf,
    markdown_prefix: String,
    markdown_src: String,
}

fn build_image_asset_target(
    document_dir: &Path,
    document_stem: &str,
    strategy: &str,
    file_name: &str,
) -> Result<ImageAssetTarget, String> {
    let safe_file_name = sanitize_image_file_name(file_name);
    let safe_document_stem = sanitize_path_segment(document_stem);
    let (directory, markdown_prefix) = match strategy {
        "copy-current-folder" => (document_dir.to_path_buf(), "./".to_string()),
        "copy-assets" => (document_dir.join("assets"), "./assets/".to_string()),
        "copy-document-assets" => {
            let folder = format!("{safe_document_stem}.assets");
            (document_dir.join(&folder), format!("./{folder}/"))
        }
        _ => return Err(format!("未知图片资源策略：{strategy}")),
    };

    let unique_file_name = unique_file_name(&directory, &safe_file_name);
    let absolute_path = directory.join(&unique_file_name);
    Ok(ImageAssetTarget {
        directory,
        absolute_path,
        markdown_prefix: markdown_prefix.clone(),
        markdown_src: format!("{markdown_prefix}{unique_file_name}"),
    })
}

fn document_stem(document_file_name: &str, document_path: &Path) -> String {
    Path::new(document_file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .or_else(|| document_path.file_stem().and_then(|value| value.to_str()))
        .unwrap_or("document")
        .to_string()
}

fn sanitize_image_file_name(file_name: &str) -> String {
    let trimmed = file_name.trim();
    let raw_name = if trimmed.is_empty() {
        "image.png"
    } else {
        trimmed
    };
    let path = Path::new(raw_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let safe_stem = sanitize_path_segment(stem);
    if extension.is_empty() {
        safe_stem
    } else {
        format!("{safe_stem}.{}", sanitize_extension(extension))
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '-'
            } else if ch.is_whitespace() {
                '-'
            } else {
                ch
            }
        })
        .collect::<String>();
    let collapsed = collapse_repeated_dash(&sanitized);
    let trimmed = collapsed.trim_matches('-');
    if trimmed.is_empty() {
        "image".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitize_extension(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if sanitized.is_empty() {
        "png".to_string()
    } else {
        sanitized.to_ascii_lowercase()
    }
}

fn collapse_repeated_dash(value: &str) -> String {
    let mut result = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch == '-' {
            if !previous_dash {
                result.push(ch);
            }
            previous_dash = true;
        } else {
            result.push(ch);
            previous_dash = false;
        }
    }
    result
}

fn unique_file_name(directory: &Path, safe_file_name: &str) -> String {
    let path = Path::new(safe_file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let mut candidate = safe_file_name.to_string();
    let mut suffix = 1;

    while directory.join(&candidate).exists() {
        candidate = if extension.is_empty() {
            format!("{stem}-{suffix}")
        } else {
            format!("{stem}-{suffix}.{extension}")
        };
        suffix += 1;
    }

    candidate
}

fn find_existing_image_with_same_fingerprint(directory: &Path, bytes: &[u8]) -> Option<String> {
    if !directory.is_dir() {
        return None;
    }

    let fingerprint = bytes_fingerprint(bytes);
    let expected_len = bytes.len() as u64;
    let entries = fs::read_dir(directory).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_supported_image_file_path(&path) {
            continue;
        }
        let Some(metadata) = entry.metadata().ok() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() != expected_len {
            continue;
        }
        let Some(existing_bytes) = fs::read(&path).ok() else {
            continue;
        };
        if bytes_fingerprint(&existing_bytes) == fingerprint {
            if let Some(file_name) = path.file_name().and_then(|value| value.to_str()) {
                return Some(file_name.to_string());
            }
        }
    }

    None
}

fn bytes_fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn ensure_path_stays_in_document_dir(document_dir: &Path, image_path: &Path) -> Result<(), String> {
    let document_dir = document_dir
        .canonicalize()
        .map_err(|error| format!("解析文档目录失败：{error}"))?;
    let image_path = image_path
        .canonicalize()
        .map_err(|error| format!("解析图片路径失败：{error}"))?;

    if image_path.starts_with(&document_dir) {
        Ok(())
    } else {
        Err("图片路径不在当前文档目录内，已跳过删除".to_string())
    }
}

fn is_supported_image_file_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif"
            )
        })
        .unwrap_or(false)
}

fn is_remote_image_src(src: &str) -> bool {
    let value = src.trim().to_ascii_lowercase();
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with("blob:")
}

/// markdown-it 会把非 ASCII 图片路径编码为百分号形式；访问磁盘前需还原为原始 UTF-8 路径。
fn decode_percent_encoded_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8(decoded).unwrap_or_else(|_| value.to_string())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn write_temp_image_file(file_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let safe_file_name = sanitize_image_file_name(file_name);
    let mut target = std::env::temp_dir().join(format!(
        "nomo-image-{}-{safe_file_name}",
        crate::config::now_ts()
    ));
    let mut index = 1;
    while target.exists() {
        target = std::env::temp_dir().join(format!(
            "nomo-image-{}-{index}-{safe_file_name}",
            crate::config::now_ts()
        ));
        index += 1;
    }
    fs::write(&target, bytes).map_err(|error| format!("写入临时图片失败：{error}"))?;
    Ok(target)
}

fn cleanup_temp_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn extract_first_url(text: &str) -> Option<String> {
    text.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, '"' | '\'' | '[' | ']' | ',' | '{' | '}')
    })
    .find(|part| part.starts_with("http://") || part.starts_with("https://"))
    .map(|part| {
        part.trim_matches(|ch| matches!(ch, '"' | '\'' | ',' | ']' | '}'))
            .to_string()
    })
}

fn post_json_to_picgo_server(server_url: &str, body: &str) -> Result<String, String> {
    let endpoint = parse_http_endpoint(server_url)?;
    let mut stream = TcpStream::connect((&endpoint.host[..], endpoint.port))
        .map_err(|error| format!("连接 PicGo Server 失败：{error}"))?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("请求 PicGo Server 失败：{error}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("读取 PicGo Server 响应失败：{error}"))?;

    let status_line = response.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(format!("PicGo Server 返回异常状态：{status_line}"));
    }

    Ok(response.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_endpoint(url: &str) -> Result<HttpEndpoint, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "PicGo Server 仅支持 http:// 本机地址".to_string())?;
    let (host_port, path) = rest.split_once('/').unwrap_or((rest, "upload"));
    let (host, port) = if let Some((host, port_text)) = host_port.rsplit_once(':') {
        let port = port_text
            .parse::<u16>()
            .map_err(|_| "PicGo Server 端口不可用".to_string())?;
        (host.to_string(), port)
    } else {
        (host_port.to_string(), 80)
    };
    if host.is_empty() {
        return Err("PicGo Server 地址缺少主机名".to_string());
    }
    Ok(HttpEndpoint {
        host,
        port,
        path: format!("/{}", path.trim_start_matches('/')),
    })
}

fn parse_picgo_server_url(response_body: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<Value>(response_body) {
        if value
            .get("success")
            .and_then(Value::as_bool)
            .is_some_and(|success| !success)
        {
            return None;
        }
        if let Some(url) = value
            .get("result")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find_map(Value::as_str))
        {
            return Some(url.to_string());
        }
        if let Some(url) = value.get("url").and_then(Value::as_str) {
            return Some(url.to_string());
        }
    }

    extract_first_url(response_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_assets_markdown_path_with_safe_file_name() {
        let dir = Path::new("D:\\Docs");
        let target = build_image_asset_target(dir, "设计 文档", "copy-assets", "截图 1:?.PNG")
            .expect("target");

        expect_path_ends_with(&target.absolute_path, "assets\\截图-1.png");
        assert_eq!(target.markdown_src, "./assets/截图-1.png");
    }

    #[test]
    fn builds_document_assets_markdown_path() {
        let dir = Path::new("D:\\Docs");
        let target =
            build_image_asset_target(dir, "设计 文档", "copy-document-assets", "image.png")
                .expect("target");

        expect_path_ends_with(&target.absolute_path, "设计-文档.assets\\image.png");
        assert_eq!(target.markdown_src, "./设计-文档.assets/image.png");
    }

    #[test]
    fn parses_picgo_server_json_result() {
        let url = parse_picgo_server_url(r#"{"success":true,"result":["https://img.test/a.png"]}"#);

        assert_eq!(url.as_deref(), Some("https://img.test/a.png"));
    }

    #[test]
    fn parses_picgo_core_stdout_url() {
        let url = extract_first_url("Upload success: https://img.test/a.png");

        assert_eq!(url.as_deref(), Some("https://img.test/a.png"));
    }

    #[test]
    fn splits_picgo_core_command_with_quoted_windows_path() {
        let parts = split_command_line(r#""C:\Program Files\picgo\picgo.exe" upload"#);

        assert_eq!(
            parts,
            vec![
                r#"C:\Program Files\picgo\picgo.exe"#.to_string(),
                "upload".to_string()
            ]
        );
    }

    #[test]
    fn splits_picgo_core_command_with_runner_and_package() {
        let parts = split_command_line("npx picgo");

        assert_eq!(parts, vec!["npx".to_string(), "picgo".to_string()]);
    }

    #[test]
    fn imports_duplicate_image_by_fingerprint_without_new_file() {
        let dir = create_test_dir("fingerprint");
        let document_path = dir.join("doc.md");
        let input = ImageAssetInput {
            document_path: document_path.to_string_lossy().to_string(),
            document_file_name: "doc.md".to_string(),
            strategy: "copy-assets".to_string(),
            file_name: "image.png".to_string(),
            bytes: vec![1, 2, 3, 4],
        };

        let first = import_image_asset(input).expect("first import");
        let second = import_image_asset(ImageAssetInput {
            document_path: document_path.to_string_lossy().to_string(),
            document_file_name: "doc.md".to_string(),
            strategy: "copy-assets".to_string(),
            file_name: "image.png".to_string(),
            bytes: vec![1, 2, 3, 4],
        })
        .expect("second import");

        assert_eq!(first.markdown_src, "./assets/image.png");
        assert_eq!(second.markdown_src, "./assets/image.png");
        assert!(!first.reused);
        assert!(second.reused);
        assert!(!dir.join("assets").join("image-1.png").exists());
        cleanup_test_dir(&dir);
    }

    #[test]
    fn deletes_document_relative_image_asset() {
        let dir = create_test_dir("delete");
        let assets_dir = dir.join("assets");
        fs::create_dir_all(&assets_dir).expect("assets dir");
        let image_path = assets_dir.join("image.png");
        fs::write(&image_path, [1, 2, 3]).expect("image");

        let result = delete_image_asset(ImageDeleteInput {
            document_path: Some(dir.join("doc.md").to_string_lossy().to_string()),
            src: "./assets/image.png".to_string(),
        });

        assert!(result.removed);
        assert!(!image_path.exists());
        cleanup_test_dir(&dir);
    }

    fn expect_path_ends_with(path: &Path, suffix: &str) {
        let normalized = path.to_string_lossy().replace('/', "\\");
        assert!(
            normalized.ends_with(suffix),
            "expected `{normalized}` to end with `{suffix}`"
        );
    }

    fn create_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nomo-image-assets-{name}-{}",
            crate::config::now_ts()
        ));
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    fn cleanup_test_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
}

//! 链接目标分流与附件预检；本地正文仍交给 Workspace 的窗口化文档会话加载。
use super::protocol::ControlError;
use std::path::{Path, PathBuf};
use url::Url;

// 沿用原版附件范围；TXT／JSON 的文档类型路由单独处理。
const LOCAL_ATTACHMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "csv", "png", "jpg", "jpeg", "gif",
    "webp", "bmp", "svg", "avif",
];

pub(crate) enum LinkTarget {
    External(String),
    Anchor(String),
    Markdown { path: PathBuf, anchor: Option<String> },
    Attachment(PathBuf),
}

pub(crate) fn resolve(href: &str, document: Option<&Path>) -> Result<LinkTarget, ControlError> {
    let href = href.trim();
    let invalid = || ControlError::Unsupported("链接地址无效或包含不支持的协议".into());
    if href.is_empty() || href.chars().any(char::is_control) { return Err(invalid()); }
    if let Some(anchor) = href.strip_prefix('#') {
        return Ok(LinkTarget::Anchor(decode(anchor)?));
    }
    let drive = href.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && href.as_bytes().get(1) == Some(&b':')
        && href.as_bytes().get(2).is_some_and(|byte| matches!(byte, b'/' | b'\\'));
    let target = if drive {
        Url::parse(&format!("file:///{}", href.replace('\\', "/"))).map_err(|_| invalid())?
    } else if cfg!(not(target_os = "windows")) && href.starts_with('/') && !href.starts_with("//") {
        Url::parse(&format!("file://{href}")).map_err(|_| invalid())?
    } else if let Ok(url) = Url::parse(href) {
        match url.scheme() {
            "http" | "https" | "mailto" => return Ok(LinkTarget::External(url.into())),
            "file" => url,
            _ => return Err(invalid()),
        }
    } else {
        let document = document.ok_or_else(|| ControlError::Unsupported("请先保存当前文档，再打开相对路径链接".into()))?;
        Url::from_file_path(document).map_err(|_| invalid())?
            .join(&href.replace('\\', "/")).map_err(|_| invalid())?
    };
    if target.scheme() != "file" || target.host_str().is_some_and(|host| !host.is_empty() && host != "localhost")
        || target.query().is_some()
    {
        return Err(invalid());
    }
    // 检查编码和控制字符，但由 URL 库对路径解码一次，避免将 %2520 再次解码。
    decode(target.path())?;
    let anchor = target.fragment().map(decode).transpose()?;
    let path = target.to_file_path().map_err(|_| invalid())?;
    if path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown"))
    {
        return Ok(LinkTarget::Markdown { path, anchor });
    }
    if is_attachment(&path) {
        return Ok(LinkTarget::Attachment(path));
    }
    Err(ControlError::Unsupported("本地链接支持 Markdown、PDF、Office、CSV 和常见图片附件".into()))
}

/// 只在用户打开附件时由后台执行；复核最终目标，防止符号链接绕过附件类型限制。
pub(super) fn prepare_attachment(path: &Path) -> Result<PathBuf, String> {
    validate_attachment_path(path)?;
    let path = path.canonicalize().map_err(|error| format!("附件不存在或无法访问：{error}"))?;
    validate_attachment_path(&path)?;
    let metadata = path.metadata().map_err(|error| format!("无法读取附件信息：{error}"))?;
    if !metadata.is_file() {
        return Err("附件路径不是文件".into());
    }
    // 系统 Shell 使用普通文件路径；URL 往返去掉 Windows canonicalize 产生的设备路径前缀。
    Url::from_file_path(&path).and_then(|url| url.to_file_path())
        .map_err(|_| "附件路径无效".into())
}

fn is_attachment(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| LOCAL_ATTACHMENT_EXTENSIONS.iter().any(|allowed| extension.eq_ignore_ascii_case(allowed)))
}

fn validate_attachment_path(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("不支持打开 UNC、网络共享或设备路径附件".into());
        }
        if path.components().any(|component| matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':'))) {
            return Err("附件路径不能包含备用数据流".into());
        }
    }
    if !path.is_absolute() || !is_attachment(path) {
        return Err("附件类型或路径不受支持".into());
    }
    Ok(())
}

fn decode(value: &str) -> Result<String, ControlError> {
    let invalid = || ControlError::Unsupported("链接的 URL 编码无效".into());
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next().ok_or_else(invalid)? as char).to_digit(16).ok_or_else(invalid)?;
            let low = (input.next().ok_or_else(invalid)? as char).to_digit(16).ok_or_else(invalid)?;
            bytes.push((high * 16 + low) as u8);
        } else { bytes.push(byte); }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
    if decoded.chars().any(char::is_control) { return Err(invalid()); }
    Ok(decoded)
}

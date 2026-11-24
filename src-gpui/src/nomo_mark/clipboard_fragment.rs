//! 内部选区随 HTML 注释传输；外部应用忽略或移除后，仍可使用标准 HTML/文本。
//! 只保存本次选区，不持有文档、视图或应用级正文缓存。
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

pub(super) const MAX_BYTES: usize = 64 * 1024 * 1024;
const PREFIX: &str = "<!--Nomo-Clipboard:1:";
const SUFFIX: &str = "-->";

fn digest(text: &str, html: &str, source: &str) -> String {
    let mut hash = Sha256::new();
    for part in [text, html, source] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn encode(text: &str, html: &str, source: &str) -> Result<String, String> {
    let encoded_len = source.len().div_ceil(3).checked_mul(4);
    let size = text.len().checked_add(html.len())
        .and_then(|n| n.checked_add(PREFIX.len() + 65 + SUFFIX.len()))
        .and_then(|n| n.checked_add(encoded_len?));
    if size.is_none_or(|n| n > MAX_BYTES) {
        return Err("复制内容超过 64 MiB 剪贴板预算；原文已保留".into());
    }
    if [text, html, source].iter().any(|s| s.contains('\0')) {
        return Err("所选内容包含空字符，正文已保留".into());
    }
    Ok(format!("{html}{PREFIX}{}:{}{SUFFIX}", digest(text, html, source), STANDARD.encode(source)))
}

pub(super) fn decode(text: &str, html: &str) -> Option<String> {
    if text.len().checked_add(html.len())? > MAX_BYTES { return None; }
    let (body, metadata) = html.strip_suffix(SUFFIX)?.rsplit_once(PREFIX)?;
    let (checksum, encoded) = metadata.split_once(':')?;
    if checksum.len() != 64 || encoded.len() > MAX_BYTES { return None; }
    let source = String::from_utf8(STANDARD.decode(encoded).ok()?).ok()?;
    if source.contains('\0') || digest(text, body, &source) != checksum { return None; }
    // 校验用于拒绝被改写、过期或损坏的片段，不赋予内容额外信任或执行权限。
    Some(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_fragment_roundtrip_and_corruption_fallback() {
        let source = "**中😀**\r\n<!-- x -->";
        let html = "<p><strong>中😀</strong></p>";
        let encoded = encode("中😀", html, source).unwrap();
        assert_eq!(decode("中😀", &encoded).as_deref(), Some(source));
        assert!(decode("changed", &encoded).is_none());
        assert!(decode("中😀", &encoded.replace("<strong>", "<em>")).is_none());
        assert!(decode("中😀", &encoded.replace("Clipboard:1:", "Clipboard:2:")).is_none());
        assert!(decode("中😀", "<!--Nomo-Clipboard:1:invalid-->").is_none());
        assert!(encode("x", "", "\0").is_err());
    }

    #[test]
    fn clipboard_fragment_budget_includes_encoded_source() {
        let source = "x".repeat(MAX_BYTES * 3 / 4);
        assert!(encode("", "", &source).is_err());
    }
}

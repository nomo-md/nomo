//! 窗口会话的无 UI 契约，Tauri 与 GPUI 使用同一份定义。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedEdit {
    pub(crate) from_byte: u64,
    pub(crate) to_byte: u64,
    pub(crate) inserted_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextDocumentError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actual_revision: Option<u64>,
    #[serde(rename = "recoveryPath", skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_path: Option<String>,
}

impl TextDocumentError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            actual_revision: None,
            recovery_path: None,
        }
    }

    pub(crate) fn revision_conflict(expected: u64, actual: u64) -> Self {
        Self {
            code: "revision-conflict",
            message: format!("文档 revision 已变化：expected={expected}, actual={actual}"),
            actual_revision: Some(actual),
            recovery_path: None,
        }
    }

    pub(crate) fn source_missing_recovered(path: &std::path::Path) -> Self {
        Self {
            code: "source-missing-recovered",
            message: format!("源文件不存在，未保存内容已恢复到 {}", path.display()),
            actual_revision: None,
            recovery_path: Some(path.to_string_lossy().into_owned()),
        }
    }
}

impl std::fmt::Display for TextDocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TextDocumentError {}

impl From<std::io::Error> for TextDocumentError {
    fn from(error: std::io::Error) -> Self {
        Self::new("io-error", error.to_string())
    }
}

pub(crate) type TextDocumentResult<T> = Result<T, TextDocumentError>;

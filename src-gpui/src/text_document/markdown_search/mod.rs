//! Markdown 专用搜索协议；与 TXT/JSON 的 ASCII 匹配语义隔离。
mod matcher;
mod service;
use super::{markdown_history::MarkdownSelection, markdown_session::MarkdownStatus};
pub(super) use matcher::{check_cancel, Checkpoint, Matcher};
use serde::{Deserialize, Serialize};
pub(super) use service::{SearchService, SearchTask};
use std::sync::Arc;

pub(crate) const SEARCH_EVENT: &str = "nomo://markdown-task-progress";
pub(super) const PAGE_SIZE: usize = 128;
pub(super) type ProgressSink = Arc<dyn Fn(TaskProgress) + Send + Sync>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchQuery {
    pub document_id: String,
    pub base_revision: u64,
    pub request_id: String,
    pub query: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceQuery {
    #[serde(flatten)]
    pub search: SearchQuery,
    pub replacement: String,
    pub operation_id: String,
    #[serde(default)]
    pub selection: Option<MarkdownSelection>,
    #[serde(default)]
    pub anchors: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchMatch {
    pub index: u64,
    pub from_byte: u64,
    pub to_byte: u64,
    pub preserved_prefix: String,
    pub preserved_suffix: String,
}
impl SearchMatch {
    pub(super) fn raw(&self) -> (u64, u64) {
        (
            self.from_byte + self.preserved_prefix.len() as u64,
            self.to_byte - self.preserved_suffix.len() as u64,
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskIdentity {
    pub request_id: String,
    pub task_id: String,
    pub document_id: String,
    pub base_revision: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskProgress {
    #[serde(flatten)]
    pub identity: TaskIdentity,
    pub state: String,
    pub scanned_bytes: u64,
    pub total_bytes: u64,
    pub match_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<super::TextDocumentError>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchPage {
    #[serde(flatten)]
    pub identity: TaskIdentity,
    pub total_count: u64,
    pub from_index: u64,
    pub matches: Vec<SearchMatch>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceResult {
    #[serde(flatten)]
    pub identity: TaskIdentity,
    pub status: MarkdownStatus,
    pub count: u64,
    pub changed_count: u64,
    pub anchors: Vec<u64>,
}

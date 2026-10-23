//! Markdown statistics have their own bounded, disposable disk index. No WebView
//! strings, session guards, syntax trees or per-character position arrays are retained.
#[cfg(test)]
mod allocation_test;
mod count;
mod delimiters;
mod scan;
mod service;
mod source;
mod tape;
#[cfg(test)]
mod tests;

use super::{TextDocumentError, TextDocumentResult};
use serde::{Deserialize, Serialize};
pub(crate) use service::WritingStatsService;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const CHUNK: usize = 64 * 1024;
pub(super) const MEMORY_BUDGET: usize = 8 * 1024 * 1024;
pub(super) const DISK_BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentStats {
    pub chars: u64,
    pub words: u64,
    pub visible_chars: u64,
    pub lines: u64,
    pub headings: u64,
    pub reading_minutes: u64,
}
impl DocumentStats {
    fn finish(mut self) -> Self {
        self.lines += 1;
        self.reading_minutes = self.words.div_ceil(280).max(1);
        self
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsQuery {
    pub document_id: String,
    pub revision: u64,
    pub request_id: u64,
    pub selection_version: u64,
    pub selection: Option<ByteRange>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct ByteRange {
    pub from: u64,
    pub to: u64,
}
impl ByteRange {
    fn normalized(self, len: u64) -> Self {
        Self {
            from: self.from.min(self.to).min(len),
            to: self.from.max(self.to).min(len),
        }
    }
    fn overlaps(self, from: u64, to: u64) -> bool {
        from < self.to && to > self.from
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsResult {
    #[serde(flatten)]
    pub identity: StatsQuery,
    pub full: DocumentStats,
    pub selected: Option<DocumentStats>,
    pub metrics: StatsMetrics,
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsMetrics {
    pub elapsed_ms: u128,
    pub snapshot_bytes_read: u64,
    pub workspace_bound_bytes: usize,
    pub index_bytes: u64,
    pub cache_hit: bool,
    pub reused_blocks: u64,
}
fn check(cancel: &AtomicBool) -> TextDocumentResult<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(TextDocumentError::new("stats-cancelled", "统计请求已取消"))
    } else {
        Ok(())
    }
}
fn invalid(message: impl Into<String>) -> TextDocumentError {
    TextDocumentError::new("stats-index", message)
}

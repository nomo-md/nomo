//! 磁盘为事实源的追加历史。撤销/重做只是移动持久化游标，缓存淘汰不删除历史。
use super::{
    piece_tree::{Piece, PieceSource, PieceTree},
    TextDocumentError, TextDocumentResult,
};
use serde::{Deserialize, Serialize};
use crate::markdown_templates::InlineTemplate;
use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

const HOT_HISTORY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownSelection {
    pub(crate) anchor_byte: u64,
    pub(crate) head_byte: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TreeEdit {
    pub(super) from: u64,
    pub(super) to: u64,
    pub(super) pieces: Vec<Piece>,
}

pub(super) fn extract_pieces(
    tree: &PieceTree,
    from: u64,
    to: u64,
) -> TextDocumentResult<Vec<Piece>> {
    tree.segments(from, to)?
        .into_iter()
        .map(|(piece, relative, length)| {
            let source = match piece.source {
                PieceSource::Original { offset, .. } => PieceSource::Original {
                    offset: offset + relative,
                    length,
                },
                PieceSource::Added { offset, .. } => PieceSource::Added {
                    offset: offset + relative,
                    length,
                },
            };
            Ok(Piece {
                source,
                line_breaks: None,
            })
        })
        .collect()
}

/// edits 按执行顺序记录，允许历史组中后一步使用前一步提交后的坐标。
pub(super) fn apply_tree_edits(
    mut tree: PieceTree,
    edits: &[TreeEdit],
) -> TextDocumentResult<PieceTree> {
    for edit in edits {
        if edit.pieces.len() <= 1 {
            tree = tree.replace_range(edit.from, edit.to, edit.pieces.first().cloned())?;
            continue;
        }
        tree = tree.replace_range(edit.from, edit.to, None)?;
        for piece in edit.pieces.iter().rev() {
            tree = tree.replace_range(edit.from, edit.from, Some(piece.clone()))?;
        }
    }
    Ok(tree)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct HistoryRecord {
    pub(super) previous_event: Option<u64>,
    pub(super) group: String,
    pub(super) parent_undo: Option<u64>,
    pub(super) previous_redo: Option<u64>,
    pub(super) edits: Vec<TreeEdit>,
    pub(super) inverse: Vec<TreeEdit>,
    /// undo 事件保存同组原始事务的偏移，redo 从旧到新应用，保持原始片段身份。
    pub(super) group_entries: Vec<u64>,
    pub(super) selection_before: Option<MarkdownSelection>,
    pub(super) selection_after: Option<MarkdownSelection>,
    /// 只有来源状态变化时随同原有历史存储；旧记录缺省为不改变。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) templates_before: Option<Vec<InlineTemplate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) templates_after: Option<Vec<InlineTemplate>>,
}

pub(super) struct DiskHistory {
    file: File,
    hot: HashMap<u64, (HistoryRecord, usize)>,
    order: VecDeque<u64>,
    used: usize,
    capacity: usize,
    read_limit: u64,
}

impl DiskHistory {
    pub(super) fn open(path: &Path) -> TextDocumentResult<Self> {
        Ok(Self {
            file: OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)?,
            hot: HashMap::new(),
            order: VecDeque::new(),
            used: 0,
            capacity: HOT_HISTORY_BYTES,
            read_limit: u64::MAX,
        })
    }
    pub(super) fn open_read_only(path: &Path) -> TextDocumentResult<Self> {
        Ok(Self { file: File::open(path)?, hot: HashMap::new(), order: VecDeque::new(), used: 0, capacity: 0, read_limit: 8 * 1024 * 1024 })
    }
    pub(super) fn append(&mut self, record: &HistoryRecord) -> TextDocumentResult<u64> {
        let bytes = serde_json::to_vec(record)
            .map_err(|e| TextDocumentError::new("history-serialize", e.to_string()))?;
        let offset = self.write_record(&bytes)?;
        self.file.sync_all()?;
        self.cache(offset, record.clone(), bytes.len().saturating_mul(2).saturating_add(256));
        Ok(offset)
    }
    /// 未发布批次不进入热缓存，也不逐条 fsync；调用方在发布 manifest 前统一持久化。
    pub(super) fn append_staged(&mut self, record: &HistoryRecord) -> TextDocumentResult<u64> {
        let bytes = serde_json::to_vec(record)
            .map_err(|e| TextDocumentError::new("history-serialize", e.to_string()))?;
        self.write_record(&bytes)
    }
    fn write_record(&mut self, bytes: &[u8]) -> TextDocumentResult<u64> {
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        self.file.write_all(&bytes)?;
        Ok(offset)
    }
    pub(super) fn durability_handle(&self) -> TextDocumentResult<File> { Ok(self.file.try_clone()?) }
    #[cfg(test)]
    pub(super) fn evict_cache(&mut self) { self.capacity = 0; self.used = 0; self.hot.clear(); self.order.clear(); }
    pub(super) fn read(&mut self, offset: u64) -> TextDocumentResult<HistoryRecord> {
        if let Some((record, _)) = self.hot.get(&offset) {
            let record = record.clone();
            self.order.retain(|key| *key != offset);
            self.order.push_back(offset);
            return Ok(record);
        }
        let file_length = self.file.metadata()?.len();
        if offset
            .checked_add(8)
            .filter(|end| *end <= file_length)
            .is_none()
        {
            return Err(TextDocumentError::new(
                "history-corrupt",
                "历史索引超出日志范围",
            ));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        let mut length = [0u8; 8];
        self.file.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        if length > file_length - offset - 8 || length > usize::MAX as u64 || length > self.read_limit {
            return Err(TextDocumentError::new(
                "history-corrupt",
                "历史记录未完整写入",
            ));
        }
        let mut bytes = vec![0u8; length as usize];
        self.file.read_exact(&mut bytes)?;
        let record: HistoryRecord = serde_json::from_slice(&bytes)
            .map_err(|e| TextDocumentError::new("history-corrupt", e.to_string()))?;
        self.cache(
            offset,
            record.clone(),
            bytes.len().saturating_mul(2).saturating_add(256),
        );
        Ok(record)
    }
    fn cache(&mut self, offset: u64, record: HistoryRecord, size: usize) {
        if size > self.capacity {
            return;
        }
        if let Some((_, old_size)) = self.hot.remove(&offset) {
            self.used -= old_size;
        }
        self.order.retain(|key| *key != offset);
        while self.used + size > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, old_size)) = self.hot.remove(&oldest) {
                self.used -= old_size;
            }
        }
        self.used += size;
        self.hot.insert(offset, (record, size));
        self.order.push_back(offset);
    }
    pub(super) fn templates_at(&mut self, mut head: Option<u64>) -> TextDocumentResult<Vec<InlineTemplate>> {
        while let Some(offset) = head {
            let record = self.read(offset)?;
            if let Some(templates) = record.templates_after { return Ok(templates); }
            if record.previous_event.is_some_and(|previous| previous >= offset) {
                return Err(TextDocumentError::new("history-corrupt", "历史记录存在循环"));
            }
            head = record.previous_event;
        }
        Ok(Vec::new())
    }
    pub(super) fn replay(
        &mut self,
        base: PieceTree,
        head: Option<u64>,
    ) -> TextDocumentResult<PieceTree> {
        self.replay_cancelled(base,head,&std::sync::atomic::AtomicBool::new(false))
    }
    pub(super) fn replay_cancelled(&mut self,base:PieceTree,head:Option<u64>,cancel:&std::sync::atomic::AtomicBool)->TextDocumentResult<PieceTree> {
        let mut chain = Vec::new();
        let mut cursor = head;
        while let Some(offset) = cursor {
            super::markdown_search::check_cancel(cancel)?;
            let record = self.read(offset)?;
            if record
                .previous_event
                .is_some_and(|previous| previous >= offset)
            {
                return Err(TextDocumentError::new(
                    "history-corrupt",
                    "历史记录存在循环",
                ));
            }
            if self.read_limit != u64::MAX && chain.len() >= 65536 {
                return Err(TextDocumentError::new("recovery-budget", "恢复历史较长，请手动选择恢复后检查内容"));
            }
            chain.push(offset);
            cursor = record.previous_event;
        }
        let mut tree = base;
        for offset in chain.into_iter().rev() {
            super::markdown_search::check_cancel(cancel)?;
            tree = apply_tree_edits(tree, &self.read(offset)?.edits)?;
        }
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_history_survives_cache_eviction_and_ignores_unpublished_events() {
        let path = std::env::temp_dir().join(format!(
            "nomo-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut history = DiskHistory::open(&path).unwrap();
        history.capacity = 0;
        let record = HistoryRecord {
            templates_before: None,
            templates_after: None,
            previous_event: None,
            group: "typing".into(),
            parent_undo: None,
            previous_redo: None,
            edits: vec![TreeEdit {
                from: 1,
                to: 1,
                pieces: vec![Piece::added(0, 3, 0)],
            }],
            inverse: vec![],
            group_entries: vec![],
            selection_before: None,
            selection_after: None,
        };
        let published = history.append(&record).unwrap();
        history.append(&record).unwrap();
        drop(history);
        let mut reopened = DiskHistory::open(&path).unwrap();
        let tree = reopened
            .replay(PieceTree::from_original(4), Some(published))
            .unwrap();
        assert_eq!(tree.len(), 7);
        assert_eq!(extract_pieces(&tree, 0, 7).unwrap().len(), 3);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
}

//! 默认编辑器的内存文档层，复用持久化 PieceTree，并维护行首索引。
//! 开发控件的磁盘会话另由 text_document 管理。

#[allow(dead_code)]
#[path = "../text_document/piece_tree.rs"]
mod piece_tree;

use piece_tree::{Piece, PieceSource, PieceTree};
use std::ops::Range;

/// PieceTree 在默认编辑器中的窄错误类型。
#[derive(Debug, Clone)]
pub(crate) struct TextDocumentError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl TextDocumentError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for TextDocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

pub(crate) type TextDocumentResult<T> = Result<T, TextDocumentError>;

/// 一次替换的逆操作，撤销时按原位置写回被删除的文本。
#[derive(Debug, Clone)]
pub struct EditRecord {
    pub from: u64,
    pub removed: String,
    pub inserted_len: u64,
}

/// 单文档正文：原始字节只读，编辑字节只追加，PieceTree 记录逻辑顺序。
///
/// 不变量：`line_starts[0] == 0`，其余元素严格递增且每个都紧跟一个 `\n`；
/// 所有偏移都是 UTF-8 字节偏移。
pub struct Document {
    original: Vec<u8>,
    added: Vec<u8>,
    tree: PieceTree,
    line_starts: Vec<u64>,
}

impl Document {
    pub fn from_bytes(original: Vec<u8>) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            original
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(index, _)| index as u64 + 1),
        );
        let tree = PieceTree::from_original(original.len() as u64);
        Self { original, added: Vec::new(), tree, line_starts }
    }

    pub fn len(&self) -> u64 {
        self.tree.len()
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// 行内容的字节范围，不含行尾 `\n` 与 `\r`。
    pub fn line_range(&self, line: usize) -> Range<u64> {
        let start = self.line_starts[line];
        let mut end = self
            .line_starts
            .get(line + 1)
            .map_or(self.len(), |next| next - 1);
        if end > start && self.byte_at(end - 1) == Some(b'\r') {
            end -= 1;
        }
        start..end
    }

    pub fn line_of_offset(&self, offset: u64) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        }
    }

    pub fn line_text(&self, line: usize) -> String {
        let range = self.line_range(line);
        self.read(range.start, range.end)
    }

    /// 读取字节区间；区间切在多字节字符中间时按 lossy 解码，原型不做编码边界修复。
    pub fn read(&self, start: u64, end: u64) -> String {
        let mut bytes = Vec::with_capacity((end - start) as usize);
        for (piece, offset, length) in self.tree.segments(start, end).unwrap_or_default() {
            let (buffer, base) = match piece.source {
                PieceSource::Original { offset, .. } => (&self.original, offset),
                PieceSource::Added { offset, .. } => (&self.added, offset),
            };
            let from = (base + offset) as usize;
            bytes.extend_from_slice(&buffer[from..from + length as usize]);
        }
        String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
    }

    fn byte_at(&self, offset: u64) -> Option<u8> {
        let (piece, inner) = self.tree.piece_at(offset)?;
        Some(match piece.source {
            PieceSource::Original { offset, .. } => self.original[(offset + inner) as usize],
            PieceSource::Added { offset, .. } => self.added[(offset + inner) as usize],
        })
    }

    /// 用 `text` 替换 `from..to`，返回可用于撤销的逆操作。
    pub fn replace(&mut self, from: u64, to: u64, text: &str) -> TextDocumentResult<EditRecord> {
        let removed = self.read(from, to);
        let inserted = (!text.is_empty()).then(|| {
            let offset = self.added.len() as u64;
            self.added.extend_from_slice(text.as_bytes());
            let breaks = text.bytes().filter(|byte| *byte == b'\n').count() as u64;
            Piece::added(offset, text.len() as u64, breaks)
        });
        self.tree = self.tree.replace_range(from, to, inserted)?;
        self.update_line_starts(from, to, text);
        Ok(EditRecord { from, removed, inserted_len: text.len() as u64 })
    }

    /// 删除落在被替换区间内的行首，平移其后的行首，再插入新文本带来的行首。
    fn update_line_starts(&mut self, from: u64, to: u64, text: &str) {
        let delta = text.len() as i64 - (to - from) as i64;
        let first_removed = self.line_starts.partition_point(|start| *start <= from);
        let first_kept = self.line_starts.partition_point(|start| *start <= to);
        let inserted = text
            .bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .map(|(index, _)| from + index as u64 + 1);
        self.line_starts.splice(first_removed..first_kept, inserted);
        let shift_from = first_removed + text.bytes().filter(|byte| *byte == b'\n').count();
        for start in &mut self.line_starts[shift_from..] {
            *start = (*start as i64 + delta) as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_keeps_line_index_consistent() {
        let mut document = Document::from_bytes(b"ab\ncd\r\nef".to_vec());
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.line_text(1), "cd");
        let undo = document.replace(1, 4, "X\nY\nZ").unwrap();
        assert_eq!(document.read(0, document.len()), "aX\nY\nZd\r\nef");
        assert_eq!(document.line_count(), 4);
        assert_eq!(document.line_text(2), "Zd");
        assert_eq!(document.line_text(3), "ef");
        document
            .replace(undo.from, undo.from + undo.inserted_len, &undo.removed)
            .unwrap();
        assert_eq!(document.read(0, document.len()), "ab\ncd\r\nef");
        assert_eq!(document.line_starts, vec![0, 3, 7]);
    }
}

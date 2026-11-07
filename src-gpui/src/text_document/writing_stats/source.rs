use super::super::{chunk_reader::DocumentSnapshot, piece_tree::PieceSource, TextDocumentResult};
use super::{check, invalid, CHUNK};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{atomic::AtomicBool, Arc},
};

pub(super) enum Input {
    Snapshot(DocumentSnapshot),
    File(File),
}
#[derive(Clone, Copy)]
pub(super) struct SourceChange {
    pub from: u64,
    pub old_to: u64,
    pub new_to: u64,
}
/// A single 64 KiB page. Lookahead and backwards context use this same page;
/// arbitrary line / delimiter lengths never determine allocation size.
pub(super) struct Source {
    input: Input,
    page: Vec<u8>,
    base: u64,
    pub len: u64,
    pub read_bytes: u64,
    operations: u32,
    pub cancel: Arc<AtomicBool>,
}
impl Source {
    /// 已确认事务保留了未变片段的源身份；按片段跳过前后缀，不再比较整篇正文。
    /// 返回保守范围，撤销、重做及多处修改也走同一条路径。
    pub fn change_since(&self, previous: &DocumentSnapshot) -> TextDocumentResult<Option<SourceChange>> {
        let Input::Snapshot(current) = &self.input else { return Ok(None); };
        if !Arc::ptr_eq(&previous.original, &current.original)
            || !Arc::ptr_eq(&previous.added, &current.added) { return Ok(None); }
        let shared = |old_at, new_at, backwards: bool| -> TextDocumentResult<u64> {
            let (old, a) = previous.tree.piece_at(old_at).ok_or_else(|| invalid("旧统计片段缺失"))?;
            let (new, b) = current.tree.piece_at(new_at).ok_or_else(|| invalid("当前统计片段缺失"))?;
            let same = match (&old.source, &new.source) {
                (PieceSource::Original { offset: x, .. }, PieceSource::Original { offset: y, .. })
                | (PieceSource::Added { offset: x, .. }, PieceSource::Added { offset: y, .. }) => x + a == y + b,
                _ => false,
            };
            Ok(if !same { 0 } else if backwards { (a + 1).min(b + 1) }
                else { (old.len() - a).min(new.len() - b) })
        };
        let mut from = 0;
        while from < previous.len().min(current.len()) {
            check(&self.cancel)?;
            let length = shared(from, from, false)?;
            if length == 0 { break; }
            from += length;
        }
        let (mut old_to, mut new_to) = (previous.len(), current.len());
        while old_to > from && new_to > from {
            check(&self.cancel)?;
            let length = shared(old_to - 1, new_to - 1, true)?.min(old_to - from).min(new_to - from);
            if length == 0 { break; }
            old_to -= length;
            new_to -= length;
        }
        Ok(Some(SourceChange { from, old_to, new_to }))
    }

    pub fn incremental_raw_counts(&mut self, previous: &DocumentSnapshot, chars: u64, lines: u64)
        -> TextDocumentResult<(u64, u64)> {
        let Some(change) = self.change_since(previous)? else { return self.raw_counts(0, self.len); };
        let mut old = Self::snapshot(previous.clone(), self.cancel.clone());
        let mut from = change.from;
        // 插入/删除 LF 会改变左侧 CR 的归属，必须把这个边界一起计数。
        if from > 0 && old.byte(from - 1)? == b'\r' { from -= 1; }
        let removed = old.raw_counts(from, change.old_to)?;
        let inserted = self.raw_counts(from, change.new_to)?;
        self.read_bytes += old.read_bytes;
        let chars = chars.checked_sub(removed.0).and_then(|value| value.checked_add(inserted.0));
        let lines = lines.checked_sub(removed.1).and_then(|value| value.checked_add(inserted.1));
        chars.zip(lines).ok_or_else(|| invalid("统计增量超出原计数范围"))
    }

    pub fn snapshot(snapshot: DocumentSnapshot, cancel: Arc<AtomicBool>) -> Self {
        let len = snapshot.len();
        Self {
            input: Input::Snapshot(snapshot),
            page: Vec::new(),
            base: u64::MAX,
            len,
            read_bytes: 0,
            operations: 0,
            cancel,
        }
    }
    pub fn file(file: File, len: u64, cancel: Arc<AtomicBool>) -> Self {
        Self {
            input: Input::File(file),
            page: Vec::new(),
            base: u64::MAX,
            len,
            read_bytes: 0,
            operations: 0,
            cancel,
        }
    }
    pub fn byte(&mut self, pos: u64) -> TextDocumentResult<u8> {
        self.operations = self.operations.wrapping_add(1);
        if self.operations & 4095 == 0 {
            check(&self.cancel)?;
        }
        if pos >= self.len {
            return Ok(0);
        }
        let base = pos / CHUNK as u64 * CHUNK as u64;
        if base != self.base {
            check(&self.cancel)?;
            let length = (self.len - base).min(CHUNK as u64) as usize;
            self.page = match &mut self.input {
                Input::Snapshot(snapshot) => snapshot.read_range(base, length)?,
                Input::File(file) => {
                    file.seek(SeekFrom::Start(base))?;
                    let mut page = vec![0; length];
                    file.read_exact(&mut page)?;
                    page
                }
            };
            self.base = base;
            self.read_bytes += length as u64;
        }
        Ok(self.page[(pos - self.base) as usize])
    }
    pub fn char(&mut self, pos: u64) -> TextDocumentResult<(char, u64)> {
        let first = self.byte(pos)?;
        let n = if first < 128 {
            1
        } else if first < 224 {
            2
        } else if first < 240 {
            3
        } else {
            4
        };
        let mut bytes = [0; 4];
        bytes[0] = first;
        for (i, b) in bytes.iter_mut().enumerate().take(n).skip(1) {
            *b = self.byte(pos + i as u64)?;
        }
        let c = std::str::from_utf8(&bytes[..n])
            .map_err(|e| invalid(e.to_string()))?
            .chars()
            .next()
            .unwrap();
        Ok((c, pos + n as u64))
    }
    pub fn starts(&mut self, pos: u64, text: &[u8]) -> TextDocumentResult<bool> {
        if pos + text.len() as u64 > self.len {
            return Ok(false);
        }
        for (i, b) in text.iter().enumerate() {
            if self.byte(pos + i as u64)? != *b {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn find(&mut self, mut pos: u64, end: u64, text: &[u8]) -> TextDocumentResult<Option<u64>> {
        while pos + text.len() as u64 <= end {
            if pos % CHUNK as u64 == 0 {
                check(&self.cancel)?;
            }
            if self.starts(pos, text)? {
                return Ok(Some(pos));
            }
            pos += 1;
        }
        Ok(None)
    }
    pub fn small(&mut self, from: u64, to: u64) -> TextDocumentResult<String> {
        if to < from || to - from > CHUNK as u64 {
            return Err(invalid("统计读取范围超过 64 KiB"));
        }
        let mut text = Vec::with_capacity((to - from) as usize);
        for p in from..to {
            text.push(self.byte(p)?);
        }
        String::from_utf8(text).map_err(|e| invalid(e.to_string()))
    }
    pub fn line(&mut self, start: u64) -> TextDocumentResult<Line> {
        let mut end = start;
        while end < self.len && !matches!(self.byte(end)?, b'\r' | b'\n') {
            end += 1;
        }
        let mut next = end;
        if next < self.len {
            next += 1;
            if self.byte(end)? == b'\r' && self.byte(next)? == b'\n' {
                next += 1;
            }
        }
        Ok(Line { start, end, next })
    }
    pub fn trim(&mut self, mut from: u64, mut to: u64) -> TextDocumentResult<(u64, u64)> {
        while from < to && self.byte(from)?.is_ascii_whitespace() {
            from += 1;
        }
        while to > from && self.byte(to - 1)?.is_ascii_whitespace() {
            to -= 1;
        }
        Ok((from, to))
    }
    pub fn raw_counts(&mut self, from: u64, to: u64) -> TextDocumentResult<(u64, u64)> {
        let mut pos = from;
        let mut chars = 0;
        let mut lines = 0;
        while pos < to {
            let (c, next) = self.char(pos)?;
            chars += c.len_utf16() as u64;
            if c == '\n' || (c == '\r' && self.byte(next)? != b'\n') {
                lines += 1;
            }
            pos = next;
        }
        Ok((chars, lines))
    }
}
#[derive(Clone, Copy)]
pub(super) struct Line {
    pub start: u64,
    pub end: u64,
    pub next: u64,
}

//! Markdown 字面量搜索：工作正文始终留在 Rust，检查点只保存可恢复的扫描位置。
use super::{SearchMatch, SearchQuery};
use crate::text_document::{chunk_reader::DocumentSnapshot, TextDocumentError, TextDocumentResult};
use regex::{Regex, RegexBuilder};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};

pub(super) const CHUNK_BYTES: usize = 64 * 1024;
pub(super) const CHECKPOINT_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::text_document) struct Checkpoint {
    pub byte: u64,
    pub count: u64,
}

pub(in crate::text_document) struct Matcher {
    expression: Option<Regex>,
    margin: usize,
    whole_word: bool,
}
pub(in crate::text_document) struct ScanResult {
    pub count: u64,
    pub checkpoints: Vec<Checkpoint>,
}

pub(in crate::text_document) fn check_cancel(cancel: &AtomicBool) -> TextDocumentResult<()> {
    if cancel.load(Ordering::Acquire) {
        Err(TextDocumentError::new(
            "task-cancelled",
            "搜索替换任务已取消",
        ))
    } else {
        Ok(())
    }
}

fn word(c: char) -> bool {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let mut bytes = [0; 4];
    WORD.get_or_init(|| Regex::new(r"^[\p{L}\p{N}_]$").expect("constant word class"))
        .is_match(c.encode_utf8(&mut bytes))
}

impl Matcher {
    pub fn new(query: &SearchQuery) -> TextDocumentResult<Self> {
        let expression = if query.query.is_empty() {
            None
        } else {
            Some(
                RegexBuilder::new(&regex::escape(&query.query))
                    .case_insensitive(!query.case_sensitive)
                    .unicode(true)
                    .build()
                    .map_err(|e| TextDocumentError::new("search-query-invalid", e.to_string()))?,
            )
        };
        // Unicode simple folding 一对一映射标量，但 UTF-8 字节长度可能改变。
        let margin = query
            .query
            .chars()
            .count()
            .saturating_mul(4)
            .saturating_add(8);
        Ok(Self {
            expression,
            margin,
            whole_word: query.whole_word,
        })
    }

    pub fn scan(
        &self,
        snapshot: &DocumentSnapshot,
        start: Checkpoint,
        cancel: &AtomicBool,
        collect_checkpoints: bool,
        mut found: impl FnMut(SearchMatch, &str) -> TextDocumentResult<bool>,
        mut progress: impl FnMut(u64, u64),
    ) -> TextDocumentResult<ScanResult> {
        let mut result = ScanResult {
            count: start.count,
            checkpoints: vec![],
        };
        let Some(expression) = &self.expression else {
            progress(snapshot.len(), 0);
            return Ok(result);
        };
        let stride = CHUNK_BYTES as u64;
        let stride = stride.max(snapshot.len().div_ceil((CHECKPOINT_LIMIT - 1) as u64));
        let mut next_checkpoint = start.byte.saturating_add(stride);
        if collect_checkpoints {
            result.checkpoints.push(start);
        }
        // 只回读一个 Unicode 标量，用于恢复检查点处的全词边界。
        let prefix = start.byte.saturating_sub(4);
        let prefix_bytes = snapshot.read_range(prefix, (start.byte - prefix) as usize)?;
        let skip = prefix_bytes
            .iter()
            .position(|b| b & 0xc0 != 0x80)
            .unwrap_or(prefix_bytes.len());
        let mut buffer = prefix_bytes[skip..].to_vec();
        let mut base = prefix + skip as u64;
        let mut read = start.byte;
        let mut next = start.byte;
        loop {
            check_cancel(cancel)?;
            if read < snapshot.len() {
                let bytes = snapshot.read_range(read, CHUNK_BYTES)?;
                if bytes.is_empty() {
                    return Err(TextDocumentError::new("unexpected-eof", "搜索快照提前结束"));
                }
                read += bytes.len() as u64;
                buffer.extend_from_slice(&bytes);
            }
            let complete = read == snapshot.len();
            let valid = match std::str::from_utf8(&buffer) {
                Ok(_) => buffer.len(),
                Err(e) if !complete && e.error_len().is_none() => e.valid_up_to(),
                Err(_) => {
                    return Err(TextDocumentError::new(
                        "invalid-markdown-encoding",
                        "搜索快照不是 UTF-8",
                    ))
                }
            };
            let text = std::str::from_utf8(&buffer[..valid]).expect("validated UTF-8");
            let mut safe_end = if complete {
                valid
            } else {
                valid.saturating_sub(self.margin)
            };
            while !text.is_char_boundary(safe_end) {
                safe_end -= 1;
            }
            let mut consumed = (next - base) as usize;
            let mut checked = 0;
            while let Some(m) = expression.find_at(text, consumed) {
                if !complete && m.start() >= safe_end {
                    break;
                }
                consumed = m.end();
                checked += 1;
                if checked % 256 == 0 {
                    check_cancel(cancel)?;
                }
                if self.whole_word
                    && (text[..m.start()].chars().next_back().is_some_and(word)
                        || text[m.end()..].chars().next().is_some_and(word))
                {
                    continue;
                }
                let prefix = m.as_str().starts_with('\n')
                    && text.as_bytes().get(m.start().wrapping_sub(1)) == Some(&b'\r');
                let suffix =
                    m.as_str().ends_with('\r') && text.as_bytes().get(m.end()) == Some(&b'\n');
                let item = SearchMatch {
                    index: result.count,
                    from_byte: base + m.start() as u64 - u64::from(prefix),
                    to_byte: base + m.end() as u64 + u64::from(suffix),
                    preserved_prefix: if prefix { "\r".into() } else { String::new() },
                    preserved_suffix: if suffix { "\n".into() } else { String::new() },
                };
                result.count += 1;
                if !found(item, m.as_str())? {
                    return Ok(result);
                }
            }
            consumed = consumed.max(safe_end);
            next = base + consumed as u64;
            if collect_checkpoints
                && next >= next_checkpoint
                && result.checkpoints.len() < CHECKPOINT_LIMIT
            {
                result.checkpoints.push(Checkpoint {
                    byte: next,
                    count: result.count,
                });
                next_checkpoint = next.saturating_add(stride);
            }
            progress(read, result.count);
            if complete {
                check_cancel(cancel)?;
                return Ok(result);
            }
            let keep = text[..consumed]
                .char_indices()
                .next_back()
                .map_or(0, |(at, _)| at);
            buffer.drain(..keep);
            base += keep as u64;
        }
    }
}

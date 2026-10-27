//! 单个打开文档的编辑状态：正文、块索引、选区、撤销历史、搜索结果和统计。
//!
//! 生命周期：随标签创建，随标签关闭释放；视图通过 `Entity<Buffer>` 共享同一份状态，
//! 分栏的两侧视图因此天然同步选区与内容。所有方法只在 GPUI 主线程调用，
//! 耗时的全文搜索与字数统计在后台线程处理快照，结果按 revision 校验后才写回。

use crate::document::{Document, EditRecord};
use crate::markdown::blocks::{BlockIndex, Heading, SnapshotLines};
use gpui::{Context, EventEmitter, Task};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

pub enum BufferEvent {
    Edited,
    SelectionChanged,
    Saved,
    SearchUpdated,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// 可与相邻输入合并为一个撤销步骤的普通键入。
    Typing,
    Other,
}

struct UndoGroup {
    records: Vec<EditRecord>,
    selection_before: (u64, u64),
    kind: EditKind,
    at: Instant,
}

/// 连续键入合并为一个撤销步骤的最长间隔。
const TYPING_GROUP_WINDOW: Duration = Duration::from_millis(1200);
/// 搜索结果上限：超过后停止收集并提示，避免极端查询占用大量内存。
const MAX_SEARCH_MATCHES: usize = 100_000;

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    pub matches: Vec<Range<u64>>,
    pub current: Option<usize>,
    pub truncated: bool,
    pub pending: bool,
    task: Option<Task<()>>,
}

/// 写作统计，对应 Web 版状态栏的四种统计类型。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub lines: usize,
    /// 汉字按字计、其他文字按词计。
    pub words: usize,
    /// 不含空白的字符数。
    pub visible_chars: usize,
    pub chars: usize,
}

pub struct Buffer {
    pub doc: Document,
    pub path: Option<PathBuf>,
    pub blocks: BlockIndex,
    pub revision: u64,
    saved_revision: u64,
    pub anchor: u64,
    pub cursor: u64,
    /// 输入法组字区间（全文字节偏移）。
    pub marked: Option<Range<u64>>,
    compose_origin: Option<(u64, String, (u64, u64))>,
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    outline: Option<(u64, Rc<Vec<Heading>>)>,
    pub search: SearchState,
    pub stats: Option<(u64, Stats)>,
    stats_task: Option<Task<()>>,
    /// 无法按 UTF-8 解码的文件只读打开，避免保存时破坏原始字节。
    pub read_only: Option<String>,
}

impl EventEmitter<BufferEvent> for Buffer {}

impl Buffer {
    pub fn new(bytes: Vec<u8>, path: Option<PathBuf>) -> Self {
        let read_only = std::str::from_utf8(&bytes).err().map(|_| "文件不是 UTF-8 编码，已只读打开".to_string());
        let doc = Document::from_bytes(bytes);
        let blocks = BlockIndex::build(&SnapshotLines::new(doc.read(0, doc.len())));
        Self {
            doc,
            path,
            blocks,
            revision: 0,
            saved_revision: 0,
            anchor: 0,
            cursor: 0,
            marked: None,
            compose_origin: None,
            undo: Vec::new(),
            redo: Vec::new(),
            outline: None,
            search: SearchState::default(),
            stats: None,
            stats_task: None,
            read_only,
        }
    }

    pub fn open(path: &Path) -> std::io::Result<Self> {
        Ok(Self::new(std::fs::read(path)?, Some(path.to_path_buf())))
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(|| "未命名.md".to_string(), |name| name.to_string_lossy().into_owned())
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn base_dir(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|path| path.parent().map(Path::to_path_buf))
    }

    pub fn selection(&self) -> Range<u64> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    pub fn selected_text(&self) -> String {
        let range = self.selection();
        self.doc.read(range.start, range.end)
    }

    pub fn set_selection(&mut self, anchor: u64, cursor: u64, cx: &mut Context<Self>) {
        let len = self.doc.len();
        self.anchor = anchor.min(len);
        self.cursor = cursor.min(len);
        cx.emit(BufferEvent::SelectionChanged);
        cx.notify();
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.doc.line_of_offset(self.cursor);
        let start = self.doc.line_range(line).start;
        let col = self.doc.read(start, self.cursor).chars().count();
        (line + 1, col + 1)
    }

    // ---- 编辑 ----

    /// 替换正文并同步块索引；不处理撤销记录与选区。
    fn replace_raw(&mut self, range: Range<u64>, text: &str) -> EditRecord {
        let first_line = self.doc.line_of_offset(range.start);
        let old_last = self.doc.line_of_offset(range.end);
        let record = self.doc.replace(range.start, range.end, text).expect("编辑区间越界");
        let new_last = self.doc.line_of_offset(range.start + text.len() as u64);
        self.blocks.apply_edit(&self.doc, first_line, old_last, new_last);
        self.revision += 1;
        record
    }

    pub fn edit(&mut self, range: Range<u64>, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        if self.read_only.is_some() {
            return;
        }
        let selection_before = (self.anchor, self.cursor);
        let record = self.replace_raw(range.clone(), text);
        let now = Instant::now();
        let mergeable = kind == EditKind::Typing
            && !text.contains('\n')
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|group| {
                group.kind == EditKind::Typing
                    && now.duration_since(group.at) < TYPING_GROUP_WINDOW
                    && group.records.last().is_some_and(|last| last.from + last.inserted_len == record.from)
            });
        if mergeable {
            let group = self.undo.last_mut().expect("checked");
            group.records.push(record);
            group.at = now;
        } else {
            self.undo.push(UndoGroup { records: vec![record], selection_before, kind, at: now });
        }
        self.redo.clear();
        let caret = range.start + text.len() as u64;
        self.anchor = caret;
        self.cursor = caret;
        self.after_edit(cx);
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        if !self.search.query.is_empty() {
            self.schedule_search(true, cx);
        }
        self.schedule_stats(cx);
        cx.emit(BufferEvent::Edited);
        cx.notify();
    }

    fn replay(&mut self, from_undo: bool, cx: &mut Context<Self>) {
        let source = if from_undo { &mut self.undo } else { &mut self.redo };
        let Some(group) = source.pop() else { return };
        let mut inverse = Vec::with_capacity(group.records.len());
        let mut caret = self.cursor;
        for record in group.records.iter().rev() {
            let applied = self.replace_raw(record.from..record.from + record.inserted_len, &record.removed);
            caret = record.from + record.removed.len() as u64;
            inverse.push(applied);
        }
        let selection_now = (self.anchor, self.cursor);
        let (anchor, cursor) = if from_undo { group.selection_before } else { (caret, caret) };
        self.anchor = anchor.min(self.doc.len());
        self.cursor = cursor.min(self.doc.len());
        let restored = UndoGroup { records: inverse, selection_before: selection_now, kind: EditKind::Other, at: Instant::now() };
        if from_undo { self.redo.push(restored) } else { self.undo.push(restored) }
        self.after_edit(cx);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        self.compose_origin = None;
        self.marked = None;
        self.replay(true, cx);
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        self.replay(false, cx);
    }

    // ---- 输入法组字 ----

    /// 设置组字文本；组字期间的中间状态不进入撤销历史。
    pub fn set_marked(&mut self, range: Range<u64>, text: &str, selected: Option<Range<usize>>, cx: &mut Context<Self>) {
        if self.read_only.is_some() {
            return;
        }
        if self.compose_origin.is_none() {
            self.compose_origin = Some((range.start, self.doc.read(range.start, range.end), (self.anchor, self.cursor)));
        }
        self.replace_raw(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len() as u64);
        let (anchor, cursor) = match selected {
            Some(selected) => (range.start + selected.start as u64, range.start + selected.end as u64),
            None => (range.start + text.len() as u64, range.start + text.len() as u64),
        };
        self.anchor = anchor;
        self.cursor = cursor;
        if self.marked.is_none() {
            // 组字被清空：等价于取消，原选中文本若非空则需可撤销恢复。
            if let Some((from, removed, selection)) = self.compose_origin.take() {
                if !removed.is_empty() {
                    let record = EditRecord { from, removed, inserted_len: 0 };
                    self.undo.push(UndoGroup { records: vec![record], selection_before: selection, kind: EditKind::Other, at: Instant::now() });
                }
            }
        }
        cx.emit(BufferEvent::Edited);
        cx.notify();
    }

    /// 提交文本；若处于组字状态，则把“组字前原文 → 最终文本”记为一个撤销步骤。
    pub fn commit_text(&mut self, range: Range<u64>, text: &str, cx: &mut Context<Self>) {
        if let Some((from, removed, selection)) = self.compose_origin.take() {
            if self.read_only.is_some() {
                return;
            }
            self.replace_raw(range.clone(), text);
            let inserted_len = range.start + text.len() as u64 - from;
            self.undo.push(UndoGroup { records: vec![EditRecord { from, removed, inserted_len }], selection_before: selection, kind: EditKind::Other, at: Instant::now() });
            self.redo.clear();
            self.marked = None;
            let caret = range.start + text.len() as u64;
            self.anchor = caret;
            self.cursor = caret;
            self.after_edit(cx);
        } else {
            self.marked = None;
            self.edit(range, text, EditKind::Typing, cx);
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
        self.compose_origin = None;
    }

    // ---- 结构化编辑 ----

    /// 回车：列表项自动续写标记，空列表项回车则移除标记。
    pub fn newline(&mut self, cx: &mut Context<Self>) {
        let selection = self.selection();
        let line = self.doc.line_of_offset(selection.start);
        let line_range = self.doc.line_range(line);
        let text = self.doc.line_text(line);
        if let Some((prefix, next_prefix, content_empty)) = list_continuation(&text) {
            if content_empty && selection.is_empty() && selection.start == line_range.end {
                self.edit(line_range.start..line_range.end, "", EditKind::Other, cx);
                return;
            }
            let _ = prefix;
            self.edit(selection, &format!("\n{next_prefix}"), EditKind::Other, cx);
            return;
        }
        self.edit(selection, "\n", EditKind::Other, cx);
    }

    /// 用标记包裹选区（加粗、斜体等）；无选区时插入成对标记并把光标放在中间。
    pub fn wrap_selection(&mut self, left: &str, right: &str, cx: &mut Context<Self>) {
        let selection = self.selection();
        let inner = self.doc.read(selection.start, selection.end);
        // 已被同一标记包裹时取消包裹。
        let before = selection.start.checked_sub(left.len() as u64).map(|start| self.doc.read(start, selection.start));
        let after_end = (selection.end + right.len() as u64).min(self.doc.len());
        let after = self.doc.read(selection.end, after_end);
        if !inner.is_empty() && before.as_deref() == Some(left) && after == right {
            let start = selection.start - left.len() as u64;
            self.edit(start..after_end, &inner, EditKind::Other, cx);
            self.anchor = start;
            self.cursor = start + inner.len() as u64;
            cx.notify();
            return;
        }
        self.edit(selection.clone(), &format!("{left}{inner}{right}"), EditKind::Other, cx);
        self.anchor = selection.start + left.len() as u64;
        self.cursor = self.anchor + inner.len() as u64;
        cx.emit(BufferEvent::SelectionChanged);
        cx.notify();
    }

    /// 对选区覆盖的每一行切换行首前缀（引用、列表、任务）。
    pub fn toggle_line_prefix(&mut self, prefix: &str, cx: &mut Context<Self>) {
        let selection = self.selection();
        let first = self.doc.line_of_offset(selection.start);
        let last = self.doc.line_of_offset(selection.end);
        let start = self.doc.line_range(first).start;
        let end = self.doc.line_range(last).end;
        let text = self.doc.read(start, end);
        let all_have = text.lines().all(|line| line.starts_with(prefix));
        let replaced: Vec<String> = text
            .split('\n')
            .enumerate()
            .map(|(index, line)| {
                let body = strip_block_prefix(line);
                if all_have {
                    line.strip_prefix(prefix).unwrap_or(line).to_string()
                } else if prefix == "1. " {
                    format!("{}. {body}", index + 1)
                } else {
                    format!("{prefix}{body}")
                }
            })
            .collect();
        let joined = replaced.join("\n");
        self.edit(start..end, &joined, EditKind::Other, cx);
        self.anchor = start;
        self.cursor = start + joined.len() as u64;
        cx.notify();
    }

    pub fn set_heading(&mut self, level: u8, cx: &mut Context<Self>) {
        let line = self.doc.line_of_offset(self.cursor);
        let range = self.doc.line_range(line);
        let text = self.doc.line_text(line);
        let body = text.trim_start_matches('#').trim_start();
        let current = text.bytes().take_while(|byte| *byte == b'#').count() as u8;
        let replaced = if current == level || level == 0 { body.to_string() } else { format!("{} {body}", "#".repeat(level as usize)) };
        self.edit(range.clone(), &replaced, EditKind::Other, cx);
    }

    /// 在光标所在行之后插入一个块（代码块、表格、分隔线、公式）。
    pub fn insert_block(&mut self, block: &str, caret_offset: usize, cx: &mut Context<Self>) {
        let line = self.doc.line_of_offset(self.cursor);
        let range = self.doc.line_range(line);
        let current = self.doc.line_text(line);
        let (at, text) = if current.trim().is_empty() {
            (range.start..range.end, block.to_string())
        } else {
            (range.end..range.end, format!("\n\n{block}"))
        };
        let start = at.start;
        let lead = text.len() - block.len();
        self.edit(at, &text, EditKind::Other, cx);
        let caret = start + (lead + caret_offset) as u64;
        self.anchor = caret;
        self.cursor = caret;
        cx.notify();
    }

    pub fn toggle_task(&mut self, at: u64, cx: &mut Context<Self>) {
        let current = self.doc.read(at, (at + 3).min(self.doc.len()));
        let next = match current.as_str() {
            "[ ]" => "[x]",
            "[x]" | "[X]" => "[ ]",
            _ => return,
        };
        let selection = (self.anchor, self.cursor);
        self.edit(at..at + 3, next, EditKind::Other, cx);
        self.anchor = selection.0;
        self.cursor = selection.1;
        cx.notify();
    }

    /// 缩进或反缩进选区所在行（两个空格），用于列表层级调整。
    pub fn indent_lines(&mut self, outdent: bool, cx: &mut Context<Self>) {
        let selection = self.selection();
        let first = self.doc.line_of_offset(selection.start);
        let last = self.doc.line_of_offset(selection.end);
        let start = self.doc.line_range(first).start;
        let end = self.doc.line_range(last).end;
        let text = self.doc.read(start, end);
        let changed: Vec<String> = text
            .split('\n')
            .map(|line| {
                if outdent {
                    line.strip_prefix("  ").or_else(|| line.strip_prefix('\t')).unwrap_or(line).to_string()
                } else {
                    format!("  {line}")
                }
            })
            .collect();
        let joined = changed.join("\n");
        let delta = joined.len() as i64 - text.len() as i64;
        let cursor = (self.cursor as i64 + if first == last { delta } else { 0 }).max(start as i64) as u64;
        self.edit(start..end, &joined, EditKind::Other, cx);
        if first == last {
            self.anchor = cursor;
            self.cursor = cursor;
        } else {
            self.anchor = start;
            self.cursor = start + joined.len() as u64;
        }
        cx.notify();
    }

    // ---- 边界计算 ----

    pub fn prev_grapheme(&self, offset: u64) -> u64 {
        if offset == 0 {
            return 0;
        }
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset <= range.start {
            return self.doc.line_range(line - 1).end;
        }
        let text = self.doc.read(range.start, offset);
        range.start + text.grapheme_indices(true).last().map_or(0, |(index, _)| index) as u64
    }

    pub fn next_grapheme(&self, offset: u64) -> u64 {
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset >= range.end {
            return if line + 1 < self.doc.line_count() { self.doc.line_range(line + 1).start } else { offset };
        }
        let text = self.doc.read(offset, range.end);
        offset + text.graphemes(true).next().map_or(0, str::len) as u64
    }

    /// 词边界：汉字逐字、拉丁按 Unicode 词，跨行时退化为字素移动。
    pub fn prev_word(&self, offset: u64) -> u64 {
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset <= range.start {
            return self.prev_grapheme(offset);
        }
        let text = self.doc.read(range.start, offset);
        let target = text
            .split_word_bound_indices()
            .filter(|(_, word)| !word.trim().is_empty())
            .last()
            .map_or(0, |(index, _)| index);
        range.start + target as u64
    }

    pub fn next_word(&self, offset: u64) -> u64 {
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset >= range.end {
            return self.next_grapheme(offset);
        }
        let text = self.doc.read(offset, range.end);
        let target = text
            .split_word_bound_indices()
            .find(|(_, word)| !word.trim().is_empty())
            .map_or(text.len(), |(index, word)| index + word.len());
        offset + target as u64
    }

    // ---- 大纲 ----

    pub fn outline(&mut self) -> Rc<Vec<Heading>> {
        if let Some((revision, headings)) = &self.outline {
            if *revision == self.revision {
                return headings.clone();
            }
        }
        let headings = Rc::new(self.blocks.headings(&self.doc));
        self.outline = Some((self.revision, headings.clone()));
        headings
    }

    // ---- 保存 ----

    /// 保存到 `path`。`snapshot` 为真且目标文件已存在时，先把旧内容复制到本地快照目录。
    pub fn save_to(&mut self, path: PathBuf, snapshot: bool, cx: &mut Context<Self>) -> std::io::Result<()> {
        if let Some(reason) = &self.read_only {
            return Err(std::io::Error::other(reason.clone()));
        }
        if snapshot && path.exists() {
            if let Err(error) = write_snapshot(&path) {
                eprintln!("保存前快照失败：{error}");
            }
        }
        let text = self.doc.read(0, self.doc.len());
        std::fs::write(&path, text.as_bytes())?;
        self.path = Some(path);
        self.saved_revision = self.revision;
        cx.emit(BufferEvent::Saved);
        cx.notify();
        Ok(())
    }

    // ---- 搜索 ----

    pub fn set_search(&mut self, query: String, case_sensitive: bool, cx: &mut Context<Self>) {
        if query == self.search.query && case_sensitive == self.search.case_sensitive {
            return;
        }
        self.search.query = query;
        self.search.case_sensitive = case_sensitive;
        self.schedule_search(false, cx);
    }

    /// 在后台线程对全文快照查找；`debounce` 用于编辑触发的重查，合并连续键入。
    fn schedule_search(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let query = self.search.query.clone();
        if query.is_empty() {
            self.search.matches.clear();
            self.search.current = None;
            self.search.truncated = false;
            self.search.pending = false;
            self.search.task = None;
            cx.emit(BufferEvent::SearchUpdated);
            cx.notify();
            return;
        }
        self.search.pending = true;
        let case_sensitive = self.search.case_sensitive;
        let revision = self.revision;
        self.search.task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(Duration::from_millis(250)).await;
            }
            let Ok(text) = this.read_with(cx, |buffer, _| buffer.doc.read(0, buffer.doc.len())) else { return };
            let (matches, truncated) = cx
                .background_executor()
                .spawn(async move { find_all(&text, &query, case_sensitive) })
                .await;
            this.update(cx, |buffer, cx| {
                if buffer.revision != revision {
                    return;
                }
                let cursor = buffer.selection().start;
                buffer.search.current = (!matches.is_empty()).then(|| matches.partition_point(|range| range.start < cursor) % matches.len());
                buffer.search.matches = matches;
                buffer.search.truncated = truncated;
                buffer.search.pending = false;
                cx.emit(BufferEvent::SearchUpdated);
                cx.notify();
            })
            .ok();
        }));
    }

    /// 切换到下一个或上一个匹配并选中它。
    pub fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) -> Option<Range<u64>> {
        let count = self.search.matches.len();
        if count == 0 {
            return None;
        }
        let current = match self.search.current {
            Some(current) if self.selection() == self.search.matches[current] => {
                if forward { (current + 1) % count } else { (current + count - 1) % count }
            }
            Some(current) => current,
            None => 0,
        };
        self.search.current = Some(current);
        let range = self.search.matches[current].clone();
        self.anchor = range.start;
        self.cursor = range.end;
        cx.emit(BufferEvent::SelectionChanged);
        cx.notify();
        Some(range)
    }

    pub fn replace_current(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(current) = self.search.current else { return };
        let Some(range) = self.search.matches.get(current).cloned() else { return };
        self.edit(range, replacement, EditKind::Other, cx);
    }

    /// 全部替换作为一个撤销步骤：从后往前替换，前面的偏移不受影响。
    pub fn replace_all(&mut self, replacement: &str, cx: &mut Context<Self>) -> usize {
        if self.read_only.is_some() || self.search.matches.is_empty() || self.search.pending {
            return 0;
        }
        let matches = std::mem::take(&mut self.search.matches);
        let selection_before = (self.anchor, self.cursor);
        let mut records = Vec::with_capacity(matches.len());
        for range in matches.iter().rev() {
            records.push(self.replace_raw(range.clone(), replacement));
        }
        self.undo.push(UndoGroup { records, selection_before, kind: EditKind::Other, at: Instant::now() });
        self.redo.clear();
        let count = matches.len();
        self.anchor = self.anchor.min(self.doc.len());
        self.cursor = self.anchor;
        self.after_edit(cx);
        count
    }

    // ---- 统计 ----

    fn schedule_stats(&mut self, cx: &mut Context<Self>) {
        let revision = self.revision;
        self.stats_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let Ok(text) = this.read_with(cx, |buffer, _| buffer.doc.read(0, buffer.doc.len())) else { return };
            let stats = cx.background_executor().spawn(async move { count_words(&text) }).await;
            this.update(cx, |buffer, cx| {
                if buffer.revision == revision {
                    buffer.stats = Some((revision, stats));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub fn ensure_stats(&mut self, cx: &mut Context<Self>) {
        if self.stats.is_none() && self.stats_task.is_none() {
            self.schedule_stats(cx);
        }
    }
}

/// 列表续写：返回（当前前缀、下一项前缀、内容是否为空）。
fn list_continuation(line: &str) -> Option<(String, String, bool)> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let mut rest = &line[indent_len..];
    let mut quote = String::new();
    while let Some(after) = rest.strip_prefix("> ") {
        quote.push_str("> ");
        rest = after;
    }
    for bullet in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "- ", "* ", "+ "] {
        if let Some(content) = rest.strip_prefix(bullet) {
            let next = if bullet.contains('[') { format!("{}[ ] ", &bullet[..2]) } else { bullet.to_string() };
            return Some((format!("{indent}{quote}{bullet}"), format!("{indent}{quote}{next}"), content.trim().is_empty()));
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && rest[digits..].starts_with(". ") {
        let number: u64 = rest[..digits].parse().ok()?;
        let content = &rest[digits + 2..];
        return Some((format!("{indent}{quote}{number}. "), format!("{indent}{quote}{}. ", number + 1), content.trim().is_empty()));
    }
    if !quote.is_empty() {
        return Some((format!("{indent}{quote}"), format!("{indent}{quote}"), rest.trim().is_empty()));
    }
    None
}

fn strip_block_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    for prefix in ["- [ ] ", "- [x] ", "> ", "- ", "* "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest;
        }
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && trimmed[digits..].starts_with(". ") {
        return &trimmed[digits + 2..];
    }
    trimmed
}

/// 全文查找。大小写不敏感时只折叠 ASCII，保证字节偏移与原文一致。
pub fn find_all(text: &str, query: &str, case_sensitive: bool) -> (Vec<Range<u64>>, bool) {
    if query.is_empty() {
        return (Vec::new(), false);
    }
    let mut matches = Vec::new();
    let (haystack, needle) = if case_sensitive {
        (std::borrow::Cow::Borrowed(text), std::borrow::Cow::Borrowed(query))
    } else {
        (std::borrow::Cow::Owned(text.to_ascii_lowercase()), std::borrow::Cow::Owned(query.to_ascii_lowercase()))
    };
    let mut from = 0;
    while let Some(found) = haystack[from..].find(needle.as_ref()) {
        let start = from + found;
        matches.push(start as u64..(start + needle.len()) as u64);
        if matches.len() >= MAX_SEARCH_MATCHES {
            return (matches, true);
        }
        from = start + needle.len().max(1);
    }
    (matches, false)
}

/// 每个文件保留的快照数量上限，超出后删除最旧的。
const MAX_SNAPSHOTS_PER_FILE: usize = 20;

/// 把磁盘上的旧版本复制到 `gpui-snapshots/<文件名>/<时间戳>.md`。
pub(crate) fn write_snapshot(path: &Path) -> std::io::Result<()> {
    let root = crate::settings::snapshot_dir().ok_or_else(|| std::io::Error::other("无法确定快照目录"))?;
    let name = path.file_name().map_or_else(|| "untitled".into(), |name| name.to_string_lossy().into_owned());
    let dir = root.join(name);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis());
    std::fs::copy(path, dir.join(format!("{stamp}.md")))?;
    let mut existing: Vec<_> = std::fs::read_dir(&dir)?.filter_map(Result::ok).map(|entry| entry.path()).collect();
    if existing.len() > MAX_SNAPSHOTS_PER_FILE {
        existing.sort();
        for old in &existing[..existing.len() - MAX_SNAPSHOTS_PER_FILE] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(())
}

pub fn count_words(text: &str) -> Stats {
    let mut words = 0;
    let mut visible_chars = 0;
    let mut chars = 0;
    let mut in_word = false;
    for ch in text.chars() {
        chars += 1;
        if !ch.is_whitespace() {
            visible_chars += 1;
        }
        let cjk = matches!(ch as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F | 0x3040..=0x30FF);
        if cjk {
            words += 1;
            in_word = false;
        } else if ch.is_alphanumeric() {
            if !in_word {
                words += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    let lines = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    Stats { lines, words, visible_chars, chars }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_continuation_handles_markers() {
        assert_eq!(list_continuation("- 项").map(|(_, next, empty)| (next, empty)), Some(("- ".into(), false)));
        assert_eq!(list_continuation("  3. x").map(|(_, next, _)| next), Some("  4. ".into()));
        assert_eq!(list_continuation("- [x] 完成").map(|(_, next, _)| next), Some("- [ ] ".into()));
        assert_eq!(list_continuation("> 引用").map(|(_, next, _)| next), Some("> ".into()));
        assert!(list_continuation("正文").is_none());
    }

    #[test]
    fn search_folds_ascii_only() {
        let (matches, truncated) = find_all("Nomo nomo 诺莫 NOMO", "nomo", false);
        assert_eq!(matches.len(), 3);
        assert!(!truncated);
        assert_eq!(find_all("诺莫诺莫", "莫", true).0, vec![3..6, 9..12]);
    }

    #[test]
    fn word_count_mixes_cjk_and_latin() {
        assert_eq!(count_words("中文 hello world，测试").words, 6);
    }
}

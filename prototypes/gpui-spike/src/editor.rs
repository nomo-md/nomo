//! GPUI 编辑视图：按源码行（段落）虚拟滚动，段落整段塑形后用 KP 断行并逐字形绘制。
//!
//! 生命周期：`Editor` 由窗口根视图持有，随窗口关闭销毁；布局缓存只保存塑形结果与断行，
//! 不持有回调或窗口对象。所有状态仅在 GPUI 主线程访问。
//!
//! IME 坐标系：macOS 输入法以 UTF-16 偏移查询文本。全文 UTF-16 偏移需要额外索引，
//! 原型改为只向输入法暴露光标所在段落，所有 `EntityInputHandler` 方法都以段首为 0。

use crate::kp::{self, Item, ItemKind, Line};
use crate::text_document::{Document, EditRecord};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, Font, FontId,
    GlobalElementId, GlyphId, Hsla, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, Style, TextRun,
    UTF16Selection, Window, actions, div, fill, point, prelude::*, px, relative, rgb, size,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

actions!(
    nomo_spike,
    [
        Backspace, Delete, Left, Right, Up, Down, SelectAll, Newline, Undo, Redo, Copy, Paste,
        PageDown, PageUp, DocStart, DocEnd, ToggleBreakMode, Quit
    ]
);

pub const FONT_SIZE: f32 = 16.0;
pub const LINE_HEIGHT: f32 = 28.0;
const PADDING_X: f32 = 48.0;
const PADDING_Y: f32 = 24.0;
/// 布局缓存上限：条数与估算字节双重约束；1024 段约覆盖 20 屏，字节上限防止超长段落撑爆缓存。
const CACHE_MAX_ENTRIES: usize = 1024;
const CACHE_MAX_BYTES: usize = 48 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BreakMode {
    KnuthPlass,
    Greedy,
}

/// 塑形后的单个字形，`index` 为段内 UTF-8 字节偏移，`x` 为整段单行塑形时的横坐标。
#[derive(Clone, Copy)]
struct Glyph {
    index: usize,
    x: f32,
    font_id: FontId,
    id: GlyphId,
    emoji: bool,
}

/// 一行视觉行：段内字节 `start..end`，`glues` 为行内胶的 (生效字节, 累计偏移) 递增表。
struct VisualLine {
    start: usize,
    end: usize,
    origin_x: f32,
    glues: Vec<(usize, f32)>,
}

/// 一个段落的排版结果；缓存键为段落文本哈希与行宽，因此与段落所在行号无关。
pub struct ParaLayout {
    text: String,
    glyphs: Vec<Glyph>,
    width: f32,
    lines: Vec<VisualLine>,
    height: f32,
}

impl ParaLayout {
    fn estimated_bytes(&self) -> usize {
        self.text.len()
            + self.glyphs.len() * std::mem::size_of::<Glyph>()
            + self.lines.iter().map(|line| 48 + line.glues.len() * 16).sum::<usize>()
    }

    /// 整段塑形坐标：第一个 `index >= byte` 的字形横坐标，越过末尾取整段宽度。
    fn shaped_x(&self, byte: usize) -> f32 {
        let at = self.glyphs.partition_point(|glyph| glyph.index < byte);
        self.glyphs.get(at).map_or(self.width, |glyph| glyph.x)
    }

    fn line_for_offset(&self, offset: usize) -> usize {
        self.lines
            .iter()
            .position(|line| offset <= line.end)
            .unwrap_or(self.lines.len() - 1)
    }

    /// 段内字节在视觉行中的横坐标（相对内容区左缘）。
    fn x_in_line(&self, line: &VisualLine, offset: usize) -> f32 {
        let offset = offset.clamp(line.start, line.end);
        let extra = line
            .glues
            .iter()
            .take_while(|(at, _)| *at <= offset)
            .last()
            .map_or(0.0, |(_, shift)| *shift);
        self.shaped_x(offset) - line.origin_x + extra
    }

    fn caret(&self, offset: usize) -> (usize, f32) {
        let line_index = self.line_for_offset(offset);
        (line_index, self.x_in_line(&self.lines[line_index], offset))
    }

    fn offset_for_x(&self, line_index: usize, x: f32) -> usize {
        let line = &self.lines[line_index];
        let mut best = (line.start, f32::INFINITY);
        let from = self.glyphs.partition_point(|glyph| glyph.index < line.start);
        let candidates = self.glyphs[from..]
            .iter()
            .take_while(|glyph| glyph.index < line.end)
            .map(|glyph| glyph.index)
            .chain(std::iter::once(line.end));
        for offset in candidates {
            let distance = (self.x_in_line(line, offset) - x).abs();
            if distance < best.1 {
                best = (offset, distance);
            }
        }
        best.0
    }
}

#[derive(Default)]
struct LayoutCache {
    entries: HashMap<(u64, u32, BreakModeKey), (Rc<ParaLayout>, u64)>,
    bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
}

type BreakModeKey = bool;

impl LayoutCache {
    fn get(&mut self, key: &(u64, u32, BreakModeKey)) -> Option<Rc<ParaLayout>> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.1 = clock;
        self.hits += 1;
        Some(entry.0.clone())
    }

    fn insert(&mut self, key: (u64, u32, BreakModeKey), layout: Rc<ParaLayout>) {
        self.misses += 1;
        self.clock += 1;
        self.bytes += layout.estimated_bytes();
        if let Some((old, _)) = self.entries.insert(key, (layout, self.clock)) {
            self.bytes -= old.estimated_bytes();
        }
        // 淘汰最久未用的条目；n 不超过 1024，线性扫描的成本可忽略。
        while self.entries.len() > CACHE_MAX_ENTRIES || self.bytes > CACHE_MAX_BYTES {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(key, _)| *key) else {
                break;
            };
            if let Some((removed, _)) = self.entries.remove(&oldest) {
                self.bytes -= removed.estimated_bytes();
            }
            if self.entries.len() <= 1 {
                break;
            }
        }
    }
}

/// 上一帧可见段落，用于鼠标命中、输入法候选框定位和滚动夹紧。
struct VisibleParagraph {
    line: usize,
    start: u64,
    top: f32,
    layout: Rc<ParaLayout>,
}

/// 帧计时：`frame` 是本元素 prepaint+paint 的 CPU 时间，`interval` 是相邻两帧 prepaint 的间隔。
#[derive(Default)]
pub struct FrameStats {
    pub frame: Vec<Duration>,
    pub layout: Vec<Duration>,
    pub interval: Vec<Duration>,
    pub kp_time: Duration,
    pub kp_paragraphs: u64,
    pub kp_max: Duration,
    last_prepaint: Option<Instant>,
}

pub struct Editor {
    pub doc: Document,
    focus: FocusHandle,
    font: Font,
    /// 选区为 `anchor..cursor`（可反向），全文 UTF-8 字节偏移。
    cursor: u64,
    anchor: u64,
    marked: Option<Range<u64>>,
    /// 组字开始前被替换的原文，提交时合并成一个撤销步骤。
    compose_origin: Option<(u64, String)>,
    undo: Vec<EditRecord>,
    redo: Vec<EditRecord>,
    scroll_line: usize,
    scroll_offset: f32,
    pending_scroll: f32,
    pending_jump: Option<f64>,
    selecting: bool,
    cache: LayoutCache,
    visible: Vec<VisibleParagraph>,
    content_bounds: Option<Bounds<Pixels>>,
    space_width: Option<f32>,
    pub mode: BreakMode,
    pub stats: FrameStats,
    pub stats_enabled: bool,
    pub first_frame_at: Option<Instant>,
    /// 基准输入前的全文哈希，撤销后用于校验内容一致。
    pub bench_text_hash: Option<u64>,
}

impl Editor {
    pub fn new(doc: Document, mode: BreakMode, cx: &mut Context<Self>) -> Self {
        Self {
            doc,
            focus: cx.focus_handle(),
            font: gpui::font("PingFang SC"),
            cursor: 0,
            anchor: 0,
            marked: None,
            compose_origin: None,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll_line: 0,
            scroll_offset: 0.0,
            pending_scroll: 0.0,
            pending_jump: None,
            selecting: false,
            cache: LayoutCache::default(),
            visible: Vec::new(),
            content_bounds: None,
            space_width: None,
            mode,
            stats: FrameStats::default(),
            stats_enabled: false,
            first_frame_at: None,
            bench_text_hash: None,
        }
    }

    pub fn scroll_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        self.pending_scroll += dy;
        cx.notify();
    }

    pub fn jump_to_fraction(&mut self, fraction: f64, cx: &mut Context<Self>) {
        self.pending_jump = Some(fraction);
        cx.notify();
    }

    pub fn visible_text_contains(&self, needle: &str) -> bool {
        self.visible.iter().any(|paragraph| paragraph.layout.text.contains(needle))
    }

    pub fn cache_summary(&self) -> (usize, usize, u64, u64) {
        (self.cache.entries.len(), self.cache.bytes, self.cache.hits, self.cache.misses)
    }

    pub fn text(&self) -> String {
        self.doc.read(0, self.doc.len())
    }

    fn selection(&self) -> Range<u64> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    // ---- 排版 ----

    fn layout_paragraph(&mut self, line: usize, width: f32, window: &mut Window) -> Rc<ParaLayout> {
        let text = self.doc.line_text(line);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let key = (hasher.finish(), width.to_bits(), self.mode == BreakMode::KnuthPlass);
        if let Some(layout) = self.cache.get(&key) {
            return layout;
        }
        let started = Instant::now();
        let layout = Rc::new(self.shape_and_break(text, width, window));
        let elapsed = started.elapsed();
        self.stats.kp_time += elapsed;
        self.stats.kp_paragraphs += 1;
        self.stats.kp_max = self.stats.kp_max.max(elapsed);
        self.cache.insert(key, layout.clone());
        layout
    }

    fn shape_and_break(&mut self, text: String, width: f32, window: &mut Window) -> ParaLayout {
        let font_size = px(FONT_SIZE);
        let run = |len| TextRun {
            len,
            font: self.font.clone(),
            color: Hsla::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let space_width = *self.space_width.get_or_insert_with(|| {
            f32::from(window.text_system().layout_line(" ", font_size, &[run(1)], None).width)
        });
        // 整段只塑形一次；断行和对齐都在字形坐标上完成，不再按行重新塑形。
        let shaped = window
            .text_system()
            .layout_line(&text, font_size, &[run(text.len())], None);
        let glyphs: Vec<Glyph> = shaped
            .runs
            .iter()
            .flat_map(|shaped_run| {
                shaped_run.glyphs.iter().map(|glyph| Glyph {
                    index: glyph.index,
                    x: f32::from(glyph.position.x),
                    font_id: shaped_run.font_id,
                    id: glyph.id,
                    emoji: glyph.is_emoji,
                })
            })
            .collect();
        let total = f32::from(shaped.width);
        let mut layout = ParaLayout { text, glyphs, width: total, lines: Vec::new(), height: LINE_HEIGHT };
        let items = kp::build_items(&layout.text, FONT_SIZE, space_width, |byte| layout.shaped_x(byte));
        let lines = match self.mode {
            BreakMode::KnuthPlass => kp::break_lines(&items, width),
            BreakMode::Greedy => kp::greedy_lines(&items, width),
        };
        layout.lines = lines.iter().map(|line| visual_line(&layout, &items, line)).collect();
        if layout.lines.is_empty() {
            layout.lines.push(VisualLine { start: 0, end: 0, origin_x: 0.0, glues: Vec::new() });
        }
        layout.height = layout.lines.len() as f32 * LINE_HEIGHT;
        layout
    }

    /// 处理挂起的滚动与跳转，从锚点段落开始排版到视口底部，并在文末夹紧。
    fn layout_visible(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        let width = (f32::from(bounds.size.width) - PADDING_X * 2.0).max(80.0);
        let viewport = f32::from(bounds.size.height);
        let last_line = self.doc.line_count() - 1;
        if let Some(fraction) = self.pending_jump.take() {
            self.scroll_line = ((last_line as f64) * fraction).round() as usize;
            self.scroll_offset = 0.0;
        }
        self.scroll_offset += std::mem::take(&mut self.pending_scroll);
        // 正向越过锚点段落时前移锚点，反向时回退并补上前一段高度。
        loop {
            let height = self.layout_paragraph(self.scroll_line, width, window).height;
            if self.scroll_offset >= height && self.scroll_line < last_line {
                self.scroll_offset -= height;
                self.scroll_line += 1;
            } else {
                break;
            }
        }
        while self.scroll_offset < 0.0 && self.scroll_line > 0 {
            self.scroll_line -= 1;
            self.scroll_offset += self.layout_paragraph(self.scroll_line, width, window).height;
        }
        if self.scroll_line == 0 && self.scroll_offset < 0.0 {
            self.scroll_offset = 0.0;
        }

        let mut visible = Vec::new();
        let mut y = PADDING_Y - self.scroll_offset;
        let mut line = self.scroll_line;
        while y < viewport && line <= last_line {
            let layout = self.layout_paragraph(line, width, window);
            let height = layout.height;
            visible.push(VisibleParagraph { line, start: self.doc.line_range(line).start, top: y, layout });
            y += height;
            line += 1;
        }
        // 到达文末仍未填满视口：把锚点向前回退，避免文末上方出现大片空白。
        let gap = viewport - PADDING_Y - y;
        if line > last_line && gap > 0.0 && (self.scroll_line > 0 || self.scroll_offset > 0.0) {
            self.pending_scroll = -gap;
            self.scroll_offset += std::mem::take(&mut self.pending_scroll);
            while self.scroll_offset < 0.0 && self.scroll_line > 0 {
                self.scroll_line -= 1;
                self.scroll_offset += self.layout_paragraph(self.scroll_line, width, window).height;
            }
            self.scroll_offset = self.scroll_offset.max(0.0);
            visible.clear();
            let mut y = PADDING_Y - self.scroll_offset;
            let mut line = self.scroll_line;
            while y < viewport && line <= last_line {
                let layout = self.layout_paragraph(line, width, window);
                let height = layout.height;
                visible.push(VisibleParagraph { line, start: self.doc.line_range(line).start, top: y, layout });
                y += height;
                line += 1;
            }
        }
        self.visible = visible;
    }

    fn locate(&self, offset: u64) -> Option<(&VisibleParagraph, usize, f32)> {
        let paragraph = self.visible.iter().find(|paragraph| {
            let range = self.doc.line_range(paragraph.line);
            offset >= range.start && offset <= range.end
        })?;
        let local = (offset - paragraph.start) as usize;
        let (line_index, x) = paragraph.layout.caret(local);
        Some((paragraph, line_index, x))
    }

    fn hit_test(&self, position: Point<Pixels>) -> u64 {
        let Some(bounds) = self.content_bounds else { return self.cursor };
        let x = f32::from(position.x - bounds.origin.x) - PADDING_X;
        let y = f32::from(position.y - bounds.origin.y);
        let Some(paragraph) = self
            .visible
            .iter()
            .find(|paragraph| y < paragraph.top + paragraph.layout.height)
            .or(self.visible.last())
        else {
            return self.cursor;
        };
        let line_index = (((y - paragraph.top) / LINE_HEIGHT).floor().max(0.0) as usize)
            .min(paragraph.layout.lines.len() - 1);
        paragraph.start + paragraph.layout.offset_for_x(line_index, x) as u64
    }

    fn ensure_cursor_visible(&mut self) {
        let line = self.doc.line_of_offset(self.cursor);
        let shown = (self.visible.first().map(|first| first.line), self.visible.last().map(|last| last.line + 1));
        if let (Some(first), Some(end)) = shown {
            if line < first || line >= end.saturating_sub(1) {
                self.scroll_line = line.saturating_sub(3);
                self.scroll_offset = 0.0;
            }
        }
    }

    // ---- 编辑 ----

    fn apply(&mut self, range: Range<u64>, text: &str, record_undo: bool) {
        let record = self.doc.replace(range.start, range.end, text).expect("replace in bounds");
        if record_undo {
            self.undo.push(record);
            self.redo.clear();
        }
        self.cursor = range.start + text.len() as u64;
        self.anchor = self.cursor;
    }

    fn previous_boundary(&self, offset: u64) -> u64 {
        if offset == 0 {
            return 0;
        }
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset <= range.start {
            // 行首退格合并上一行：上一行的行尾（跳过 \r\n）。
            return self.doc.line_range(line - 1).end;
        }
        let text = self.doc.read(range.start, offset);
        use unicode_segmentation::UnicodeSegmentation;
        let last = text.grapheme_indices(true).last().map_or(0, |(index, _)| index);
        range.start + last as u64
    }

    fn next_boundary(&self, offset: u64) -> u64 {
        let line = self.doc.line_of_offset(offset);
        let range = self.doc.line_range(line);
        if offset >= range.end {
            return if line + 1 < self.doc.line_count() { self.doc.line_range(line + 1).start } else { offset };
        }
        let text = self.doc.read(offset, range.end);
        use unicode_segmentation::UnicodeSegmentation;
        let first = text.graphemes(true).next().map_or(0, str::len);
        offset + first as u64
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let mut range = self.selection();
        if range.is_empty() {
            range.start = self.previous_boundary(range.end);
        }
        self.apply(range, "", true);
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let mut range = self.selection();
        if range.is_empty() {
            range.end = self.next_boundary(range.start);
        }
        self.apply(range, "", true);
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(self.selection(), "\n", true);
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = self.previous_boundary(self.cursor);
        self.anchor = self.cursor;
        cx.notify();
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = self.next_boundary(self.cursor);
        self.anchor = self.cursor;
        cx.notify();
    }

    /// 按视觉行上下移动；目标行不在上一帧可见区时退化为按段落移动。
    fn vertical(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some((paragraph, line_index, x)) = self.locate(self.cursor) {
            let target_y = paragraph.top + line_index as f32 * LINE_HEIGHT
                + if down { LINE_HEIGHT * 1.5 } else { -LINE_HEIGHT * 0.5 };
            if let Some(bounds) = self.content_bounds {
                let position = point(bounds.origin.x + px(x + PADDING_X), bounds.origin.y + px(target_y));
                if target_y >= 0.0 && target_y < f32::from(bounds.size.height) {
                    self.cursor = self.hit_test(position);
                    self.anchor = self.cursor;
                    cx.notify();
                    return;
                }
            }
        }
        let line = self.doc.line_of_offset(self.cursor);
        let target = if down { (line + 1).min(self.doc.line_count() - 1) } else { line.saturating_sub(1) };
        self.cursor = self.doc.line_range(target).start;
        self.anchor = self.cursor;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.cursor = self.doc.len();
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_once(cx);
    }

    /// 光标移到上一帧可见区第一段段首，供基准在文档中部输入。
    pub fn place_cursor_at_visible_top(&mut self) {
        if let Some(first) = self.visible.get(1).or(self.visible.first()) {
            self.cursor = first.start;
            self.anchor = first.start;
        }
    }

    pub fn undo_once(&mut self, cx: &mut Context<Self>) {
        if let Some(record) = self.undo.pop() {
            let inverse = self
                .doc
                .replace(record.from, record.from + record.inserted_len, &record.removed)
                .expect("undo in bounds");
            self.cursor = record.from + record.removed.len() as u64;
            self.anchor = self.cursor;
            self.redo.push(inverse);
            self.ensure_cursor_visible();
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(record) = self.redo.pop() {
            let inverse = self
                .doc
                .replace(record.from, record.from + record.inserted_len, &record.removed)
                .expect("redo in bounds");
            self.cursor = record.from + record.removed.len() as u64;
            self.anchor = self.cursor;
            self.undo.push(inverse);
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.doc.read(range.start, range.end)));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.apply(self.selection(), &text, true);
            self.ensure_cursor_visible();
            cx.notify();
        }
    }

    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        let page = f32::from(window.viewport_size().height) * 0.9;
        self.scroll_by(page, cx);
    }

    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        let page = f32::from(window.viewport_size().height) * 0.9;
        self.scroll_by(-page, cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = 0;
        self.anchor = 0;
        self.jump_to_fraction(0.0, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = self.doc.len();
        self.anchor = self.cursor;
        self.jump_to_fraction(1.0, cx);
    }

    fn toggle_break_mode(&mut self, _: &ToggleBreakMode, _: &mut Window, cx: &mut Context<Self>) {
        self.mode = match self.mode {
            BreakMode::KnuthPlass => BreakMode::Greedy,
            BreakMode::Greedy => BreakMode::KnuthPlass,
        };
        cx.notify();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.selecting = true;
        self.cursor = self.hit_test(event.position);
        if !event.modifiers.shift {
            self.anchor = self.cursor;
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.cursor = self.hit_test(event.position);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        self.scroll_by(-f32::from(delta.y), cx);
    }

    // ---- 输入法段落坐标 ----

    /// 输入法所见段落：组字中取组字起点所在段，否则取光标所在段。
    fn ime_paragraph(&self) -> (u64, String) {
        let at = self.marked.as_ref().map_or(self.cursor, |marked| marked.start);
        let range = self.doc.line_range(self.doc.line_of_offset(at));
        (range.start, self.doc.read(range.start, range.end))
    }

    fn global_from_utf16(&self, base: u64, text: &str, range: &Range<usize>) -> Range<u64> {
        base + utf16_to_utf8(text, range.start) as u64..base + utf16_to_utf8(text, range.end) as u64
    }

    fn local_utf16(&self, base: u64, text: &str, range: &Range<u64>) -> Range<usize> {
        let clamp = |offset: u64| (offset.saturating_sub(base) as usize).min(text.len());
        utf8_to_utf16(text, clamp(range.start))..utf8_to_utf16(text, clamp(range.end))
    }
}

fn visual_line(layout: &ParaLayout, items: &[Item], line: &Line) -> VisualLine {
    if line.items.is_empty() {
        let at = items.get(line.items.start).map_or(layout.text.len(), |item| item.from);
        return VisualLine { start: at, end: at, origin_x: layout.shaped_x(at), glues: Vec::new() };
    }
    let start = items[line.items.start].from;
    let end = items[line.items.end - 1].to;
    let mut shift = 0.0;
    let mut glues = Vec::new();
    for item in &items[line.items.clone()] {
        if let ItemKind::Glue { stretch, shrink } = item.kind {
            shift += if line.ratio >= 0.0 { line.ratio * stretch } else { line.ratio * shrink };
            glues.push((item.to, shift));
        }
    }
    VisualLine { start, end, origin_x: layout.shaped_x(start), glues }
}

fn utf16_to_utf8(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn utf8_to_utf16(text: &str, utf8: usize) -> usize {
    text[..utf8.min(text.len())].chars().map(char::len_utf16).sum()
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (base, text) = self.ime_paragraph();
        let range = utf16_to_utf8(&text, range_utf16.start)..utf16_to_utf8(&text, range_utf16.end);
        actual_range.replace(utf8_to_utf16(&text, range.start)..utf8_to_utf16(&text, range.end));
        let _ = base;
        Some(text[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let (base, text) = self.ime_paragraph();
        Some(UTF16Selection {
            range: self.local_utf16(base, &text, &self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.marked.clone()?;
        let (base, text) = self.ime_paragraph();
        Some(self.local_utf16(base, &text, &marked))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
        self.compose_origin = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (base, text) = self.ime_paragraph();
        let range = range_utf16
            .map(|range| self.global_from_utf16(base, &text, &range))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        if let Some((from, removed)) = self.compose_origin.take() {
            // 提交组字：撤销记录直接回到组字前的原文，中间的拼音状态不进历史。
            self.apply(range.clone(), new_text, false);
            let inserted_len = range.start + new_text.len() as u64 - from;
            self.undo.push(EditRecord { from, removed, inserted_len });
            self.redo.clear();
        } else {
            self.apply(range, new_text, true);
        }
        self.marked = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (base, text) = self.ime_paragraph();
        let range = range_utf16
            .map(|range| self.global_from_utf16(base, &text, &range))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        if self.compose_origin.is_none() {
            self.compose_origin = Some((range.start, self.doc.read(range.start, range.end)));
        }
        self.apply(range.clone(), new_text, false);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len() as u64);
        if self.marked.is_none() {
            // 组字被清空：等价于取消，原文已恢复为空串，撤销记录不再需要。
            if let Some((from, removed)) = self.compose_origin.take() {
                if !removed.is_empty() {
                    self.undo.push(EditRecord { from, removed, inserted_len: 0 });
                }
            }
        }
        // 新选区相对于组字文本起点；gpui 官方 input 示例此处把 end 加上了 range.end，
        // 在“替换已有组字”时会越界，这里统一相对 range.start。
        if let Some(selected) = new_selected_range_utf16 {
            let start = range.start + utf16_to_utf8(new_text, selected.start) as u64;
            let end = range.start + utf16_to_utf8(new_text, selected.end) as u64;
            self.anchor = start;
            self.cursor = end;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (base, text) = self.ime_paragraph();
        let start = base + utf16_to_utf8(&text, range_utf16.start) as u64;
        let (paragraph, line_index, x) = self.locate(start)?;
        let top = paragraph.top + line_index as f32 * LINE_HEIGHT;
        let bounds = Bounds::new(
            point(element_bounds.origin.x + px(PADDING_X + x), element_bounds.origin.y + px(top)),
            size(px(2.0), px(LINE_HEIGHT)),
        );
        if std::env::var_os("NOMO_SPIKE_IME_LOG").is_some() {
            eprintln!("{{\"event\":\"ime_bounds\",\"range_utf16\":[{},{}],\"x\":{:.1},\"y\":{:.1}}}", range_utf16.start, range_utf16.end, f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        }
        Some(bounds)
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let offset = self.hit_test(position);
        let (base, text) = self.ime_paragraph();
        (offset >= base && offset <= base + text.len() as u64)
            .then(|| utf8_to_utf16(&text, (offset - base) as usize))
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context("Editor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .bg(rgb(0xffffff))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::toggle_break_mode))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(DocumentElement { editor: cx.entity() })
    }
}

struct DocumentElement {
    editor: Entity<Editor>,
}

impl IntoElement for DocumentElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    started: Instant,
}

impl Element for DocumentElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let started = Instant::now();
        self.editor.update(cx, |editor, _| {
            if editor.stats_enabled {
                if let Some(last) = editor.stats.last_prepaint.replace(started) {
                    editor.stats.interval.push(started - last);
                }
            }
            editor.content_bounds = Some(bounds);
            editor.layout_visible(bounds, window);
            if editor.stats_enabled {
                editor.stats.layout.push(started.elapsed());
            }
        });
        PrepaintState { started }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let editor = self.editor.read(cx);
        let origin = bounds.origin;
        let selection = editor.selection();
        let marked = editor.marked.clone();
        let text_color: Hsla = rgb(0x1f2328).into();
        let selection_color: Hsla = gpui::rgba(0x3b82f633).into();
        let caret_color: Hsla = rgb(0x2563eb).into();
        let viewport = f32::from(bounds.size.height);
        let font_size = px(FONT_SIZE);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for paragraph in &editor.visible {
                let layout = &paragraph.layout;
                for (line_index, line) in layout.lines.iter().enumerate() {
                    let top = paragraph.top + line_index as f32 * LINE_HEIGHT;
                    if top + LINE_HEIGHT < 0.0 || top > viewport {
                        continue;
                    }
                    let line_global = paragraph.start + line.start as u64..paragraph.start + line.end as u64;
                    // 选区与组字下划线都按视觉行裁剪后绘制。
                    for (range, underline) in [(Some(selection.clone()), false), (marked.clone(), true)] {
                        let Some(range) = range else { continue };
                        let from = range.start.max(line_global.start);
                        let to = range.end.min(line_global.end);
                        if from >= to {
                            continue;
                        }
                        let x0 = layout.x_in_line(line, (from - paragraph.start) as usize);
                        let x1 = layout.x_in_line(line, (to - paragraph.start) as usize);
                        let (y, h) = if underline { (top + LINE_HEIGHT - 5.0, 1.5) } else { (top + 2.0, LINE_HEIGHT - 4.0) };
                        window.paint_quad(fill(
                            Bounds::new(point(origin.x + px(PADDING_X + x0), origin.y + px(y)), size(px(x1 - x0), px(h))),
                            if underline { text_color } else { selection_color },
                        ));
                    }
                    let baseline = top + LINE_HEIGHT * 0.5 + FONT_SIZE * 0.36;
                    let first = layout.glyphs.partition_point(|glyph| glyph.index < line.start);
                    let mut glue = 0;
                    let mut shift = 0.0;
                    for glyph in layout.glyphs[first..].iter().take_while(|glyph| glyph.index < line.end) {
                        while glue < line.glues.len() && line.glues[glue].0 <= glyph.index {
                            shift = line.glues[glue].1;
                            glue += 1;
                        }
                        let x = glyph.x - line.origin_x + shift;
                        let at = point(origin.x + px(PADDING_X + x), origin.y + px(baseline));
                        let _ = if glyph.emoji {
                            window.paint_emoji(at, glyph.font_id, glyph.id, font_size)
                        } else {
                            window.paint_glyph(at, glyph.font_id, glyph.id, font_size, text_color)
                        };
                    }
                }
            }
            if focus.is_focused(window) {
                if let Some((paragraph, line_index, x)) = editor.locate(editor.cursor) {
                    let top = paragraph.top + line_index as f32 * LINE_HEIGHT;
                    window.paint_quad(fill(
                        Bounds::new(point(origin.x + px(PADDING_X + x), origin.y + px(top + 4.0)), size(px(2.0), px(LINE_HEIGHT - 8.0))),
                        caret_color,
                    ));
                }
            }
            // 滚动条只表示锚点所在行比例，不支持拖动。
            let lines = editor.doc.line_count().max(1) as f32;
            let thumb_top = viewport * (editor.scroll_line as f32 / lines);
            let thumb_height = (viewport * 0.05).max(24.0);
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + bounds.size.width - px(8.0), origin.y + px(thumb_top.min(viewport - thumb_height))),
                    size(px(5.0), px(thumb_height)),
                ),
                gpui::rgba(0x00000040),
            ));
        });

        let elapsed = prepaint.started.elapsed();
        self.editor.update(cx, |editor, _| {
            if editor.stats_enabled {
                editor.stats.frame.push(elapsed);
            }
            if editor.first_frame_at.is_none() {
                editor.first_frame_at = Some(Instant::now());
            }
        });
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("up", Up, None),
        KeyBinding::new("down", Down, None),
        KeyBinding::new("enter", Newline, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("pagedown", PageDown, None),
        KeyBinding::new("pageup", PageUp, None),
        KeyBinding::new("cmd-up", DocStart, None),
        KeyBinding::new("cmd-down", DocEnd, None),
        KeyBinding::new("cmd-k", ToggleBreakMode, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

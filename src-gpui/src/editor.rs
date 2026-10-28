//! 编辑视图：源码模式按行、语义模式按顶层块虚拟滚动，二者共用同一套几何、命中测试与编辑逻辑。
//!
//! 语义模式的编辑方式：视图获得焦点时，光标所在块（或光标所在的块间空行）就地展开为带高亮的源码行，
//! 其余块按语义渲染；点击渲染块会把光标映射回源码并展开该块。这是块级“实时预览”，
//! 不是 ProseMirror 式的逐字所见即所得。
//!
//! 生命周期：视图由标签创建并持有，标签关闭时随之释放；布局缓存只保存文本与块布局，
//! 不持有窗口或回调。输入法只看到光标所在源码行，UTF-16 偏移以行首为 0。

use crate::buffer::{Buffer, BufferEvent, EditKind};
use crate::cache::LruCache;
use crate::embeds::{self, EmbedKind, EmbedSpec, EmbedState, EmbedStore};
use crate::images::{self, ImageStore};
use crate::layout::{ColorRole, InlineStyle, ParaSpec, TextLayout, Wrap, layout_text, paint_box};
use crate::markdown::blocks::BlockKind;
use crate::markdown::highlight::LineContext;
use crate::markdown::render::{BlockLayout, DisplayItem, RenderEnv, layout_block, source_line_spec};
use crate::settings::Settings;
use crate::theme::{Family, Metrics, Theme};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, Style,
    Subscription, UTF16Selection, Window, actions, div, point, prelude::*, px, relative, size,
};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    editor,
    [
        MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown, MoveWordLeft,
        MoveWordRight, SelectWordLeft, SelectWordRight, MoveLineStart, MoveLineEnd, SelectLineStart, SelectLineEnd,
        MoveDocStart, MoveDocEnd, SelectDocStart, SelectDocEnd, Backspace, Delete, DeleteWordLeft, DeleteLineLeft,
        Newline, Indent, Outdent, SelectAll, Undo, Redo, Copy, Cut, Paste, PageUp, PageDown, CharacterPalette
    ]
);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Source,
    Semantic,
}

pub enum ViewEvent {
    /// 用户主动滚动后的顶部位置（源码行号，可带小数），供分栏同步与大纲高亮。
    Scrolled(f64),
    Focused,
}

const PAD_TOP: f32 = 32.0;
const PAD_BOTTOM: f32 = 160.0;
const SCROLLBAR_WIDTH: f32 = 12.0;
/// 光标自动滚动时与视口上下边缘保持的距离。
const REVEAL_MARGIN: f32 = 48.0;

enum Target {
    Line(f64),
    Fraction(f64),
}

enum Part {
    Line { line: usize, start: u64, y: f32, layout: Rc<TextLayout> },
    /// `start..end` 为块源码的全文字节区间（不含块尾换行）。
    Block { start: u64, end: u64, y: f32, layout: Rc<BlockLayout> },
}

impl Part {
    fn y(&self) -> f32 {
        match self {
            Part::Line { y, .. } | Part::Block { y, .. } => *y,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Part::Line { layout, .. } => layout.height(),
            Part::Block { layout, .. } => layout.height,
        }
    }
}

/// 一行（源码模式）或一个块（语义模式）的可见几何；`top` 为窗口坐标，`parts` 的 y 相对 `top`。
struct VisibleRow {
    row: usize,
    top: f32,
    height: f32,
    gap: f32,
    parts: Vec<Part>,
}

/// 语义模式中展开为源码的区域。
#[derive(Clone, PartialEq, Debug)]
enum Active {
    None,
    Block(usize),
    /// 光标位于块间空行：`after` 为其前一块（None 表示文首空行），`lines` 为整段空行。
    Gap { after: Option<usize>, lines: Range<usize> },
}

struct RowCtx<'a> {
    buffer: &'a Buffer,
    store: &'a ImageStore,
    metrics: Metrics,
    wrap: Wrap,
    base_dir: Option<PathBuf>,
    active: Active,
    missing: RefCell<Vec<PathBuf>>,
    code_line_numbers: bool,
    inline_code: bool,
    embeds: &'a EmbedStore,
    /// 本帧布局中遇到但尚未请求的公式与图表，布局结束后统一提交后台渲染。
    embed_missing: RefCell<Vec<EmbedSpec>>,
    /// 公式前景色与图表配色依据。
    math_color: [u8; 3],
    dark: bool,
}

pub struct EditorView {
    pub buffer: Entity<Buffer>,
    pub mode: ViewMode,
    focus: FocusHandle,
    focused: bool,
    scroll_row: usize,
    scroll_offset: f32,
    pending_scroll: f32,
    target: Option<Target>,
    reveal_cursor: bool,
    user_scrolled: bool,
    visible: Vec<VisibleRow>,
    bounds: Option<Bounds<Pixels>>,
    content_x: f32,
    content_width: f32,
    /// 表格等宽内容可用宽度：正文左缘到编辑区右缘（扣除滚动条）。
    wide_width: f32,
    text_cache: LruCache<TextLayout>,
    block_cache: LruCache<BlockLayout>,
    selecting: bool,
    dragging_scrollbar: bool,
    goal_x: Option<f32>,
    keep_goal: bool,
    /// 分栏模式下为 true：内容宽度改为铺满面板。
    pub split: bool,
    /// 基准模式下记录每帧耗时；正常运行为 None，不产生开销。
    pub bench: Option<FrameStats>,
    _subscriptions: Vec<Subscription>,
}

/// 逐帧计时：`frame` 为本视图 prepaint+paint 的 CPU 时间，`layout` 为其中排版部分，
/// `interval` 为相邻两帧 prepaint 的间隔（反映实际上屏节奏）。
#[derive(Default)]
pub struct FrameStats {
    pub frame: Vec<std::time::Duration>,
    pub layout: Vec<std::time::Duration>,
    pub interval: Vec<std::time::Duration>,
    last: Option<std::time::Instant>,
    started: Option<std::time::Instant>,
}

impl EventEmitter<ViewEvent> for EditorView {}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EditorView {
    pub fn new(buffer: Entity<Buffer>, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe(&buffer, |view, _, event, cx| {
                if matches!(event, BufferEvent::Edited | BufferEvent::SelectionChanged) {
                    if view.focused {
                        view.reveal_cursor = true;
                    }
                    if view.keep_goal {
                        view.keep_goal = false;
                    } else {
                        view.goal_x = None;
                    }
                }
                cx.notify();
            }),
            cx.on_focus(&focus, window, |_, _, cx| cx.emit(ViewEvent::Focused)),
        ];
        Self {
            buffer,
            mode,
            focus,
            focused: false,
            scroll_row: 0,
            scroll_offset: 0.0,
            pending_scroll: 0.0,
            target: None,
            reveal_cursor: false,
            user_scrolled: false,
            visible: Vec::new(),
            bounds: None,
            content_x: 0.0,
            content_width: 600.0,
            wide_width: 600.0,
            // 文本布局缓存（源码行、活动块行）与块布局缓存（语义渲染），都按条数与估算字节双重限额。
            text_cache: LruCache::new(4096, 32 * 1024 * 1024),
            block_cache: LruCache::new(1024, 48 * 1024 * 1024),
            selecting: false,
            dragging_scrollbar: false,
            goal_x: None,
            keep_goal: false,
            split: false,
            bench: None,
            _subscriptions: subscriptions,
        }
    }

    /// 滚动到指定源码行（可带小数），用于大纲跳转与分栏同步。
    pub fn scroll_to_line(&mut self, line: f64, cx: &mut Context<Self>) {
        self.target = Some(Target::Line(line));
        cx.notify();
    }

    /// 当前视口顶部对应的源码行（小数部分表示行或块内的进度）。
    pub fn top_line(&self, cx: &App) -> f64 {
        let Some(first) = self.visible.iter().find(|row| row.top + row.height > 0.0).or(self.visible.first()) else {
            return 0.0;
        };
        let bounds_top = self.bounds.map_or(0.0, |bounds| f32::from(bounds.origin.y));
        let into = (bounds_top - first.top - first.gap).max(0.0);
        let fraction = (into / (first.height - first.gap).max(1.0)).clamp(0.0, 1.0) as f64;
        let buffer = self.buffer.read(cx);
        match self.mode {
            ViewMode::Source => first.row as f64 + fraction,
            ViewMode::Semantic => match buffer.blocks.blocks.get(first.row) {
                Some(block) => block.start as f64 + fraction * block.lines as f64,
                None => 0.0,
            },
        }
    }

    fn row_count(&self, buffer: &Buffer) -> usize {
        match self.mode {
            ViewMode::Source => buffer.doc.line_count(),
            ViewMode::Semantic => buffer.blocks.blocks.len().max(1),
        }
    }

    fn row_of_line(&self, buffer: &Buffer, line: usize) -> usize {
        match self.mode {
            ViewMode::Source => line.min(buffer.doc.line_count() - 1),
            ViewMode::Semantic => {
                let blocks = &buffer.blocks.blocks;
                if blocks.is_empty() {
                    return 0;
                }
                match buffer.blocks.block_containing(line) {
                    Some(index) => index,
                    None => blocks.partition_point(|block| block.start <= line).saturating_sub(1),
                }
            }
        }
    }

    fn active_region(&self, buffer: &Buffer) -> Active {
        if self.mode != ViewMode::Semantic || !(self.focused || buffer.marked.is_some() && !self.split) {
            return Active::None;
        }
        let line = buffer.doc.line_of_offset(buffer.marked.as_ref().map_or(buffer.cursor, |marked| marked.start));
        let blocks = &buffer.blocks.blocks;
        if let Some(index) = buffer.blocks.block_containing(line) {
            return Active::Block(index);
        }
        let after = blocks.partition_point(|block| block.start <= line);
        let start = if after == 0 { 0 } else { blocks[after - 1].end() };
        let end = blocks.get(after).map_or(buffer.doc.line_count(), |block| block.start);
        Active::Gap { after: after.checked_sub(1), lines: start..end }
    }

    // ---- 排版 ----

    fn line_layout(&mut self, line: usize, ctx: &RowCtx, window: &Window) -> Rc<TextLayout> {
        let buffer = ctx.buffer;
        let text = buffer.doc.line_text(line);
        let (mut context, mut lang, mut heading) = (LineContext::Normal, String::new(), None);
        if let Some(index) = buffer.blocks.block_containing(line) {
            let block = buffer.blocks.blocks[index];
            match block.kind {
                BlockKind::Fence => {
                    let first = buffer.doc.line_text(block.start);
                    lang = first.trim_start().trim_start_matches(['`', '~']).split_whitespace().next().unwrap_or("").to_lowercase();
                    let closing = line + 1 == block.end() && line != block.start && text.trim_start().starts_with(first.trim_start().chars().next().unwrap_or('`'));
                    context = if line == block.start || closing { LineContext::FenceMarker } else { LineContext::Code };
                }
                BlockKind::Math => context = LineContext::Math,
                BlockKind::FrontMatter => context = LineContext::FrontMatter,
                BlockKind::Heading(level) => heading = Some(level),
                _ => {}
            }
        }
        let metrics = ctx.metrics;
        let (family, mut font_size, mut line_height) = match self.mode {
            ViewMode::Source => (Family::Mono, metrics.source_size(), metrics.source_line_height()),
            ViewMode::Semantic => (Family::Text, metrics.body_size(), metrics.body_line_height()),
        };
        if self.mode == ViewMode::Semantic {
            if let Some(level) = heading {
                font_size = metrics.heading(level).0;
                line_height = font_size * 1.35;
            }
            if matches!(context, LineContext::Code | LineContext::FenceMarker) {
                font_size = metrics.code_size();
                line_height = metrics.code_line_height();
            }
        }
        let spec = source_line_spec(&text, context, &lang, family, font_size, line_height);
        let width = self.content_width;
        let key = spec.cache_key(width);
        self.text_cache.get_or_insert_with(key, || {
            let layout = layout_text(&spec, width, window);
            let bytes = layout.estimated_bytes();
            (layout, bytes)
        })
    }

    fn block_layout(&mut self, index: usize, ctx: &RowCtx, window: &Window) -> (u64, u64, Rc<BlockLayout>) {
        let buffer = ctx.buffer;
        let block = buffer.blocks.blocks[index];
        let start = buffer.doc.line_range(block.start).start;
        let end = buffer.doc.line_range(block.end() - 1).end;
        let text = buffer.doc.read(start, end);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        self.content_width.to_bits().hash(&mut hasher);
        if text.contains('|') {
            self.wide_width.to_bits().hash(&mut hasher);
        }
        ctx.metrics.zoom.to_bits().hash(&mut hasher);
        ctx.metrics.font_size.to_bits().hash(&mut hasher);
        ctx.metrics.line_height.to_bits().hash(&mut hasher);
        crate::layout::generation().hash(&mut hasher);
        ctx.wrap.hash(&mut hasher);
        if text.contains("![") {
            ctx.store.generation.hash(&mut hasher);
            ctx.base_dir.hash(&mut hasher);
        }
        // 含公式或 Mermaid 的块：渲染结果就绪（或被淘汰）时 generation 变化，触发重排；配色随深浅色变化。
        if text.contains('$') || text.contains("mermaid") {
            ctx.embeds.generation.hash(&mut hasher);
            ctx.dark.hash(&mut hasher);
            ctx.math_color.hash(&mut hasher);
        }
        let key = hasher.finish();
        let width = self.content_width;
        let wide_width = self.wide_width;
        let layout = self.block_cache.get_or_insert_with(key, || {
            let image_size = |path: &std::path::Path| {
                let size = ctx.store.size(path);
                if size.is_none() && !ctx.store.is_pending_or_known(path) && path.exists() {
                    ctx.missing.borrow_mut().push(path.to_path_buf());
                }
                size
            };
            let em_px = ctx.metrics.body_size();
            let embed = |kind: EmbedKind, source: &str| {
                let spec = EmbedSpec { kind, source: source.to_string(), em_px, color: ctx.math_color, dark: ctx.dark };
                let key = spec.key();
                match ctx.embeds.state(key) {
                    Some(state) => (key, state),
                    None => {
                        ctx.embed_missing.borrow_mut().push(spec);
                        (key, EmbedState::Pending)
                    }
                }
            };
            let env = RenderEnv {
                embed: &embed,
                metrics: ctx.metrics,
                wrap: ctx.wrap,
                base_dir: ctx.base_dir.as_deref(),
                image_size: &image_size,
                image_failure: &|path| ctx.store.failure(path),
                text_layouts: None,
                wide_width,
                code_line_numbers: ctx.code_line_numbers,
                inline_code: ctx.inline_code,
            };
            let layout = layout_block(&text, width, &env, window);
            let bytes = layout.estimated_bytes();
            (layout, bytes)
        });
        (start, end, layout)
    }

    /// 块的上下外边距，相邻块间距取上一块下边距与本块上边距的较大者（等价于 CSS 外边距折叠）。
    fn margins(kind: BlockKind, metrics: Metrics) -> (f32, f32) {
        match kind {
            BlockKind::Heading(level) => {
                let (_, top, bottom) = metrics.heading(level);
                (top, bottom)
            }
            BlockKind::Rule => (4.0 * metrics.zoom, 4.0 * metrics.zoom),
            _ => (metrics.paragraph_gap(), metrics.paragraph_gap()),
        }
    }

    /// 构建一行的几何：返回（行首间距、各部分）。部分的 y 已包含间距。
    fn build_row(&mut self, row: usize, ctx: &RowCtx, window: &Window) -> (f32, Vec<Part>) {
        let buffer = ctx.buffer;
        let mut parts = Vec::new();
        let mut y;
        let lines_part = |view: &mut Self, lines: Range<usize>, y: &mut f32, parts: &mut Vec<Part>| {
            for line in lines {
                let layout = view.line_layout(line, ctx, window);
                let height = layout.height();
                parts.push(Part::Line { line, start: buffer.doc.line_range(line).start, y: *y, layout });
                *y += height;
            }
        };
        match self.mode {
            ViewMode::Source => {
                let gap = if row == 0 { PAD_TOP } else { 0.0 };
                y = gap;
                lines_part(self, row..row + 1, &mut y, &mut parts);
                (gap, parts)
            }
            ViewMode::Semantic => {
                let blocks = &buffer.blocks.blocks;
                if blocks.is_empty() {
                    y = PAD_TOP;
                    lines_part(self, 0..buffer.doc.line_count(), &mut y, &mut parts);
                    return (PAD_TOP, parts);
                }
                let block = blocks[row];
                let gap = if row == 0 {
                    PAD_TOP
                } else {
                    let previous = Self::margins(blocks[row - 1].kind, ctx.metrics).1;
                    previous.max(Self::margins(block.kind, ctx.metrics).0)
                };
                y = gap;
                if let Active::Gap { after: None, lines } = &ctx.active {
                    if row == 0 {
                        lines_part(self, lines.clone(), &mut y, &mut parts);
                    }
                }
                if ctx.active == Active::Block(row) {
                    lines_part(self, block.start..block.end(), &mut y, &mut parts);
                } else {
                    let (start, end, layout) = self.block_layout(row, ctx, window);
                    let height = layout.height;
                    parts.push(Part::Block { start, end, y, layout });
                    y += height;
                }
                if let Active::Gap { after: Some(after), lines } = &ctx.active {
                    if *after == row {
                        lines_part(self, lines.clone(), &mut y, &mut parts);
                    }
                }
                (gap, parts)
            }
        }
    }

    fn row_height(&mut self, row: usize, ctx: &RowCtx, window: &Window) -> f32 {
        let (_, parts) = self.build_row(row, ctx, window);
        parts.last().map_or(PAD_TOP, |part| part.y() + part.height())
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.focused = self.focus.is_focused(window);
        self.bounds = Some(bounds);
        let settings = cx.global::<Settings>().clone();
        let metrics = settings.metrics();
        let total_width = f32::from(bounds.size.width) - SCROLLBAR_WIDTH;
        self.content_width = match self.mode {
            ViewMode::Source => (total_width - 72.0).clamp(160.0, 1100.0 * metrics.zoom),
            ViewMode::Semantic if self.split => (total_width - 72.0).clamp(160.0, 900.0 * metrics.zoom),
            ViewMode::Semantic => (total_width * settings.content_width_percent).max((total_width - 48.0).min(560.0)).clamp(160.0, 1000.0 * metrics.zoom),
        };
        self.content_x = f32::from(bounds.origin.x) + (total_width - self.content_width) / 2.0;
        self.wide_width = (f32::from(bounds.origin.x) + total_width - 16.0 - self.content_x).max(self.content_width);
        let viewport = f32::from(bounds.size.height);
        let top = f32::from(bounds.origin.y);

        let buffer_entity = self.buffer.clone();
        let (math_color, dark) = {
            let theme = cx.global::<Theme>();
            let color = gpui::Rgba::from(theme.fg);
            let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            ([byte(color.r), byte(color.g), byte(color.b)], theme.dark)
        };
        let missing;
        let embed_missing;
        let mut scrolled_to = None;
        {
            let buffer = buffer_entity.read(cx);
            let ctx = RowCtx {
                buffer,
                store: cx.global::<ImageStore>(),
                metrics,
                wrap: settings.wrap(),
                base_dir: buffer.base_dir(),
                active: self.active_region(buffer),
                missing: RefCell::new(Vec::new()),
                code_line_numbers: settings.code_line_numbers,
                inline_code: settings.inline_code_rendering,
                embeds: cx.global::<EmbedStore>(),
                embed_missing: RefCell::new(Vec::new()),
                math_color,
                dark,
            };
            let count = self.row_count(buffer);
            let last = count - 1;
            self.scroll_row = self.scroll_row.min(last);
            match self.target.take() {
                Some(Target::Line(line)) => {
                    let whole = line.floor().max(0.0) as usize;
                    self.scroll_row = self.row_of_line(buffer, whole);
                    let height = self.row_height(self.scroll_row, &ctx, window);
                    let (start, lines) = match self.mode {
                        ViewMode::Source => (self.scroll_row as f64, 1.0),
                        ViewMode::Semantic => buffer.blocks.blocks.get(self.scroll_row).map_or((0.0, 1.0), |block| (block.start as f64, block.lines.max(1) as f64)),
                    };
                    let fraction = ((line - start) / lines).clamp(0.0, 1.0) as f32;
                    self.scroll_offset = height * fraction;
                }
                Some(Target::Fraction(fraction)) => {
                    self.scroll_row = ((last as f64) * fraction).round() as usize;
                    self.scroll_offset = 0.0;
                }
                None => {}
            }
            for pass in 0..3 {
                self.scroll_offset += std::mem::take(&mut self.pending_scroll);
                // 前进越过锚点行时后移锚点；后退为负时回到上一行并补其高度。
                loop {
                    let height = self.row_height(self.scroll_row, &ctx, window);
                    if self.scroll_offset >= height && self.scroll_row < last {
                        self.scroll_offset -= height;
                        self.scroll_row += 1;
                    } else {
                        break;
                    }
                }
                while self.scroll_offset < 0.0 && self.scroll_row > 0 {
                    self.scroll_row -= 1;
                    self.scroll_offset += self.row_height(self.scroll_row, &ctx, window);
                }
                if self.scroll_row == 0 {
                    self.scroll_offset = self.scroll_offset.max(0.0);
                }
                let mut rows = Vec::new();
                let mut y = top - self.scroll_offset;
                let mut row = self.scroll_row;
                while y < top + viewport && row <= last {
                    let (gap, parts) = self.build_row(row, &ctx, window);
                    let height = parts.last().map_or(gap, |part| part.y() + part.height());
                    rows.push(VisibleRow { row, top: y, height, gap, parts });
                    y += height;
                    row += 1;
                }
                // 文末留白后仍未填满视口：回退锚点，避免底部出现大片空白。
                let slack = top + viewport - (y + PAD_BOTTOM);
                if row > last && slack > 0.0 && (self.scroll_row > 0 || self.scroll_offset > 0.0) && pass < 2 {
                    self.pending_scroll = -slack;
                    self.visible = rows;
                    continue;
                }
                self.visible = rows;
                if self.reveal_cursor && pass < 2 {
                    self.reveal_cursor = false;
                    let cursor = buffer.cursor;
                    match self.caret_geometry(buffer) {
                        Some((_, caret_top, height)) => {
                            if caret_top < top + REVEAL_MARGIN.min(viewport / 4.0) {
                                self.pending_scroll = caret_top - top - REVEAL_MARGIN.min(viewport / 4.0);
                            } else if caret_top + height > top + viewport - REVEAL_MARGIN.min(viewport / 4.0) {
                                self.pending_scroll = caret_top + height - (top + viewport - REVEAL_MARGIN.min(viewport / 4.0));
                            }
                        }
                        None => {
                            let line = buffer.doc.line_of_offset(cursor);
                            self.scroll_row = self.row_of_line(buffer, line);
                            self.scroll_offset = 0.0;
                            self.pending_scroll = -viewport * 0.3;
                        }
                    }
                    if self.pending_scroll != 0.0 {
                        continue;
                    }
                }
                break;
            }
            missing = ctx.missing.into_inner();
            embed_missing = ctx.embed_missing.into_inner();
            if self.user_scrolled {
                self.user_scrolled = false;
                scrolled_to = Some(());
            }
        }
        if !missing.is_empty() {
            images::request(missing, cx);
        }
        if !embed_missing.is_empty() {
            embeds::request(embed_missing, cx);
        }
        if scrolled_to.is_some() {
            let line = self.top_line(cx);
            cx.emit(ViewEvent::Scrolled(line));
        }
    }

    /// 光标所在源码行的绘制位置：(x, 顶部 y, 行高)，窗口坐标。
    fn caret_geometry(&self, buffer: &Buffer) -> Option<(f32, f32, f32)> {
        let cursor = buffer.cursor;
        let line = buffer.doc.line_of_offset(cursor);
        for row in &self.visible {
            for part in &row.parts {
                if let Part::Line { line: part_line, start, y, layout } = part {
                    if *part_line == line {
                        let (visual, x) = layout.caret((cursor - start) as usize);
                        return Some((self.content_x + x, row.top + y + visual as f32 * layout.line_height, layout.line_height));
                    }
                }
            }
        }
        None
    }

    /// 窗口坐标命中到全文字节偏移。
    fn hit_test(&self, position: Point<Pixels>, buffer: &Buffer) -> u64 {
        let x = f32::from(position.x) - self.content_x;
        let y = f32::from(position.y);
        let Some(first) = self.visible.first() else { return buffer.cursor };
        if y < first.top {
            return self.part_offset(first, &first.parts[0], x, 0.0, buffer);
        }
        let row = self.visible.iter().find(|row| y < row.top + row.height).unwrap_or_else(|| self.visible.last().expect("non-empty"));
        let local = y - row.top;
        let part = row
            .parts
            .iter()
            .find(|part| local < part.y() + part.height())
            .unwrap_or_else(|| row.parts.last().expect("row has parts"));
        self.part_offset(row, part, x, local - part.y(), buffer)
    }

    fn part_offset(&self, _row: &VisibleRow, part: &Part, x: f32, y: f32, _buffer: &Buffer) -> u64 {
        match part {
            Part::Line { start, layout, .. } => start + layout.offset_at(x, y.max(0.0)) as u64,
            Part::Block { start, layout, .. } => start + layout.source_at(x, y.max(0.0)).unwrap_or(0) as u64,
        }
    }

    /// 命中语义块中的可点击元件：任务复选框（返回 `[` 的全文偏移）或代码块复制按钮。
    fn block_action_at(&self, position: Point<Pixels>) -> Option<BlockAction> {
        let x = f32::from(position.x) - self.content_x;
        let y = f32::from(position.y);
        for row in &self.visible {
            for part in &row.parts {
                let Part::Block { start, y: part_y, layout, .. } = part else { continue };
                for item in &layout.items {
                    let (ix, iy, side) = match item {
                        DisplayItem::Checkbox { x, y, size, .. } | DisplayItem::CopyButton { x, y, size, .. } => (*x, *y, *size),
                        _ => continue,
                    };
                    let top = row.top + part_y + iy;
                    if x >= ix - 4.0 && x <= ix + side + 4.0 && y >= top - 4.0 && y <= top + side + 4.0 {
                        return match item {
                            DisplayItem::Checkbox { source, .. } => Some(BlockAction::ToggleTask(start + *source as u64)),
                            DisplayItem::CopyButton { text, .. } => Some(BlockAction::Copy(text.clone())),
                            _ => None,
                        };
                    }
                }
            }
        }
        None
    }

    // ---- 绘制 ----

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
        let theme = cx.global::<Theme>();
        let store = cx.global::<ImageStore>();
        let buffer = self.buffer.read(cx);
        let selection = buffer.selection();
        let show_selection = !selection.is_empty();
        let viewport_top = f32::from(bounds.origin.y);
        let viewport_bottom = viewport_top + f32::from(bounds.size.height);
        let search = &buffer.search;
        let current_match = search.current.and_then(|index| search.matches.get(index)).cloned();
        let ox = self.content_x;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &self.visible {
                for part in &row.parts {
                    let top = row.top + part.y();
                    let clip = (viewport_top - top)..(viewport_bottom - top);
                    match part {
                        Part::Line { start, layout, .. } => {
                            let end = start + layout.text.len() as u64;
                            let origin = point(px(ox), px(top));
                            // 搜索结果：只取与本行相交的匹配，二分定位起点。
                            let first = search.matches.partition_point(|range| range.end < *start);
                            for range in search.matches[first..].iter().take_while(|range| range.start <= end) {
                                let color = if current_match.as_ref() == Some(range) { theme.search_current } else { theme.search_match };
                                paint_ranges(layout, origin, (range.start.max(*start) - start) as usize..(range.end.min(end) - start) as usize, color, window);
                            }
                            if show_selection && selection.start <= end && selection.end >= *start {
                                let local = (selection.start.max(*start) - start) as usize..(selection.end.min(end) - start) as usize;
                                paint_ranges(layout, origin, local, theme.selection, window);
                                if selection.end > end && selection.start <= end {
                                    // 选区跨过行尾：画出换行符占位。
                                    let (visual, x) = layout.caret(layout.text.len());
                                    paint_box(Bounds::new(point(px(ox + x), px(top + visual as f32 * layout.line_height)), size(px(6.0), px(layout.line_height))), theme.selection, 0.0, None, window);
                                }
                            }
                            layout.paint(origin, clip, theme, window);
                            if let Some(marked) = &buffer.marked {
                                if marked.start <= end && marked.end >= *start {
                                    let local = (marked.start.max(*start) - start) as usize..(marked.end.min(end) - start) as usize;
                                    for (visual, x0, x1) in layout.range_rects(local) {
                                        let y = top + (visual + 1) as f32 * layout.line_height - 4.0;
                                        paint_box(Bounds::new(point(px(ox + x0), px(y)), size(px(x1 - x0), px(1.5))), theme.fg, 0.0, None, window);
                                    }
                                }
                            }
                        }
                        Part::Block { start, end, layout, .. } => {
                            crate::nomo_mark::paint::paint_block(layout, point(px(ox), px(top)), clip.clone(), theme, store, window, cx);
                            let first = search.matches.partition_point(|range| range.end < *start);
                            for range in search.matches[first..].iter().take_while(|range| range.start <= *end) {
                                let rel = (range.start.saturating_sub(*start)) as usize..(range.end.min(*end) - start) as usize;
                                let rects = layout.source_rects(rel);
                                let color = if current_match.as_ref() == Some(range) { theme.search_current } else { theme.search_match };
                                for (x, y, w, h) in rects {
                                    paint_box(Bounds::new(point(px(ox + x), px(top + y)), size(px(w), px(h))), color, 2.0, None, window);
                                }
                            }
                            if show_selection && selection.end > *start && selection.start <= *end {
                                let rel = selection.start.saturating_sub(*start) as usize..(selection.end.min(*end) - start) as usize;
                                for (x, y, w, h) in layout.source_rects(rel) {
                                    paint_box(Bounds::new(point(px(ox + x), px(top + y)), size(px(w), px(h))), theme.selection, 2.0, None, window);
                                }
                            }
                        }
                    }
                }
            }
            if self.focused && selection.is_empty() {
                if let Some((x, y, height)) = self.caret_geometry(buffer) {
                    paint_box(Bounds::new(point(px(x), px(y + 3.0)), size(px(2.0), px(height - 6.0))), theme.caret, 1.0, None, window);
                }
            }
            self.paint_scrollbar(bounds, buffer, theme, window);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_scrollbar(&self, bounds: Bounds<Pixels>, buffer: &Buffer, theme: &Theme, window: &mut Window) {
        let count = self.row_count(buffer).max(1) as f32;
        let viewport = f32::from(bounds.size.height);
        let visible_rows = self.visible.len().max(1) as f32;
        let thumb = (viewport * (visible_rows / count)).clamp(32.0, viewport);
        if thumb >= viewport - 1.0 {
            return;
        }
        let progress = self.scroll_row as f32 / (count - 1.0).max(1.0);
        let y = f32::from(bounds.origin.y) + (viewport - thumb) * progress;
        let x = f32::from(bounds.origin.x + bounds.size.width) - SCROLLBAR_WIDTH + 3.0;
        let width = if self.dragging_scrollbar { 7.0 } else { 6.0 };
        paint_box(Bounds::new(point(px(x), px(y)), size(px(width), px(thumb))), theme.scrollbar, 3.5, None, window);
    }

    // ---- 编辑辅助 ----

    fn update_buffer<R>(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut Buffer, &mut Context<Buffer>) -> R) -> R {
        self.buffer.update(cx, update)
    }

    fn move_to(&mut self, offset: u64, extend: bool, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| {
            let anchor = if extend { buffer.anchor } else { offset };
            buffer.unmark();
            buffer.set_selection(anchor, offset, cx);
        });
        self.reveal_cursor = true;
    }

    fn horizontal(&mut self, forward: bool, word: bool, extend: bool, cx: &mut Context<Self>) {
        let target = {
            let buffer = self.buffer.read(cx);
            let selection = buffer.selection();
            if !extend && !selection.is_empty() && !word {
                if forward { selection.end } else { selection.start }
            } else {
                match (forward, word) {
                    (true, false) => buffer.next_grapheme(buffer.cursor),
                    (false, false) => buffer.prev_grapheme(buffer.cursor),
                    (true, true) => buffer.next_word(buffer.cursor),
                    (false, true) => buffer.prev_word(buffer.cursor),
                }
            }
        };
        self.move_to(target, extend, cx);
    }

    /// 上下移动：在可见几何上按目标横坐标命中；目标行不可见时退化为按源码行移动。
    fn vertical(&mut self, down: bool, extend: bool, pages: f32, cx: &mut Context<Self>) {
        let target = {
            let buffer = self.buffer.read(cx);
            let Some((x, y, height)) = self.caret_geometry(buffer) else {
                let line = buffer.doc.line_of_offset(buffer.cursor);
                let next = if down { (line + 1).min(buffer.doc.line_count() - 1) } else { line.saturating_sub(1) };
                let target = buffer.doc.line_range(next).start;
                self.move_to(target, extend, cx);
                return;
            };
            let goal = *self.goal_x.get_or_insert(x);
            let step = if pages > 0.0 { self.bounds.map_or(400.0, |bounds| f32::from(bounds.size.height)) * pages } else { height * 0.5 + 2.0 };
            let target_y = if down { y + height + step - height * 0.5 } else { y - step + if pages > 0.0 { 0.0 } else { 0.0 } };
            let viewport = self.bounds.map_or(0.0..0.0, |bounds| f32::from(bounds.origin.y)..f32::from(bounds.origin.y + bounds.size.height));
            if pages > 0.0 || viewport.contains(&target_y) {
                Some(self.hit_test(point(px(goal), px(target_y.clamp(viewport.start + 1.0, viewport.end - 1.0))), buffer))
            } else {
                let line = buffer.doc.line_of_offset(buffer.cursor);
                let next = if down { (line + 1).min(buffer.doc.line_count() - 1) } else { line.saturating_sub(1) };
                Some(buffer.doc.line_range(next).start)
            }
        };
        if pages > 0.0 {
            let delta = self.bounds.map_or(400.0, |bounds| f32::from(bounds.size.height)) * pages * if down { 1.0 } else { -1.0 };
            self.pending_scroll += delta;
        }
        if let Some(target) = target {
            self.keep_goal = true;
            self.move_to(target, extend, cx);
        }
    }

    fn line_bounds(&self, cx: &App) -> (u64, u64) {
        let buffer = self.buffer.read(cx);
        let line = buffer.doc.line_of_offset(buffer.cursor);
        let range = buffer.doc.line_range(line);
        (range.start, range.end)
    }

    // ---- 动作 ----

    fn on_move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, false, false, cx);
    }
    fn on_move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, false, false, cx);
    }
    fn on_select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, false, true, cx);
    }
    fn on_select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, false, true, cx);
    }
    fn on_move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, true, false, cx);
    }
    fn on_move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, true, false, cx);
    }
    fn on_select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, true, true, cx);
    }
    fn on_select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, true, true, cx);
    }
    fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, 0.0, cx);
    }
    fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, false, 0.0, cx);
    }
    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, true, 0.0, cx);
    }
    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, true, 0.0, cx);
    }
    fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, 0.85, cx);
    }
    fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, false, 0.85, cx);
    }
    fn on_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let (start, _) = self.line_bounds(cx);
        self.move_to(start, false, cx);
    }
    fn on_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (_, end) = self.line_bounds(cx);
        self.move_to(end, false, cx);
    }
    fn on_select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let (start, _) = self.line_bounds(cx);
        self.move_to(start, true, cx);
    }
    fn on_select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (_, end) = self.line_bounds(cx);
        self.move_to(end, true, cx);
    }
    fn on_doc_start(&mut self, _: &MoveDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }
    fn on_doc_end(&mut self, _: &MoveDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.buffer.read(cx).doc.len();
        self.move_to(end, false, cx);
    }
    fn on_select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, true, cx);
    }
    fn on_select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.buffer.read(cx).doc.len();
        self.move_to(end, true, cx);
    }
    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| {
            let end = buffer.doc.len();
            buffer.set_selection(0, end, cx);
        });
    }

    fn delete_with(&mut self, cx: &mut Context<Self>, extend: impl FnOnce(&Buffer) -> Range<u64>) {
        self.update_buffer(cx, |buffer, cx| {
            let mut range = buffer.selection();
            if range.is_empty() {
                range = extend(buffer);
            }
            if !range.is_empty() {
                buffer.edit(range, "", EditKind::Other, cx);
            }
        });
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_with(cx, |buffer| buffer.prev_grapheme(buffer.cursor)..buffer.cursor);
    }
    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_with(cx, |buffer| buffer.cursor..buffer.next_grapheme(buffer.cursor));
    }
    fn on_delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_with(cx, |buffer| buffer.prev_word(buffer.cursor)..buffer.cursor);
    }
    fn on_delete_line_left(&mut self, _: &DeleteLineLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_with(cx, |buffer| {
            let line = buffer.doc.line_of_offset(buffer.cursor);
            buffer.doc.line_range(line).start..buffer.cursor
        });
    }
    fn on_newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| buffer.newline(cx));
    }
    fn on_indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        let code_indent = cx.global::<Settings>().code_indent;
        self.update_buffer(cx, |buffer, cx| {
            let line = buffer.doc.line_of_offset(buffer.cursor);
            let in_fence = buffer.blocks.block_containing(line).is_some_and(|index| {
                let block = buffer.blocks.blocks[index];
                block.kind == BlockKind::Fence && line > block.start && line + 1 < block.end()
            });
            if in_fence && buffer.selection().is_empty() {
                let selection = buffer.selection();
                buffer.edit(selection, code_indent, EditKind::Typing, cx);
                return;
            }
            let line = buffer.doc.line_text(buffer.doc.line_of_offset(buffer.cursor));
            let trimmed = line.trim_start();
            let is_list = trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") || trimmed.split_once(". ").is_some_and(|(digits, _)| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()));
            if is_list || !buffer.selection().is_empty() {
                buffer.indent_lines(false, cx);
            } else {
                let selection = buffer.selection();
                buffer.edit(selection, "  ", EditKind::Typing, cx);
            }
        });
    }
    fn on_outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| buffer.indent_lines(true, cx));
    }
    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| buffer.undo(cx));
    }
    fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, cx| buffer.redo(cx));
    }
    /// 待复制的文本：语义模式下若关闭了“复制时保留 Markdown 语法”，转为纯文本。
    fn clipboard_text(&self, cx: &App) -> String {
        let text = self.buffer.read(cx).selected_text();
        if self.mode == ViewMode::Semantic && !cx.global::<Settings>().copy_markdown_syntax {
            crate::markdown::render::plain_text(&text)
        } else {
            text
        }
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.clipboard_text(cx);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.clipboard_text(cx);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.update_buffer(cx, |buffer, cx| {
                let range = buffer.selection();
                buffer.edit(range, "", EditKind::Other, cx);
            });
        }
    }
    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            self.update_buffer(cx, |buffer, cx| {
                let range = buffer.selection();
                buffer.edit(range, &text, EditKind::Other, cx);
            });
        }
    }

    // ---- 鼠标 ----

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(bounds) = self.bounds else { return };
        let scrollbar_x = f32::from(bounds.origin.x + bounds.size.width) - SCROLLBAR_WIDTH - 2.0;
        if f32::from(event.position.x) >= scrollbar_x {
            self.dragging_scrollbar = true;
            self.drag_scrollbar_to(event.position, cx);
            return;
        }
        match self.block_action_at(event.position) {
            Some(BlockAction::ToggleTask(at)) => {
                self.update_buffer(cx, |buffer, cx| buffer.toggle_task(at, cx));
                return;
            }
            Some(BlockAction::Copy(text)) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                return;
            }
            None => {}
        }
        let offset = self.hit_test(event.position, self.buffer.read(cx));
        self.selecting = true;
        match event.click_count {
            2 => {
                let (start, end) = {
                    let buffer = self.buffer.read(cx);
                    word_range(buffer, offset)
                };
                self.update_buffer(cx, |buffer, cx| buffer.set_selection(start, end, cx));
            }
            3 => {
                let (start, end) = {
                    let buffer = self.buffer.read(cx);
                    let line = buffer.doc.line_of_offset(offset);
                    let range = buffer.doc.line_range(line);
                    (range.start, range.end)
                };
                self.update_buffer(cx, |buffer, cx| buffer.set_selection(start, end, cx));
            }
            _ => self.move_to(offset, event.modifiers.shift, cx),
        }
        // 点击会让语义块展开，几何变化前不自动滚动，避免内容跳动。
        self.reveal_cursor = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging_scrollbar {
            self.drag_scrollbar_to(event.position, cx);
            return;
        }
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let offset = self.hit_test(event.position, self.buffer.read(cx));
            self.update_buffer(cx, |buffer, cx| {
                let anchor = buffer.anchor;
                buffer.set_selection(anchor, offset, cx);
            });
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        if self.dragging_scrollbar {
            self.dragging_scrollbar = false;
            cx.notify();
        }
    }

    fn drag_scrollbar_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.bounds else { return };
        let fraction = ((f32::from(position.y - bounds.origin.y)) / f32::from(bounds.size.height)).clamp(0.0, 1.0);
        self.target = Some(Target::Fraction(fraction as f64));
        self.user_scrolled = true;
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>();
        if settings.ctrl_wheel_zoom && (event.modifiers.platform || event.modifiers.control) {
            // 设置“Ctrl + 滚轮缩放”：macOS 上 ⌘ 与 ⌃ 都可触发；按 5% 步进，与缩放级别一致。
            let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
            if delta.abs() > 0.5 {
                let next = (settings.zoom + delta.signum() * 0.05).clamp(0.8, 1.6);
                Settings::set(cx, "zoomPercent", serde_json::json!((next * 100.0).round() as i64));
            }
            return;
        }
        let line_height = settings.metrics().body_line_height();
        let delta = event.delta.pixel_delta(px(line_height));
        self.pending_scroll -= f32::from(delta.y);
        self.user_scrolled = true;
        let _ = window;
        cx.notify();
    }
}

enum BlockAction {
    ToggleTask(u64),
    Copy(Rc<str>),
}

fn paint_ranges(layout: &TextLayout, origin: Point<Pixels>, range: Range<usize>, color: Hsla, window: &mut Window) {
    for (visual, x0, x1) in layout.range_rects(range) {
        let y = visual as f32 * layout.line_height;
        paint_box(Bounds::new(origin + point(px(x0), px(y + 1.0)), size(px((x1 - x0).max(1.0)), px(layout.line_height - 2.0))), color, 2.0, None, window);
    }
}

fn word_range(buffer: &Buffer, offset: u64) -> (u64, u64) {
    let line = buffer.doc.line_of_offset(offset);
    let range = buffer.doc.line_range(line);
    let text = buffer.doc.read(range.start, range.end);
    let local = (offset - range.start) as usize;
    for (index, word) in text.split_word_bound_indices() {
        if local >= index && local <= index + word.len() && !word.trim().is_empty() {
            return (range.start + index as u64, range.start + (index + word.len()) as u64);
        }
    }
    (offset, offset)
}

// ---- 输入法 ----

impl EditorView {
    /// 输入法所见文本：组字起点或光标所在的源码行。
    fn ime_line(&self, cx: &App) -> (u64, String) {
        let buffer = self.buffer.read(cx);
        let at = buffer.marked.as_ref().map_or(buffer.cursor, |marked| marked.start);
        let range = buffer.doc.line_range(buffer.doc.line_of_offset(at));
        (range.start, buffer.doc.read(range.start, range.end))
    }
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

fn local_utf16(base: u64, text: &str, range: &Range<u64>) -> Range<usize> {
    let clamp = |offset: u64| (offset.saturating_sub(base) as usize).min(text.len());
    utf8_to_utf16(text, clamp(range.start))..utf8_to_utf16(text, clamp(range.end))
}

impl EntityInputHandler for EditorView {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        let (_, text) = self.ime_line(cx);
        let range = utf16_to_utf8(&text, range_utf16.start)..utf16_to_utf8(&text, range_utf16.end);
        actual.replace(utf8_to_utf16(&text, range.start)..utf8_to_utf16(&text, range.end));
        Some(text[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let (base, text) = self.ime_line(cx);
        let buffer = self.buffer.read(cx);
        Some(UTF16Selection { range: local_utf16(base, &text, &buffer.selection()), reversed: buffer.cursor < buffer.anchor })
    }

    fn marked_text_range(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.buffer.read(cx).marked.clone()?;
        let (base, text) = self.ime_line(cx);
        Some(local_utf16(base, &text, &marked))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.update_buffer(cx, |buffer, _| buffer.unmark());
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let (base, line) = self.ime_line(cx);
        self.update_buffer(cx, |buffer, cx| {
            let range = range_utf16
                .map(|range| base + utf16_to_utf8(&line, range.start) as u64..base + utf16_to_utf8(&line, range.end) as u64)
                .or(buffer.marked.clone())
                .unwrap_or_else(|| buffer.selection());
            buffer.commit_text(range, text, cx);
        });
    }

    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, selected_utf16: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let (base, line) = self.ime_line(cx);
        self.update_buffer(cx, |buffer, cx| {
            let range = range_utf16
                .map(|range| base + utf16_to_utf8(&line, range.start) as u64..base + utf16_to_utf8(&line, range.end) as u64)
                .or(buffer.marked.clone())
                .unwrap_or_else(|| buffer.selection());
            // 新选区相对组字文本起点（gpui 官方示例此处误加 range.end，会越界）。
            let selected = selected_utf16.map(|selected| utf16_to_utf8(text, selected.start)..utf16_to_utf8(text, selected.end));
            buffer.set_marked(range, text, selected, cx);
        });
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, _: Bounds<Pixels>, _: &mut Window, cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let (base, line_text) = self.ime_line(cx);
        let offset = base + utf16_to_utf8(&line_text, range_utf16.start) as u64;
        let buffer = self.buffer.read(cx);
        let line = buffer.doc.line_of_offset(offset);
        for row in &self.visible {
            for part in &row.parts {
                if let Part::Line { line: part_line, start, y, layout } = part {
                    if *part_line == line {
                        let (visual, x) = layout.caret((offset - start) as usize);
                        let top = row.top + y + visual as f32 * layout.line_height;
                        return Some(Bounds::new(point(px(self.content_x + x), px(top)), size(px(2.0), px(layout.line_height))));
                    }
                }
            }
        }
        None
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        let offset = self.hit_test(position, self.buffer.read(cx));
        let (base, text) = self.ime_line(cx);
        (offset >= base && offset <= base + text.len() as u64).then(|| utf8_to_utf16(&text, (offset - base) as usize))
    }
}

// ---- 渲染 ----

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focused = self.focus.is_focused(window);
        let theme = cx.global::<Theme>();
        let background = if self.mode == ViewMode::Source && self.split { theme.surface } else { theme.surface };
        div()
            .size_full()
            .key_context("Editor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .bg(background)
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_move_word_left))
            .on_action(cx.listener(Self::on_move_word_right))
            .on_action(cx.listener(Self::on_select_word_left))
            .on_action(cx.listener(Self::on_select_word_right))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_line_start))
            .on_action(cx.listener(Self::on_line_end))
            .on_action(cx.listener(Self::on_select_line_start))
            .on_action(cx.listener(Self::on_select_line_end))
            .on_action(cx.listener(Self::on_doc_start))
            .on_action(cx.listener(Self::on_doc_end))
            .on_action(cx.listener(Self::on_select_doc_start))
            .on_action(cx.listener(Self::on_select_doc_end))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_delete_word_left))
            .on_action(cx.listener(Self::on_delete_line_left))
            .on_action(cx.listener(Self::on_newline))
            .on_action(cx.listener(Self::on_indent))
            .on_action(cx.listener(Self::on_outdent))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(|_, _: &CharacterPalette, window, _| window.show_character_palette()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement { view: cx.entity() })
    }
}

struct EditorElement {
    view: Entity<EditorView>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            let started = std::time::Instant::now();
            view.layout(bounds, window, cx);
            if let Some(stats) = &mut view.bench {
                if let Some(last) = stats.last.replace(started) {
                    stats.interval.push(started - last);
                }
                stats.layout.push(started.elapsed());
                stats.started = Some(started);
            }
        });
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        self.view.read(cx).paint(bounds, window, cx);
        self.view.update(cx, |view, _| {
            if let Some(stats) = &mut view.bench {
                if let Some(started) = stats.started.take() {
                    stats.frame.push(started.elapsed());
                }
            }
        });
    }
}

/// 一次性排版一个短文本（用于界面中的少量自绘文字）。
#[allow(dead_code)]
pub fn label_layout(text: &str, size_px: f32, color: ColorRole, window: &Window) -> TextLayout {
    let spec = ParaSpec::plain(text.to_string(), InlineStyle { color, ..Default::default() }, Family::Text, size_px, size_px * 1.4, Wrap::Greedy);
    layout_text(&spec, 10_000.0, window)
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let context = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("alt-left", MoveWordLeft, context),
        KeyBinding::new("alt-right", MoveWordRight, context),
        KeyBinding::new("alt-shift-left", SelectWordLeft, context),
        KeyBinding::new("alt-shift-right", SelectWordRight, context),
        KeyBinding::new("cmd-left", MoveLineStart, context),
        KeyBinding::new("cmd-right", MoveLineEnd, context),
        KeyBinding::new("home", MoveLineStart, context),
        KeyBinding::new("end", MoveLineEnd, context),
        KeyBinding::new("cmd-shift-left", SelectLineStart, context),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
        KeyBinding::new("cmd-up", MoveDocStart, context),
        KeyBinding::new("cmd-down", MoveDocEnd, context),
        KeyBinding::new("cmd-shift-up", SelectDocStart, context),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("alt-backspace", DeleteWordLeft, context),
        KeyBinding::new("cmd-backspace", DeleteLineLeft, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("cmd-x", Cut, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("ctrl-cmd-space", CharacterPalette, context),
    ]);
}

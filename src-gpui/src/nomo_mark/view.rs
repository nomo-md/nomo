//! 可独立挂载的 NomoMark 视图。滚动、窗口选择、正文和历史由外部宿主持有。
use super::{
    commands::{self, EditIntent},
    host::WindowHostAdapter,
    mapping::{local_range, utf8_to_utf16, utf16_to_utf8},
    paint::paint_block,
    parse::InlineFormat,
    protocol::*,
    resources::ResourceAdapter,
};
use crate::{
    editor::{self, ViewMode},
    embeds::EmbedStore,
    images::ImageStore,
    layout::{layout_text, paint_box},
    markdown::render::{
        BlockLayout, DisplayItem, MapSeg, RenderEnv, layout_parsed, source_line_spec,
    },
    settings::Settings,
    theme::{Family, Theme},
};
use gpui::{
    App, Bounds, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, Style, UTF16Selection, Window, div, point, prelude::*, px,
    relative, size,
};
use std::{cell::RefCell, ops::Range, path::PathBuf, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;
#[path = "object_ui.rs"]
mod object_ui;
#[path = "composition.rs"]
mod composition;
#[path = "keyboard.rs"]
mod keyboard;
#[path = "quick_insert_view.rs"]
mod quick_insert_view;

gpui::actions!(nomo_mark, [Dismiss, SoftBreak, DeleteWordRight, SelectPageUp, SelectPageDown]);

pub struct NomoMark {
    pub host: WindowHostAdapter,
    pub mode: ViewMode,
    pub base_dir: Option<PathBuf>,
    pub(crate) workspace_hosted: bool,
    pub(crate) windowed_navigation: bool,
    pub(crate) open_profile: Option<crate::open_profile::OpenProfile>,
    focus: FocusHandle,
    layout: Option<Rc<BlockLayout>>,
    layout_key: Option<(LayoutStamp, ViewMode, Vec<Range<usize>>, u64)>,
    layout_resources: super::resources::ResourceDependencies,
    bounds: Option<Bounds<Pixels>>,
    visible_bounds: Option<Bounds<Pixels>>,
    composition_templates: super::template_projection::CompositionTemplates,
    external_composition: Option<composition::ExternalComposition>,
    preferred_x: Option<f32>,
    pending_page: Option<(f32, f32, Option<u64>)>,
    height: f32,
    history_serial: u64,
    menu_open: bool,
    pub(crate) quick_insert: Option<super::quick_insert::QuickInsertSession>,
    slash_range: Option<Range<usize>>,
    menu_index: usize,
    menu_scroll: gpui::ScrollHandle,
    object: Option<object_ui::ObjectEditor>,
    emoji_catalog: Option<std::sync::Arc<super::emoji::EmojiCatalog>>,
    emoji_mode: bool,
    emoji_requested: bool,
    pub(super) auxiliary_input: bool,
    last_geometry: Option<ControlEvent>,
    horizontal_drag: Option<(usize, f32, f32)>,
}
impl EventEmitter<ControlEvent> for NomoMark {}
impl Focusable for NomoMark {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl NomoMark {
    pub(crate) fn effective_mode(&self) -> ViewMode {
        if self.host.parsed.context.source_only { ViewMode::Source } else { self.mode }
    }
    pub fn new(host: WindowHostAdapter, cx: &mut Context<Self>) -> Self {
        Self {
            host,
            mode: ViewMode::Semantic,
            base_dir: None,
            focus: cx.focus_handle(),
            layout: None,
            layout_key: None,
            layout_resources: Default::default(),
            bounds: None,
            visible_bounds: None,
            composition_templates: Default::default(),
            external_composition: None,
            preferred_x: None,
            pending_page: None,
            height: 32.0,
            history_serial: 0,
            menu_open: false,
            quick_insert: None,
            slash_range: None,
            menu_index: 0,
            menu_scroll: gpui::ScrollHandle::new(),
            object: None,
            emoji_catalog: None,
            emoji_mode: false,
            emoji_requested: false,
            auxiliary_input: false,
            workspace_hosted: false,
            windowed_navigation: false,
            open_profile: None,
            last_geometry: None,
            horizontal_drag: None,
        }
    }
    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) -> Result<(), ControlError> {
        if self.host.interaction.composing || self.host.interaction.draft {
            return Err(ControlError::Busy);
        }
        self.mode = mode;
        self.dismiss_quick_insert(cx);
        self.invalidate();
        cx.notify();
        Ok(())
    }
    pub fn update_window(
        &mut self,
        window: DocumentWindow,
        generation: u64,
        context: WindowContext,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        self.host.replace_window(window, generation, context)?;
        self.dismiss_quick_insert(cx);
        self.pending_page = None;
        self.menu_open = false;
        self.slash_range = None;
        self.menu_index = 0;
        self.emoji_mode = false;
        self.emoji_requested = false;
        self.invalidate();
        cx.notify();
        Ok(())
    }
    pub(crate) fn refresh_references(&mut self, context: ParseContext, cx: &mut Context<Self>) -> Result<bool, ControlError> {
        let changed = self.host.refresh_references(context)?;
        if changed {
            self.dismiss_quick_insert(cx);
            self.invalidate();
            cx.notify();
        }
        Ok(changed)
    }
    pub fn acknowledge(
        &mut self,
        operation: &str,
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        self.host.acknowledge(operation, revision)?;
        if let Some(session) = &mut self.quick_insert {
            if session.matches(&self.host.parsed, self.host.selection) {
                session.identity = self.host.parsed.identity.clone();
            } else { self.dismiss_quick_insert(cx); }
        }
        self.submit(cx);
        cx.notify();
        Ok(())
    }
    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.host.submission_failed(message.clone());
        cx.emit(ControlEvent::Failure(message));
        cx.notify();
    }
    pub fn retry(&mut self, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.host.retry()?;
        self.submit(cx);
        cx.notify();
        Ok(())
    }
    pub fn hide(&mut self, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.dismiss_quick_insert(cx);
        self.host.suspend()?;
        self.layout = None;
        self.layout_key = None;
        self.layout_resources = Default::default();
        self.pending_page = None;
        self.bounds = None;
        self.visible_bounds = None;
        self.last_geometry = None;
        self.horizontal_drag = None;
        self.menu_open = false;
        self.slash_range = None;
        cx.notify();
        Ok(())
    }
    pub fn restore(&mut self, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.host.resume()?;
        cx.notify();
        Ok(())
    }
    pub fn prepare_unmount(&self) -> Result<(), ControlError> {
        self.host
            .interaction
            .prepare_detach(self.host.pending_count(), self.host.presentation)
    }
    pub fn detach(&mut self, discard: bool, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.dismiss_quick_insert(cx);
        let identity = self.host.parsed.identity.clone();
        self.host.detach(discard)?;
        self.layout = None;
        self.layout_key = None;
        self.layout_resources = Default::default();
        self.pending_page = None;
        self.bounds = None;
        self.visible_bounds = None;
        self.last_geometry = None;
        self.horizontal_drag = None;
        self.object = None;
        self.external_composition = None;
        self.menu_open = false;
        self.slash_range = None;
        self.emoji_catalog = None;
        cx.emit(ControlEvent::Released(identity));
        cx.notify();
        Ok(())
    }
    pub fn estimated_bytes(&self) -> usize {
        self.host.parsed.estimated_bytes()
            + self.quick_insert.as_ref().map_or(0,|session|std::mem::size_of_val(session)+session.trigger.capacity()+session.identity.document_id.capacity()+session.identity.window_id.capacity()+session.error.as_ref().map_or(0,String::capacity))
            + self.layout_resources.estimated_bytes()
            + self.host.pending_bytes()
            + self.composition_templates.estimated_bytes()
            + self.external_composition.as_ref().map_or(0, |composition| composition.estimated_bytes())
            + self
                .layout
                .as_ref()
                .map_or(0, |layout| layout.estimated_bytes())
            + self
                .object
                .as_ref()
                .map_or(0, |object| object.estimated_bytes())
    }
    fn invalidate(&mut self) {
        self.layout_key = None;
        self.last_geometry = None;
    }
    fn submit(&mut self, cx: &mut Context<Self>) {
        if let Some(event) = self.host.next_submission() {
            cx.emit(event);
        }
    }
    fn input_text(&self) -> &str {
        if let Some(composition) = &self.external_composition { return &composition.text; }
        if local_range(&self.host.parsed.source, self.host.parsed.identity.from_byte, self.host.selection.range()).is_err() {
            return "";
        }
        &self.host.parsed.source
    }
    fn queue_result(&mut self, result: commands::EditResult, group: &str, cx: &mut Context<Self>) -> Result<(), ControlError> {
        if !self.auxiliary_input && self.host.projected_window_bytes(&result.edits)? > super::session_window::policy().group_bytes as usize {
            self.host.queue_rewindow(result.edits, result.selection, group, result.template_update)?;
        } else {
            self.host.queue_template_edit(result.edits, result.selection, group, result.template_update)?;
        }
        self.invalidate();
        self.submit(cx);
        cx.notify();
        Ok(())
    }
    pub(super) fn queue_derived(&mut self, edits: Vec<SourceEdit>, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.host.queue_derived(edits)?;
        self.invalidate();
        self.submit(cx);
        cx.notify();
        Ok(())
    }
    fn error(&self, error: ControlError, cx: &mut Context<Self>) {
        match error {
            ControlError::NeedsContext(reason) => cx.emit(ControlEvent::NeedsContext(reason)),
            other => cx.emit(ControlEvent::Failure(other.to_string())),
        }
    }
    pub fn command(&mut self, intent: EditIntent, cx: &mut Context<Self>) {
        self.dismiss_quick_insert(cx);
        let range = self.host.selection.range();
        let identity = &self.host.parsed.identity;
        let boundary_delete = range.is_empty()
            && ((matches!(intent, EditIntent::Backspace) && range.start == identity.from_byte && identity.from_byte > 0)
                || (matches!(intent, EditIntent::Delete) && range.end == identity.to_byte));
        if range.start < identity.from_byte || range.end > identity.to_byte || boundary_delete {
            cx.emit(ControlEvent::EditOutsideWindow {
                identity: identity.clone(),
                selection: self.host.selection,
                intent,
            });
            return;
        }
        if !matches!(intent, EditIntent::Replace(_)) {
            self.history_serial += 1;
        }
        let result = if self.effective_mode() == ViewMode::Semantic {
            commands::apply_semantic(&self.host.parsed, self.host.selection, intent)
        } else {
            commands::apply(&self.host.parsed, self.host.selection, intent)
        };
        match result {
            Ok(result) => {
                let group = format!(
                    "{}:typing:{}",
                    self.host.parsed.identity.window_id, self.history_serial
                );
                match self.queue_result(result, &group, cx) {
                    Ok(()) => {
                        self.update_completion(cx);
                        self.preferred_x = None;
                    }
                    Err(error) => self.error(error, cx),
                }
            }
            Err(error) => self.error(error, cx),
        }
    }
    pub fn table_command(&mut self, intent: super::table::TableIntent, cx: &mut Context<Self>) {
        use super::{html_table::HtmlTableIntent, table::TableIntent};
        let at = self.local_cursor();
        let result = if let Some(table) = self
            .host
            .parsed
            .html_tables
            .iter()
            .find(|table| table.source.contains(&at))
        {
            let intent = match intent {
                TableIntent::InsertRow { after } => HtmlTableIntent::InsertRow { at: after + 1 },
                TableIntent::DeleteRow { row } => HtmlTableIntent::DeleteRow { row },
                TableIntent::InsertColumn { at } => HtmlTableIntent::InsertColumn { at },
                TableIntent::DeleteColumn { column } => HtmlTableIntent::DeleteColumn { column },
                TableIntent::Align { column, alignment } => {
                    HtmlTableIntent::Align { column, alignment }
                }
                TableIntent::Resize { rows, columns } => HtmlTableIntent::Resize { rows, columns },
                TableIntent::Header { enabled } => HtmlTableIntent::Header { enabled },
            };
            table.edit(
                &self.host.parsed.source,
                self.host.parsed.identity.from_byte,
                intent,
            )
        } else if let Some(table) = super::table::Table::at(&self.host.parsed, at) {
            table.edit(&self.host.parsed, self.host.selection, intent)
        } else {
            return;
        };
        match result {
            Ok(result) => {
                self.history_serial += 1;
                let group = format!(
                    "{}:table:{}",
                    self.host.parsed.identity.window_id, self.history_serial
                );
                match self.queue_result(result, &group, cx) {
                    Ok(()) => {}
                    Err(error) => self.error(error, cx),
                }
            }
            Err(error) => self.error(error, cx),
        }
    }
    pub fn active_cell(&self) -> Option<(usize, usize)> {
        if let Some(cell) = self
            .host
            .parsed
            .html_tables
            .iter()
            .find_map(|table| table.cell_at(self.local_cursor()))
        {
            return Some((cell.row, cell.column));
        }
        super::table::Table::at(&self.host.parsed, self.local_cursor())?
            .cell_at(self.local_cursor())
    }
    pub fn edit_table_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.local_cursor();
        let range = self
            .host
            .parsed
            .html_tables
            .iter()
            .find(|table| table.source.contains(&at))
            .map(|table| table.source.clone())
            .or_else(|| super::table::Table::at(&self.host.parsed, at).map(|table| table.source));
        if let Some(range) = range {
            self.begin_object(range, super::widgets::ObjectKind::TableSize, window, cx);
        }
    }
    fn tab(&mut self, backwards: bool, cx: &mut Context<Self>) {
        if self.effective_mode() == ViewMode::Semantic {
            if let Some(table) = self
                .host
                .parsed
                .html_tables
                .iter()
                .find(|table| table.source.contains(&self.local_cursor()))
            {
                if let Some(cell) = table.next_cell(self.local_cursor(), backwards) {
                    self.move_to(cell.start, false, cx);
                } else if !backwards {
                    let row = table.rows;
                    let source = table.source.start;
                    self.table_command(super::table::TableIntent::InsertRow { after: row - 1 }, cx);
                    if let Some(at) = self
                        .host
                        .parsed
                        .html_tables
                        .iter()
                        .find(|table| table.source.start == source)
                        .and_then(|table| table.cells.iter().find(|cell| cell.row == row))
                        .and_then(|cell| cell.runs.first())
                        .map(|run| run.source.start)
                    {
                        self.move_to(at, false, cx);
                    }
                }
                return;
            }
            if let Some(table) = super::table::Table::at(&self.host.parsed, self.local_cursor()) {
                if let Some(cell) = table.next_cell(self.local_cursor(), backwards) {
                    self.move_to(cell.start, false, cx);
                    return;
                }
                if !backwards {
                    let after = table.rows.len() - 1;
                    let at = table.source.start;
                    self.table_command(super::table::TableIntent::InsertRow { after }, cx);
                    if let Some(table) = super::table::Table::at(&self.host.parsed, at) {
                        if let Some(cell) =
                            table.rows.get(after + 1).and_then(|row| row.cells.first())
                        {
                            self.move_to(cell.start, false, cx);
                        }
                    }
                }
                return;
            }
        }
        self.command(EditIntent::Indent { outdent: backwards }, cx);
    }
    fn local_cursor(&self) -> usize {
        self.host
            .selection
            .head_byte
            .saturating_sub(self.host.parsed.identity.from_byte)
            .min(self.host.parsed.source.len() as u64) as usize
    }
    pub fn select(&mut self, selection: DocumentSelection, cx: &mut Context<Self>) {
        self.dismiss_quick_insert(cx);
        if self.finish_external_composition(true, cx) { return; }
        self.host.selection = selection;
        self.ensure_horizontal_caret();
        self.history_serial += 1;
        cx.emit(ControlEvent::SelectionChanged {
            identity: self.host.parsed.identity.clone(),
            selection,
        });
        cx.notify();
    }
    /// 宿主恢复/同步全局选区，不伪造用户选区事件或切换活动输入视图。
    pub(crate) fn restore_selection(&mut self, selection:DocumentSelection, cx:&mut Context<Self>) {
        self.dismiss_quick_insert(cx);
        self.host.selection=selection;
        self.ensure_horizontal_caret();
        cx.notify();
    }
    /// 宿主已验证目标身份后提交局部编辑；仍走同一乐观事务和会话确认队列。
    pub(crate) fn apply_host_edit(&mut self, result:commands::EditResult, cx:&mut Context<Self>) -> Result<(),ControlError> {
        self.prepare_unmount()?;
        self.history_serial+=1;
        let group=format!("{}:popover:{}",self.host.parsed.identity.window_id,self.history_serial);
        self.queue_result(result,&group,cx)
    }
    fn move_to(&mut self, at: usize, extend: bool, cx: &mut Context<Self>) {
        let at = self.host.parsed.identity.from_byte + at as u64;
        self.select(
            DocumentSelection {
                anchor_byte: if extend {
                    self.host.selection.anchor_byte
                } else {
                    at
                },
                head_byte: at,
            },
            cx,
        );
    }
    fn horizontal(&mut self, forward: bool, word: bool, extend: bool, cx: &mut Context<Self>) {
        if self.keyboard_protected() {
            return;
        }
        if self.quick_insert.is_some() && !word && !extend {
            let category=self.quick_insert.as_ref().unwrap().category;
            self.quick_insert_step(Some(category),forward,cx);return;
        }
        let source = &self.host.parsed.source;
        let at = self.local_cursor();
        let selected = self.host.selection.range();
        if !extend && !selected.is_empty() {
            let target = if forward {
                selected.end
            } else {
                selected.start
            };
            let identity = &self.host.parsed.identity;
            if target < identity.from_byte || target > identity.to_byte {
                cx.emit(ControlEvent::Navigate {
                    identity: identity.clone(),
                    source_byte: target,
                    focus: true,
                });
            } else {
                self.move_to((target - identity.from_byte) as usize, false, cx);
            }
            return;
        }
        let unit = if word { super::navigation::CursorUnit::Word } else { super::navigation::CursorUnit::Grapheme };
        let next = if self.effective_mode() == ViewMode::Semantic {
            self.layout.as_ref().and_then(|layout| layout.horizontal_caret(at, unit, forward)).unwrap_or(if forward { source.len() } else { 0 })
        } else {
            super::navigation::boundary(source, at, unit, forward)
        };
        if next == at || (word && self.windowed_navigation && self.effective_mode() == ViewMode::Source && (next == 0 || next == source.len())) {
            if !self.windowed_navigation {
                cx.emit(ControlEvent::WindowBoundary { forward, identity: self.host.parsed.identity.clone(), extend_selection: extend });
                return;
            }
            cx.emit(ControlEvent::CursorBoundary {
                forward,
                identity: self.host.parsed.identity.clone(),
                extend_selection: extend,
                unit,
                delete: false,
            });
        } else {
            self.preferred_x = None;
            self.move_to(next, extend, cx);
        }
    }
    fn vertical(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        if self.quick_insert.is_some() && !extend {self.quick_insert_step(None,down,cx);return;}
        if self.menu_open && !extend {
            let count = self.menu_entries().len();
            if count > 0 {
                self.menu_index = if down {
                    (self.menu_index + 1) % count
                } else {
                    (self.menu_index + count - 1) % count
                };
                self.menu_scroll.scroll_to_item(self.menu_index);
                cx.notify();
            }
            return;
        }
        let Some(layout) = &self.layout else {
            return;
        };
        let Some((x, y, h)) = layout.caret_at(self.local_cursor()) else {
            return;
        };
        let x = self.preferred_x.unwrap_or(x);
        self.preferred_x = Some(x);
        let target = y + if down { h * 1.5 } else { -h * 0.5 };
        if target < 0.0 || target >= layout.height {
            if !self.windowed_navigation {
                cx.emit(ControlEvent::WindowBoundary { forward: down, identity: self.host.parsed.identity.clone(), extend_selection: extend });
                return;
            }
            cx.emit(ControlEvent::VerticalNavigation {
                down,
                identity: self.host.parsed.identity.clone(),
                extend_selection: extend,
                page: false,
            });
            return;
        }
        if let Some(at) = layout.source_at(x, target) {
            self.move_to(at, extend, cx);
        }
    }
    fn home_end(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        let source = &self.host.parsed.source;
        let at = self.local_cursor();
        let target = self.layout.as_ref().and_then(|layout| layout.line_boundary(at, end)).unwrap_or_else(|| if end {
            source[at..]
                .find(['\r', '\n'])
                .map_or(source.len(), |p| at + p)
        } else {
            commands::line_start(source, at)
        });
        self.preferred_x = None;
        self.move_to(target, extend, cx);
    }
    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let selection = self.host.selection;
        let Ok(range) = local_range(
            &self.host.parsed.source,
            self.host.parsed.identity.from_byte,
            selection.range(),
        ) else {
            cx.emit(ControlEvent::ClipboardRequest {
                identity: self.host.parsed.identity.clone(),
                selection,
                cut,
            });
            return;
        };
        if range.is_empty() {
            return;
        }
        let text = self.host.parsed.source[range].to_string();
        match crate::clipboard::write_text(&text, cx) {
            Ok(()) if cut => self.command(EditIntent::Replace(String::new()), cx),
            Ok(()) => {}
            Err(error) => cx.emit(ControlEvent::Failure(error)),
        }
    }
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.command(EditIntent::Replace(text), cx);
        }
    }
    pub fn open_menu(&mut self, cx: &mut Context<Self>) {
        self.dismiss_quick_insert(cx);
        self.menu_open = !self.menu_open;
        self.slash_range = None;
        self.menu_index = 0;
        self.emoji_mode = false;
        self.menu_scroll.set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }
    pub fn set_emoji_catalog(
        &mut self,
        catalog: std::sync::Arc<super::emoji::EmojiCatalog>,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        if self.host.presentation == Presentation::Detached {
            return Err(ControlError::Detached);
        }
        self.emoji_catalog = Some(catalog);
        self.emoji_requested = false;
        cx.notify();
        Ok(())
    }
    pub fn emoji_catalog_failed(&mut self, cx: &mut Context<Self>) {
        self.emoji_requested = false;
        cx.notify();
    }
    fn menu_entries(&self) -> Vec<super::menu::MenuEntry> {
        let query = self
            .slash_range
            .as_ref()
            .and_then(|r| self.host.parsed.source.get(r.start + 1..r.end))
            .unwrap_or("");
        if !self.menu_open {
            return Vec::new();
        }
        if self.emoji_mode {
            return self
                .emoji_catalog
                .as_ref()
                .map_or_else(Vec::new, |catalog| {
                    catalog
                        .search(query, 8)
                        .iter()
                        .map(|entry| super::menu::MenuEntry {
                            label: format!("{} {}", entry.unicode, entry.zh),
                            keywords: "",
                            intent: EditIntent::Replace(entry.unicode.clone()),
                        })
                        .collect()
                });
        }
        super::menu::search(query)
    }
    fn accept_menu_or_newline(&mut self, cx: &mut Context<Self>) {
        if self.request_quick_insert(cx) {return;}
        if self.menu_open {
            if let Some(entry) = self.menu_entries().get(self.menu_index) {
                self.choose_menu(entry.intent.clone(), cx);
            }
            return;
        }
        self.command(
            if self.effective_mode() == ViewMode::Semantic {
                EditIntent::ParagraphBreak
            } else {
                EditIntent::Newline
            },
            cx,
        );
    }
    fn choose_menu(&mut self, intent: EditIntent, cx: &mut Context<Self>) {
        if let Some(range) = self.slash_range.take() {
            // 先在同一局部投影计算命令，再把斜杠查询与命令折成一笔源码替换。
            let original = self.host.parsed.clone();
            let base = original.identity.from_byte;
            let mut source = original.source.to_string();
            source.replace_range(range.clone(), "");
            let mut identity = original.identity.clone();
            identity.to_byte = base + source.len() as u64;
            let mut context = original.context.clone();
            crate::markdown_templates::map_templates(
                &mut context.inline_templates,
                [(base + range.start as u64, base + range.end as u64, 0)],
                false,
            );
            let parsed = super::parse::ParsedWindow::from_source_with_context(
                identity,
                source.clone().into(),
                context,
            );
            let selected = DocumentSelection::caret(base + range.start as u64);
            let command = if self.effective_mode() == ViewMode::Semantic { commands::apply_semantic(&parsed, selected, intent) } else { commands::apply(&parsed, selected, intent) };
            match command {
                Ok(result) => {
                    for edit in result.edits.into_iter().rev() {
                        source.replace_range(
                            (edit.from_byte - base) as usize..(edit.to_byte - base) as usize,
                            &edit.inserted_text,
                        );
                    }
                    let mut from = 0;
                    let mut to_old = original.source.len();
                    let mut to_new = source.len();
                    while from < to_old
                        && from < to_new
                        && original.source.as_bytes()[from] == source.as_bytes()[from]
                    {
                        from += 1;
                    }
                    while !source.is_char_boundary(from) || !original.source.is_char_boundary(from)
                    {
                        from -= 1;
                    }
                    while to_old > from
                        && to_new > from
                        && original.source.as_bytes()[to_old - 1] == source.as_bytes()[to_new - 1]
                    {
                        to_old -= 1;
                        to_new -= 1;
                    }
                    while !source.is_char_boundary(to_new)
                        || !original.source.is_char_boundary(to_old)
                    {
                        to_new += 1;
                        to_old += 1;
                    }
                    let edit = SourceEdit {
                        from_byte: base + from as u64,
                        to_byte: base + to_old as u64,
                        inserted_text: source[from..to_new].into(),
                    };
                    self.history_serial += 1;
                    let group = format!(
                        "{}:menu:{}",
                        original.identity.window_id, self.history_serial
                    );
                    match self.host.queue_template_edit(
                        vec![edit],
                        result.selection,
                        &group,
                        result.template_update,
                    ) {
                        Ok(()) => {
                            self.invalidate();
                            self.submit(cx);
                        }
                        Err(error) => self.error(error, cx),
                    }
                }
                Err(error) => self.error(error, cx),
            }
        } else {
            self.command(intent, cx);
        }
        self.menu_open = false;
        cx.notify();
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.finish_external_composition(true, cx) { return; }
        if self.host.interaction.draft {
            return;
        }
        window.focus(&self.focus, cx);
        if !matches!(
            self.host.presentation,
            Presentation::Editable | Presentation::ReadOnly
        ) {
            return;
        }
        if let (Some(layout), Some(bounds)) = (&self.layout, self.bounds) {
            let x = f32::from(event.position.x - bounds.origin.x);
            let y = f32::from(event.position.y - bounds.origin.y);
            for item in &layout.items {
                match item {
                    DisplayItem::Horizontal {
                        x: ix,
                        y: iy,
                        w,
                        h,
                        content_width,
                        source,
                        offset,
                        ..
                    } if x >= *ix && x <= ix + w && y >= iy + h - 8.0 && y <= iy + h => {
                        let ratio = w / content_width;
                        let value =
                            ((x - ix) / ratio - w / 2.0).clamp(0.0, (content_width - w).max(0.0));
                        offset.set(value);
                        self.horizontal_drag = Some((source.start, x, value));
                        self.host.interaction.horizontal_offsets.insert(
                            self.host.parsed.identity.from_byte + source.start as u64,
                            value,
                        );
                        self.host.interaction.dragging = true;
                        cx.notify();
                        return;
                    }
                    DisplayItem::Disclosure {
                        x: ix,
                        y: iy,
                        size,
                        source,
                        open,
                    } if x >= *ix && x <= ix + size && y >= *iy && y <= iy + size => {
                        if self.host.interaction.protected() {
                            self.error(ControlError::Busy, cx);
                            return;
                        }
                        let base = self.host.parsed.identity.from_byte;
                        self.host
                            .interaction
                            .details_open
                            .insert(base + *source as u64, !open);
                        if *open {
                            if let Some(block) = self
                                .host
                                .parsed
                                .details
                                .iter()
                                .find(|b| b.source.start == *source)
                            {
                                if self.host.selection.range().start < base + block.body.end as u64
                                    && base + (block.body.start as u64)
                                        < self.host.selection.range().end
                                {
                                    self.host.selection =
                                        DocumentSelection::caret(base + block.summary.start as u64);
                                }
                            }
                        }
                        self.invalidate();
                        cx.notify();
                        return;
                    }
                    DisplayItem::Checkbox {
                        x: ix,
                        y: iy,
                        size,
                        checked,
                        source,
                    } if x >= *ix && x <= ix + size && y >= *iy && y <= iy + size => {
                        let base = self.host.parsed.identity.from_byte;
                        let from = base + *source as u64 + 1;
                        let text = if *checked { " " } else { "x" }.to_owned();
                        let result = self.host.queue_edit(
                            vec![SourceEdit {
                                from_byte: from,
                                to_byte: from + 1,
                                inserted_text: text,
                            }],
                            DocumentSelection::caret(from + 2),
                            "task-toggle",
                        );
                        match result {
                            Ok(()) => {
                                self.invalidate();
                                self.submit(cx);
                                cx.notify();
                            }
                            Err(error) => self.error(error, cx),
                        }
                        return;
                    }
                    DisplayItem::CopyButton {
                        x: ix,
                        y: iy,
                        size,
                        text,
                    } if x >= *ix && x <= ix + size && y >= *iy && y <= iy + size => {
                        if let Err(error) = crate::clipboard::write_text(text, cx) {
                            cx.emit(ControlEvent::Failure(error));
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        let object_at = (self.effective_mode() == ViewMode::Semantic
            && (event.click_count == 2 || event.modifiers.control || event.modifiers.platform))
            .then(|| {
                let bounds = self.content_bounds()?;
                self.layout.as_ref()?.object_source_at(
                    f32::from(event.position.x - bounds.left()),
                    f32::from(event.position.y - bounds.top()),
                )
            }).flatten();
        let Some(at) = object_at.or_else(|| self.hit(event.position)) else {
            return;
        };
        if (event.modifiers.control || event.modifiers.platform)
            && !self.host.parsed.context.reference_pending(&(at..at))
        {
            if let Some(target) = self
                .host
                .parsed
                .html_tables
                .iter()
                .flat_map(|table| &table.cells)
                .flat_map(|cell| &cell.runs)
                .find(|run| run.source.contains(&at))
                .and_then(|run| run.link.clone().or_else(|| match run.atom.as_ref() { Some(super::html_table::InlineAtom::Footnote(id)) => Some(format!("#fn-{}",id.replace('%',"%25"))), _ => None }))
            {
                cx.emit(ControlEvent::OpenLink(target));
                return;
            }
            if let Some(super::html_table::InlineAtom::Footnote(id)) = super::table_inline::atom_at(&self.host.parsed, at).and_then(|run| run.atom) {
                cx.emit(ControlEvent::OpenLink(format!("#fn-{}",id.replace('%',"%25"))));
                return;
            }
            for (event, range) in &self.host.parsed.events {
                if range.contains(&at) {
                    if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                        dest_url,
                        ..
                    }) = event
                    {
                        cx.emit(ControlEvent::OpenLink(dest_url.to_string()));
                        return;
                    }
                }
            }
        }
        if event.click_count == 2 && self.effective_mode() == ViewMode::Semantic
            && !self.host.parsed.context.reference_pending(&(at..at))
        {
            if let Some((range, kind)) = super::table_inline::atom_at(&self.host.parsed, at).and_then(|run| {
                    use super::html_table::InlineAtom;
                    let kind = match run.atom.as_ref()? { InlineAtom::Image { .. } => super::widgets::ObjectKind::Image,
                        InlineAtom::Math(_) => super::widgets::ObjectKind::InlineMath,
                        InlineAtom::Comment(_) | InlineAtom::Footnote(_) => super::widgets::ObjectKind::Html };
                    Some((run.source.clone(), kind))
                }) {
                    self.begin_object(range, kind, window, cx); return;
                }
            for (event, range) in &self.host.parsed.events {
                if !range.contains(&at) {
                    continue;
                }
                let kind = match event {
                    pulldown_cmark::Event::Html(html) | pulldown_cmark::Event::InlineHtml(html)
                        if html.to_ascii_lowercase().contains("<img") => Some(super::widgets::ObjectKind::Image),
                    pulldown_cmark::Event::Html(html) | pulldown_cmark::Event::InlineHtml(html)
                        if html.starts_with("<!--") => Some(super::widgets::ObjectKind::Html),
                    pulldown_cmark::Event::InlineMath(_) => {
                        Some(super::widgets::ObjectKind::InlineMath)
                    }
                    pulldown_cmark::Event::DisplayMath(_) => {
                        Some(super::widgets::ObjectKind::BlockMath)
                    }
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. }) => {
                        Some(super::widgets::ObjectKind::Image)
                    }
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. }) => {
                        Some(super::widgets::ObjectKind::Link)
                    }
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(kind)) => Some(
                        if matches!(kind,pulldown_cmark::CodeBlockKind::Fenced(info)if info.starts_with("mermaid"))
                        {
                            super::widgets::ObjectKind::Mermaid
                        } else {
                            super::widgets::ObjectKind::Code
                        },
                    ),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let range = range.clone();
                    self.begin_object(range, kind, window, cx);
                    return;
                }
            }
        }
        if event.click_count == 2 {
            let source = &self.host.parsed.source;
            if let Some((start, word)) = source
                .split_word_bound_indices()
                .find(|(start, word)| *start <= at && at < start + word.len())
            {
                let base = self.host.parsed.identity.from_byte;
                self.select(
                    DocumentSelection {
                        anchor_byte: base + start as u64,
                        head_byte: base + (start + word.len()) as u64,
                    },
                    cx,
                );
            }
        } else if event.click_count >= 3 {
            let source = &self.host.parsed.source;
            let start = commands::line_start(source, at);
            let end = source[at..].find('\n').map_or(source.len(), |p| at + p + 1);
            let base = self.host.parsed.identity.from_byte;
            self.select(
                DocumentSelection {
                    anchor_byte: base + start as u64,
                    head_byte: base + end as u64,
                },
                cx,
            );
        } else {
            self.move_to(at, event.modifiers.shift, cx);
        }
        self.host.interaction.dragging = true;
        self.host.interaction.drag_selection_started = false;
        self.preferred_x = None;
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((source, start, initial)) = self.horizontal_drag {
            if let (Some(bounds), Some(layout)) = (self.bounds, &self.layout) {
                if let Some(DisplayItem::Horizontal{w,content_width,offset,..})=layout.items.iter().find(|item|matches!(item,DisplayItem::Horizontal{source:range,..}if range.start==source)){
                    let x=f32::from(event.position.x-bounds.origin.x);
                    let value=(initial+(x-start)*content_width/w).clamp(0.0,(content_width-w).max(0.0));offset.set(value);
                    self.host.interaction.horizontal_offsets.insert(self.host.parsed.identity.from_byte+source as u64,value);cx.notify();
                }
            }
            return;
        }
        if self.host.interaction.dragging {
            if let Some(at) = self.hit(event.position) {
                self.move_to(at, true, cx);
                self.host.interaction.drag_selection_started |=
                    !self.host.selection.range().is_empty();
            }
        }
    }
    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal_drag = None;
        self.host.interaction.dragging = false;
        self.host.interaction.drag_selection_started = false;
        cx.notify();
    }
    fn scroll_horizontal(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.0));
        let dx = if f32::from(delta.x).abs() > 0.01 {
            f32::from(delta.x)
        } else if event.modifiers.shift {
            f32::from(delta.y)
        } else {
            return;
        };
        if self.host.interaction.composing || self.host.interaction.draft {
            return;
        }
        let (Some(bounds), Some(layout)) = (self.bounds, &self.layout) else {
            return;
        };
        let x = f32::from(event.position.x - bounds.origin.x);
        let y = f32::from(event.position.y - bounds.origin.y);
        for item in &layout.items {
            if let DisplayItem::Horizontal {
                x: ix,
                y: iy,
                w,
                h,
                content_width,
                source,
                offset,
                ..
            } = item
            {
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h {
                    let value = (offset.get() - dx).clamp(0.0, (content_width - w).max(0.0));
                    offset.set(value);
                    self.host.interaction.horizontal_offsets.insert(
                        self.host.parsed.identity.from_byte + source.start as u64,
                        value,
                    );
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
            }
        }
    }
    fn ensure_horizontal_caret(&mut self) {
        if self
            .layout_key
            .as_ref()
            .is_none_or(|key| key.0.identity != self.host.parsed.identity)
        {
            return;
        }
        let Some(layout) = &self.layout else {
            return;
        };
        let at = self.local_cursor();
        for item in &layout.items {
            if let DisplayItem::Horizontal {
                source,
                w,
                content_width,
                offset,
                layout,
                ..
            } = item
            {
                if source.start <= at && at <= source.end {
                    if let Some((x, _, _)) = layout.caret_at(at) {
                        let current = offset.get();
                        let next = if x < current + 4.0 {
                            x - 4.0
                        } else if x > current + w - 6.0 {
                            x - w + 6.0
                        } else {
                            current
                        };
                        let next = next.clamp(0.0, (content_width - w).max(0.0));
                        offset.set(next);
                        self.host.interaction.horizontal_offsets.insert(
                            self.host.parsed.identity.from_byte + source.start as u64,
                            next,
                        );
                    }
                }
            }
        }
    }
    fn hit(&self, position: Point<Pixels>) -> Option<usize> {
        if self.layout_key.as_ref()?.0.identity != self.host.parsed.identity {
            return None;
        }
        let bounds = self.bounds?;
        let layout = self.layout.as_ref()?;
        let y = f32::from(position.y - bounds.origin.y);
        let at = if y < 0.0 {
            layout.line_boundary(0, false)
        } else if y >= layout.height {
            layout.line_boundary(self.host.parsed.source.len(), true)
        } else {
            layout.source_at(f32::from(position.x - bounds.origin.x), y)
        }?;
        let mut at = at.min(self.host.parsed.source.len());
        while !self.host.parsed.source.is_char_boundary(at) {
            at -= 1;
        }
        Some(at)
    }
    pub(crate) fn content_bounds(&self) -> Option<Bounds<Pixels>> {
        (self.layout_key.as_ref()?.0.identity == self.host.parsed.identity).then_some(self.bounds).flatten()
    }
    /// 纸面留白沿用正文命中及选区事件，不创建文档补丁或历史记录。
    pub(crate) fn place_caret_at(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.interaction.protected() || self.host.presentation != Presentation::Editable { return; }
        if let Some(at) = self.hit(position) {
            window.focus(&self.focus, cx);
            self.move_to(at, false, cx);
            self.preferred_x = None;
        }
    }
    /// 当前父容器已采用控件的实际布局高度时返回，用于宿主结束滚动定位。
    pub(crate) fn measured_height(&self) -> Option<f32> {
        if self.layout_key.as_ref()?.0.identity != self.host.parsed.identity { return None; }
        (self.bounds.is_some_and(|bounds| (f32::from(bounds.size.height) - self.height).abs() < 0.5))
            .then_some(self.height)
    }
    /// 返回当前局部布局的光标行；最终可见性还须等待父容器采用实际高度。
    pub(crate) fn caret_line(&self, byte: u64) -> Option<(WindowIdentity, f32, f32)> {
        let identity = &self.host.parsed.identity;
        if self.layout_key.as_ref()?.0.identity != *identity { return None; }
        let local = byte.checked_sub(identity.from_byte)? as usize;
        if local > self.host.parsed.source.len() { return None; }
        let (_, y, height) = self.layout.as_ref()?.caret_at(local)?;
        Some((identity.clone(), y, height))
    }
    /// 从当前布局的屏幕位置提取完整文档字节锚点，供宿主保持阅读位置和同步双栏。
    pub(crate) fn source_at_screen_y(&self, y: f32) -> Option<(u64, f32)> {
        if self.layout_key.as_ref()?.0.identity != self.host.parsed.identity { return None; }
        let bounds=self.bounds?;
        if y<f32::from(bounds.top()) || y>=f32::from(bounds.bottom()) {return None;}
        let layout=self.layout.as_ref()?;
        let local=layout.source_at(0.0,y-f32::from(bounds.top()))?;
        let (_,line_y,_)=layout.caret_at(local)?;
        Some((self.host.parsed.identity.from_byte+local as u64,f32::from(bounds.top())+line_y))
    }
    pub fn caret_bounds(&self, byte: u64) -> Option<(WindowIdentity, Bounds<Pixels>)> {
        if let Some(composition) = &self.external_composition {
            if byte == self.host.selection.head_byte { return self.external_caret_bounds(composition.selected.end); }
        }
        self.document_caret_bounds(byte)
    }
    fn document_caret_bounds(&self, byte: u64) -> Option<(WindowIdentity, Bounds<Pixels>)> {
        if self.layout_key.as_ref()?.0.identity != self.host.parsed.identity {
            return None;
        }
        let bounds = self.bounds?;
        let local = byte.checked_sub(self.host.parsed.identity.from_byte)? as usize;
        if local > self.host.parsed.source.len() {
            return None;
        }
        let (x, y, h) = self.layout.as_ref()?.caret_at(local)?;
        Some((
            self.host.parsed.identity.clone(),
            Bounds::new(bounds.origin + point(px(x), px(y)), size(px(1.5), px(h))),
        ))
    }
    /// 弹层使用当前可见窗口几何，不能以整段内容顶部为锚点。
    fn popup_position(&self, width: f32, height: f32, above: bool) -> (f32, f32) {
        let Some(bounds) = self.bounds else {
            return (0.0, 0.0);
        };
        let visible = self.visible_bounds.unwrap_or(bounds);
        let left = f32::from(visible.origin.x - bounds.origin.x).max(0.0);
        let top = f32::from(visible.origin.y - bounds.origin.y).max(0.0);
        let right = left + f32::from(visible.size.width);
        let bottom = top + f32::from(visible.size.height);
        let caret = self
            .caret_bounds(self.host.selection.head_byte)
            .map(|(_, bounds)| bounds);
        let x = caret.map_or(left, |caret| f32::from(caret.origin.x - bounds.origin.x));
        let y = caret.map_or(top, |caret| {
            f32::from(caret.origin.y - bounds.origin.y)
                + if above {
                    -height - 4.0
                } else {
                    f32::from(caret.size.height) + 4.0
                }
        });
        (
            x.clamp(left, (right - width).max(left)),
            y.clamp(top, (bottom - height).max(top)),
        )
    }
    fn report_geometry(&mut self, cx: &mut Context<Self>) {
        let (Some((stamp, ..)), Some(layout), Some(bounds), Some(visible)) = (
            &self.layout_key,
            &self.layout,
            self.bounds,
            self.visible_bounds,
        ) else {
            return;
        };
        if stamp.identity != self.host.parsed.identity {
            return;
        }
        let rect = |bounds: Bounds<Pixels>| GeometryRect {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        };
        let base = self.host.parsed.identity.from_byte;
        let top = f32::from(visible.origin.y - bounds.origin.y).max(0.0);
        let anchor = layout.source_at(0.0, top).unwrap_or(0);
        let anchor_y =
            layout.caret_at(anchor).map_or(top, |(_, y, _)| y) + f32::from(bounds.origin.y);
        let selected = self.host.selection.range();
        let selection = layout
            .source_rects_clipped(
                selected.start.saturating_sub(base) as usize
                    ..selected.end.saturating_sub(base) as usize,
                top..top + f32::from(visible.size.height),
            )
            .into_iter()
            .filter_map(|(x, y, w, h)| {
                if y + h < top || y > top + f32::from(visible.size.height) {
                    return None;
                }
                Some(GeometryRect {
                    x: x + f32::from(bounds.origin.x),
                    y: y + f32::from(bounds.origin.y),
                    width: w,
                    height: h,
                })
            })
            .collect();
        let event = ControlEvent::Geometry {
            stamp: stamp.clone(),
            viewport: rect(visible),
            anchor_byte: base + anchor as u64,
            anchor_y,
            caret: self
                .caret_bounds(self.host.selection.head_byte)
                .map(|(_, bounds)| rect(bounds)),
            selection,
        };
        if self.last_geometry.as_ref() != Some(&event) {
            self.last_geometry = Some(event.clone());
            cx.emit(event);
        }
    }
    fn prepare_layout(&mut self, width: f32, window: &Window, cx: &mut Context<Self>) {
        if width <= 0.0
            || matches!(
                self.host.presentation,
                Presentation::Suspended | Presentation::Detached
            )
        {
            return;
        }
        self.layout_external_composition(width, window, cx);
        let settings = cx.global::<Settings>();
        let metrics = settings.metrics();
        let wrap = settings.wrap();
        let resource_generation = self.layout_resources.generation(cx.global::<ImageStore>(), cx.global::<EmbedStore>());
        let mut stamp = LayoutStamp {
            identity: self.host.parsed.identity.clone(),
            width,
            font_generation: crate::layout::generation(),
            resource_generation,
            zoom: metrics.zoom,
        };
        let selection = self.host.selection.range();
        let base = self.host.parsed.identity.from_byte;
        let local = (selection.start.saturating_sub(base) as usize)
            ..(selection.end.saturating_sub(base) as usize);
        let revealed = if self.effective_mode() == ViewMode::Semantic && self.focus.is_focused(window) {
            self.host
                .interaction
                .markers(&self.host.parsed, local)
                .to_vec()
        } else {
            Vec::new()
        };
        use std::hash::{Hash, Hasher};
        let mut configuration = std::collections::hash_map::DefaultHasher::new();
        metrics.font_size.to_bits().hash(&mut configuration);
        metrics.line_height.to_bits().hash(&mut configuration);
        settings.wrap().hash(&mut configuration);
        settings.inline_code_rendering.hash(&mut configuration);
        settings.code_line_numbers.hash(&mut configuration);
        cx.global::<Theme>().dark.hash(&mut configuration);
        self.base_dir.hash(&mut configuration);
        self.host.interaction.details_open.hash(&mut configuration);
        let mut key = (
            stamp.clone(),
            self.mode,
            revealed.clone(),
            configuration.finish(),
        );
        if self.layout_key.as_ref() == Some(&key) {
            return;
        }
        let mut reuse = crate::layout::TextLayoutReuse::default();
        if let Some(layout) = &self.layout { layout.remember_text(&mut reuse); }
        let reuse = RefCell::new(reuse);
        self.layout_resources = Default::default();
        let mut layout = if self.effective_mode() == ViewMode::Source {
            let mut items = Vec::new();
            let mut y = 0.0;
            for line in super::source_view::lines(&self.host.parsed) {
                let text = &self.host.parsed.source[line.range.clone()];
                let spec = source_line_spec(
                    text,
                    line.context,
                    line.language,
                    Family::Mono,
                    metrics.source_size(),
                    metrics.source_line_height(),
                );
                let layout = reuse.borrow_mut().layout(&spec, width, window);
                let height = layout.height();
                items.push(DisplayItem::Text {
                    x: 0.0,
                    y,
                    layout,
                    map: vec![MapSeg {
                        text: 0..text.len(),
                        source: line.range.start,
                        source_end: line.range.end,
                        exact: true,
                    }],
                });
                y += height;
            }
            BlockLayout { height: y, items }
        } else {
            let theme = cx.global::<Theme>();
            let rgba = gpui::Rgba::from(theme.fg);
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let resources = ResourceAdapter {
                images: cx.global::<ImageStore>(),
                embeds: cx.global::<EmbedStore>(),
                em_px: metrics.body_size(),
                dark: theme.dark,
                color: [byte(rgba.r), byte(rgba.g), byte(rgba.b)],
                demands: RefCell::new(Vec::new()),
                dependencies: Default::default(),
            };
            let image_size = |path: &std::path::Path| resources.image_size(path);
            let image_failure = |path: &std::path::Path| resources.images.failure(path);
            let embed = |kind, source: &str| resources.embed(kind, source);
            let env = RenderEnv {
                metrics,
                wrap: settings.wrap(),
                base_dir: self.base_dir.as_deref(),
                image_size: &image_size,
                image_failure: &image_failure,
                text_layouts: Some(&reuse),
                wide_width: width,
                code_line_numbers: settings.code_line_numbers,
                inline_code: settings.inline_code_rendering,
                embed: &embed,
            };
            let layout = layout_parsed(
                &self.host.parsed,
                &revealed,
                &self.host.interaction.details_open,
                width,
                &env,
                window,
            );
            let demands = resources.demands.into_inner();
            self.layout_resources = resources.dependencies.into_inner();
            if !demands.is_empty() {
                cx.emit(ControlEvent::ResourceDemand {
                    identity: self.host.parsed.identity.clone(),
                    requests: demands,
                });
            }
            layout
        };
        // 空文档和仅含空白的片段也必须具有真实行高、命中位置及 IME 光标。
        if layout.items.is_empty() {
            let spec = crate::layout::ParaSpec::plain(
                String::new(),
                Default::default(),
                Family::Text,
                metrics.body_size(),
                metrics.body_line_height(),
                wrap,
            );
            let text = Rc::new(layout_text(&spec, width, window));
            layout.height = text.height();
            layout.items.push(DisplayItem::Text {
                x: 0.0,
                y: 0.0,
                layout: text,
                map: vec![MapSeg {
                    text: 0..0,
                    source: self.host.parsed.source.len(),
                    source_end: self.host.parsed.source.len(),
                    exact: true,
                }],
            });
        }
        let old = self.height;
        for item in &layout.items {
            if let DisplayItem::Horizontal {
                source,
                offset,
                w,
                content_width,
                ..
            } = item
            {
                if let Some(value) = self
                    .host
                    .interaction
                    .horizontal_offsets
                    .get(&(base + source.start as u64))
                {
                    offset.set(value.clamp(0.0, (content_width - w).max(0.0)));
                }
            }
        }
        self.height = layout.height.max(metrics.body_line_height());
        stamp.resource_generation = self.layout_resources.generation(cx.global::<ImageStore>(), cx.global::<EmbedStore>());
        key.0 = stamp.clone();
        self.layout = Some(Rc::new(layout));
        self.layout_key = Some(key);
        self.resolve_page_cursor(cx);
        self.ensure_horizontal_caret();
        cx.emit(ControlEvent::ResourceUsage {
            identity: self.host.parsed.identity.clone(),
            parsed_bytes: self.host.parsed.estimated_bytes(),
            layout_bytes: self
                .layout
                .as_ref()
                .map_or(0, |layout| layout.estimated_bytes()),
            pending_bytes: self.host.pending_bytes(),
        });
        if (old - self.height).abs() > 0.1 {
            cx.emit(ControlEvent::HeightChanged {
                stamp: stamp.clone(),
                height: self.height,
            });
            cx.notify();
        }
        // 宿主还没提交初始高度前保持 Preparing；下一帧用实际布局验证后开放输入。
        if self
            .bounds
            .is_some_and(|b| (f32::from(b.size.height) - self.height).abs() < 0.5)
        {
            if let Ok(event) = self.host.mark_layout_ready(stamp, self.height) {
                cx.emit(event);
            }
        }
    }
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
        let animate_theme = Theme::ui_transition_active(cx);
        let Some(layout) = &self.layout else {
            return;
        };
        let theme = Theme::ui(cx);
        let base = self.host.parsed.identity.from_byte;
        let selected = self.host.selection.range();
        let clip = self.visible_bounds.map_or(0.0..f32::from(bounds.size.height), |visible| {
            f32::from(visible.origin.y - bounds.origin.y).max(0.0)..f32::from(visible.origin.y + visible.size.height - bounds.origin.y)
        });
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (x, y, w, h) in layout.source_rects_clipped(
                selected.start.saturating_sub(base) as usize
                    ..selected.end.saturating_sub(base) as usize,
                clip.clone(),
            ) {
                paint_box(
                    Bounds::new(bounds.origin + point(px(x), px(y)), size(px(w), px(h))),
                    theme.selection,
                    0.0,
                    None,
                    window,
                );
            }
            paint_block(
                layout,
                bounds.origin,
                clip.clone(),
                &theme,
                cx.global::<ImageStore>(),
                window,
                cx,
            );
            if let Some(marked) = &self.host.interaction.marked {
                for (x, y, w, h) in layout.source_rects_clipped(marked.clone(),clip.clone()) {
                    paint_box(
                        Bounds::new(
                            bounds.origin + point(px(x), px(y + h - 1.0)),
                            size(px(w), px(1.0)),
                        ),
                        theme.caret,
                        0.0,
                        None,
                        window,
                    );
                }
            }
            if self.focus.is_focused(window) && self.external_composition.is_none() && selected.is_empty() {
                if let Some((_, caret)) = self.caret_bounds(selected.start) {
                    paint_box(caret, theme.caret, 0.0, None, window);
                }
            }
            self.paint_external_composition(&theme, window);
        });
        if animate_theme { window.request_animation_frame(); }
    }
}

impl EntityInputHandler for NomoMark {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.input_text();
        let range = utf16_to_utf8(text, range.start)..utf16_to_utf8(text, range.end);
        if range.start > range.end {
            return None;
        }
        *actual = Some(utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end));
        Some(text[range].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if let Some(composition) = &self.external_composition {
            return Some(UTF16Selection { range: utf8_to_utf16(&composition.text, composition.selected.start)..utf8_to_utf16(&composition.text, composition.selected.end), reversed: false });
        }
        let text = &self.host.parsed.source;
        let selected = match local_range(
            text,
            self.host.parsed.identity.from_byte,
            self.host.selection.range(),
        ) {
            Ok(range) => range,
            Err(_) => return Some(UTF16Selection { range: 0..0, reversed: false }),
        };
        Some(UTF16Selection {
            range: utf8_to_utf16(text, selected.start)..utf8_to_utf16(text, selected.end),
            reversed: self.host.selection.head_byte < self.host.selection.anchor_byte,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        if let Some(composition) = &self.external_composition { return Some(0..composition.text.encode_utf16().count()); }
        let marked = self.host.interaction.marked.as_ref()?;
        let text = &self.host.parsed.source;
        Some(utf8_to_utf16(text, marked.start)..utf8_to_utf16(text, marked.end))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.finish_external_composition(true, cx) { return; }
        self.composition_templates.clear();
        self.host.interaction.end_composition();
        self.history_serial += 1;
        cx.emit(ControlEvent::CompositionChanged {
            identity: self.host.parsed.identity.clone(),
            active: false,
        });
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_ime(range, text, None, false, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_ime(range, text, selected, true, cx);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if let Some(composition) = &self.external_composition {
            return self.external_caret_bounds(utf16_to_utf8(&composition.text, range.start)).map(|(_, bounds)| bounds);
        }
        if local_range(&self.host.parsed.source, self.host.parsed.identity.from_byte, self.host.selection.range()).is_err() {
            let identity = &self.host.parsed.identity;
            return self.document_caret_bounds(self.host.selection.range().start.clamp(identity.from_byte, identity.to_byte)).map(|(_, bounds)| bounds);
        }
        self.caret_bounds(
            self.host.parsed.identity.from_byte
                + utf16_to_utf8(&self.host.parsed.source, range.start) as u64,
        )
        .map(|(_, bounds)| bounds)
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        if self.external_composition.is_some()
            || local_range(&self.host.parsed.source, self.host.parsed.identity.from_byte, self.host.selection.range()).is_err()
        { return None; }
        Some(utf8_to_utf16(&self.host.parsed.source, self.hit(position)?))
    }
}
impl NomoMark {
    fn update_completion(&mut self, cx: &mut Context<Self>) {
        if self.auxiliary_input || self.host.interaction.composing {
            return;
        }
        let literal = self
            .host
            .parsed
            .nodes
            .iter()
            .any(|node| node.kind == "code" && node.source.contains(&self.local_cursor()));
        if literal {
            self.menu_open = false;
            self.slash_range = None;
            self.emoji_mode = false;
            return;
        }
        let at = self.local_cursor();
        if let Some(query) = super::emoji::query(&self.host.parsed, at) {
            self.slash_range = Some(query.range);
            self.menu_open = true;
            self.menu_index = 0;
            self.emoji_mode = true;
            if self.emoji_catalog.is_none() && !self.emoji_requested {
                self.emoji_requested = true;
                cx.emit(ControlEvent::EmojiCatalogRequested {
                    identity: self.host.parsed.identity.clone(),
                });
            }
        } else {
            self.slash_range = None;
            self.menu_open = false;
            self.emoji_mode = false;
        }
    }
    fn replace_ime(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        mark: bool,
        cx: &mut Context<Self>,
    ) {
        if self.external_composition.is_some()
            || local_range(&self.host.parsed.source, self.host.parsed.identity.from_byte, self.host.selection.range()).is_err()
        {
            self.replace_external_composition(range, text, selected, mark, cx);
            return;
        }
        let source = &self.host.parsed.source;
        let base = self.host.parsed.identity.from_byte;
        let selected_range = match local_range(source, base, self.host.selection.range()) {
            Ok(range) => range,
            Err(_) if !mark && range.is_none() && !self.host.interaction.composing => {
                cx.emit(ControlEvent::EditOutsideWindow { identity: self.host.parsed.identity.clone(), selection: self.host.selection, intent: EditIntent::Replace(text.into()) });
                return;
            }
            Err(error) => {
                self.error(error, cx);
                return;
            }
        };
        let range = range
            .map(|range| utf16_to_utf8(source, range.start)..utf16_to_utf8(source, range.end))
            .or(self.host.interaction.marked.clone())
            .unwrap_or(selected_range);
        let start = range.start;
        let open_quick = self.quick_insert_input(&range,text,mark,cx);
        self.dismiss_quick_insert(cx);
        if mark && !text.is_empty() && self.host.interaction.marked.is_none() {
            self.composition_templates.begin(&self.host.parsed, &range);
        }
        let template_update = if self.host.interaction.marked.as_ref() == Some(&range) {
            self.composition_templates.cancelled(base, &range, text)
        } else {
            None
        };
        let html_input = self.effective_mode() == ViewMode::Semantic
            && self.host.parsed.html_tables.iter().any(|table| {
                table
                    .cell_at(range.start)
                    .is_some_and(|cell| range.end <= cell.content.end)
            });
        let markdown_cell_input = self.effective_mode() == ViewMode::Semantic
            && super::table_inline::markdown_cell(&self.host.parsed, &range);
        let inserted = if html_input {
            match commands::html_replacement(&self.host.parsed, range.clone(), text) {
                Ok((text, _)) => text,
                Err(error) => {
                    self.error(error, cx);
                    return;
                }
            }
        } else if markdown_cell_input {
            super::table_inline::markdown_input(&self.host.parsed.source, start, text)
        } else {
            text.to_owned()
        };
        let offset = |units| {
            let bytes = utf16_to_utf8(text, units);
            if html_input {
                html_escape::encode_text(&text[..bytes]).len()
            } else if markdown_cell_input {
                super::table_inline::markdown_input(&self.host.parsed.source, start, &text[..bytes]).len()
            } else {
                bytes
            }
        };
        let end = start
            + if html_input {
                html_escape::encode_text(text).len()
            } else {
                inserted.len()
            };
        let after = selected
            .map(|r| DocumentSelection {
                anchor_byte: base + (start + offset(r.start)) as u64,
                head_byte: base + (start + offset(r.end)) as u64,
            })
            .unwrap_or(DocumentSelection::caret(base + end as u64));
        let edit = SourceEdit {
            from_byte: base + range.start as u64,
            to_byte: base + range.end as u64,
            inserted_text: inserted,
        };
        if html_input && text.len() <= super::session_window::policy().group_bytes as usize {
            if let Err(error) =
                commands::validate_html_edits(&self.host.parsed, std::slice::from_ref(&edit))
            {
                self.error(error, cx);
                return;
            }
        }
        let group = format!(
            "{}:typing:{}",
            self.host.parsed.identity.window_id, self.history_serial
        );
        let edits = vec![edit];
        let queued = if !self.auxiliary_input && !mark && self.host.projected_window_bytes(&edits)
            .is_ok_and(|bytes| bytes > super::session_window::policy().group_bytes as usize)
        {
            self.host.queue_rewindow(edits, after, &group, template_update)
        } else {
            self.host.queue_template_edit(edits, after, &group, template_update)
        };
        match queued {
            Ok(()) => {
                let was_composing = self.host.interaction.composing;
                if mark {
                    self.host.interaction.mark_text(start..end);
                    self.menu_open = false;
                    self.slash_range = None;
                } else {
                    self.host.interaction.end_composition();
                }
                if was_composing != self.host.interaction.composing {
                    cx.emit(ControlEvent::CompositionChanged {
                        identity: self.host.parsed.identity.clone(),
                        active: self.host.interaction.composing,
                    });
                }
                if !self.host.interaction.composing {
                    self.composition_templates.clear();
                    self.update_completion(cx);
                    if open_quick {self.open_quick_insert(start..end,text,cx);}
                }
                self.invalidate();
                self.submit(cx);
                cx.notify();
            }
            Err(error) => self.error(error, cx),
        }
    }
}

impl Render for NomoMark {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let entries = self.menu_entries();
        let (menu_x, menu_y) = self.popup_position(
            220.0,
            (entries.len().max(1) as f32 * 38.0).min(350.0),
            false,
        );
        let mut menu = div()
            .id("command-menu")
            .absolute()
            .top(px(menu_y))
            .left(px(menu_x))
            .w(px(220.0))
            .max_h(px(350.0))
            .overflow_y_scroll()
            .track_scroll(&self.menu_scroll)
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg();
        if self.emoji_mode && self.emoji_catalog.is_none() {
            menu = menu.child(div().p_3().child("等待宿主 Emoji 词库"));
        } else if entries.is_empty() {
            menu = menu.child(div().p_3().text_color(theme.muted).child("没有匹配项"));
        }
        for (index, entry) in entries.into_iter().enumerate() {
            menu = menu.child(
                div()
                    .id(("command", index))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .when(index == self.menu_index, |s| s.bg(theme.hover))
                    .hover(|s| s.bg(theme.hover))
                    .child(entry.label)
                    .on_click(cx.listener(move |v, _, window, cx| {
                        v.choose_menu(entry.intent.clone(), cx);
                        window.focus(&v.focus, cx);
                    })),
            );
        }
        div()
            .relative()
            .w_full()
            .key_context("NomoMark")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view,event:&gpui::KeyDownEvent,_,cx|{
                let keys=&event.keystroke;
                if view.quick_insert.is_some() && (keys.modifiers.control || keys.modifiers.platform || keys.modifiers.alt || keys.modifiers.shift
                    || !matches!(keys.key.as_str(),"up"|"down"|"left"|"right"|"enter"|"escape")) {
                    view.dismiss_quick_insert(cx);
                }
            }))
            .cursor(CursorStyle::IBeam)
            .on_action(
                cx.listener(|v, _: &editor::MoveLeft, _, cx| v.horizontal(false, false, false, cx)),
            )
            .on_action(
                cx.listener(|v, _: &editor::MoveRight, _, cx| v.horizontal(true, false, false, cx)),
            )
            .on_action(
                cx.listener(|v, _: &editor::SelectLeft, _, cx| {
                    v.horizontal(false, false, true, cx)
                }),
            )
            .on_action(
                cx.listener(|v, _: &editor::SelectRight, _, cx| {
                    v.horizontal(true, false, true, cx)
                }),
            )
            .on_action(cx.listener(|v, _: &editor::MoveWordLeft, _, cx| {
                v.horizontal(false, true, false, cx)
            }))
            .on_action(cx.listener(|v, _: &editor::MoveWordRight, _, cx| {
                v.horizontal(true, true, false, cx)
            }))
            .on_action(cx.listener(|v, _: &editor::SelectWordLeft, _, cx| v.horizontal(false, true, true, cx)))
            .on_action(cx.listener(|v, _: &editor::SelectWordRight, _, cx| v.horizontal(true, true, true, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveDocStart, _, cx| v.document_edge(false, false, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveDocEnd, _, cx| v.document_edge(true, false, cx)))
            .on_action(cx.listener(|v, _: &editor::SelectDocStart, _, cx| v.document_edge(false, true, cx)))
            .on_action(cx.listener(|v, _: &editor::SelectDocEnd, _, cx| v.document_edge(true, true, cx)))
            .on_action(cx.listener(|v, _: &editor::PageUp, window, cx| v.page(false, false, window, cx)))
            .on_action(cx.listener(|v, _: &editor::PageDown, window, cx| v.page(true, false, window, cx)))
            .on_action(cx.listener(|v, _: &SelectPageUp, window, cx| v.page(false, true, window, cx)))
            .on_action(cx.listener(|v, _: &SelectPageDown, window, cx| v.page(true, true, window, cx)))
            .on_action(cx.listener(|v, _: &editor::DeleteWordLeft, _, cx| v.delete_boundary(false, super::navigation::CursorUnit::Word, cx)))
            .on_action(cx.listener(|v, _: &DeleteWordRight, _, cx| v.delete_boundary(true, super::navigation::CursorUnit::Word, cx)))
            .on_action(cx.listener(|v, _: &editor::DeleteLineLeft, _, cx| v.delete_boundary(false, super::navigation::CursorUnit::Line, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveUp, _, cx| v.vertical(false, false, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveDown, _, cx| v.vertical(true, false, cx)))
            .on_action(cx.listener(|v, _: &editor::SelectUp, _, cx| v.vertical(false, true, cx)))
            .on_action(cx.listener(|v, _: &editor::SelectDown, _, cx| v.vertical(true, true, cx)))
            .on_action(
                cx.listener(|v, _: &editor::MoveLineStart, _, cx| v.home_end(false, false, cx)),
            )
            .on_action(cx.listener(|v, _: &editor::MoveLineEnd, _, cx| v.home_end(true, false, cx)))
            .on_action(
                cx.listener(|v, _: &editor::SelectLineStart, _, cx| v.home_end(false, true, cx)),
            )
            .on_action(
                cx.listener(|v, _: &editor::SelectLineEnd, _, cx| v.home_end(true, true, cx)),
            )
            .on_action(
                cx.listener(|v, _: &editor::Backspace, _, cx| v.command(EditIntent::Backspace, cx)),
            )
            .on_action(
                cx.listener(|v, _: &editor::Delete, _, cx| v.command(EditIntent::Delete, cx)),
            )
            .on_action(cx.listener(|v, _: &editor::Newline, _, cx| v.accept_menu_or_newline(cx)))
            .on_action(cx.listener(|v, _: &SoftBreak, _, cx| {
                v.command(
                    if v.effective_mode() == ViewMode::Semantic {
                        EditIntent::SoftBreak
                    } else {
                        EditIntent::Newline
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|v, _: &Dismiss, window, cx| {
                if v.finish_external_composition(false, cx) { return; }
                if v.object.is_some() {
                    v.finish_object(false, window, cx);
                } else if v.quick_insert.is_some() {
                    v.dismiss_quick_insert(cx);
                } else if v.menu_open {
                    v.menu_open = false;
                    v.slash_range = None;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|v, _: &editor::Indent, _, cx| v.tab(false, cx)))
            .on_action(cx.listener(|v, _: &editor::Outdent, _, cx| v.tab(true, cx)))
            .on_action(cx.listener(|_, _: &editor::Undo, _, cx| cx.emit(ControlEvent::Undo)))
            .on_action(cx.listener(|_, _: &editor::Redo, _, cx| cx.emit(ControlEvent::Redo)))
            .on_action(
                cx.listener(|_, _: &editor::SelectAll, _, cx| cx.emit(ControlEvent::SelectAll)),
            )
            .on_action(cx.listener(|v, _: &editor::Copy, _, cx| v.copy(false, cx)))
            .on_action(cx.listener(|v, _: &editor::Cut, _, cx| v.copy(true, cx)))
            .on_action(cx.listener(|v, _: &editor::Paste, _, cx| v.paste(cx)))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Bold, _, cx| {
                v.command(EditIntent::ToggleInline(InlineFormat::Strong), cx)
            }))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Italic, _, cx| {
                v.command(EditIntent::ToggleInline(InlineFormat::Emphasis), cx)
            }))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Strike, _, cx| {
                v.command(EditIntent::ToggleInline(InlineFormat::Strike), cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::InlineCode, _, cx| {
                    v.command(EditIntent::ToggleInline(InlineFormat::Code), cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Heading1, _, cx| {
                v.command(EditIntent::Heading(1), cx)
            }))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Heading2, _, cx| {
                v.command(EditIntent::Heading(2), cx)
            }))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Heading3, _, cx| {
                v.command(EditIntent::Heading(3), cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::Paragraph, _, cx| {
                    v.command(EditIntent::Heading(0), cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Quote, _, cx| {
                v.command(EditIntent::LinePrefix("> ".into()), cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::BulletList, _, cx| {
                    v.command(EditIntent::LinePrefix("- ".into()), cx)
                }),
            )
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::OrderedList, _, cx| {
                    v.command(EditIntent::LinePrefix("1. ".into()), cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::ui::workspace::TaskList, _, cx| {
                v.command(EditIntent::LinePrefix("- [ ] ".into()), cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::CodeBlock, _, cx| {
                    if v.workspace_hosted {cx.propagate();return;}
                    v.command(EditIntent::InsertCodeBlock("text".into()), cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Table, _, cx| {
                if v.workspace_hosted {cx.propagate();return;}
                v.command(
                    EditIntent::InsertBlock("| 列一 | 列二 |\n| --- | --- |\n|  |  |".into()),
                    cx,
                )
            }))
            .on_action(
                cx.listener(|v, _: &crate::ui::workspace::MathBlock, _, cx| {
                    v.command(EditIntent::InsertMathBlock, cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Rule, _, cx| {
                v.command(EditIntent::InsertBlock("---".into()), cx)
            }))
            .on_action(cx.listener(|v, _: &crate::ui::workspace::Link, _, cx| {
                if v.workspace_hosted {cx.propagate();return;}
                v.command(
                    EditIntent::Link {
                        label: "链接文字".into(),
                        target: "https://".into(),
                    },
                    cx,
                )
            }))
            // 模式是宿主级状态，交给 Workspace 同步标签、全部挂载窗口与滚动锚点。
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|v, event: &MouseDownEvent, window, cx| {
                    if v.host.interaction.protected() {
                        return;
                    }
                    window.focus(&v.focus, cx);
                    if v.host.selection.range().is_empty() {
                        if let Some(at) = v.hit(event.position) {
                            v.move_to(at, false, cx);
                        }
                    }
                    v.dismiss_quick_insert(cx);
                    v.emoji_mode = false;
                    v.menu_open = true;
                    v.menu_index = 0;
                    v.slash_range = None;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::scroll_horizontal))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(ControlElement {
                control: cx.entity(),
            })
            .when(self.menu_open, |view| view.child(menu))
            .children(self.object_panel(cx))
    }
}

pub fn bind_keys(cx: &mut App) {
    use editor::*;
    use gpui::KeyBinding;
    let context = Some("NomoMark");
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("home", MoveLineStart, context),
        KeyBinding::new("end", MoveLineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("shift-pageup", SelectPageUp, context),
        KeyBinding::new("shift-pagedown", SelectPageDown, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("shift-enter", SoftBreak, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new("escape", Dismiss, context),
        crate::ui::key_binding("secondary-a", SelectAll, context, cx),
        crate::ui::key_binding("secondary-z", Undo, context, cx),
        crate::ui::key_binding("secondary-shift-z", Redo, context, cx),
        KeyBinding::new("ctrl-y", Redo, context),
        crate::ui::key_binding("secondary-c", Copy, context, cx),
        crate::ui::key_binding("secondary-x", Cut, context, cx),
        crate::ui::key_binding("secondary-v", Paste, context, cx),
        crate::ui::key_binding("secondary-b", crate::ui::workspace::Bold, context, cx),
        crate::ui::key_binding("secondary-i", crate::ui::workspace::Italic, context, cx),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("alt-left", MoveWordLeft, context), KeyBinding::new("alt-right", MoveWordRight, context),
            KeyBinding::new("alt-shift-left", SelectWordLeft, context), KeyBinding::new("alt-shift-right", SelectWordRight, context),
            KeyBinding::new("cmd-left", MoveLineStart, context), KeyBinding::new("cmd-right", MoveLineEnd, context),
            KeyBinding::new("cmd-shift-left", SelectLineStart, context), KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
            KeyBinding::new("cmd-up", MoveDocStart, context), KeyBinding::new("cmd-down", MoveDocEnd, context),
            KeyBinding::new("cmd-shift-up", SelectDocStart, context), KeyBinding::new("cmd-shift-down", SelectDocEnd, context),
            KeyBinding::new("alt-backspace", DeleteWordLeft, context), KeyBinding::new("alt-delete", DeleteWordRight, context),
            KeyBinding::new("cmd-backspace", DeleteLineLeft, context),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-left", MoveWordLeft, context), KeyBinding::new("ctrl-right", MoveWordRight, context),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, context), KeyBinding::new("ctrl-shift-right", SelectWordRight, context),
            KeyBinding::new("ctrl-home", MoveDocStart, context), KeyBinding::new("ctrl-end", MoveDocEnd, context),
            KeyBinding::new("ctrl-shift-home", SelectDocStart, context), KeyBinding::new("ctrl-shift-end", SelectDocEnd, context),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, context), KeyBinding::new("ctrl-delete", DeleteWordRight, context),
        ]);
    }
}
struct ControlElement {
    control: Entity<NomoMark>,
}
impl IntoElement for ControlElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for ControlElement {
    type RequestLayoutState = ();
    type PrepaintState = ();
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
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = px(self.control.read(cx).height).into();
        style.flex_shrink = 0.0;
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.control.update(cx, |v, cx| {
            v.bounds = Some(bounds);
            let mask = window.content_mask().bounds;
            let left = bounds.origin.x.max(mask.origin.x);
            let top = bounds.origin.y.max(mask.origin.y);
            let right = (bounds.origin.x + bounds.size.width).min(mask.origin.x + mask.size.width);
            let bottom =
                (bounds.origin.y + bounds.size.height).min(mask.origin.y + mask.size.height);
            v.visible_bounds = Some(Bounds::new(
                point(left, top),
                size((right - left).max(px(0.0)), (bottom - top).max(px(0.0))),
            ));
            v.prepare_layout(f32::from(bounds.size.width), window, cx);
            if let Some((stamp, _, _, _)) = &v.layout_key {
                if (f32::from(bounds.size.height) - v.height).abs() < 0.5
                    && v.host.presentation == Presentation::Preparing
                {
                    if let Ok(event) = v.host.mark_layout_ready(stamp.clone(), v.height) {
                        cx.emit(event);
                    }
                }
            }
            v.report_geometry(cx);
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let control = self.control.read(cx);
        if control.host.presentation == Presentation::Editable {
            window.handle_input(
                &control.focus,
                ElementInputHandler::new(bounds, self.control.clone()),
                cx,
            );
        }
        control.paint(bounds, window, cx);
        if control.open_profile.is_some() && control.host.presentation != Presentation::Preparing {
            let bytes = control.host.parsed.source.len();
            let source = control.host.parsed.context.source_only;
            let frame = control.open_profile.as_ref().and_then(|profile| profile.frame(source));
            if let Some(stage) = frame {
                let profile = if source && control.mode == ViewMode::Semantic {
                    control.open_profile.clone()
                } else {
                    self.control.update(cx, |control, _| control.open_profile.take())
                };
                if let Some(profile) = profile {
                    window.on_next_frame(move |window, cx| profile.finish(stage, bytes, source, window, cx));
                }
            }
        }
    }
}

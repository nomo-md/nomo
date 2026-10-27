//! 正式页面的文档宿主接口。正文和历史只在会话中，UI 只持有状态与有界窗口。
use super::*;
use crate::text_document::markdown_session::MarkdownStatus;
use gpui::{App, EventEmitter, FocusHandle};
use std::path::PathBuf;

pub(super) struct Metadata {
    pub status: MarkdownStatus,
}
impl Metadata {
    pub fn new(status: MarkdownStatus) -> Self {
        Self { status }
    }
}

#[derive(Clone)]
pub enum DocumentEvent {
    Changed,
    Status,
    ToolbarState,
    TocSettled(Option<String>),
    Failure(String),
}
#[derive(Clone)]
pub(crate) struct EditTarget {
    identity:WindowIdentity,
    selection:DocumentSelection,
    mode:ViewMode,
}
impl EditTarget {
    pub(crate) fn is_semantic(&self) -> bool { self.mode == ViewMode::Semantic }
}
impl EventEmitter<DocumentEvent> for DocumentHost {}
impl Focusable for DocumentHost {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.control.focus_handle(cx)
    }
}

impl DocumentHost {
    pub(crate) fn workspace(
        port: NativeWindowPort,
        adapter: WindowHostAdapter,
        status: MarkdownStatus,
        mode: ViewMode,
        cx: &mut Context<Self>,
    ) -> Self {
        let base_dir = status
            .path
            .as_ref()
            .and_then(|p| PathBuf::from(p).parent().map(PathBuf::from));
        let control = cx.new(|cx| {
            let mut view = EditorControl::new(adapter, cx);
            view.base_dir = base_dir;
            view.mode = mode;
            view.workspace_hosted = true;
            view
        });
        let subscription = cx.subscribe(&control, Self::event);
        let mut host = Self {
            total_bytes: status.byte_length,
            metadata: Metadata::new(status),
            port,
            queries: Default::default(),
            embedded: true,
            closed: false,
            control: control.clone(),
            message: String::new(),
            hidden: false,
            close_prompt: false,
            saving: false,
            production: true,
            navigating: false,
            request: Default::default(),
            panes: vec![window_panes::Pane::new(control.clone(), subscription, mode)],
            active_pane: 0, split_ratio: 0.5, split_dragging: false, split_focus: cx.focus_handle(),
            focus_pending: false, toolbar_marks: 0,

        };
        host.reset_viewport(cx);
        host.schedule_metadata(cx);
        host.schedule_toc(300, false, cx);
        host
    }
    pub(crate) fn port(&self) -> NativeWindowPort {
        self.port.clone()
    }
    pub(crate) fn status(&self) -> &MarkdownStatus {
        &self.metadata.status
    }
    pub(crate) fn path(&self) -> Option<PathBuf> {
        self.status().path.as_ref().map(PathBuf::from)
    }
    pub(crate) fn title(&self) -> String {
        self.path()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "未命名".into())
    }
    pub(crate) fn is_dirty(&self, cx: &App) -> bool {
        self.status().dirty || self.control.read(cx).host.pending_count() > 0
    }
    pub(crate) fn has_failed_input(&self, cx: &App) -> bool {
        self.control.read(cx).host.failure().is_some()
    }
    pub(crate) fn selection(&self, cx: &App) -> DocumentSelection {
        self.control.read(cx).host.selection
    }
    pub(crate) fn selected_text(&self, cx: &App) -> String {
        let view = self.control.read(cx);
        let range = view.host.selection.range();
        if range.end.saturating_sub(range.start) > 200 {
            return String::new();
        }
        super::super::mapping::local_range(
            &view.host.parsed.source,
            view.host.parsed.identity.from_byte,
            range,
        )
        .ok()
        .map(|r| view.host.parsed.source[r].to_string())
        .unwrap_or_default()
    }
    pub(crate) fn ready(&self, cx: &App) -> Result<(), ControlError> {
        if self.closed || self.saving || self.navigating || self.close_prompt {
            return Err(ControlError::Busy);
        }
        for mount in self.panes.iter().flat_map(|pane|&pane.viewport.mounts) {
            mount.view.read(cx).prepare_unmount()?;
        }
        Ok(())
    }
    pub(crate) fn set_active(
        &mut self,
        active: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        if active == !self.hidden {
            return Ok(());
        }
        self.ready(cx)?;
        self.toggle_hidden(cx);
        if self.hidden == active {
            return Err(ControlError::Busy);
        }
        if active {
            self.schedule_metadata(cx);
            self.schedule_toc(300, false, cx);
        } else {
            self.cancel_queries(cx);
        }
        Ok(())
    }
    pub(crate) fn begin_save(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(NativeWindowPort, crate::text_document::markdown_session::SnapshotHandle), ControlError> {
        self.ready(cx)?;
        let snapshot = self.port.capture_for_save(self.status().revision)?;
        self.saving = true;
        cx.notify();
        Ok((self.port.clone(), snapshot))
    }
    pub(crate) fn finish_save(
        &mut self,
        result: Result<u64, ControlError>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.saving = false;
        let success = result.is_ok();
        if let Err(error) = result {
            self.report(error, cx);
        }
        self.refresh_status(cx);
        let base_dir = self.path().and_then(|p| p.parent().map(PathBuf::from));
        for mount in self.panes.iter().flat_map(|pane|&pane.viewport.mounts) {
            mount.view.update(cx, |view, cx| {
                view.base_dir = base_dir.clone();
                cx.notify();
            });
        }
        self.schedule_viewport(16, cx);
        success
    }
    pub(crate) fn close_document(
        &mut self,
        discard: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        self.ready(cx)?;
        self.pause_viewport(cx);
        self.request.cancel();
        self.cancel_toc();
        self.cancel_queries(cx);
        self.port.close(discard)?;
        self.closed = true;
        for mount in self.panes.iter().flat_map(|pane|&pane.viewport.mounts) {
            mount.view.update(cx, |view, cx| view.detach(discard, cx))?;
        }
        for pane in &mut self.panes { pane.viewport.mounts.clear(); }
        self.update_resident_cost(cx);
        Ok(())
    }
    pub(crate) fn refresh_status(&mut self, cx: &mut Context<Self>) {
        match self.port.status() {
            Ok(status) => {
                self.total_bytes = status.byte_length;
                self.metadata.status = status;
                cx.emit(DocumentEvent::Status);
            }
            Err(error) => self.report(error, cx),
        }
        cx.notify();
    }
    /// 文件树重命名只更新路径和相对资源基准，保留现有窗口、选区和撤销历史。
    pub(crate) fn refresh_renamed_path(&mut self, cx: &mut Context<Self>) -> bool {
        let status = match self.port.status() {
            Ok(status) => status,
            Err(error) => { self.report(error, cx); return false; }
        };
        if status.path == self.metadata.status.path { return false; }
        self.metadata.status = status;
        let base_dir = self.path().and_then(|path| path.parent().map(PathBuf::from));
        let mut controls = Vec::new();
        for pane in &self.panes {
            for view in std::iter::once(&pane.control).chain(pane.viewport.mounts.iter().map(|mount| &mount.view)) {
                if !controls.contains(view) { controls.push(view.clone()); }
            }
        }
        for view in controls {
            view.update(cx, |view, cx| { view.base_dir = base_dir.clone(); cx.notify(); });
        }
        self.schedule_viewport(16, cx);
        cx.emit(DocumentEvent::Status);
        cx.notify();
        true
    }
    pub(super) fn document_changed(&mut self, cx: &mut Context<Self>) {
        let changed = self
            .port
            .status()
            .is_ok_and(|status| status.revision != self.status().revision);
        self.refresh_status(cx);
        if !changed {
            return;
        }
        self.invalidate_other_panes(cx);
        self.schedule_metadata(cx);
        self.schedule_toc(300, false, cx);
        if !self.queries.search.query.is_empty() {
            self.set_search(
                self.queries.search.query.clone(),
                self.queries.search.case_sensitive,
                self.queries.search.whole_word,
                cx,
            );
        }
        cx.emit(DocumentEvent::Changed);
    }
    pub(crate) fn report(&mut self, error: ControlError, cx: &mut Context<Self>) {
        self.message = error.to_string();
        cx.emit(DocumentEvent::Failure(self.message.clone()));
        cx.notify();
    }
    /// 光标移动只检查当前已加载片段；状态不变时不刷新整个工作区。
    pub(super) fn refresh_toolbar_marks(&mut self, cx: &mut Context<Self>) {
        let view = self.control.read(cx);
        let parsed = &view.host.parsed;
        let mut bits = 0u8;
        if view.mode == ViewMode::Semantic {
            if let Ok(range) = super::super::mapping::local_range(&parsed.source, parsed.identity.from_byte, view.host.selection.range()) {
                for span in &parsed.inline {
                    if span.open.start > range.start { break; }
                    if span.open.start <= range.start && range.end <= span.close.end {
                        bits |= match span.format {
                            InlineFormat::Strong => 1,
                            InlineFormat::Emphasis => 2,
                            InlineFormat::Strike => 4,
                            InlineFormat::Underline => 8,
                            InlineFormat::Highlight => 16,
                            InlineFormat::Superscript => 32,
                            InlineFormat::Subscript => 64,
                            _ => 0,
                        };
                    }
                }
            }
        }
        if self.toolbar_marks != bits {
            self.toolbar_marks = bits;
            cx.emit(DocumentEvent::ToolbarState);
        }
    }
    pub(crate) fn toolbar_marks(&self) -> u8 { self.toolbar_marks }

    pub(crate) fn command(&mut self, intent: EditIntent, cx: &mut Context<Self>) {
        self.control.update(cx, |v, cx| v.command(intent, cx));
    }
    pub(crate) fn edit_target(&self,cx:&App)->Result<EditTarget,ControlError> {
        self.ready(cx)?;
        let view=self.control.read(cx);
        if view.host.presentation!=Presentation::Editable {return Err(ControlError::ReadOnly);}
        Ok(EditTarget {identity:view.host.parsed.identity.clone(),selection:view.host.selection,mode:view.mode})
    }
    pub(crate) fn link_draft(&self,cx:&App)->Result<(EditTarget,crate::editor_control::link_edit::LinkDraft),ControlError> {
        let target=self.edit_target(cx)?;
        if target.mode!=ViewMode::Semantic {return Err(ControlError::Unsupported("请先切换到语义编辑区再编辑链接".into()));}
        let draft=crate::editor_control::link_edit::LinkDraft::begin(&self.control.read(cx).host.parsed,target.selection)?;
        Ok((target,draft))
    }
    fn validate_target(&self,target:&EditTarget,cx:&App)->Result<(),ControlError> {
        self.ready(cx)?;
        let view=self.control.read(cx);
        if self.hidden || view.host.parsed.identity!=target.identity || view.host.selection!=target.selection || view.mode!=target.mode {return Err(ControlError::Stale);}
        Ok(())
    }
    pub(crate) fn command_at_target(&mut self, target: &EditTarget, intent: EditIntent, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.validate_target(target, cx)?;
        self.command(intent, cx);
        Ok(())
    }
    pub(crate) fn begin_inline_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.ready(cx)?;
        self.control.update(cx, |view, cx| view.begin_inline_comment(window, cx))
    }
    pub(crate) fn begin_inline_math(&mut self, target: &EditTarget, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.validate_target(target, cx)?;
        self.control.update(cx, |view, cx| view.begin_inline_math(window, cx))
    }
    pub(crate) fn apply_link(&mut self,target:&EditTarget,draft:&crate::editor_control::link_edit::LinkDraft,text:&str,href:Option<&str>,cx:&mut Context<Self>)->Result<(),ControlError> {
        self.validate_target(target,cx)?;
        let parsed=&self.control.read(cx).host.parsed;
        let result=draft.edit(parsed,text,href)?;
        crate::editor_control::commands::validate_html_edits(parsed,&result.edits)?;
        self.control.update(cx,|view,cx|view.apply_host_edit(result,cx))
    }
    pub(crate) fn insert_table(&mut self,target:&EditTarget,rows:usize,columns:usize,cx:&mut Context<Self>)->Result<(),ControlError> {
        self.validate_target(target,cx)?;
        if !(1..=5).contains(&rows) || !(1..=6).contains(&columns) {return Err(ControlError::InvalidRange);}
        let columns=columns.max(2);
        let line=format!("|{}", "  |".repeat(columns));
        let separator=format!("|{}", " --- |".repeat(columns));
        let mut table=format!("{line}\n{separator}");
        for _ in 0..rows {table.push('\n');table.push_str(&line);}
        let view=self.control.read(cx);
        if view.mode==ViewMode::Semantic {
            let parsed=&view.host.parsed;
            let range=crate::editor_control::mapping::local_range(&parsed.source,parsed.identity.from_byte,target.selection.range())?;
            let crosses_inline=parsed.inline.iter().any(|span|span.intersects(&range)
                && !(range.start<=span.open.start && span.close.end<=range.end));
            let nested=parsed.nodes.iter().any(|node|matches!(node.kind,"code"|"table"|"list"|"quote"|"html")
                && node.source.start<=range.start && range.start<node.source.end)
                || parsed.html_tables.iter().any(|table|table.source.start<=range.start && range.start<table.source.end);
            if crosses_inline || nested || !parsed.context.ancestors.is_empty() {
                return Err(ControlError::Unsupported("当前格式或容器内的表格插入尚待接入；原文已保留".into()));
            }
        }
        let mut result=if view.mode==ViewMode::Semantic {crate::editor_control::commands::apply_semantic(&view.host.parsed,target.selection,EditIntent::InsertBlock(table))?}
            else {crate::editor_control::commands::apply(&view.host.parsed,target.selection,EditIntent::InsertBlock(table))?};
        if let Some(edit)=result.edits.first() {if let Some(at)=edit.inserted_text.find('|') {result.selection=DocumentSelection::caret(edit.from_byte+at as u64+2);}}
        self.control.update(cx,|view,cx|view.apply_host_edit(result,cx))
    }
    pub(crate) fn selection_anchor(&self,cx:&App)->Option<gpui::Bounds<gpui::Pixels>> {
        let view=self.control.read(cx);
        view.caret_bounds(view.host.selection.head_byte).map(|(_,bounds)|bounds)
    }
    pub(crate) fn undo_redo(&mut self, redo: bool, cx: &mut Context<Self>) {
        self.history(redo, cx);
    }
    pub(crate) fn set_heading(&mut self, level: u8, cx: &mut Context<Self>) {
        self.command(EditIntent::Heading(level), cx);
    }
    pub(crate) fn toggle_line_prefix(&mut self, prefix: &str, cx: &mut Context<Self>) {
        self.command(EditIntent::LinePrefix(prefix.into()), cx);
    }
    pub(crate) fn insert_block(&mut self, text: &str, caret: usize, cx: &mut Context<Self>) {
        self.command(EditIntent::InsertBlockAt { block: text.into(), caret }, cx);
    }
    pub(crate) fn wrap_selection(&mut self, before: &str, _: &str, cx: &mut Context<Self>) {
        let format = match before {
            "**" => InlineFormat::Strong,
            "*" => InlineFormat::Emphasis,
            "~~" => InlineFormat::Strike,
            "<u>" => InlineFormat::Underline,
            "<mark>" => InlineFormat::Highlight,
            "`" => InlineFormat::Code,
            "[" => {
                self.command(
                    EditIntent::Link {
                        label: self.selected_text(cx),
                        target: "https://".into(),
                    },
                    cx,
                );
                return;
            }
            _ => return,
        };
        self.command(EditIntent::ToggleInline(format), cx);
    }
    pub(crate) fn navigate(
        &mut self,
        byte: u64,
        selection: Option<DocumentSelection>,
        cx: &mut Context<Self>,
    ) {
        self.load_at(byte, selection, cx);
    }
}

impl Render for DocumentHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "editor-control-demo")]
        if !self.embedded {
            return self.render_development(window, cx).into_any_element();
        }
        if self.focus_pending && !self.hidden {
            self.focus_pending = false;
            window.focus(&self.control.focus_handle(cx), cx);
        }
        let animate_theme = Theme::ui_transition_active(cx);
        let failure = self.control.read(cx).host.failure().map(str::to_string);
        let error_bar = failure.map(|failure| {
            let theme = Theme::ui(cx);
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .text_color(theme.fg)
                        .child(format!("修改尚未写入文档会话：{failure}")),
                )
                .child(
                    crate::ui::components::button(
                        "retry-document-edit",
                        "重试保存修改",
                        None,
                        crate::ui::components::ButtonKind::Secondary,
                        &theme,
                    )
                    .on_click(cx.listener(|host, _, _, cx| {
                        if let Err(error) = host.control.update(cx, |view, cx| view.retry(cx)) {
                            host.report(error, cx);
                        }
                    })),
                )
        });
        div().size_full().relative().flex().flex_col().bg(Theme::ui(cx).surface)
            .children(animate_theme.then(crate::ui::motion::theme_frame))
            .children(error_bar).child(self.render_panes(cx)).into_any_element()
    }
}

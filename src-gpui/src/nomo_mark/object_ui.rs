//! 控件内部的对象草稿视图。输入只修改草稿，确认后才向宿主提交一个事务。
use super::*;
use crate::nomo_mark::widgets::{BlockFrame, ObjectDraft, ObjectKind};
use crate::nomo_mark::resources::ResourceRequest;
use crate::ui::text_field::TextField;
use gpui::Subscription;
use std::cell::Cell;

struct ImageForm {
    original: super::super::image_properties::ImageProperties,
    fields: Vec<Entity<TextField>>,
    inline: bool,
}

pub(super) struct ObjectEditor {
    draft: ObjectDraft,
    frame: Option<BlockFrame>,
    exit_paragraph: Option<bool>,
    comment_exit: Option<(bool, bool)>,
    child: Entity<NomoMark>,
    _subscription: Subscription,
    _blur: Subscription,
    _viewer_keys: Subscription,
    child_bytes: usize,
    image: Option<ImageForm>,
    payload: Option<super::super::html_table::InlineAtom>,
    property_bytes: usize,
    _field_subscriptions: Vec<Subscription>,
    preview_task: Option<gpui::Task<()>>,
    preview_key: Option<u64>,
    // 只保留共享缓存键；输入等待期间沿用最后一次完成的预览，不额外持有位图。
    preview_ready_key: Cell<Option<u64>>,
    image_preview: bool,
    preview_return: bool,
    preview_mode: u8,
    preview_modes: bool,
    image_scale: f32,
    image_offset: Point<Pixels>,
    image_drag: Option<Point<Pixels>>,
}
impl ObjectEditor {
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + if self.preview_task.is_some() { self.draft.text.len() + 256 } else { 0 }
            + self.draft.estimated_bytes()
            + self.frame.as_ref().map_or(0, BlockFrame::estimated_bytes)
            + self.child_bytes
            + self.property_bytes
            + self
                .payload
                .as_ref()
                .map_or(0, super::super::html_table::InlineAtom::estimated_bytes)
    }
}

impl NomoMark {
    pub(super) fn inline_editing(&self) -> bool {
        self.object.as_ref().is_some_and(|object| matches!(object.draft.kind, ObjectKind::InlineComment | ObjectKind::InlineMath))
    }
    pub(super) fn math_edit(&self) -> Option<(Range<usize>, &str)> {
        self.object.as_ref().filter(|object| object.draft.kind == ObjectKind::InlineMath)
            .map(|object| (object.draft.range.clone(), object.draft.text.as_str()))
    }
    pub(super) fn object_range(&self) -> Option<Range<usize>> { self.object.as_ref().map(|object| object.draft.range.clone()) }
    pub(super) fn object_edit_height(&self, window: &Window, cx: &App) -> Option<f32> {
        if !self.block_editing() { return None; }
        let object = self.object.as_ref()?;
        let metrics = cx.global::<Settings>().metrics();
        if let Some((input, preview)) = self.embed_edit_heights(window, cx) {
            let spacing = if object.draft.kind == ObjectKind::Mermaid { 54.0 } else { 8.0 };
            return Some(input + preview + (spacing + 4.0) * metrics.zoom + 4.0);
        }
        let input = (object.draft.text.lines().count().max(2).min(8) as f32 * metrics.source_line_height()).clamp(64.0, 220.0);
        Some(input + 112.0 + if object.preview_key.is_some() { 190.0 } else { 0.0 })
    }
    fn embed_edit_heights(&self, window: &Window, cx: &App) -> Option<(f32, f32)> {
        let object = self.object.as_ref().filter(|object| matches!(object.draft.kind, ObjectKind::BlockMath | ObjectKind::Mermaid))?;
        let diagram = object.draft.kind == ObjectKind::Mermaid;
        let zoom = cx.global::<Settings>().metrics().zoom;
        let viewport = f32::from(window.viewport_size().height);
        let rows = object.draft.text.split('\n').take(16).count().max(if diagram { 4 } else { 2 });
        let minimum = if diagram { 96.0 } else { 48.0 } * zoom;
        let maximum = (260.0 * zoom).min(viewport * 0.34).max(minimum);
        let input = (rows as f32 * 19.5 * zoom + if diagram { 22.0 } else { 18.0 } * zoom).clamp(minimum, maximum);
        let minimum = if diagram { 96.0 } else { 40.0 } * zoom;
        let maximum = (420.0 * zoom).min(viewport * 0.52).max(160.0 * zoom);
        let preview = match self.object_preview(cx) {
            Some((_, crate::embeds::EmbedState::Ready(info))) => info.height + if diagram { 24.0 } else { 16.0 } * zoom,
            Some((_, crate::embeds::EmbedState::Failed(_))) => 96.0 * zoom,
            _ => minimum,
        }.clamp(minimum, maximum);
        Some((input, preview))
    }
    pub(super) fn comment_editing(&self) -> bool {
        self.object.as_ref().is_some_and(|object| object.draft.kind == ObjectKind::InlineComment)
    }
    pub(super) fn comment_edit(&self) -> Option<(Range<usize>, &str)> {
        self.object.as_ref().filter(|object| object.draft.kind == ObjectKind::InlineComment)
            .map(|object| (object.draft.range.clone(), object.draft.text.as_str()))
    }
    pub(super) fn finish_comment(&mut self, confirm: bool, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(object) = self.object.as_mut().filter(|object| matches!(object.draft.kind, ObjectKind::InlineComment | ObjectKind::InlineMath)) else { return; };
        object.comment_exit = Some((confirm, forward));
        // 失焦和保存必须等候 IME 最终输入；compositionend 后的下一帧再提交。
        if object.child.read(cx).has_composition(cx) { return; }
        self.finish_object(confirm, window, cx);
    }
    fn comment_key(&mut self, keys: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(object) = self.object.as_ref().filter(|object| matches!(object.draft.kind, ObjectKind::InlineComment | ObjectKind::InlineMath)) else { return false; };
        let child = object.child.read(cx);
        if child.has_composition(cx) { cx.stop_propagation(); return true; }
        let selection = child.host.selection.range();
        let plain = !keys.modifiers.control && !keys.modifiers.platform && !keys.modifiers.alt && !keys.modifiers.shift;
        let exit = match keys.key.as_str() {
            "enter" if !keys.modifiers.shift => Some((true, true)),
            "escape" => Some((false, true)),
            "left" if plain && selection.is_empty() && selection.start == 0 => Some((true, false)),
            "right" if plain && selection.is_empty() && selection.end == child.host.parsed.source.len() as u64 => Some((true, true)),
            _ => None,
        };
        let Some((confirm, forward)) = exit else { return false; };
        self.finish_comment(confirm, forward, window, cx);
        cx.stop_propagation();
        true
    }
    pub(super) fn comment_controls(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.effective_mode() != ViewMode::Semantic || self.auxiliary_input { return None; }
        let layout = self.layout.as_ref()?;
        let zoom = cx.global::<Settings>().metrics().zoom;
        let width = self.bounds.map_or(f32::INFINITY, |bounds| f32::from(bounds.size.width));
        let visible = self.visible_bounds.zip(self.bounds).map(|(visible, bounds)|
            f32::from(visible.top() - bounds.top())..f32::from(visible.bottom() - bounds.top()));
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let mut overlay = div().absolute().top_0().left_0().w_full().h(px(0.0));
        for (range, x, y, w, h, _, clip) in layout.comment_rects() {
            if visible.as_ref().is_some_and(|clip| y + h < clip.start || y > clip.end) { continue; }
            let left = clip.start.max(0.0);
            let right = clip.end.min(width);
            if right <= left { continue; }
            let mut chip = div().id(("inline-comment", range.start)).absolute().left(px(left)).top(px(y))
                .w(px(right - left)).h(px(h)).overflow_hidden();
            if let Some(object) = self.object.as_ref().filter(|object| object.draft.kind == ObjectKind::InlineComment && object.draft.range == range) {
                chip = chip.cursor_text().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().relative().left(px(x - left + 10.0 * zoom)).top(px(4.0 * zoom))
                        .w(px((w - 20.0 * zoom).max(1.0))).h(px((h - 8.0 * zoom).max(1.0))).overflow_hidden().child(object.child.clone()));
            } else {
                let title = super::super::comments::at(&self.host.parsed, range.start).map(|(_, text)| text).unwrap_or_default();
                let title = if title.trim().is_empty() { locale.text("empty_comment").to_owned() } else { title };
                let identity = self.host.parsed.identity.clone();
                chip = chip.cursor_pointer().tooltip(crate::ui::components::tooltip(title))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                        if event.modifiers.shift { return; }
                        cx.stop_propagation();
                        if view.host.parsed.identity != identity { return; }
                        let old_length = view.host.parsed.source.len();
                        let follows_edit = view.object.as_ref().is_some_and(|object| object.draft.range.end <= range.start);
                        if view.comment_editing() { view.finish_comment(true, true, window, cx); }
                        let delta = if follows_edit { view.host.parsed.source.len() as i128 - old_length as i128 } else { 0 };
                        if view.object.is_none() { view.enter_block_at((range.start as i128 + delta) as usize, true, window, cx); }
                    }));
            }
            overlay = overlay.child(chip);
        }
        Some(overlay.into_any_element())
    }
    pub(crate) fn begin_quick_object(&mut self,range:Range<usize>,kind:ObjectKind,window:&mut Window,cx:&mut Context<Self>)->Result<(),ControlError> {
        if kind == ObjectKind::InsertInlineComment { return self.insert_comment(range, Some(String::new()), window, cx); }
        self.prepare_unmount()?;
        super::super::table_inline::validate_insertion(&self.host.parsed,&range)?;
        if matches!(kind,ObjectKind::InsertBlockMath|ObjectKind::InsertBlockComment)
            && (super::super::table_inline::markdown_cell(&self.host.parsed,&range) || super::super::table_inline::html_cell(&self.host.parsed,&range).is_some()) {
            return Err(ControlError::Unsupported("当前表格单元格内不能插入块级对象；原文已保留".into()));
        }
        if matches!(kind,ObjectKind::InsertBlockMath|ObjectKind::InsertBlockComment) {
            let result = super::super::quick_insert::apply_with(&self.host.parsed,range.clone(),|parsed,selection| {
                let intent = EditIntent::InsertBlockAt { block:"$$\n$$".into(), caret:3 };
                let mut result = super::super::quick_insert::apply_block(parsed,selection,intent.clone(),true)?;
                if kind == ObjectKind::InsertBlockMath {
                    // 保留块后段落，但焦点进入公式正文；两次计算都只作用于当前有界片段。
                    result.selection = super::super::quick_insert::apply_block(parsed,selection,intent,false)?.selection;
                }
                Ok(result)
            })?;
            if kind == ObjectKind::InsertBlockMath {
                // $$ 输入规则和快捷插入也先建立真实公式块，再进入同一个块内编辑器。
                self.history_serial += 1;
                let group = format!("{}:math:{}", self.host.parsed.identity.window_id, self.history_serial);
                self.queue_result(result, &group, cx)?;
                self.pending_block_entry = Some(true);
                window.focus(&self.focus, cx);
                return Ok(());
            }
        }
        self.begin_object_with_text(range,kind,Some(String::new()),window,cx);
        if self.object.is_some(){Ok(())}else{Err(ControlError::Busy)}
    }
    pub(crate) fn begin_inline_math(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        if self.effective_mode() != ViewMode::Semantic { return Err(ControlError::Unsupported("请先切换到语义编辑区再插入行内公式".into())); }
        let parsed = &self.host.parsed;
        let range = super::super::mapping::local_range(&parsed.source, parsed.identity.from_byte, self.host.selection.range())?;
        if parsed.nodes.iter().any(|node| node.kind == "code" && node.source.contains(&range.start)) {
            return Err(ControlError::Unsupported("当前代码块内不能插入行内公式；原文已保留".into()));
        }
        super::super::table_inline::validate_insertion(parsed, &range)?;
        self.begin_object(range, ObjectKind::InsertInlineMath, window, cx);
        Ok(())
    }
    pub(crate) fn begin_inline_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        if self.host.presentation != Presentation::Editable { return Err(ControlError::ReadOnly); }
        let parsed = &self.host.parsed;
        let range = super::super::mapping::local_range(&parsed.source, parsed.identity.from_byte, self.host.selection.range())?;
        if parsed.nodes.iter().any(|node| node.kind == "code" && node.source.contains(&range.start)) {
            return Err(ControlError::Unsupported("当前代码块内不能插入行内注释；原文已保留".into()));
        }
        super::super::table_inline::validate_insertion(parsed, &range)?;
        self.insert_comment(range, None, window, cx)
    }
    fn insert_comment(&mut self, range: Range<usize>, initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.prepare_unmount()?;
        if self.effective_mode() != ViewMode::Semantic { return Err(ControlError::Unsupported("请先切换到语义编辑区再插入行内注释".into())); }
        super::super::table_inline::validate_insertion(&self.host.parsed, &range)?;
        let text = match initial {
            Some(text) => text,
            None if range.is_empty() => crate::ui::toolbar_i18n::ToolbarLocale::current(cx).text("comment_insert").into(),
            None => super::super::clipboard_content::selected(&self.host.parsed, range.clone(), false)?.text,
        }.replace("-->", "-- >");
        let markup = if super::super::table_inline::markdown_cell(&self.host.parsed, &range) || super::super::table_inline::html_cell(&self.host.parsed, &range).is_some() {
            let html = super::super::html_table::InlineAtom::Comment(text.clone()).with_text(&text).unwrap();
            super::super::table_inline::markup(&self.host.parsed, &range, &html)?
        } else if text.contains(['\r', '\n']) { super::super::comments::html(&text) }
        else { format!("<!-- {text} -->") };
        let draft = ObjectDraft::begin(&self.host.parsed, range.clone(), ObjectKind::InlineComment)?;
        let mut result = draft.confirm_text(&self.host.parsed, &markup)?;
        result.selection = DocumentSelection::caret(self.host.parsed.identity.from_byte + range.start as u64);
        self.history_serial += 1;
        let group = format!("{}:comment:{}", self.host.parsed.identity.window_id, self.history_serial);
        self.queue_result(result, &group, cx)?;
        // 先让主会话确认插入，再创建局部输入控件；不绕过窗口事务或宿主确认。
        self.pending_block_entry = Some(false);
        window.focus(&self.focus, cx);
        Ok(())
    }
    pub(crate) fn begin_front_matter(&mut self, select_title: bool, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.prepare_unmount()?;
        if self.host.parsed.identity.from_byte != 0 { return Err(ControlError::InvalidRange); }
        let end = super::super::nomo_syntax::front_matter_end(&self.host.parsed.source)
            .ok_or_else(|| ControlError::NeedsContext("文档属性未完整载入；原文已保留".into()))?;
        self.begin_object(0..end, ObjectKind::FrontMatter, window, cx);
        let object = self.object.as_ref().ok_or(ControlError::Busy)?;
        let text = &object.draft.text;
        let selection = if select_title && text.starts_with("title: ") {
            DocumentSelection { anchor_byte: 7, head_byte: text.find(['\r', '\n']).unwrap_or(text.len()) as u64 }
        } else { DocumentSelection::caret(text.len() as u64) };
        object.child.update(cx, |view, cx| view.select(selection, cx));
        Ok(())
    }

    pub(super) fn begin_object(
        &mut self,
        range: Range<usize>,
        kind: ObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_object_with_text(range,kind,None,window,cx);
    }
    fn begin_object_with_text(
        &mut self, mut range:Range<usize>, mut kind:ObjectKind, initial:Option<String>,
        window:&mut Window,cx:&mut Context<Self>,
    ) {
        if self.host.presentation != Presentation::Editable {
            return;
        }
        if kind == ObjectKind::Html {
            let raw = &self.host.parsed.source[range.clone()];
            if range.start == 0 && self.host.parsed.identity.from_byte == 0 && super::super::nomo_syntax::front_matter_end(raw).is_some() { kind = ObjectKind::FrontMatter; }
            else if raw.trim().starts_with("<!--") && raw.trim().ends_with("-->") && raw.trim().contains('\n') { kind = ObjectKind::BlockComment; }
        }
        let comment = matches!(kind, ObjectKind::Html | ObjectKind::InlineComment)
            .then(|| super::super::comments::at(&self.host.parsed, range.start)).flatten()
            .filter(|(target, _)| target.start == range.start && target.end <= range.end);
        if comment.is_some() && kind == ObjectKind::Html { kind = ObjectKind::InlineComment; }
        if kind == ObjectKind::InlineComment {
            let Some((target, _)) = &comment else { return; };
            range = target.clone();
        }
        // 普通代码直接使用主编辑会话，不能再创建对象草稿和“编辑代码”弹层。
        if kind == ObjectKind::Code {
            let at = super::super::code_blocks::FencedCode::at(&self.host.parsed, range.start)
                .map_or(range.start, |code| self.local_cursor().clamp(code.body.start, code.body.end));
            self.move_to(at, false, cx);
            window.focus(&self.focus, cx);
            return;
        }
        if matches!(kind, ObjectKind::Link | ObjectKind::Image) && self.host.parsed.context.reference_pending(&range) {
            self.error(ControlError::NeedsContext("引用目标仍在准备，暂不能编辑目标属性".into()), cx);
            return;
        }
        if let Err(error) = self.prepare_unmount() {
            self.error(error, cx);
            return;
        }
        if kind == ObjectKind::Image && self.host.parsed.source[range.end..].starts_with('{') {
            if let Some(end) = self.host.parsed.source[range.end..].find('}') {
                range.end += end + 1;
            }
        }
        let mut draft = match ObjectDraft::begin(&self.host.parsed, range, kind) {
            Ok(draft) => draft,
            Err(error) => {
                self.error(error, cx);
                return;
            }
        };
        if kind == ObjectKind::InsertInlineComment && draft.text.is_empty() {
            draft.text = crate::ui::toolbar_i18n::ToolbarLocale::current(cx).text("comment").into();
        }
        if let Some(initial)=initial {draft.text=initial;}
        let payload = super::super::table_inline::atom_at(&self.host.parsed, draft.range.start)
            .filter(|run| run.source == draft.range).and_then(|run| run.atom).filter(|atom| atom.text().is_some());
        if let Some(text) = payload.as_ref().and_then(|a| a.text()) {
            draft.text = text.into();
        }
        if kind == ObjectKind::InlineComment { draft.text = comment.as_ref().unwrap().1.clone(); }
        if kind == ObjectKind::TableSize {
            let shape = self
                .host
                .parsed
                .html_tables
                .iter()
                .find(|table| table.source.start == draft.range.start)
                .map(|table| (table.rows, table.columns))
                .or_else(|| {
                    super::super::table::Table::at(&self.host.parsed, draft.range.start)
                        .map(|table| (table.rows.len(), table.separator.cells.len()))
                });
            let Some((rows, columns)) = shape else {
                return;
            };
            draft.text = format!("{rows} {columns}");
        }
        let frame = payload.is_none().then(|| BlockFrame::split(&draft.text, kind)).flatten().map(|(frame, body)| {
            draft.text = body;
            frame
        });
        let input = DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "object-draft".into(),
            revision: 0,
            from_byte: 0,
            to_byte: draft.text.len() as u64,
            text: draft.text.clone(),
        };
        let host = match WindowHostAdapter::new(input, "object-draft", 0, WindowContext::Complete) {
            Ok(host) => host,
            Err(error) => {
                self.error(error, cx);
                return;
            }
        };
        let child = cx.new(|cx| {
            let mut view = NomoMark::new(host, cx);
            view.mode = ViewMode::Source;
            view.auxiliary_input = true;
            if matches!(kind, ObjectKind::InlineComment | ObjectKind::InlineMath) {
                let zoom = cx.global::<Settings>().metrics().zoom;
                let em = self.layout.as_ref().and_then(|layout| layout.comment_rects().into_iter().find(|(range, ..)| *range == draft.range))
                    .map_or(cx.global::<Settings>().metrics().body_size() * 0.9, |(_, _, _, _, _, em, _)| em);
                view.auxiliary_comment = Some(em / zoom);
                view.auxiliary_math = kind == ObjectKind::InlineMath;
                view.host.selection = DocumentSelection::caret(draft.text.len() as u64);
            }
            view.auxiliary_block = matches!(kind, ObjectKind::Code | ObjectKind::Mermaid | ObjectKind::BlockMath | ObjectKind::BlockComment | ObjectKind::FrontMatter);
            view.auxiliary_embed = matches!(kind, ObjectKind::Mermaid | ObjectKind::BlockMath);
            view.auxiliary_language = frame.as_ref().filter(|_| matches!(kind, ObjectKind::Code | ObjectKind::Mermaid)).map(|frame| frame.language().to_string());
            view
        });
        let subscription = cx.subscribe_in(&child, window, |parent, child, event: &ControlEvent, window, cx| {
            match event {
                ControlEvent::ObjectExit { forward } => parent.exit_block(*forward, window, cx),
                ControlEvent::Submit(tx) => {
                    if let Some(object) = &mut parent.object {
                        object.draft.text = child.read(cx).host.parsed.source.to_string();
                        object.child_bytes = child.read(cx).estimated_bytes();
                    }
                    child.update(cx, |view, cx| {
                        let revision = view.host.parsed.identity.revision + 1;
                        if let Err(error) = view.acknowledge(&tx.operation_id, revision, cx) {
                            view.error(error, cx);
                        }
                    });
                    parent.request_object_preview(false, cx);
                    if parent.inline_editing() || parent.block_editing() { parent.invalidate(); }
                    cx.emit(ControlEvent::DraftChanged {
                        identity: parent.host.parsed.identity.clone(), active: true,
                    });
                }
                ControlEvent::SelectAll => child.update(cx, |view, cx| {
                    view.select(
                        DocumentSelection {
                            anchor_byte: 0,
                            head_byte: view.host.parsed.source.len() as u64,
                        },
                        cx,
                    );
                }),
                // 草稿不创建第二份历史。撤销请求要先确认/取消对象，避免错误作用于宿主正文。
                ControlEvent::Undo | ControlEvent::Redo => {
                    if parent.inline_editing() {
                        parent.finish_comment(true, true, window, cx);
                        if parent.object.is_none() { parent.pending_comment_history = Some(matches!(event, ControlEvent::Redo)); }
                    } else {
                        parent.error(ControlError::NeedsContext("请先确认或取消对象草稿，再操作文档历史".into()), cx);
                    }
                }
                ControlEvent::CompositionChanged { active: false, .. } if parent.inline_editing() || parent.block_editing() => {
                    let owner = cx.entity().downgrade();
                    let child_id = child.entity_id();
                    window.on_next_frame(move |window, cx| {
                        let _ = owner.update(cx, |parent, cx| {
                            let exit = parent.object.as_ref().filter(|object| object.child.entity_id() == child_id).and_then(|object| object.comment_exit);
                            if let Some((confirm, forward)) = exit {
                                if parent.inline_editing() { parent.finish_comment(confirm, forward, window, cx); }
                                else { parent.finish_object(confirm, window, cx); }
                            }
                        });
                    });
                }
                ControlEvent::Failure(message) | ControlEvent::NeedsContext(message) => cx.emit(ControlEvent::Failure(message.clone())),
                _ => {}
            }
            cx.notify();
        });
        let image = if kind == ObjectKind::Image {
            super::super::image_properties::ImageProperties::read(&draft.text)
                .or_else(|| super::super::table_inline::image_properties(&self.host.parsed, &draft.range)).map(|original| {
                let values = [
                    &original.src,
                    &original.alt,
                    &original.title,
                    &original.width,
                    &original.align,
                ];
                let fields = values
                    .into_iter()
                    .map(|text| {
                        cx.new(|cx| {
                            let mut field = TextField::new("", cx);
                            field.set_text(text, cx);
                            field
                        })
                    })
                    .collect::<Vec<_>>();
                let inline = self.host.parsed.nodes.iter().any(|n| {
                    matches!(n.kind, "heading" | "cell")
                        && n.source.start <= draft.range.start
                        && draft.range.end <= n.source.end
                }) || self
                    .host
                    .parsed
                    .html_tables
                    .iter()
                    .any(|t| t.cell_at(draft.range.start).is_some());
                ImageForm {
                    original,
                    fields,
                    inline,
                }
            })
        } else {
            None
        };
        let mut field_subscriptions = Vec::new();
        let property_bytes = image.as_ref().map_or(0, |form| {
            form.fields
                .iter()
                .map(|f| f.read(cx).content.len() * 3 + 128)
                .sum()
        });
        if let Some(form) = &image {
            for field in &form.fields {
                field_subscriptions.push(cx.subscribe_in(
                    field,
                    window,
                    |parent, _, event, window, cx| {
                        match event {
                            crate::ui::text_field::FieldEvent::Submit { .. } => {
                                parent.finish_object(true, window, cx);
                                return;
                            }
                            crate::ui::text_field::FieldEvent::Cancel => {
                                parent.finish_object(false, window, cx);
                                return;
                            }
                            _ => {}
                        }
                        if let Some(object) = &mut parent.object {
                            object.property_bytes = object.image.as_ref().map_or(0, |form| {
                                form.fields
                                    .iter()
                                    .map(|f| f.read(cx).content.len() * 3 + 128)
                                    .sum()
                            });
                        }
                        cx.notify();
                    },
                ));
            }
        }
        self.host.interaction.draft = true;
        self.host.set_readonly(true);
        let blur = cx.on_focus_out(&self.focus.clone(), window, |view, _, window, cx| {
            if (view.block_editing() || view.inline_editing()) && !view.object.as_ref().is_some_and(|object| object.child.focus_handle(cx).contains_focused(window, cx)) {
                // 离开当前编辑器才保存，子控件之间切换焦点不结束草稿。
                if view.inline_editing() { view.finish_comment(true, true, window, cx); }
                else { view.finish_object(true, window, cx); }
            }
        });
        let owner = cx.entity().downgrade();
        let viewer_keys = cx.intercept_keystrokes(move |event, window, cx| {
            let _ = owner.update(cx, |view, cx| {
                if view.focus.contains_focused(window, cx) {
                    if !view.comment_key(&event.keystroke, window, cx) { view.viewer_key(&event.keystroke, window, cx); }
                }
            });
        });
        window.focus(&if image.is_some() { self.focus.clone() } else { child.focus_handle(cx) }, cx);
        self.object = Some(ObjectEditor {
            image_preview: image.is_some(),
            preview_return: false,
            preview_mode: if kind == ObjectKind::Mermaid { 1 } else { 0 },
            preview_modes: false,
            image_scale: 1.0,
            image_offset: point(px(0.0), px(0.0)),
            image_drag: None,
            draft,
            frame,
            exit_paragraph: None,
            comment_exit: None,
            child_bytes: child.read(cx).estimated_bytes(),
            child,
            _subscription: subscription,
            _blur: blur,
            _viewer_keys: viewer_keys,
            image,
            payload,
            property_bytes,
            _field_subscriptions: field_subscriptions,
            preview_task: None,
            preview_key: None,
            preview_ready_key: Cell::new(None),
        });
        self.request_object_preview(true, cx);
        if self.inline_editing() || self.block_editing() { self.invalidate(); }
        if let Some(path) = self.preview_image_path() {
            cx.emit(ControlEvent::ResourceDemand { identity: self.host.parsed.identity.clone(), requests: vec![ResourceRequest::Image(path)] });
        }
        cx.emit(ControlEvent::DraftChanged {
            identity: self.host.parsed.identity.clone(),
            active: true,
        });
        cx.notify();
    }

    pub(crate) fn has_composition(&self, cx: &App) -> bool {
        self.host.interaction.composing || self.external_composition.is_some()
            || self.object.as_ref().is_some_and(|object| object.child.read(cx).has_composition(cx)
                || object.image.as_ref().is_some_and(|form| form.fields.iter().any(|field| field.read(cx).is_composing())))
    }

    /// 用户离开编辑区时提交当前输入；异步确认尚未完成时由宿主接续原操作。
    pub(crate) fn finish_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<bool, ControlError> {
        if let Some(error) = self.host.failure() {
            return Err(ControlError::HostFailure(error.into()));
        }
        if self.host.presentation == Presentation::Detached { return Err(ControlError::Detached); }
        if self.external_composition.is_some() {
            self.try_finish_external_composition(true, cx)?;
            window.notify_text_input_changed(&self.focus);
        } else if self.host.interaction.composing {
            self.unmark_text(window, cx);
            window.notify_text_input_changed(&self.focus);
        }
        self.host.interaction.dragging = false;
        self.host.interaction.drag_selection_started = false;
        self.table_drag_anchor = None;
        self.horizontal_drag = None;
        if let Some(object) = &self.object {
            if !object.child.update(cx, |child, cx| child.finish_input(window, cx))? { return Ok(false); }
            if let Some(form) = &object.image {
                for field in &form.fields {
                    field.update(cx, |field, cx| field.unmark_text(window, cx));
                }
            }
            self.try_finish_object(true, window, cx)?;
        }
        Ok(self.host.pending_count() == 0)
    }

    pub(super) fn finish_object(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(object) = self.object.as_mut().filter(|object| object.child.read(cx).has_composition(cx)) {
            object.comment_exit = Some((confirm, true));
            return;
        }
        if let Err(error) = self.try_finish_object(confirm, window, cx) {
            self.error(error, cx);
        }
    }
    pub(super) fn try_finish_object(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        let refocus = self.focus.contains_focused(window, cx);
        if confirm {
            let image = self
                .object
                .as_ref()
                .and_then(|object| object.image.as_ref());
            if let Some(form) = image {
                if form
                    .fields
                    .iter()
                    .any(|field| field.read(cx).is_composing())
                {
                    return Err(ControlError::Busy);
                }
                let mut properties = form.original.clone();
                let values: Vec<_> = form
                    .fields
                    .iter()
                    .map(|field| field.read(cx).content.clone())
                    .collect();
                properties.src = values[0].clone();
                properties.alt = values[1].clone();
                properties.title = values[2].clone();
                properties.width = values[3].clone();
                properties.align = values[4].clone();
                let object = self.object.as_ref().unwrap();
                match properties.write(
                    &object.draft.text,
                    form.inline,
                    commands::line_ending(&self.host.parsed.source, object.draft.range.start),
                ) {
                    Ok(text) => {
                        let text = if text != object.draft.text && super::super::table_inline::markdown_cell(&self.host.parsed, &object.draft.range) {
                            super::super::table_inline::escape_html_in_markdown(&properties.html(true))
                        } else { text };
                        self.object.as_mut().unwrap().draft.text = text;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let Some(object) = &self.object else {
            return Ok(());
        };
        object.child.read(cx).prepare_unmount()?;
        let result = if confirm && matches!(object.draft.kind,ObjectKind::InsertBlockMath|ObjectKind::InsertBlockComment) {
            (||{
                if object.draft.identity!=self.host.parsed.identity {return Err(ControlError::Stale);}
                let value=&object.draft.text;
                let (block,caret)=if object.draft.kind==ObjectKind::InsertBlockMath {
                    if value.contains("$$"){return Err(ControlError::Unsupported("公式内容不能包含块围栏".into()));}
                    (format!("$$\n{value}\n$$"),3)
                }else{
                    if value.contains("-->"){return Err(ControlError::Unsupported("注释内容不能包含 -->".into()));}
                    (format!("<!--\n{value}\n-->"),5)
                };
                super::super::quick_insert::apply_with(&self.host.parsed,object.draft.range.clone(),|parsed,selection|
                    super::super::quick_insert::apply_block(parsed,selection,EditIntent::InsertBlockAt{block,caret},true))
            })()
        } else if confirm && object.draft.kind == ObjectKind::TableSize {
            (|| {
                if object.draft.identity != self.host.parsed.identity {
                    return Err(ControlError::Stale);
                }
                let values = object
                    .draft
                    .text
                    .split_whitespace()
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| ControlError::InvalidRange)?;
                let [rows, columns] = values.as_slice() else {
                    return Err(ControlError::InvalidRange);
                };
                if let Some(table) = self
                    .host
                    .parsed
                    .html_tables
                    .iter()
                    .find(|table| table.source.start == object.draft.range.start)
                {
                    table.edit(
                        &self.host.parsed.source,
                        self.host.parsed.identity.from_byte,
                        super::super::html_table::HtmlTableIntent::Resize {
                            rows: *rows,
                            columns: *columns,
                        },
                    )
                } else {
                    super::super::table::Table::at(&self.host.parsed, object.draft.range.start)
                        .ok_or(ControlError::InvalidRange)?
                        .edit(
                            &self.host.parsed,
                            self.host.selection,
                            super::super::table::TableIntent::Resize {
                                rows: *rows,
                                columns: *columns,
                            },
                        )
                }
            })()
        } else if confirm && object.draft.kind == ObjectKind::InlineComment {
            let value = object.draft.text.replace("-->", "-- >");
            let original = super::super::comments::at(&self.host.parsed, object.draft.range.start).ok_or(ControlError::Stale)?;
            if value == original.1 { object.draft.confirm_text(&self.host.parsed, &self.host.parsed.source[object.draft.range.clone()]) }
            else if let Some(payload) = &object.payload {
                let html = payload.with_text(&value).ok_or(ControlError::InvalidRange)?;
                let text = super::super::table_inline::markup(&self.host.parsed, &object.draft.range, &html)?;
                object.draft.confirm_text(&self.host.parsed, &text)
            } else {
                let markup = if value.contains(['\r', '\n']) || super::super::comments::span_at(&self.host.parsed.source, object.draft.range.start).is_some() {
                    super::super::comments::html(&value)
                } else { format!("<!-- {value} -->") };
                object.draft.confirm_text(&self.host.parsed, &markup)
            }
        } else if confirm && object.draft.kind == ObjectKind::InsertInlineComment {
            let value = &object.draft.text;
            if super::super::table_inline::markdown_cell(&self.host.parsed, &object.draft.range) || super::super::table_inline::html_cell(&self.host.parsed, &object.draft.range).is_some() {
                let text = super::super::html_table::InlineAtom::Comment(value.clone()).with_text(value).unwrap();
                super::super::table_inline::markup(&self.host.parsed, &object.draft.range, &text).and_then(|text| object.draft.confirm_text(&self.host.parsed, &text))
            } else if value.contains("-->") { Err(ControlError::Unsupported("注释内容不能包含 -->".into())) }
            else { object.draft.confirm_text(&self.host.parsed, &format!("<!-- {value} -->")) }
        } else if confirm && object.draft.kind == ObjectKind::InsertInlineMath {
            let value = &object.draft.text;
            if value.trim().is_empty() || value.contains(['\r', '\n']) {
                Err(ControlError::Unsupported("请输入非空单行公式".into()))
            } else {
                let mut tex = String::with_capacity(value.len());
                let mut slashes = 0usize;
                for ch in value.chars() {
                    if ch == '$' && slashes % 2 == 0 { tex.push('\\'); }
                    tex.push(ch);
                    if ch == '\\' { slashes += 1; } else { slashes = 0; }
                }
                let text = if super::super::table_inline::markdown_cell(&self.host.parsed, &object.draft.range) || super::super::table_inline::html_cell(&self.host.parsed, &object.draft.range).is_some() {
                    let text = super::super::html_table::InlineAtom::Math(value.clone()).with_text(value).unwrap();
                    super::super::table_inline::markup(&self.host.parsed, &object.draft.range, &text)
                } else { Ok(format!("${tex}$")) };
                text.and_then(|text| object.draft.confirm_text(&self.host.parsed, &text))
            }
        } else if confirm {
            if let Some(payload) = &object.payload {
                let text = if payload.text() != Some(object.draft.text.as_str()) {
                    let text = payload.with_text(&object.draft.text).expect("text atom");
                    if super::super::table_inline::markdown_cell(&self.host.parsed, &object.draft.range) { super::super::table_inline::escape_html_in_markdown(&text) } else { text }
                } else {
                    self.host.parsed.source[object.draft.range.clone()].to_string()
                };
                object.draft.confirm_text(&self.host.parsed, &text)
            } else if let Some(frame) = &object.frame {
                frame.join(&object.draft.text).and_then(|text| object.draft.confirm_text(&self.host.parsed, &text))
            } else {
                object.draft.confirm(&self.host.parsed)
            }
        } else {
            Ok(commands::EditResult {
                template_update: None,
                edits: Vec::new(),
                selection: self.host.selection,
            })
        };
        let mut result = result?;
        if matches!(object.draft.kind, ObjectKind::InlineComment | ObjectKind::InlineMath) {
            let start = self.host.parsed.identity.from_byte + object.draft.range.start as u64;
            let forward = object.comment_exit.is_none_or(|(_, forward)| forward);
            let length = result.edits.first().map_or(object.draft.range.len(), |edit| edit.inserted_text.len());
            result.selection = DocumentSelection::caret(start + if forward { length as u64 } else { 0 });
        }
        if confirm {
            if let Some(before) = object.exit_paragraph {
                let at = self.host.parsed.identity.from_byte + object.draft.range.start as u64;
                let paragraph = commands::apply(&self.host.parsed, DocumentSelection::caret(at), EditIntent::InsertParagraph { before })?;
                let insertion = paragraph.edits.into_iter().next().expect("paragraph insertion");
                let mut caret = paragraph.selection.head_byte;
                if let Some(edit) = result.edits.first_mut() {
                    if insertion.from_byte >= edit.to_byte {
                        caret = (caret as i128 + edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128) as u64;
                    }
                    if insertion.from_byte == edit.from_byte {
                        edit.inserted_text.insert_str(0, &insertion.inserted_text);
                    } else if insertion.from_byte == edit.to_byte {
                        edit.inserted_text.push_str(&insertion.inserted_text);
                    } else {
                        result.edits.push(insertion);
                        result.edits.sort_by_key(|edit| edit.from_byte);
                    }
                } else { result.edits.push(insertion); }
                result.selection = DocumentSelection::caret(caret);
            }
        }
        if confirm {
            commands::validate_html_edits(&self.host.parsed, &result.edits)?;
        }
        self.host.interaction.draft = false;
        self.host.set_readonly(false);
        self.history_serial += 1;
        let group = format!(
            "{}:object:{}",
            self.host.parsed.identity.window_id, self.history_serial
        );
        let unchanged = result.edits.is_empty();
        let selection = result.selection;
        if let Err(error) = self.queue_result(result, &group, cx) {
            self.host.interaction.draft = true;
            self.host.set_readonly(true);
            return Err(error);
        }
        if unchanged { self.host.selection = selection; }
        self.object = None;
        self.invalidate();
        self.submit(cx);
        if refocus { window.focus(&self.focus, cx); }
        cx.emit(ControlEvent::DraftChanged {
            identity: self.host.parsed.identity.clone(),
            active: false,
        });
        cx.notify();
        Ok(())
    }

    pub(super) fn block_editing(&self) -> bool {
        self.object.as_ref().is_some_and(|o| (!o.image_preview || o.preview_return) && matches!(o.draft.kind, ObjectKind::Code | ObjectKind::Mermaid | ObjectKind::BlockMath | ObjectKind::BlockComment | ObjectKind::FrontMatter))
    }
    pub(super) fn finish_block_paragraph(&mut self, before: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(object) = &mut self.object { object.exit_paragraph = Some(before); }
        self.finish_object(true, window, cx);
    }
    pub(super) fn horizontal_block(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.keyboard_protected() && self.host.selection.range().is_empty() && self.quick_insert.is_none() {
            let at = self.local_cursor();
            let target = if forward { at } else { at.saturating_sub(1) };
            if let Some((range, _)) = super::super::comments::at(&self.host.parsed, target) {
                if (forward && at == range.start || !forward && at == range.end) && self.enter_block_at(target, !forward, window, cx) { return; }
            }
            if let Some(at) = self.layout.as_ref().and_then(|layout| layout.horizontal_caret(self.local_cursor(), super::super::navigation::CursorUnit::Grapheme, forward)) {
                if self.enter_block_at(at, !forward, window, cx) { return; }
            }
        }
        self.horizontal(forward, false, false, cx);
    }
    pub(super) fn enter_block_at(&mut self, at: usize, last: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.effective_mode() != ViewMode::Semantic || self.auxiliary_input || self.object.is_some() { return false; }
        let target = super::super::comments::at(&self.host.parsed, at).map(|(range, _)| (range, ObjectKind::InlineComment))
            .or_else(|| super::super::table_inline::atom_at(&self.host.parsed, at).filter(|run| matches!(run.atom, Some(super::super::html_table::InlineAtom::Math(_)))).map(|run| (run.source, ObjectKind::InlineMath)))
            .or_else(|| self.host.parsed.events.iter().find_map(|(event, range)| {
            if !range.contains(&at) { return None; }
            let kind = match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(info)))
                    if info.split_whitespace().next() == Some("mermaid") => ObjectKind::Mermaid,
                pulldown_cmark::Event::DisplayMath(_) => ObjectKind::BlockMath,
                pulldown_cmark::Event::InlineMath(_) => ObjectKind::InlineMath,
                _ => return None,
            };
            Some((range.clone(), kind))
        }));
        let Some((range, kind)) = target else { return false; };
        let start = range.start;
        self.move_to(at, false, cx);
        if self.host.pending_count() != 0 || self.host.presentation == Presentation::Preparing {
            self.pending_block_entry = Some(!last);
            cx.notify();
            return true;
        }
        self.begin_object(range, kind, window, cx);
        if let Some(object) = &self.object {
            let body_start = start + object.frame.as_ref().map_or(0, BlockFrame::body_start);
            let cursor = if last { object.draft.text.len() } else if kind == ObjectKind::InlineComment { 0 } else { at.saturating_sub(body_start).min(object.draft.text.len()) };
            object.child.update(cx, |view, cx| view.select(DocumentSelection::caret(cursor as u64), cx));
        }
        self.object.is_some()
    }
    fn exit_block(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(object) = &self.object else { return; };
        let start = object.draft.range.start;
        let old_end = object.draft.range.end;
        let target = self.layout.as_ref().and_then(|layout| layout.horizontal_caret(
            if forward { old_end.saturating_sub(1) } else { start },
            super::super::navigation::CursorUnit::Grapheme, forward));
        self.finish_object(true, window, cx);
        if self.object.is_some() { return; }
        if let Some(target) = target.filter(|target| if forward { *target >= old_end } else { *target < start }) {
            let target = if forward { self.local_cursor() + target.saturating_sub(old_end) } else { target };
            self.move_to(target, false, cx);
            return;
        }
        if forward && !self.host.parsed.context.has_following_block { return; }
        if !forward { self.move_to(start, false, cx); }
        self.pending_block_navigation = Some(forward);
        self.flush_block_navigation(cx);
    }
    pub(super) fn flush_block_navigation(&mut self, cx: &mut Context<Self>) {
        if self.host.pending_count() != 0 { return; }
        let Some(forward) = self.pending_block_navigation.take() else { return; };
        if self.windowed_navigation {
            cx.emit(ControlEvent::CursorBoundary { identity: self.host.parsed.identity.clone(),
                unit: super::super::navigation::CursorUnit::Grapheme, forward, extend_selection: false, delete: false });
        } else { self.horizontal(forward, false, false, cx); }
    }
    fn object_preview_spec(&self, cx: &App) -> Option<crate::embeds::EmbedSpec> {
        use crate::embeds::{EmbedKind, EmbedSpec};
        let object = self.object.as_ref()?;
        let text = object.draft.text.trim();
        let (kind, source) = match object.draft.kind {
            ObjectKind::InlineMath | ObjectKind::InsertInlineMath =>
                (EmbedKind::Math { display: false }, text.strip_prefix('$').and_then(|s| s.strip_suffix('$')).unwrap_or(text)),
            ObjectKind::BlockMath | ObjectKind::InsertBlockMath =>
                (EmbedKind::Math { display: true }, text.strip_prefix("$$").and_then(|s| s.strip_suffix("$$")).unwrap_or(text)),
            ObjectKind::Mermaid => {
                if object.frame.is_some() {
                    (EmbedKind::Mermaid, text)
                } else {
                    let source = text.split_once('\n').map(|(_, body)| body).unwrap_or(text);
                    let source = source.rsplit_once('\n').filter(|(_, last)| last.trim().starts_with("```") || last.trim().starts_with("~~~")).map_or(source, |(body, _)| body);
                    (EmbedKind::Mermaid, source)
                }
            }
            _ => return None,
        };
        if source.trim().is_empty() { return None; }
        let theme = cx.global::<Theme>();
        let color: gpui::Rgba = theme.fg.into();
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(EmbedSpec { kind: kind.with_palette(theme.mermaid), source: source.to_string(), em_px: cx.global::<Settings>().metrics().body_size(),
            color: [byte(color.r), byte(color.g), byte(color.b)], dark: theme.dark })
    }
    fn object_preview(&self, cx: &App) -> Option<(u64, crate::embeds::EmbedState)> {
        use crate::embeds::EmbedState;
        let object = self.object.as_ref()?;
        let key = object.preview_key?;
        let store = cx.global::<EmbedStore>();
        match store.state(key) {
            Some(EmbedState::Ready(info)) => {
                object.preview_ready_key.set(Some(key));
                Some((key, EmbedState::Ready(info)))
            }
            Some(EmbedState::Failed(message)) => Some((key, EmbedState::Failed(message))),
            _ => object.preview_ready_key.get().and_then(|ready| match store.state(ready) {
                Some(EmbedState::Ready(info)) => Some((ready, EmbedState::Ready(info))),
                _ => None,
            }).or(Some((key, EmbedState::Pending))),
        }
    }
    fn request_object_preview(&mut self, immediate: bool, cx: &mut Context<Self>) {
        // 切换请求前记住当前已完成结果。旧结果只在新请求等待时显示，失败仍明确显示错误。
        self.object_preview(cx);
        let spec = self.object_preview_spec(cx);
        let identity = self.host.parsed.identity.clone();
        let Some(object) = &mut self.object else { return; };
        if spec.as_ref().map(|spec| spec.key()) == object.preview_key
            && (object.preview_task.is_some() || object.preview_key.is_some_and(|key| cx.global::<EmbedStore>().state(key).is_some())) { return; }
        object.preview_task = None;
        object.preview_key = spec.as_ref().map(|s| s.key());
        let Some(spec) = spec else { object.preview_ready_key.set(None); return; };
        let key = spec.key();
        if cx.global::<EmbedStore>().state(key).is_some() { return; }
        if immediate {
            cx.emit(ControlEvent::ResourceDemand { identity, requests: vec![ResourceRequest::Embed(spec)] });
            return;
        }
        let child_id = object.child.entity_id();
        object.preview_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
            let _ = this.update(cx, |view, cx| {
                if view.host.parsed.identity != identity || !view.object.as_ref().is_some_and(|o| o.preview_key == Some(key) && o.child.entity_id() == child_id) { return; }
                if let Some(object) = &mut view.object { object.preview_task = None; }
                cx.emit(ControlEvent::ResourceDemand { identity, requests: vec![ResourceRequest::Embed(spec)] });
            });
        }));
    }
    fn preview_image_path(&self) -> Option<PathBuf> {
        let src = &self.object.as_ref()?.image.as_ref()?.original.src;
        crate::images::resolve_source(src, self.base_dir.as_deref())
    }
    pub(super) fn open_diagram_viewer(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let returning = self.object.is_some();
        if !returning { self.begin_object(range, ObjectKind::Mermaid, window, cx); }
        if let Some(object) = &mut self.object {
            object.image_preview = true;
            object.preview_return = returning;
            object.preview_mode = 1;
            object.image_offset = point(px(0.0), px(0.0));
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
    pub(super) fn close_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(object) = self.object.as_mut().filter(|o| o.image_preview) else { return false; };
        if object.preview_return {
            object.image_preview = false;
            object.image_drag = None;
            window.focus(&object.child.focus_handle(cx), cx);
            cx.notify();
        } else { self.finish_object(false, window, cx); }
        true
    }
    fn preview_media(&self, cx: &App) -> Option<(std::sync::Arc<gpui::RenderImage>, (f32, f32))> {
        let object = self.object.as_ref()?;
        if object.draft.kind == ObjectKind::Mermaid {
            let (key, state) = self.object_preview(cx)?;
            let store = cx.global::<EmbedStore>();
            let crate::embeds::EmbedState::Ready(info) = state else { return None; };
            Some((store.image(key)?, (info.width, info.height)))
        } else {
            let path = self.preview_image_path()?;
            let store = cx.global::<ImageStore>();
            let (width, height) = store.size(&path)?;
            Some((store.image(&path)?, (width as f32, height as f32)))
        }
    }
    fn preview_scale(&self, window: &Window, cx: &App) -> f32 {
        let Some(object) = &self.object else { return 1.0; };
        let Some((_, (width, height))) = self.preview_media(cx) else { return 1.0; };
        let viewport = window.viewport_size();
        let available = ((f32::from(viewport.width) - 80.0).max(1.0), (f32::from(viewport.height) - 120.0).max(1.0));
        match object.preview_mode {
            0 => (available.0 / width.max(1.0)).min(available.1 / height.max(1.0)).min(1.0).max(0.01),
            1 => (available.0 / width.max(1.0)).min(1.0).max(0.85),
            2 => 1.0,
            _ => object.image_scale,
        }
    }
    pub(super) fn viewer_key(&mut self, keys: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.object.as_ref().is_some_and(|o| o.image_preview) { return false; }
        match keys.key.as_str() {
            "escape" => { self.close_viewer(window, cx); }
            "+" | "=" => self.zoom_viewer(0.1, None, window, cx),
            "-" => self.zoom_viewer(-0.1, None, window, cx),
            "0" => if let Some(object) = &mut self.object {
                object.preview_mode = if object.draft.kind == ObjectKind::Mermaid { 1 } else { 0 };
                object.image_offset = point(px(0.0), px(0.0)); cx.notify();
            },
            _ => {}
        }
        cx.stop_propagation();
        true
    }
    pub(super) fn zoom_viewer(&mut self, delta: f32, pivot: Option<Point<Pixels>>, window: &Window, cx: &mut Context<Self>) {
        let old = self.preview_scale(window, cx).max(0.01);
        let viewport = window.viewport_size();
        let center = point(viewport.width / 2.0, viewport.height / 2.0);
        if let Some(object) = &mut self.object {
            let next = (old + delta).clamp(0.1, 3.0);
            let pivot = pivot.unwrap_or(center) - center;
            object.image_offset = pivot - (pivot - object.image_offset) * (next / old);
            object.image_scale = next;
            object.preview_mode = 3;
        }
        cx.notify();
    }
    fn image_preview_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use crate::ui::components::{icon, IconName, tooltip};
        let object = self.object.as_ref().filter(|object| object.image_preview)?;
        let theme = cx.global::<Theme>().clone();
        let viewport = window.viewport_size();
        let path = self.preview_image_path();
        let store = cx.global::<ImageStore>();
        let loaded = self.preview_media(cx);
        let scale = self.preview_scale(window, cx);
        let mut contents = div().absolute().inset_0().overflow_hidden().flex().items_center().justify_center();
        if let Some((image, (width, height))) = loaded {
            contents = contents.child(div().relative().left(object.image_offset.x).top(object.image_offset.y)
                .child(gpui::img(image).w(px(width * scale)).h(px(height * scale))));
        } else {
            let embed_failure = object.preview_key.and_then(|key| cx.global::<EmbedStore>().state(key)).and_then(|state| match state {
                crate::embeds::EmbedState::Failed(message) => Some(message), _ => None });
            let message = embed_failure.as_deref().or_else(|| path.as_deref().and_then(|path| store.failure(path)))
                .unwrap_or("正在加载预览…");
            contents = contents.child(message.to_string());
        }
        let mut controls = div().absolute().top(px(16.0)).right(px(16.0)).flex().gap(px(6.0)).items_center().p(px(5.0)).rounded(px(8.0)).bg(theme.surface)
            .child(format!("{}%", (scale * 100.0).round() as u32));
        if object.draft.kind == ObjectKind::Mermaid {
            let label = match object.preview_mode { 0 => "适应窗口", 1 => "易读", 2 => "原始大小", _ => "自定义" };
            controls = controls.child(div().id("diagram-view-mode").relative().px(px(8.0)).h(px(30.0)).cursor_pointer().child(label)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|view, _, _, cx| { if let Some(o) = &mut view.object { o.preview_modes = !o.preview_modes; } cx.notify(); })));
            if object.preview_modes {
                let mut menu = div().id("diagram-view-modes").absolute().right(px(148.0)).top(px(38.0)).w(px(120.0)).bg(theme.surface).border_1().border_color(theme.border).rounded(px(6.0));
                for (mode, label) in [(1, "易读"), (0, "适应窗口"), (2, "原始大小")] {
                    menu = menu.child(div().id(("diagram-mode", mode as usize)).p(px(8.0)).cursor_pointer().hover(|s| s.bg(theme.hover)).child(label)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, _, cx| { if let Some(o) = &mut view.object { o.preview_mode = mode; o.preview_modes = false; o.image_offset = point(px(0.0), px(0.0)); } cx.notify(); })));
                }
                controls = controls.child(menu);
            }
        }
        for (id, label, glyph) in [(0, "缩小", IconName::ZoomOut), (1, "重置缩放", IconName::RotateCcw), (2, "放大", IconName::ZoomIn), (3, "关闭", IconName::Close)] {
            controls = controls.child(div().id(("image-preview-control", id as usize)).size(px(30.0)).flex().items_center().justify_center().cursor_pointer()
                .rounded(px(4.0)).hover(|s| s.bg(theme.hover)).tooltip(tooltip(label)).child(icon(glyph, 16.0, theme.fg))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, window, cx| {
                    if id == 3 { view.close_viewer(window, cx); return; }
                    if id != 1 { view.zoom_viewer(if id == 0 { -0.1 } else { 0.1 }, None, window, cx); return; }
                    if let Some(object) = &mut view.object {
                        object.preview_mode = if object.draft.kind == ObjectKind::Mermaid { 1 } else { 0 };
                        if id == 1 { object.image_offset = point(px(0.0), px(0.0)); }
                    }
                    cx.notify();
                })));
        }
        let mut panel = div().id("image-preview").w(viewport.width).h(viewport.height).relative().bg(theme.surface).text_color(theme.fg).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|view, event: &MouseDownEvent, _, cx| {
                if let Some(object) = &mut view.object { object.image_drag = Some(event.position); } cx.stop_propagation();
            }))
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if let Some(object) = &mut view.object {
                    if let Some(from) = object.image_drag { object.image_offset += event.position - from; object.image_drag = Some(event.position); cx.notify(); }
                }
                cx.stop_propagation();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|view, _, _, cx| { if let Some(object) = &mut view.object { object.image_drag = None; } cx.stop_propagation(); }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|view, _, _, cx| { if let Some(object) = &mut view.object { object.image_drag = None; } cx.stop_propagation(); }))
            .on_scroll_wheel(cx.listener(|view, event: &gpui::ScrollEvent, window, cx| {
                let delta = event.delta.pixel_delta(px(16.0));
                if event.modifiers.control || event.modifiers.platform {
                    view.zoom_viewer(if delta.y > px(0.0) { 0.1 } else { -0.1 }, Some(event.position), window, cx);
                } else if let Some(object) = &mut view.object { object.image_offset += delta; cx.notify(); }
                cx.stop_propagation();
            }))
            .child(contents).child(controls).into_any_element();
        Some(gpui::deferred(gpui::canvas(move |_, window, cx| {
            panel.layout_as_root(size(gpui::AvailableSpace::Definite(viewport.width), gpui::AvailableSpace::Definite(viewport.height)), window, cx);
            panel.prepaint_at(point(px(0.0), px(0.0)), window, cx);
            panel
        }, |_, mut panel: gpui::AnyElement, window, cx| {
            window.with_content_mask(None, |window| panel.paint(window, cx));
        }).absolute().inset_0()).with_priority(10).into_any_element())
    }
    pub(super) fn show_image_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(object) = &mut self.object {
            object.image_preview = false;
            if let Some(form) = &object.image { window.focus(&form.fields[0].focus_handle(cx), cx); }
            cx.notify();
        }
    }
    pub(super) fn image_range_at(&self, at: usize) -> Option<Range<usize>> {
        if let Some(run) = super::super::table_inline::atom_at(&self.host.parsed, at) {
            if matches!(run.atom, Some(super::super::html_table::InlineAtom::Image { .. })) { return Some(run.source); }
        }
        self.host.parsed.events.iter().find_map(|(event, range)| {
            if !range.contains(&at) { return None; }
            let image = match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. }) => true,
                pulldown_cmark::Event::Html(html) | pulldown_cmark::Event::InlineHtml(html) => html.contains("<img"),
                _ => false,
            };
            image.then(|| range.clone())
        })
    }
    fn embed_editor_panel(&self, rect: (f32, f32, f32, f32), window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use crate::embeds::EmbedState;
        use crate::ui::components::{icon, IconName, tooltip};
        let object = self.object.as_ref()?;
        let (input_height, preview_height) = self.embed_edit_heights(window, cx)?;
        let (left, top, width, height) = rect;
        let diagram = object.draft.kind == ObjectKind::Mermaid;
        let zoom = cx.global::<Settings>().metrics().zoom;
        let theme = cx.global::<Theme>().clone();
        let inset = 2.0 + 2.0 * zoom;
        let input_padding = if diagram { 12.0 } else { 10.0 } * zoom;
        let content_width = (width - inset * 2.0).max(1.0);
        object.child.update(cx, |view, cx| view.prepare_layout((content_width - input_padding * 2.0 - 2.0).max(1.0), window, cx));
        let input = div().id("embed-source-scroll").h(px(input_height)).flex_shrink_0()
            .px(px(input_padding)).py(px(if diagram { 10.0 } else { 8.0 } * zoom))
            .overflow_y_scroll().bg(theme.surface).border_1().border_color(theme.accent).rounded(px(3.0 * zoom))
            .child(object.child.clone());
        let mut panel = div().id("embed-editor").absolute().left(px(left)).top(px(top)).w(px(width)).h(px(height))
            .flex().flex_col().p(px(2.0 * zoom)).border_2().border_color(theme.accent).rounded(px(3.0 * zoom))
            .bg(theme.hover).text_color(theme.fg)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(input);
        if diagram {
            let label = match object.preview_mode { 0 => "适应窗口", 2 => "原始大小", _ => "易读" };
            let mut mode = div().id("embed-preview-mode").relative().h(px(32.0 * zoom)).px(px(8.0 * zoom))
                .flex().items_center().gap(px(4.0 * zoom)).bg(theme.surface).border_1().border_color(theme.border)
                .rounded(px(3.0 * zoom)).cursor_pointer().text_size(px(12.0 * zoom))
                .child(label).child(icon(IconName::ChevronDown, 14.0 * zoom, theme.muted))
                .on_click(cx.listener(|view, _, _, cx| {
                    if let Some(object) = &mut view.object { object.preview_modes = !object.preview_modes; }
                    cx.notify();
                }));
            if object.preview_modes {
                let mut menu = div().id("embed-preview-modes").absolute().right_0().top(px(34.0 * zoom)).w(px(120.0 * zoom))
                    .bg(theme.surface).border_1().border_color(theme.border).rounded(px(3.0 * zoom)).shadow_md();
                for (value, label) in [(1usize, "易读"), (0, "适应窗口"), (2, "原始大小")] {
                    menu = menu.child(div().id(("embed-preview-mode-item", value)).p(px(8.0 * zoom)).hover(|s| s.bg(theme.hover)).child(label)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(object) = &mut view.object { object.preview_mode = value as u8; object.preview_modes = false; }
                            cx.notify();
                        })));
                }
                mode = mode.child(gpui::deferred(menu).with_priority(1));
            }
            let range = object.draft.range.clone();
            panel = panel.child(div().h(px(40.0 * zoom)).mt(px(14.0 * zoom)).pb(px(8.0 * zoom)).flex_shrink_0()
                .flex().justify_end().gap(px(8.0 * zoom)).child(mode)
                .child(div().id("draft-diagram-fullscreen").size(px(32.0 * zoom)).flex().items_center().justify_center()
                    .bg(theme.surface).border_1().border_color(theme.border).rounded(px(3.0 * zoom)).cursor_pointer()
                    .tooltip(tooltip("放大查看")).child(gpui::svg().path("icons/maximize.svg").size(px(16.0 * zoom)).text_color(theme.muted))
                    .on_click(cx.listener(move |view, _, window, cx| view.open_diagram_viewer(range.clone(), window, cx)))));
        }
        let pad = if diagram { 12.0 } else { 8.0 } * zoom;
        let mut preview = div().id("embed-preview-scroll").h(px(preview_height)).flex_shrink_0().p(px(pad))
            .overflow_x_scroll().overflow_y_scroll().text_size(px(12.0 * zoom))
            .when(!diagram, |s| s.mt(px(8.0 * zoom)));
        if let Some((key, state)) = self.object_preview(cx) {
            match state {
                EmbedState::Ready(info) => if let Some(image) = cx.global::<EmbedStore>().image(key) {
                    let available_width = (content_width - pad * 2.0).max(1.0);
                    let available_height = (preview_height - pad * 2.0).max(1.0);
                    let scale = if diagram && object.preview_mode == 0 {
                        (available_width / info.width.max(1.0)).min(available_height / info.height.max(1.0)).min(1.0)
                    } else { 1.0 };
                    // 维持正常阅读尺寸；宽图／公式在块内滚动，不能为了固定预览框把内容缩成缩略图。
                    preview = preview.child(div().w(px((info.width * scale).max(available_width))).min_h(px(available_height))
                        .flex().items_center().justify_center()
                        .child(gpui::img(image).w(px(info.width * scale)).h(px(info.height * scale)).flex_shrink_0()));
                },
                EmbedState::Failed(message) => preview = preview.text_color(theme.warning).child(message.to_string()),
                EmbedState::Pending => preview = preview.text_color(theme.muted).child("正在生成预览…"),
            }
        }
        Some(panel.child(preview).into_any_element())
    }
    pub(super) fn object_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.comment_editing() { return None; }
        if let Some(preview) = self.image_preview_panel(window, cx) { return Some(preview); }
        let object = self.object.as_ref()?;
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let theme = cx.global::<Theme>();
        let local = self.block_editing().then(|| self.layout.as_ref()?.items.iter().find_map(|item| match item {
            DisplayItem::BlockControls { x, y, w, h, source, kind: "object-edit" } if *source == object.draft.range => Some((*x, *y, *w, *h)),
            _ => None,
        })).flatten();
        // 块内编辑没有临时弹窗回退：首帧几何尚未就绪时等待布局，不显示错位的通用面板。
        if self.block_editing() && local.is_none() { return None; }
        if matches!(object.draft.kind, ObjectKind::BlockMath | ObjectKind::Mermaid) {
            return self.embed_editor_panel(local?, window, cx);
        }
        if object.draft.kind == ObjectKind::InlineMath {
            let (x, y, w, h) = self.layout.as_ref()?.math_input_rect()?;
            let zoom = cx.global::<Settings>().metrics().zoom;
            let mut overlay = div().absolute().top_0().left_0().w_full().h(px(0.0))
                .child(div().absolute().left(px(x + 6.0 * zoom)).top(px(y + 3.0 * zoom))
                    .w(px((w - 12.0 * zoom).max(1.0))).h(px((h - 6.0 * zoom).max(1.0))).overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(object.child.clone()));
            if let Some((key, state)) = self.object_preview(cx) {
                let store = cx.global::<EmbedStore>();
                let mut preview = div().id("inline-math-preview").p(px(10.0 * zoom)).max_w(px(360.0 * zoom))
                    .bg(theme.surface).border_1().border_color(theme.border).rounded(px(6.0 * zoom)).shadow_md().text_size(px(12.0 * zoom));
                match state {
                    crate::embeds::EmbedState::Ready(info) => if let Some(image) = store.image(key) {
                        let scale = (320.0 * zoom / info.width.max(1.0)).min(100.0 * zoom / info.height.max(1.0)).min(1.0);
                        preview = preview.child(gpui::img(image).w(px(info.width * scale)).h(px(info.height * scale)));
                    },
                    crate::embeds::EmbedState::Failed(message) => preview = preview.child(message.to_string()),
                    _ => preview = preview.child("正在生成预览…"),
                }
                let top = self.visible_bounds.zip(self.bounds).map_or(y + h + 4.0, |(visible, bounds)| {
                    if y + h + 130.0 * zoom > f32::from(visible.bottom() - bounds.top()) { (y - 130.0 * zoom).max(0.0) } else { y + h + 4.0 }
                });
                let right = self.bounds.map_or(x + 360.0 * zoom, |bounds| f32::from(bounds.size.width));
                overlay = overlay.child(div().absolute().left(px(x.min((right - 360.0 * zoom).max(0.0)))).top(px(top))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(preview));
            }
            return Some(overlay.into_any_element());
        }
        let width = local.map(|(_, _, w, _)| w).unwrap_or_else(|| self
            .bounds
            .map_or(600.0, |bounds| f32::from(bounds.size.width) - 48.0));
        let (_, top) = self.popup_position(width, 500.0, false);
        let (left, top, _, panel_height) = local.unwrap_or((24.0, top, width, 500.0));
        let name = match object.payload.as_ref() {
            Some(super::super::html_table::InlineAtom::Math(_)) => locale.text("formula"),
            Some(super::super::html_table::InlineAtom::Comment(_)) => locale.text("comment"),
            Some(super::super::html_table::InlineAtom::Footnote(_)) => locale.text("object_footnote"),
            _ => match object.draft.kind {
                ObjectKind::Code => locale.text("object_code"),
                ObjectKind::BlockComment => locale.text("comment"),
                ObjectKind::FrontMatter => "Front Matter",
                ObjectKind::InlineMath | ObjectKind::InsertInlineMath | ObjectKind::BlockMath | ObjectKind::InsertBlockMath => locale.text("formula"),
            ObjectKind::InlineComment | ObjectKind::InsertInlineComment | ObjectKind::InsertBlockComment => locale.text("comment_insert"),
                ObjectKind::Mermaid => "Mermaid",
                ObjectKind::Image => locale.text("object_image"),
                ObjectKind::Link => locale.text("object_link"),
                ObjectKind::Details => locale.text("object_details"),
                ObjectKind::Html => "HTML",
                ObjectKind::TableSize => locale.text("object_table"),
            },
        };
        let preview = object.preview_key.map(|key| {
            let store = cx.global::<EmbedStore>();
            let mut panel = div().id("object-preview").mt(px(10.0)).h(px(180.0)).overflow_y_scroll()
                .flex().items_center().justify_center().bg(theme.code_bg).rounded(px(6.0)).text_size(px(12.0));
            match store.state(key) {
                Some(crate::embeds::EmbedState::Ready(info)) => if let Some(image) = store.image(key) {
                    let scale = ((width - 40.0).max(20.0) / info.width.max(1.0)).min(164.0 / info.height.max(1.0)).min(1.0);
                    panel = panel.child(gpui::img(image).w(px(info.width * scale)).h(px(info.height * scale)));
                },
                Some(crate::embeds::EmbedState::Failed(message)) => panel = panel.child(message.to_string()),
                _ => panel = panel.child("正在生成预览…"),
            }
            if object.draft.kind == ObjectKind::Mermaid {
                let range = object.draft.range.clone();
                panel = panel.relative().child(div().id("draft-diagram-fullscreen").absolute().top(px(8.0)).right(px(8.0))
                    .size(px(28.0)).cursor_pointer().tooltip(crate::ui::components::tooltip("放大查看"))
                    .child(gpui::svg().path("icons/maximize.svg").size(px(16.0)).text_color(theme.muted)).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |view, _, window, cx| view.open_diagram_viewer(range.clone(), window, cx))));
            }
            panel
        });
        let mut input = div()
            .id("object-scroll")
            .h(px(if local.is_some() { (panel_height - 112.0 - if preview.is_some() { 190.0 } else { 0.0 }).max(32.0) }
                else if preview.is_some() { 190.0 } else { 280.0 }))
            .overflow_y_scroll();
        if let Some(form) = &object.image {
            for (label, field) in [
                locale.text("image_src"),
                locale.text("image_alt"),
                locale.text("image_title"),
                locale.text("image_width"),
                locale.text("image_align"),
            ]
            .into_iter()
            .zip(&form.fields)
            {
                input = input.child(div().mt_2().child(label).child(field.clone()));
            }
        } else {
            input = input.child(object.child.clone());
        }
        Some(
            div()
                .absolute()
                .top(px(top))
                .left(px(left))
                .w(px(width))
                .max_h(px(panel_height))
                .bg(theme.surface)
                .text_color(theme.fg)
                .border_1()
                .border_color(theme.border)
                .rounded_lg()
                .shadow_lg()
                .p_4()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(locale.text("edit_object").replace("{name}", name))
                .child(input)
                .children(preview)
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .pt_3()
                        .child(
                            div()
                                .id("object-confirm")
                                .px_3()
                                .py_1()
                                .cursor_pointer()
                                .child(locale.text("confirm"))
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.finish_object(true, window, cx)
                                })),
                        )
                        .child(
                            div()
                                .id("object-cancel")
                                .px_3()
                                .py_1()
                                .cursor_pointer()
                                .child(locale.text("cancel"))
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.finish_object(false, window, cx)
                                })),
                        ),
                ).into_any_element(),
        )
    }
}

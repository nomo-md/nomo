//! 编辑区右键菜单：只记录当前窗口身份、选区和局部目标；动作复用现有宿主事务。
use super::*;
use crate::nomo_mark::{link_edit::LinkDraft, table::{Table, TableIntent, Alignment}, widgets::{ObjectKind, BlockFrame}};
use crate::ui::toolbar_i18n::ToolbarLocale;
use gpui::{ScrollHandle, SharedString};
use pulldown_cmark::{Event, Tag, CodeBlockKind};

#[derive(Clone)]
pub(super) enum Target {
    Text, Selection, Link(usize), Heading(u8),
    Block(Range<usize>, ObjectKind), Table(Range<usize>, usize, usize, bool),
    Details(Range<usize>, bool), Image(Range<usize>),
}

pub(super) struct ContextMenu {
    pub identity: WindowIdentity,
    pub selection: DocumentSelection,
    pub target: Target,
    pub position: Point<Pixels>,
    pub pointer: Point<Pixels>,
    pub hit: usize,
    pub focus: FocusHandle,
    pub selected: Option<usize>,
    pub submenu: Option<usize>,
    pub subselected: Option<usize>,
    pub keyboard: bool,
    pub scroll: ScrollHandle,
    pub subscroll: ScrollHandle,
    pub row_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl ContextMenu {
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.identity.document_id.capacity() + self.identity.window_id.capacity()
    }
}

#[derive(Clone)]
pub(super) enum MenuAction {
    None, Edit(EditIntent), Undo, Redo, Cut, Copy, Paste(bool), SelectAll, Find,
    LinkEdit, LinkOpen, LinkCopy, LinkRemove, InlineComment, InlineMath, ChooseImage,
    ObjectEdit, ObjectCopy, CodeLanguage, CodeParagraph, Delete,
    Table(TableIntent), DetailsDefault, DetailsUnwrap,
    ImageAlign(&'static str), ImageOriginal, ImageSize, ImageAlt, ImageLocation, ImageCopy, ImagePath,
}

pub(super) struct Item {
    pub label: SharedString,
    pub icon: &'static str,
    pub shortcut: String,
    pub action: MenuAction,
    pub children: Vec<Item>,
    pub enabled: bool,
    pub checked: bool,
    pub danger: bool,
    pub separator: bool,
}
impl Item {
    fn new(locale: ToolbarLocale, key: &'static str, icon: &'static str, action: MenuAction) -> Self {
        Self { label: locale.context_text(key).into(), icon, shortcut: String::new(), action,
            children: Vec::new(), enabled: true, checked: false, danger: false, separator: false }
    }
    fn shortcut(mut self, shortcut: &str) -> Self {
        self.shortcut = if cfg!(target_os = "macos") { shortcut.replace("Ctrl+", "⌘").replace("Shift+", "⇧") } else { shortcut.into() }; self
    }
    fn enabled(mut self, enabled: bool) -> Self { self.enabled = enabled; self }
    fn checked(mut self, checked: bool) -> Self { self.checked = checked; self }
    fn children(mut self, children: Vec<Item>) -> Self { self.children = children; self }
    fn danger(mut self) -> Self { self.danger = true; self }
    fn separator(locale: ToolbarLocale) -> Self { Self { separator: true, ..Self::new(locale, "", "", MenuAction::None) } }
}

impl NomoMark {
    pub(crate) fn open_context_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.interaction.protected() || self.auxiliary_input { return; }
        let at = self.content_bounds().and_then(|bounds| self.layout.as_ref()?.object_source_at(
            f32::from(position.x - bounds.left()), f32::from(position.y - bounds.top())))
            .or_else(|| self.hit(position)).unwrap_or_else(|| self.local_cursor());
        let absolute = self.host.parsed.identity.from_byte + at as u64;
        let selection = self.host.selection.range();
        if selection.is_empty() || absolute < selection.start || absolute > selection.end {
            self.move_to(at, false, cx);
        }
        self.dismiss_quick_insert(cx);
        self.menu_open = false;
        self.emoji_mode = false;
        self.code_language = None;
        self.block_menu_open = false;
        self.table_size_picker = false;
        self.context_form = None;
        let target = self.context_target(at);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.context_menu = Some(ContextMenu {
            identity: self.host.parsed.identity.clone(), selection: self.host.selection, target, position, pointer: position, hit: at, focus,
            selected: None, submenu: None, subselected: None, keyboard: false,
            scroll: ScrollHandle::new(), subscroll: ScrollHandle::new(), row_bounds: Default::default(),
        });
        cx.emit(ControlEvent::ContextMenuOpened);
        cx.notify();
    }

    fn context_target(&self, at: usize) -> Target {
        if self.effective_mode() == ViewMode::Semantic {
            let parsed = &self.host.parsed;
            if let Some(mut range) = self.image_range_at(at) {
                if parsed.source[range.end..].starts_with('{') {
                    if let Some(end) = parsed.source[range.end..].find('}') { range.end += end + 1; }
                }
                return Target::Image(range);
            }
            if LinkDraft::begin(parsed, DocumentSelection::caret(parsed.identity.from_byte + at as u64)).is_ok_and(|link| link.active)
                || parsed.events.iter().any(|(event, range)| matches!(event, Event::Start(Tag::Link { .. })) && range.contains(&at)) {
                return Target::Link(at);
            }
            if let Some(block) = parsed.details.iter().filter(|b| b.summary.contains(&at) || b.source.start == at).min_by_key(|b| b.source.len()) {
                return Target::Details(block.source.clone(), block.default_open);
            }
            for (event, range) in &parsed.events {
                if !range.contains(&at) { continue; }
                match event {
                    Event::Start(Tag::Heading { level, .. }) => return Target::Heading(*level as u8),
                    Event::Start(Tag::CodeBlock(kind)) => return Target::Block(range.clone(),
                        if matches!(kind, CodeBlockKind::Fenced(info) if info.split_whitespace().next() == Some("mermaid")) { ObjectKind::Mermaid } else { ObjectKind::Code }),
                    Event::DisplayMath(_) => return Target::Block(range.clone(), ObjectKind::BlockMath),
                    _ => {}
                }
            }
            if let Some(table) = parsed.html_tables.iter().find(|t| t.source.contains(&at)) {
                if let Some(cell) = table.cell_at(at) { return Target::Table(table.source.clone(), cell.row, cell.column, table.cells.iter().filter(|c| c.row == 0).all(|c| c.header)); }
            }
            if let Some(table) = Table::at(parsed, at) {
                if let Some((row, column)) = table.cell_at(at) { return Target::Table(table.source, row, column, true); }
            }
        }
        if self.host.selection.range().is_empty() { Target::Text } else { Target::Selection }
    }

    pub(super) fn context_items(&self, cx: &App) -> Vec<Item> {
        let Some(menu) = &self.context_menu else { return Vec::new(); };
        let locale = ToolbarLocale::current(cx);
        let editable = self.host.presentation == Presentation::Editable && !self.host.interaction.protected();
        let semantic = self.effective_mode() == ViewMode::Semantic;
        let item = |key, icon, action| Item::new(locale, key, icon, action);
        let edit = |key, icon, intent| item(key, icon, MenuAction::Edit(intent)).enabled(editable);
        let sep = || Item::separator(locale);
        let heading = |level| {
            let mut row = edit("headingLevel", "heading", EditIntent::Heading(level));
            row.label = locale.context_text("headingLevel").replace("{level}", &level.to_string()).into(); row
        };
        match &menu.target {
            Target::Text => {
                let mut insert: Vec<_> = (1..=6).map(|level| heading(level).shortcut(&format!("Ctrl+{level}"))).collect();
                insert.push(sep());
                insert.extend([
                    edit("unorderedList", "list", EditIntent::LinePrefix("- ".into())),
                    edit("orderedList", "list", EditIntent::LinePrefix("1. ".into())),
                    edit("taskList", "list", EditIntent::LinePrefix("- [ ] ".into())),
                    edit("quote", "quote", EditIntent::LinePrefix("> ".into())),
                    edit("callout", "quote", EditIntent::InsertBlockAt { block: "> [!NOTE]\n> ".into(), caret: 12 }),
                    edit("details", "chevrons-up", EditIntent::InsertDetails), sep(),
                    edit("codeBlock", "code-xml", EditIntent::InsertCodeBlock(cx.global::<Settings>().default_code_language.clone())),
                    edit("table", "table-2", EditIntent::InsertBlockAt { block: "|  |  |  |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |\n|  |  |  |".into(), caret: 2 }),
                    item("image", "image", MenuAction::ChooseImage).enabled(editable),
                    item("mathInline", "sigma", MenuAction::InlineMath).enabled(editable && semantic),
                    edit("mathBlock", "sigma", EditIntent::InsertBlock("$$\nE = mc^2\n$$".into())),
                    edit("mermaidDiagram", "maximize-2", EditIntent::InsertDiagram(crate::nomo_mark::quick_insert::DIAGRAMS[crate::nomo_mark::quick_insert::default_diagram_index(&cx.global::<crate::settings::Settings>().string("defaultDiagramType"))].2.into())),
                    edit("horizontalRule", "separator-horizontal", EditIntent::ToggleRule),
                ]);
                vec![
                    item("undo", "undo-2", MenuAction::Undo).shortcut("Ctrl+Z").enabled(editable && self.context_history.0),
                    item("redo", "redo-2", MenuAction::Redo).shortcut(if cfg!(target_os = "macos") { "Ctrl+Shift+Z" } else { "Ctrl+Y" }).enabled(editable && self.context_history.1),
                    item("paste", "clipboard", MenuAction::Paste(false)).shortcut("Ctrl+V").enabled(editable),
                    item("pasteAsPlainText", "clipboard", MenuAction::Paste(true)).shortcut("Ctrl+Shift+V").enabled(editable), sep(),
                    item("insert", "list-plus", MenuAction::None).enabled(editable).children(insert), sep(),
                    item("selectAll", "text-cursor-input", MenuAction::SelectAll).shortcut("Ctrl+A"),
                    item("find", "search", MenuAction::Find).shortcut("Ctrl+F").enabled(self.workspace_hosted),
                ]
            }
            Target::Selection => {
                let mut formats = Vec::new();
                for (key, icon, format, shortcut) in [
                    ("bold", "crop", InlineFormat::Strong, "Ctrl+B"), ("italic", "crop", InlineFormat::Emphasis, "Ctrl+I"),
                    ("underline", "crop", InlineFormat::Underline, "Ctrl+U"), ("strikethrough", "crop", InlineFormat::Strike, ""),
                    ("inlineCode", "code-xml", InlineFormat::Code, "Ctrl+`"), ("highlight", "crop", InlineFormat::Highlight, ""),
                ] {
                    let selected = local_range(&self.host.parsed.source, self.host.parsed.identity.from_byte, menu.selection.range()).ok();
                    let active = selected.as_ref().is_some_and(|range| self.host.parsed.inline.iter().any(|span|
                        span.format == format && span.content.start <= range.start && range.end <= span.content.end));
                    formats.push(edit(key, icon, EditIntent::ToggleInline(format)).shortcut(shortcut).checked(active));
                }
                formats.extend([sep(), edit("clearStyle", "crop", EditIntent::ClearFormatting)]);
                vec![item("cut", "scissors", MenuAction::Cut).shortcut("Ctrl+X").enabled(editable),
                    item("copy", "copy", MenuAction::Copy).shortcut("Ctrl+C"),
                    item("paste", "clipboard", MenuAction::Paste(false)).shortcut("Ctrl+V").enabled(editable),
                    item("pasteAsPlainText", "clipboard", MenuAction::Paste(true)).shortcut("Ctrl+Shift+V").enabled(editable), sep(),
                    item("format", "crop", MenuAction::None).enabled(editable).children(formats),
                    item("createLink", "link", MenuAction::LinkEdit).shortcut(&crate::ui::shortcuts::current("menu-link", cx)).enabled(editable && semantic),
                    item("insertInlineComment", "pencil", MenuAction::InlineComment).enabled(editable && semantic), sep(),
                    item("selectAll", "text-cursor-input", MenuAction::SelectAll).shortcut("Ctrl+A"),
                    item("find", "search", MenuAction::Find).shortcut("Ctrl+F").enabled(self.workspace_hosted)]
            }
            Target::Link(at) => {
                let ready = !self.host.parsed.context.reference_pending(&(*at..*at));
                vec![item("openLink", "eye", MenuAction::LinkOpen).enabled(ready),
                    item("editLink", "pencil", MenuAction::LinkEdit).enabled(editable && ready),
                    item("copyLinkAddress", "copy", MenuAction::LinkCopy).enabled(ready), sep(),
                    item("removeLink", "link-2-off", MenuAction::LinkRemove).enabled(editable && ready)]
            }
            Target::Heading(level) => vec![item("heading", "heading", MenuAction::None)
                    .children((1..=6).map(|n| heading(n).checked(n == *level)).collect()),
                edit("paragraph", "crop", EditIntent::Heading(0)), sep(),
                edit("liftHeading", "heading", EditIntent::AdjustHeading { increase: true }).enabled(editable && *level > 1),
                edit("sinkHeading", "heading", EditIntent::AdjustHeading { increase: false }).enabled(editable && *level < 6)],
            Target::Block(range, kind) => {
                let (edit_key, copy_key) = match kind {
                    ObjectKind::Code => ("editCode", "copyCode"), ObjectKind::Mermaid => ("editDiagramSource", "copyDiagramSource"),
                    _ => ("editFormulaSource", "copyFormulaSource"),
                };
                let mut items = vec![item(edit_key, "pencil", MenuAction::ObjectEdit).enabled(editable), item(copy_key, "copy", MenuAction::ObjectCopy)];
                if *kind == ObjectKind::Code { items.extend([
                    item("selectLanguage", "code-xml", MenuAction::CodeLanguage).enabled(editable && crate::nomo_mark::code_blocks::FencedCode::at(&self.host.parsed, range.start).is_some()),
                    item("convertToParagraph", "crop", MenuAction::CodeParagraph).enabled(editable),
                ]); }
                items.extend([sep(), item("deleteAction", "trash-2", MenuAction::Delete).enabled(editable).danger()]); items
            }
            Target::Table(_, row, column, header) => {
                let table = |key, icon, intent| item(key, icon, MenuAction::Table(intent)).enabled(editable);
                vec![table("addRowBefore", "table-2", TableIntent::InsertRowBefore { row: *row }),
                    table("addRowAfter", "table-2", TableIntent::InsertRow { after: *row }),
                    table("addColumnBefore", "table-2", TableIntent::InsertColumn { at: *column }),
                    table("addColumnAfter", "table-2", TableIntent::InsertColumn { at: *column + 1 }), sep(),
                    table("alignLeft", "align-left", TableIntent::Align { column: *column, alignment: Alignment::Left }),
                    table("alignCenter", "align-center", TableIntent::Align { column: *column, alignment: Alignment::Center }),
                    table("alignRight", "align-right", TableIntent::Align { column: *column, alignment: Alignment::Right }),
                    table("toggleTableHeader", "table-2", TableIntent::Header { enabled: !header }), sep(),
                    table("deleteRow", "trash-2", TableIntent::DeleteRow { row: *row }),
                    table("deleteColumn", "trash-2", TableIntent::DeleteColumn { column: *column }),
                    item("deleteTable", "trash-2", MenuAction::Delete).enabled(editable).danger()]
            }
            Target::Details(_, open) => vec![item("detailsDefaultOpen", "chevrons-down", MenuAction::DetailsDefault).enabled(editable).checked(*open),
                item("unwrapDetails", "pencil", MenuAction::DetailsUnwrap).enabled(editable)],
            Target::Image(range) => {
                let properties = self.context_image_properties(range);
                let ready = properties.is_some() && !self.host.parsed.context.reference_pending(range);
                let align = properties.as_ref().map_or("", |p| p.align.as_str());
                vec![item("alignLeft", "align-left", MenuAction::ImageAlign("left")).enabled(editable && ready).checked(align == "left"),
                    item("alignCenter", "align-center", MenuAction::ImageAlign("center")).enabled(editable && ready).checked(align == "center"),
                    item("alignRight", "align-right", MenuAction::ImageAlign("right")).enabled(editable && ready).checked(align == "right"), sep(),
                    item("originalSize", "image", MenuAction::ImageOriginal).enabled(editable && ready).checked(properties.as_ref().is_some_and(|p| p.width.is_empty())),
                    item("setSize", "pencil", MenuAction::ImageSize).enabled(editable && ready),
                    item("editImageAlt", "pencil", MenuAction::ImageAlt).enabled(editable && ready), sep(),
                    item("openImageLocation", "eye", MenuAction::ImageLocation).enabled(ready),
                    item("copyImage", "copy", MenuAction::ImageCopy).enabled(ready),
                    item("copyImagePath", "copy", MenuAction::ImagePath).enabled(ready), sep(),
                    item("deleteAction", "trash-2", MenuAction::Delete).enabled(editable).danger()]
            }
        }
    }

    pub(super) fn dismiss_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.context_menu.take() {
            if menu.focus.contains_focused(window, cx) { window.focus(&self.focus, cx); }
            cx.notify();
        }
    }

    pub(super) fn run_context_item(&mut self, index: usize, nested: bool, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.context_items(cx);
        let Some(menu) = &self.context_menu else { return; };
        let item = if nested { menu.submenu.and_then(|i| items.get(i)).and_then(|item| item.children.get(index)) } else { items.get(index) };
        let Some(item) = item.filter(|item| item.enabled && !item.separator) else { return; };
        if !item.children.is_empty() {
            let menu = self.context_menu.as_mut().unwrap();
            window.focus(&menu.focus, cx);
            menu.submenu = Some(index);
            menu.subselected = menu.keyboard.then(|| item.children.iter().position(|i| i.enabled && !i.separator)).flatten();
            menu.subscroll.set_offset(point(px(0.0), px(0.0))); cx.notify(); return;
        }
        let action = item.action.clone();
        let menu = self.context_menu.take().unwrap();
        window.focus(&self.focus, cx);
        cx.notify();
        if menu.identity != self.host.parsed.identity || menu.selection != self.host.selection {
            self.error(ControlError::Stale, cx); return;
        }
        self.run_context_action(action, menu, window, cx);
    }

    fn run_context_action(&mut self, action: MenuAction, menu: ContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        use MenuAction as A;
        // 对象命令以右键命中目标为准，即使用户保留了另一端的选区。
        let target_at = match &menu.target { Target::Link(at) => Some(*at), Target::Block(r, _) | Target::Image(r) | Target::Details(r, _) => Some(r.start),
            Target::Table(range, row, column, _) => self.context_table_cell(range, *row, *column), Target::Heading(_) => Some(menu.hit), _ => None };
        if let Some(at) = target_at { self.move_to(at, false, cx); }
        match action {
            A::None => {}, A::Edit(intent) => self.command(intent, cx),
            A::Undo => cx.emit(ControlEvent::Undo), A::Redo => cx.emit(ControlEvent::Redo),
            A::Cut => self.copy(true, cx), A::Copy => self.copy(false, cx), A::Paste(plain) => self.paste(plain, cx),
            A::SelectAll => cx.emit(ControlEvent::SelectAll), A::Find => window.dispatch_action(Box::new(crate::ui::workspace::Find), cx),
            A::InlineComment => { if let Err(error) = self.begin_inline_comment(window, cx) { self.error(error, cx); } },
            A::InlineMath => { if let Err(error) = self.begin_inline_math(window, cx) { self.error(error, cx); } },
            A::LinkEdit => {
                if self.workspace_hosted { window.dispatch_action(Box::new(crate::ui::workspace::Link), cx); }
                else { self.begin_object(self.local_cursor()..self.local_cursor(), ObjectKind::Link, window, cx); }
            }
            A::LinkOpen | A::LinkCopy | A::LinkRemove => {
                match LinkDraft::begin(&self.host.parsed, self.host.selection) {
                    Ok(link) if link.active => match action {
                        A::LinkOpen => cx.emit(ControlEvent::OpenLink(link.href)),
                        A::LinkCopy => self.context_copy_text(&link.href, cx),
                        _ => match link.edit(&self.host.parsed, &link.text, None) {
                            Ok(result) => self.context_queue(result, cx), Err(error) => self.error(error, cx),
                        }
                    }, Ok(_) => self.error(ControlError::Stale, cx), Err(error) => self.error(error, cx),
                }
            }
            A::ObjectEdit => if let Target::Block(range, kind) = menu.target { self.begin_object(range, kind, window, cx); },
            A::ObjectCopy => if let Target::Block(range, kind) = menu.target {
                let text = BlockFrame::split(&self.host.parsed.source[range.clone()], kind).map(|(_, text)| text)
                    .unwrap_or_else(|| self.host.parsed.events.iter().filter(|(_, span)| range.start <= span.start && span.end <= range.end)
                        .filter_map(|(event, _)| match event { Event::Text(text) | Event::DisplayMath(text) => Some(text.as_ref()), _ => None }).collect());
                self.context_copy_text(&text, cx);
            },
            A::CodeParagraph => {
                // 沿用代码转换命令的完整结构检查，不能绕过嵌套／跨窗口保护。
                match commands::apply(&self.host.parsed, self.host.selection, EditIntent::InsertCodeBlock(String::new())) {
                    Ok(mut result) => {
                        for edit in &mut result.edits {
                            let body = edit.inserted_text.trim_end_matches(['\r', '\n']);
                            let suffix = &edit.inserted_text[body.len()..];
                            let eol = commands::line_ending(&self.host.parsed.source, self.local_cursor());
                            edit.inserted_text = format!("{}{suffix}", crate::nomo_mark::clipboard_content::literal_paragraphs(body).replace('\n', eol));
                        }
                        self.context_queue(result, cx);
                    }, Err(error) => self.error(error, cx),
                }
            }
            A::CodeLanguage => if let Target::Block(range, _) = menu.target {
                self.open_code_language(range.start, window, cx);
            },
            A::Delete => match menu.target { Target::Block(r, _) | Target::Table(r, ..) | Target::Image(r) => self.context_replace(r, String::new(), cx), _ => {} },
            A::Table(intent) => self.table_command(intent, cx),
            A::DetailsDefault | A::DetailsUnwrap => if let Target::Details(range, open) = menu.target {
                if let Some(block) = self.host.parsed.details.iter().find(|b| b.source == range) {
                    let (range, text) = if matches!(action, A::DetailsDefault) {
                        let end = range.start + self.host.parsed.source[range.clone()].find('>').unwrap_or(0) + 1;
                        (range.start..end, if open { "<details>" } else { "<details open>" }.to_owned())
                    } else { (range, block.unwrapped(&self.host.parsed.source)) };
                    self.context_replace(range, text, cx);
                }
            },
            A::ChooseImage => {
                let identity = self.host.parsed.identity.clone(); let selection = self.host.selection; let mode = self.effective_mode();
                let paths = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some(ToolbarLocale::current(cx).text("quick_choose_images").into()) });
                cx.spawn_in(window, async move |this, cx| {
                    let result = paths.await;
                    let _ = this.update_in(cx, |view, _, cx| match result {
                        Ok(Ok(Some(paths))) => {
                            if view.host.parsed.identity != identity || view.host.selection != selection || view.effective_mode() != mode
                                || view.host.presentation != Presentation::Editable { view.error(ControlError::Stale, cx); return; }
                            cx.emit(ControlEvent::InsertImages { identity, selection, inputs: paths.into_iter().map(crate::image_import::ImageInput::File).collect() });
                        }, Ok(Ok(None)) => {}, _ => view.error(ControlError::HostFailure("无法打开图片选择窗口".into()), cx),
                    });
                }).detach();
            }
            A::ImageAlign(_) | A::ImageOriginal | A::ImageSize | A::ImageAlt | A::ImageLocation | A::ImageCopy | A::ImagePath => {
                if let Target::Image(range) = menu.target { self.context_image_action(action, range, menu.position, window, cx); }
            }
        }
    }

    fn context_table_cell(&self, range: &Range<usize>, row: usize, column: usize) -> Option<usize> {
        self.host.parsed.html_tables.iter().find(|t| t.source == *range).and_then(|t| t.cells.iter().find(|c| c.row == row && c.column == column)).map(|c| c.content.start)
            .or_else(|| Table::at(&self.host.parsed, range.start).and_then(|t| t.rows.get(row).and_then(|r| r.cells.get(column)).map(|r| r.start)))
    }
    pub(super) fn context_copy_text(&self, text: &str, cx: &mut Context<Self>) {
        if let Err(error) = crate::clipboard::write_text(text, cx) { cx.emit(ControlEvent::Failure(error)); }
    }
    pub(super) fn context_replace(&mut self, range: Range<usize>, text: String, cx: &mut Context<Self>) {
        let base = self.host.parsed.identity.from_byte;
        let edits = vec![SourceEdit { from_byte: base + range.start as u64, to_byte: base + range.end as u64, inserted_text: text }];
        self.context_queue(commands::EditResult { selection: commands::map_selection(self.host.selection, &edits), edits, template_update: None }, cx);
    }
    pub(super) fn context_queue(&mut self, result: commands::EditResult, cx: &mut Context<Self>) {
        self.history_serial += 1;
        let group = format!("{}:context:{}", self.host.parsed.identity.window_id, self.history_serial);
        if let Err(error) = self.queue_result(result, &group, cx) { self.error(error, cx); }
    }
}

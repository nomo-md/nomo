//! 控件内部的对象草稿视图。输入只修改草稿，确认后才向宿主提交一个事务。
use super::*;
use crate::editor_control::widgets::{ObjectDraft, ObjectKind};
use crate::ui::text_field::TextField;
use gpui::Subscription;

struct ImageForm {
    original: super::super::image_properties::ImageProperties,
    fields: Vec<Entity<TextField>>,
    inline: bool,
}

pub(super) struct ObjectEditor {
    draft: ObjectDraft,
    child: Entity<EditorControl>,
    _subscription: Subscription,
    child_bytes: usize,
    image: Option<ImageForm>,
    payload: Option<super::super::html_table::InlineAtom>,
    property_bytes: usize,
    _field_subscriptions: Vec<Subscription>,
}
impl ObjectEditor {
    pub(super) fn estimated_bytes(&self) -> usize {
        self.draft.estimated_bytes()
            + self.child_bytes
            + self.property_bytes
            + self
                .payload
                .as_ref()
                .map_or(0, super::super::html_table::InlineAtom::estimated_bytes)
    }
}

impl EditorControl {
    pub(super) fn begin_object(
        &mut self,
        mut range: Range<usize>,
        kind: ObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.host.presentation != Presentation::Editable {
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
        let payload = self
            .host
            .parsed
            .html_tables
            .iter()
            .flat_map(|t| &t.cells)
            .flat_map(|c| &c.runs)
            .find(|r| r.source == draft.range)
            .and_then(|r| r.atom.as_ref())
            .filter(|a| a.text().is_some())
            .cloned();
        if let Some(text) = payload.as_ref().and_then(|a| a.text()) {
            draft.text = text.into();
        }
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
            let mut view = EditorControl::new(host, cx);
            view.mode = ViewMode::Source;
            view.auxiliary_input = true;
            view
        });
        let subscription = cx.subscribe(&child, |parent, child, event: &ControlEvent, cx| {
            match event {
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
                ControlEvent::Undo | ControlEvent::Redo => parent.error(
                    ControlError::NeedsContext("请先确认或取消对象草稿，再操作文档历史".into()),
                    cx,
                ),
                ControlEvent::Failure(message) => cx.emit(ControlEvent::Failure(message.clone())),
                _ => {}
            }
            cx.notify();
        });
        let image = if kind == ObjectKind::Image {
            super::super::image_properties::ImageProperties::read(&draft.text).map(|original| {
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
        window.focus(&image.as_ref().map_or_else(
            || child.focus_handle(cx),
            |form| form.fields[0].focus_handle(cx),
        ));
        self.object = Some(ObjectEditor {
            draft,
            child_bytes: child.read(cx).estimated_bytes(),
            child,
            _subscription: subscription,
            image,
            payload,
            property_bytes,
            _field_subscriptions: field_subscriptions,
        });
        cx.emit(ControlEvent::DraftChanged {
            identity: self.host.parsed.identity.clone(),
            active: true,
        });
        cx.notify();
    }

    pub(super) fn finish_object(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    self.error(ControlError::Busy, cx);
                    return;
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
                    Ok(text) => self.object.as_mut().unwrap().draft.text = text,
                    Err(error) => {
                        self.error(error, cx);
                        return;
                    }
                }
            }
        }
        let Some(object) = &self.object else {
            return;
        };
        if let Err(error) = object.child.read(cx).prepare_unmount() {
            self.error(error, cx);
            return;
        }
        let result = if confirm && object.draft.kind == ObjectKind::TableSize {
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
        } else if confirm {
            if let Some(payload) = &object.payload {
                let text = if payload.text() != Some(object.draft.text.as_str()) {
                    payload.with_text(&object.draft.text).expect("text atom")
                } else {
                    self.host.parsed.source[object.draft.range.clone()].to_string()
                };
                object.draft.confirm_text(&self.host.parsed, &text)
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
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.error(error, cx);
                return;
            }
        };
        self.host.interaction.draft = false;
        self.host.set_readonly(false);
        self.history_serial += 1;
        let group = format!(
            "{}:object:{}",
            self.host.parsed.identity.window_id, self.history_serial
        );
        if let Err(error) = self.host.queue_edit(result.edits, result.selection, &group) {
            self.host.interaction.draft = true;
            self.host.set_readonly(true);
            self.error(error, cx);
            return;
        }
        self.object = None;
        self.invalidate();
        self.submit(cx);
        window.focus(&self.focus);
        cx.emit(ControlEvent::DraftChanged {
            identity: self.host.parsed.identity.clone(),
            active: false,
        });
        cx.notify();
    }

    pub(super) fn object_panel(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let object = self.object.as_ref()?;
        let theme = cx.global::<Theme>();
        let width = self
            .bounds
            .map_or(600.0, |bounds| f32::from(bounds.size.width) - 48.0);
        let (_, top) = self.popup_position(width, 500.0, false);
        let name = match object.payload.as_ref() {
            Some(super::super::html_table::InlineAtom::Math(_)) => "公式",
            Some(super::super::html_table::InlineAtom::Comment(_)) => "注释",
            Some(super::super::html_table::InlineAtom::Footnote(_)) => "脚注编号",
            _ => match object.draft.kind {
                ObjectKind::Code => "代码",
                ObjectKind::InlineMath | ObjectKind::BlockMath => "公式",
                ObjectKind::Mermaid => "Mermaid",
                ObjectKind::Image => "图片",
                ObjectKind::Link => "链接",
                ObjectKind::Details => "折叠块",
                ObjectKind::Html => "HTML",
                ObjectKind::TableSize => "表格行列数量（依次输入行数、列数）",
            },
        };
        let mut input = div()
            .id("object-scroll")
            .max_h(px(380.0))
            .overflow_y_scroll();
        if let Some(form) = &object.image {
            for (label, field) in [
                "图片地址",
                "替代文字",
                "标题",
                "宽度（像素，留空为原尺寸）",
                "对齐（left / center / right）",
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
                .left(px(24.0))
                .right(px(24.0))
                .max_h(px(500.0))
                .bg(theme.surface)
                .text_color(theme.fg)
                .border_1()
                .border_color(theme.border)
                .rounded_lg()
                .shadow_lg()
                .p_4()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(format!("编辑{name}"))
                .child(input)
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
                                .child("确认")
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
                                .child("取消")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.finish_object(false, window, cx)
                                })),
                        ),
                ),
        )
    }
}

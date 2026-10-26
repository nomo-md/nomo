//! 只投影宿主明确提供的空模板；占位解析保持字节长度，原文从不改写。
use super::{
    commands::{EditIntent, EditResult, map_selection},
    parse::ParsedWindow,
    protocol::*,
};
use pulldown_cmark::Event;
use std::{borrow::Cow, ops::Range};

/// 组字期间暂存来源位置，取消时交给宿主事务恢复；不保存正文或撤销栈。
#[derive(Default)]
pub struct CompositionTemplates {
    at: Option<usize>,
    origins: Vec<InlineTemplate>,
}
impl CompositionTemplates {
    pub fn begin(&mut self, parsed: &ParsedWindow, range: &Range<usize>) {
        self.clear();
        if !range.is_empty() {
            return;
        }
        let base = parsed.identity.from_byte;
        let at = base + range.start as u64;
        self.at = Some(range.start);
        self.origins = parsed
            .context
            .inline_templates
            .iter()
            .filter(|t| t.open_end_byte <= at && at <= t.close_start_byte)
            .cloned()
            .map(|mut t| {
                t.from_byte -= base;
                t.open_end_byte -= base;
                t.close_start_byte -= base;
                t.to_byte -= base;
                t
            })
            .collect();
    }
    pub fn cancelled(&self, base: u64, range: &Range<usize>, text: &str) -> Option<TemplateUpdate> {
        if self.at != Some(range.start) || !text.is_empty() || self.origins.is_empty() {
            return None;
        }
        Some(TemplateUpdate {
            add: self
                .origins
                .iter()
                .cloned()
                .map(|mut t| {
                    t.from_byte += base;
                    t.open_end_byte += base;
                    t.close_start_byte += base;
                    t.to_byte += base;
                    t
                })
                .collect(),
        })
    }
    pub fn clear(&mut self) {
        self.at = None;
        self.origins = Vec::new();
    }
    pub fn estimated_bytes(&self) -> usize {
        self.origins.capacity() * std::mem::size_of::<InlineTemplate>()
    }
}

pub fn masked_source<'a>(source: &'a str, base: u64, templates: &[InlineTemplate]) -> Cow<'a, str> {
    if templates.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut masked = source.as_bytes().to_vec();
    for t in templates.iter().filter(|t| t.valid_in(source, base)) {
        for (a, b) in [
            (t.from_byte, t.open_end_byte),
            (t.close_start_byte, t.to_byte),
        ] {
            masked[(a - base) as usize..(b - base) as usize].fill(b'x');
        }
    }
    // 只覆盖经验证的 ASCII 标记，不会切开 UTF-8。
    Cow::Owned(String::from_utf8(masked).expect("ASCII template markers"))
}

pub fn restore_events(
    events: Vec<(Event<'static>, Range<usize>)>,
    source: &str,
    base: u64,
    templates: &[InlineTemplate],
) -> Vec<(Event<'static>, Range<usize>)> {
    if templates.is_empty() {
        return events;
    }
    let mut markers = Vec::new();
    for t in templates.iter().filter(|t| t.valid_in(source, base)) {
        let (open, close) = t.format.html_markers();
        markers.push((
            (t.from_byte - base) as usize..(t.open_end_byte - base) as usize,
            open,
            false,
        ));
        markers.push((
            (t.close_start_byte - base) as usize..(t.to_byte - base) as usize,
            close,
            true,
        ));
    }
    markers.sort_by_key(|(r, _, _)| r.start);
    let mut output = Vec::with_capacity(events.len() + markers.len() * 2);
    for (event, range) in events {
        if !matches!(event, Event::Text(_)) {
            output.push((event, range));
            continue;
        }
        let start = markers.partition_point(|(r, _, _)| r.end <= range.start);
        let mut at = range.start;
        let relevant: Vec<_> = markers[start..]
            .iter()
            .take_while(|(r, _, _)| r.start < range.end)
            .collect();
        if relevant.is_empty() {
            output.push((event, range));
            continue;
        }
        for (marker, tag, close) in relevant {
            if marker.start < range.start || marker.end > range.end {
                continue;
            }
            if at < marker.start {
                output.push((
                    Event::Text(source[at..marker.start].to_string().into()),
                    at..marker.start,
                ));
            } else if *close {
                output.push((Event::Text("".into()), at..at));
            }
            output.push((Event::InlineHtml((*tag).into()), marker.clone()));
            at = marker.end;
        }
        if at < range.end {
            output.push((
                Event::Text(source[at..range.end].to_string().into()),
                at..range.end,
            ));
        }
    }
    output
}

/// 模板符号可能与 Markdown 的普通分隔线同形，命令只信任来源记录。
pub fn command(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    intent: &EditIntent,
) -> Option<EditResult> {
    if !selection.range().is_empty() {
        return None;
    }
    let requested = match intent {
        EditIntent::ToggleInline(kind) => Some(*kind),
        _ => return None,
    };
    let templates: Vec<_> = parsed
        .context
        .inline_templates
        .iter()
        .filter(|t| {
            t.open_end_byte <= selection.head_byte && selection.head_byte <= t.close_start_byte
        })
        .collect();
    let target = requested.and_then(|kind| {
        templates
            .iter()
            .copied()
            .filter(|t| t.format == kind)
            .min_by_key(|t| t.to_byte - t.from_byte)
    });
    let opposite = requested
        .and_then(super::inline_commands::opposite)
        .and_then(|kind| {
            templates
                .iter()
                .copied()
                .filter(|t| t.format == kind)
                .min_by_key(|t| t.to_byte - t.from_byte)
        });
    let chosen: Vec<_> = if let Some(t) = target {
        vec![t]
    } else if let Some(t) = opposite {
        vec![t]
    } else if requested.is_none() {
        templates
    } else {
        return None;
    };
    if chosen.is_empty() {
        return None;
    }
    let mut edits = Vec::new();
    let mut update = TemplateUpdate::default();
    for t in chosen {
        let switch = opposite.is_some() && target.is_none() && requested.is_some();
        let (open, close) = if switch {
            let format = requested.unwrap();
            if t.html {
                format.html_markers()
            } else {
                format.markers()
            }
        } else {
            ("", "")
        };
        edits.push(SourceEdit {
            from_byte: t.from_byte,
            to_byte: t.open_end_byte,
            inserted_text: open.into(),
        });
        edits.push(SourceEdit {
            from_byte: t.close_start_byte,
            to_byte: t.to_byte,
            inserted_text: close.into(),
        });
        if switch {
            let open_end_byte = t.from_byte + open.len() as u64;
            let close_start_byte = open_end_byte + t.close_start_byte - t.open_end_byte;
            update.add.push(InlineTemplate {
                format: requested.unwrap(),
                from_byte: t.from_byte,
                open_end_byte,
                close_start_byte,
                to_byte: close_start_byte + close.len() as u64,
                html: t.html,
            });
        }
    }
    edits.sort_by_key(|e| e.from_byte);
    Some(EditResult {
        selection: map_selection(selection, &edits),
        edits,
        template_update: Some(update),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_control::{commands, host::WindowHostAdapter, parse::InlineFormat};

    fn ready(host: &mut WindowHostAdapter) {
        host.mark_layout_ready(
            LayoutStamp {
                identity: host.parsed.identity.clone(),
                width: 500.0,
                font_generation: 0,
                resource_generation: 0,
                zoom: 1.0,
            },
            30.0,
        )
        .unwrap();
    }
    fn host(source: &str) -> WindowHostAdapter {
        WindowHostAdapter::new(
            DocumentWindow {
                document_id: "templates".into(),
                revision: 0,
                from_byte: 0,
                to_byte: source.len() as u64,
                text: source.into(),
                inline_templates: vec![],
            },
            "template-view",
            0,
            WindowContext::Complete,
        )
        .unwrap()
    }
    fn edit(host: &mut WindowHostAdapter, intent: EditIntent, group: &str) {
        ready(host);
        let result = commands::apply(&host.parsed, host.selection, intent).unwrap();
        host.queue_template_edit(
            result.edits,
            result.selection,
            group,
            result.template_update,
        )
        .unwrap();
    }
    #[test]
    fn eight_explicit_templates_keep_inline_layout_and_source_positions() {
        for kind in [
            InlineFormat::Strong,
            InlineFormat::Emphasis,
            InlineFormat::Code,
            InlineFormat::Strike,
            InlineFormat::Underline,
            InlineFormat::Highlight,
            InlineFormat::Superscript,
            InlineFormat::Subscript,
        ] {
            let mut h = host("");
            edit(&mut h, EditIntent::ToggleInline(kind), "add");
            let (open, close) = kind.markers();
            assert_eq!(&*h.parsed.source, format!("{open}{close}"));
            assert_eq!(h.parsed.context.inline_templates.len(), 1, "{kind:?}");
            assert_eq!(h.parsed.inline[0].format, kind);
            assert_eq!(h.selection.head_byte, open.len() as u64);
            assert!(
                !h.parsed
                    .events
                    .iter()
                    .any(|(e, _)| matches!(e, Event::Rule)),
                "{kind:?}"
            );
            assert!(
                h.parsed
                    .events
                    .iter()
                    .any(|(e, r)| matches!(e,Event::Text(t) if t.is_empty())
                        && r.start == open.len())
            );
            let revealed = h.parsed.markers_for(&(open.len()..open.len()));
            let displayed: String = h
                .parsed
                .display_events(&revealed)
                .iter()
                .filter_map(|(e, _)| match e {
                    Event::Text(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(displayed, format!("{open}{close}"));
            edit(&mut h, EditIntent::ToggleInline(kind), "off");
            assert!(h.parsed.source.is_empty());
            assert!(h.parsed.context.inline_templates.is_empty());
        }
        let handwritten = host("****");
        assert!(handwritten.parsed.context.inline_templates.is_empty());
        assert!(
            handwritten
                .parsed
                .events
                .iter()
                .any(|(e, _)| matches!(e, Event::Rule))
        );
    }
    #[test]
    fn nested_template_removal_and_script_switch_preserve_provenance() {
        let mut h = host("");
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Strong),
            "bold",
        );
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Emphasis),
            "italic",
        );
        assert_eq!(&*h.parsed.source, "******");
        assert_eq!(h.parsed.context.inline_templates.len(), 2);
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Strong),
            "unbold",
        );
        assert_eq!(&*h.parsed.source, "**");
        assert_eq!(
            h.parsed.context.inline_templates[0].format,
            InlineFormat::Emphasis
        );
        assert_eq!(h.selection.head_byte, 1);
        edit(&mut h, EditIntent::ClearFormatting, "clear");
        assert_eq!(&*h.parsed.source, "");
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Subscript),
            "sub",
        );
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Superscript),
            "sup",
        );
        assert_eq!(&*h.parsed.source, "^^");
        assert_eq!(
            h.parsed.context.inline_templates[0].format,
            InlineFormat::Superscript
        );
        edit(&mut h, EditIntent::Replace("中文😀".into()), "typing");
        assert_eq!(&*h.parsed.source, "^中文😀^");
        assert!(h.parsed.context.inline_templates.is_empty());
    }
    #[test]
    fn cancelled_composition_restores_nested_origins_and_identical_input_is_not_submitted() {
        let mut h = host("");
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Strong),
            "bold",
        );
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Emphasis),
            "italic",
        );
        let mut composition = CompositionTemplates::default();
        composition.begin(&h.parsed, &(3..3));
        let pending = h.pending_count();
        ready(&mut h);
        h.queue_edit(
            vec![SourceEdit {
                from_byte: 0,
                to_byte: 6,
                inserted_text: "******".into(),
            }],
            h.selection,
            "noop",
        )
        .unwrap();
        assert_eq!(h.pending_count(), pending);
        assert_eq!(h.parsed.context.inline_templates.len(), 2);
        edit(&mut h, EditIntent::Replace("ni".into()), "composition");
        assert!(h.parsed.context.inline_templates.is_empty());
        ready(&mut h);
        h.queue_template_edit(
            vec![SourceEdit {
                from_byte: 3,
                to_byte: 5,
                inserted_text: String::new(),
            }],
            DocumentSelection::caret(3),
            "composition",
            composition.cancelled(0, &(3..5), ""),
        )
        .unwrap();
        assert_eq!(&*h.parsed.source, "******");
        assert_eq!(h.parsed.context.inline_templates.len(), 2);
        assert!(
            !h.parsed
                .events
                .iter()
                .any(|(e, _)| matches!(e, Event::Rule))
        );
    }
    #[test]
    fn partial_code_toggle_keeps_literal_punctuation_and_code_sides() {
        for (source, from, to, plain) in [
            ("`a*b*c`", 2, 5, "a*b*c"),
            ("``a`b`c``", 3, 6, "a`b`c"),
            ("`a\r\nb`", 2, 4, "a b"),
        ] {
            let mut h = host(source);
            h.selection = DocumentSelection {
                anchor_byte: from,
                head_byte: to,
            };
            edit(
                &mut h,
                EditIntent::ToggleInline(InlineFormat::Code),
                "partial-code",
            );
            let text: String = h
                .parsed
                .events
                .iter()
                .filter_map(|(e, _)| match e {
                    Event::Text(s) | Event::Code(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, plain, "{}", h.parsed.source);
            assert!(
                !h.parsed
                    .inline
                    .iter()
                    .any(|s| s.format == InlineFormat::Emphasis)
            );
            assert_eq!(
                h.parsed
                    .inline
                    .iter()
                    .filter(|s| s.format == InlineFormat::Code)
                    .count(),
                2
            );
        }
        let mut h = host("`*x*`");
        h.selection = DocumentSelection::caret(2);
        edit(&mut h, EditIntent::ClearFormatting, "clear-code");
        assert_eq!(&*h.parsed.source, "\\*x\\*");
        assert!(h.parsed.inline.is_empty());
    }
    #[cfg(feature = "editor-control-demo")]
    #[test]
    fn template_ack_retry_undo_redo_hide_and_full_snapshot_use_original_session() {
        use crate::editor_control::native_port::NativeWindowPort;
        let root = std::env::temp_dir().join(format!(
            "nomo-template-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let port = NativeWindowPort::open(
            root.clone(),
            "template-test".into(),
            None,
            Some("before\r\n\nend".into()),
        )
        .unwrap();
        let mut h = WindowHostAdapter::new(
            port.read_window(0, 8..8).unwrap(),
            "w",
            0,
            WindowContext::Complete,
        )
        .unwrap();
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Subscript),
            "sub",
        );
        edit(
            &mut h,
            EditIntent::ToggleInline(InlineFormat::Superscript),
            "sup",
        );
        assert!(h.suspend().is_err());
        let flush = |h: &mut WindowHostAdapter| {
            while let Some(ControlEvent::Submit(tx)) = h.next_submission() {
                let revision = port.submit(tx.clone()).unwrap();
                assert_eq!(port.submit(tx.clone()).unwrap(), revision);
                h.acknowledge(&tx.operation_id, revision).unwrap();
            }
            let id = &h.parsed.identity;
            h.verify_authoritative(
                &port
                    .read_window(id.revision, id.from_byte..id.to_byte)
                    .unwrap(),
            )
            .unwrap();
        };
        flush(&mut h);
        assert_eq!(&*h.parsed.source, "^^");
        h.suspend().unwrap();
        let status = port.status().unwrap();
        let (revision, selection) = port.history(status.revision, false).unwrap();
        h.replace_window(
            port.read_window(revision, 8..10).unwrap(),
            1,
            WindowContext::Complete,
        )
        .unwrap();
        h.selection = selection.unwrap();
        assert_eq!(&*h.parsed.source, "~~");
        assert_eq!(
            h.parsed.context.inline_templates[0].format,
            InlineFormat::Subscript
        );
        let (revision, selection) = port.history(revision, true).unwrap();
        h.replace_window(
            port.read_window(revision, 8..10).unwrap(),
            2,
            WindowContext::Complete,
        )
        .unwrap();
        h.selection = selection.unwrap();
        edit(&mut h, EditIntent::Replace("中😀".into()), "input");
        flush(&mut h);
        assert!(h.parsed.context.inline_templates.is_empty());
        let revision = port.status().unwrap().revision;
        let (revision, selection) = port.history(revision, false).unwrap();
        h.replace_window(
            port.read_window(revision, 8..10).unwrap(),
            3,
            WindowContext::Complete,
        )
        .unwrap();
        h.selection = selection.unwrap();
        assert_eq!(
            h.parsed.context.inline_templates[0].format,
            InlineFormat::Superscript
        );
        let cut = port.read_window(revision, 9..10).unwrap();
        assert!(ParsedWindow::parse(cut, "cut", 0, WindowContext::Complete).is_err());
        let saved = root.join("saved.md");
        port.save_to(saved.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(saved).unwrap(), "before\r\n^^\nend");
        port.close(false).unwrap();
        drop(port);
        std::fs::remove_dir_all(root).unwrap();
    }
}

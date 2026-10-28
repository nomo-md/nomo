//! 跨窗口选区的临时组字。只保存有界预编辑文字，确认前不删除原选区或写入会话。
use super::*;
use crate::layout::{ParaSpec, TextLayout, Wrap};

const MAX_PREEDIT_BYTES: usize = 64 * 1024;

pub(super) struct ExternalComposition {
    pub selection: DocumentSelection,
    pub text: String,
    pub selected: Range<usize>,
    anchor: u64,
    preview: Option<Rc<TextLayout>>,
    preview_key: Option<(u32, u32, u32, u64)>,
}
impl ExternalComposition {
    pub fn estimated_bytes(&self) -> usize {
        self.text.capacity() + self.preview.as_ref().map_or(0, |layout| layout.estimated_bytes())
    }
}

impl NomoMark {
    pub(super) fn replace_external_composition(
        &mut self, range: Option<Range<usize>>, text: &str,
        selected: Option<Range<usize>>, mark: bool, cx: &mut Context<Self>,
    ) {
        let first = self.external_composition.is_none();
        if first && !mark {
            let range = self.host.selection.range();
            let after = DocumentSelection::caret(range.start + text.len() as u64);
            self.history_serial += 1;
            let group = format!("{}:input:{}", self.host.parsed.identity.window_id, self.history_serial);
            match self.host.queue_rewindow(vec![SourceEdit {
                from_byte: range.start, to_byte: range.end, inserted_text: text.into(),
            }], after, &group, None) {
                Ok(()) => { self.submit(cx); cx.notify(); }
                Err(error) => self.error(error, cx),
            }
            return;
        }
        if text.len() > MAX_PREEDIT_BYTES {
            self.finish_external_composition(false, cx);
            self.error(ControlError::NeedsContext("预编辑文字超过 64 KiB 预算，原选区已保留".into()), cx);
            return;
        }
        if first {
            if self.host.presentation != Presentation::Editable || self.host.failure().is_some() {
                self.error(ControlError::ReadOnly, cx);
                return;
            }
            if mark && text.is_empty() { return; }
            let selection = self.host.selection;
            let identity = &self.host.parsed.identity;
            let mut anchor = selection.range().start.clamp(identity.from_byte, identity.to_byte);
            if let Some(visible) = self.visible_bounds {
                let in_view = self.document_caret_bounds(anchor).is_some_and(|(_, caret)| visible.contains(&caret.origin));
                if !in_view {
                    // 全局选区的替换起点可能未加载；临时组字留在当前可见区域，确认后再定位真实起点。
                    if let Some(local) = self.hit(visible.origin + point(px(1.0), px(1.0))) {
                        anchor = identity.from_byte + local as u64;
                    }
                }
            }
            self.external_composition = Some(ExternalComposition {
                selection, text: String::new(), selected: 0..0, anchor,
                preview: None, preview_key: None,
            });
            self.host.interaction.begin_composition((anchor - identity.from_byte) as usize);
            self.history_serial += 1;
            cx.emit(ControlEvent::CompositionChanged { identity: identity.clone(), active: true });
        }
        let composition = self.external_composition.as_mut().unwrap();
        // 首次回调仍针对原文全局选区；后续回调使用这个独立预编辑缓冲的 UTF-16 坐标。
        let range = if first { 0..0 } else {
            range.map(|range| utf16_to_utf8(&composition.text, range.start)..utf16_to_utf8(&composition.text, range.end))
                .unwrap_or(0..composition.text.len())
        };
        if range.start > range.end || composition.text.len() - range.len() + text.len() > MAX_PREEDIT_BYTES {
            self.finish_external_composition(false, cx);
            self.error(ControlError::NeedsContext("预编辑文字超过 64 KiB 预算，原选区已保留".into()), cx);
            return;
        }
        let start = range.start;
        composition.text.replace_range(range, text);
        composition.selected = selected.map(|range| {
            start + utf16_to_utf8(text, range.start)..start + utf16_to_utf8(text, range.end)
        }).unwrap_or(start + text.len()..start + text.len());
        composition.preview = None;
        composition.preview_key = None;
        if composition.text.is_empty() {
            self.finish_external_composition(false, cx);
        } else if !mark {
            self.finish_external_composition(true, cx);
        }
        cx.notify();
    }
    pub(super) fn finish_external_composition(&mut self, commit: bool, cx: &mut Context<Self>) -> bool {
        let Some(composition) = self.external_composition.as_ref() else { return false; };
        if commit {
            let range = composition.selection.range();
            let after = DocumentSelection::caret(range.start + composition.text.len() as u64);
            let group = format!("{}:composition:{}", self.host.parsed.identity.window_id, self.history_serial);
            if let Err(error) = self.host.queue_rewindow(vec![SourceEdit {
                from_byte: range.start, to_byte: range.end, inserted_text: composition.text.clone(),
            }], after, &group, None) {
                self.error(error, cx);
                return true;
            }
        }
        self.external_composition = None;
        self.composition_templates.clear();
        self.host.interaction.end_composition();
        self.history_serial += 1;
        cx.emit(ControlEvent::CompositionChanged { identity: self.host.parsed.identity.clone(), active: false });
        if commit { self.submit(cx); }
        cx.notify();
        true
    }
    pub(super) fn layout_external_composition(&mut self, width: f32, window: &Window, cx: &mut Context<Self>) {
        let Some(composition) = self.external_composition.as_mut() else { return; };
        let metrics = cx.global::<Settings>().metrics();
        let key = (width.to_bits(), metrics.body_size().to_bits(), metrics.body_line_height().to_bits(), crate::layout::generation());
        if composition.preview_key == Some(key) { return; }
        let spec = ParaSpec::plain(composition.text.clone(), Default::default(), Family::Text,
            metrics.body_size(), metrics.body_line_height(), Wrap::Greedy);
        composition.preview = Some(Rc::new(layout_text(&spec, width.max(1.0), window)));
        composition.preview_key = Some(key);
        cx.emit(ControlEvent::ResourceUsage {
            identity: self.host.parsed.identity.clone(), parsed_bytes: self.host.parsed.estimated_bytes(),
            layout_bytes: self.layout.as_ref().map_or(0, |layout| layout.estimated_bytes())
                + composition.estimated_bytes(),
            pending_bytes: self.host.pending_bytes(),
        });
    }
    fn external_preview(&self) -> Option<(Point<Pixels>, &TextLayout)> {
        let composition = self.external_composition.as_ref()?;
        let layout = composition.preview.as_ref()?;
        let (_, caret) = self.document_caret_bounds(composition.anchor)?;
        let bounds = self.bounds?;
        let x = caret.origin.x.min(bounds.origin.x + (bounds.size.width - px(layout.max_line_width())).max(px(0.0)));
        Some((point(x, caret.origin.y), layout))
    }
    pub(super) fn external_caret_bounds(&self, at: usize) -> Option<(WindowIdentity, Bounds<Pixels>)> {
        let (origin, layout) = self.external_preview()?;
        let (line, x) = layout.caret(at.min(layout.text.len()));
        Some((self.host.parsed.identity.clone(), Bounds::new(origin + point(px(x), px(line as f32 * layout.line_height)),
            size(px(1.5), px(layout.line_height)))))
    }
    pub(super) fn paint_external_composition(&self, theme: &Theme, window: &mut Window) {
        let Some(composition) = self.external_composition.as_ref() else { return; };
        let Some((origin, layout)) = self.external_preview() else { return; };
        paint_box(Bounds::new(origin, size(px(layout.max_line_width().max(2.0)), px(layout.height()))), theme.surface, 0.0, None, window);
        for (line, x, width) in layout.range_rects(composition.selected.clone()) {
            paint_box(Bounds::new(origin + point(px(x), px(line as f32 * layout.line_height)), size(px(width), px(layout.line_height))),
                theme.selection, 0.0, None, window);
        }
        layout.paint(origin, 0.0..layout.height(), theme, window);
        for (line, x, width) in layout.range_rects(0..composition.text.len()) {
            paint_box(Bounds::new(origin + point(px(x), px((line + 1) as f32 * layout.line_height - 1.0)), size(px(width), px(1.0))),
                theme.caret, 0.0, None, window);
        }
        if let Some((_, caret)) = self.external_caret_bounds(composition.selected.end) {
            paint_box(caret, theme.caret, 0.0, None, window);
        }
    }
}

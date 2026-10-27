//! 依据虚拟行最终布局宽度截断文本，避免弹性布局测量阶段丢失省略号。
use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, HighlightStyle, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, ShapedLine, SharedString, Style, TextRun, TextStyle,
    Window, point, px, relative,
};
use std::ops::Range;

pub(super) struct TreeLabel {
    text: SharedString,
    matched: Option<Range<usize>>,
    highlight: Hsla,
}

impl TreeLabel {
    pub fn new(text: String, query: &str, highlight: Hsla) -> Self {
        // Unix 文件名可能含换行，GPUI 的单行排版不能直接接收换行符。
        let text = if text.chars().any(char::is_control) {
            text.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
        } else { text };
        let lowered = text.to_lowercase();
        let matched = if !query.is_empty() && lowered.len() == text.len() {
            lowered.find(query).map(|start| start..start + query.len())
                .filter(|range| text.is_char_boundary(range.start) && text.is_char_boundary(range.end))
        } else { None };
        Self { text: text.into(), matched, highlight }
    }
}

impl IntoElement for TreeLabel {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for TreeLabel {
    type RequestLayoutState = TextStyle;
    type PrepaintState = Option<(ShapedLine, Point<Pixels>, Pixels)>;

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, TextStyle) {
        let text_style = window.text_style();
        let mut layout = Style::default();
        layout.size.width = relative(1.0).into();
        layout.size.height = px(20.0).into();
        (window.request_layout(layout, [], cx), text_style)
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, style: &mut TextStyle, window: &mut Window, cx: &mut App) -> Self::PrepaintState {
        if bounds.size.width <= px(0.0) { return None; }
        // 旧版 Windows 界面字体栈：Segoe UI 的中文字符回退到微软雅黑。
        if cfg!(target_os = "windows") && style.font_family.as_ref() == "Segoe UI" && style.font_fallbacks.is_none() {
            static FALLBACKS: std::sync::OnceLock<gpui::FontFallbacks> = std::sync::OnceLock::new();
            style.font_fallbacks = Some(FALLBACKS.get_or_init(|| gpui::FontFallbacks::from_fonts(vec!["Microsoft YaHei".into()])).clone());
        }
        let mut runs: Vec<TextRun> = Vec::new();
        if let Some(range) = &self.matched {
            if range.start > 0 { runs.push(style.to_run(range.start)); }
            runs.push(style.clone().highlight(HighlightStyle { background_color: Some(self.highlight), ..Default::default() }).to_run(range.len()));
            if range.end < self.text.len() { runs.push(style.to_run(self.text.len() - range.end)); }
        } else { runs.push(style.to_run(self.text.len())); }
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut wrapper = cx.text_system().line_wrapper(style.font(), font_size);
        let (text, runs) = wrapper.truncate_line(self.text.clone(), bounds.size.width, "…", &runs, gpui::TruncateFrom::End);
        let line = window.text_system().shape_line(text, font_size, &runs, None);
        let height = style.line_height_in_pixels(window.rem_size());
        let origin = point(bounds.left(), bounds.top() + (bounds.size.height - height) / 2.0);
        Some((line, origin, height))
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut TextStyle, prepared: &mut Self::PrepaintState, window: &mut Window, cx: &mut App) {
        if let Some((line, origin, height)) = prepared.take() {
            let _ = line.paint_background(origin, height, gpui::TextAlign::Left, None, window, cx);
            let _ = line.paint(origin, height, gpui::TextAlign::Left, None, window, cx);
        }
    }
}

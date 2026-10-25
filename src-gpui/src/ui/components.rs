//! 基础组件：在 GPUI 原语（div / svg）之上补齐 Web 版常用的界面元件。
//!
//! GPUI 0.2.2 没有现成的按钮、图标、提示、分段控件等组件，这里统一封装尺寸、圆角、
//! 悬停 / 激活 / 禁用态与主题色，调用方只关心图标、文案与点击行为。组件都是无状态的构建函数，
//! 返回可继续链式设置的元素；颜色在构建时从全局主题读取，主题切换后随整窗重绘生效。

use super::icon_assets;
use crate::theme::Theme;
use gpui::{
    AnyView, App, AssetSource, Context, Div, ElementId, FontWeight, Hsla, IntoElement, ParentElement, Render,
    SharedString, Stateful, Styled, Svg, Window, div, prelude::*, px, svg,
};
use std::borrow::Cow;

/// 内置图标（Lucide，与 Web 版同名）。新增图标需同时加入 scripts/export_icons.mjs 并重新导出。
/// 部分图标为 Roadmap 中即将使用的预留项。
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IconName {
    Heading1,
    Heading2,
    Heading3,
    Bold,
    Italic,
    Strikethrough,
    Underline,
    Highlighter,
    CodeXml,
    Link,
    Quote,
    List,
    ListOrdered,
    SquareCheck,
    Braces,
    Table,
    Sigma,
    Minus,
    Search,
    BookOpenText,
    Columns,
    ListTree,
    Sun,
    Moon,
    FileText,
    Folder,
    FolderOpen,
    ChevronRight,
    ChevronDown,
    Plus,
    Close,
    ArrowUp,
    ArrowDown,
    CaseSensitive,
    Replace,
    ReplaceAll,
    PanelLeft,
    ZoomIn,
    ZoomOut,
    Copy,
    Check,
    Pilcrow,
    Superscript,
    Subscript,
    Info,
    SquarePen,
    TextQuote,
    History,
    Settings2,
    Palette,
    FileImage,
    ChartColumn,
    SlidersHorizontal,
    Keyboard,
    Settings,
    RotateCcw,
}

impl IconName {
    pub fn path(self) -> &'static str {
        match self {
            IconName::Heading1 => "icons/heading-1.svg",
            IconName::Heading2 => "icons/heading-2.svg",
            IconName::Heading3 => "icons/heading-3.svg",
            IconName::Bold => "icons/bold.svg",
            IconName::Italic => "icons/italic.svg",
            IconName::Strikethrough => "icons/strikethrough.svg",
            IconName::Underline => "icons/underline.svg",
            IconName::Highlighter => "icons/highlighter.svg",
            IconName::CodeXml => "icons/code-xml.svg",
            IconName::Link => "icons/link.svg",
            IconName::Quote => "icons/quote.svg",
            IconName::List => "icons/list.svg",
            IconName::ListOrdered => "icons/list-ordered.svg",
            IconName::SquareCheck => "icons/square-check.svg",
            IconName::Braces => "icons/braces.svg",
            IconName::Table => "icons/table-2.svg",
            IconName::Sigma => "icons/sigma.svg",
            IconName::Minus => "icons/minus.svg",
            IconName::Search => "icons/search.svg",
            IconName::BookOpenText => "icons/book-open-text.svg",
            IconName::Columns => "icons/columns-2.svg",
            IconName::ListTree => "icons/list-tree.svg",
            IconName::Sun => "icons/sun.svg",
            IconName::Moon => "icons/moon.svg",
            IconName::FileText => "icons/file-text.svg",
            IconName::Folder => "icons/folder.svg",
            IconName::FolderOpen => "icons/folder-open.svg",
            IconName::ChevronRight => "icons/chevron-right.svg",
            IconName::ChevronDown => "icons/chevron-down.svg",
            IconName::Plus => "icons/plus.svg",
            IconName::Close => "icons/x.svg",
            IconName::ArrowUp => "icons/arrow-up.svg",
            IconName::ArrowDown => "icons/arrow-down.svg",
            IconName::CaseSensitive => "icons/case-sensitive.svg",
            IconName::Replace => "icons/replace.svg",
            IconName::ReplaceAll => "icons/replace-all.svg",
            IconName::PanelLeft => "icons/panel-left.svg",
            IconName::ZoomIn => "icons/zoom-in.svg",
            IconName::ZoomOut => "icons/zoom-out.svg",
            IconName::Copy => "icons/copy.svg",
            IconName::Check => "icons/check.svg",
            IconName::Pilcrow => "icons/pilcrow.svg",
            IconName::Superscript => "icons/superscript.svg",
            IconName::Subscript => "icons/subscript.svg",
            IconName::Info => "icons/info.svg",
            IconName::SquarePen => "icons/square-pen.svg",
            IconName::TextQuote => "icons/text-quote.svg",
            IconName::History => "icons/history.svg",
            IconName::Settings2 => "icons/settings-2.svg",
            IconName::Palette => "icons/palette.svg",
            IconName::FileImage => "icons/file-image.svg",
            IconName::ChartColumn => "icons/chart-column.svg",
            IconName::SlidersHorizontal => "icons/sliders-horizontal.svg",
            IconName::Keyboard => "icons/keyboard.svg",
            IconName::Settings => "icons/settings.svg",
            IconName::RotateCcw => "icons/rotate-ccw.svg",
        }
    }
}

/// 应用内嵌资源：目前只有图标，全部编译进二进制。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(icon_assets::icon_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(icon_assets::ICON_PATHS.iter().filter(|candidate| candidate.starts_with(path)).map(|candidate| (*candidate).into()).collect())
    }
}

/// 单色图标，颜色取当前文字颜色语义（调用方传入）。
pub fn icon(name: IconName, size: f32, color: Hsla) -> Svg {
    svg().path(name.path()).size(px(size)).flex_none().text_color(color)
}

/// 工具栏图标按钮：28×28，悬停底色，激活态使用强调色底。
pub fn icon_button(id: impl Into<ElementId>, name: IconName, active: bool, theme: &Theme) -> Stateful<Div> {
    let color = if active { theme.accent_strong } else { theme.fg };
    div()
        .id(id)
        .size(px(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .when(active, |el| el.bg(theme.sidebar_active))
        .hover(|style| style.bg(theme.hover))
        .cursor_pointer()
        .child(icon(name, 16.0, color))
}

/// 小号图标按钮（搜索面板、标签关闭等）。
pub fn small_icon_button(id: impl Into<ElementId>, name: IconName, active: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(22.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .when(active, |el| el.bg(theme.sidebar_active))
        .hover(|style| style.bg(theme.hover))
        .cursor_pointer()
        .child(icon(name, 14.0, if active { theme.accent_strong } else { theme.muted }))
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
}

/// 文字按钮，可带前置图标。
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, leading: Option<IconName>, kind: ButtonKind, theme: &Theme) -> Stateful<Div> {
    let (background, foreground, border) = match kind {
        ButtonKind::Primary => (theme.accent, theme.on_accent, None),
        ButtonKind::Secondary => (theme.surface, theme.fg, Some(theme.border)),
        ButtonKind::Ghost => (gpui::transparent_black(), theme.fg, None),
    };
    let hover = match kind {
        ButtonKind::Primary => theme.accent_strong,
        _ => theme.hover,
    };
    div()
        .id(id)
        .h(px(30.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(7.0))
        .bg(background)
        .text_color(foreground)
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .when_some(border, |el, color| el.border_1().border_color(color))
        .hover(move |style| style.bg(hover))
        .cursor_pointer()
        .when_some(leading, |el, name| el.child(icon(name, 15.0, foreground)))
        .child(label.into())
}

/// 竖向分隔线（工具栏分组）。
pub fn vertical_divider(theme: &Theme) -> Div {
    div().w(px(1.0)).h(px(16.0)).mx(px(4.0)).flex_none().bg(theme.border)
}

/// 分段控件的一个选项：图标 + 文字；选中态为强调色底。
pub fn segment(id: impl Into<ElementId>, name: IconName, label: &'static str, active: bool, theme: &Theme) -> Stateful<Div> {
    let color = if active { theme.on_accent } else { theme.muted };
    div()
        .id(id)
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .text_size(px(12.0))
        .text_color(color)
        .when(active, |el| el.bg(theme.accent))
        .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
        .cursor_pointer()
        .child(icon(name, 14.0, color))
        .child(label)
}

/// 分段控件容器。
pub fn segmented(theme: &Theme) -> Div {
    div().flex().p(px(2.0)).gap(px(2.0)).rounded(px(7.0)).bg(theme.app_bg).border_1().border_color(theme.border)
}

/// 浮层卡片（大纲、搜索面板等）：表面色、细边框、圆角与阴影。
pub fn floating_card(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div().id(id).absolute().rounded(px(10.0)).bg(theme.surface).border_1().border_color(theme.border).shadow_lg()
}

/// 悬停提示：调用方用 `.tooltip(tooltip("文字"))` 挂到任意可交互元素上。
pub fn tooltip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tooltip { text: text.into() }).into()
}

pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(theme.heading)
            .text_color(theme.surface)
            .text_size(px(12.0))
            .child(self.text.clone())
    }
}

/// 开关（Switch）：36×20 胶囊，开启时为强调色。点击行为由调用方通过 `.on_click` 挂接。
pub fn switch(id: impl Into<ElementId>, on: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(36.0))
        .h(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .px(px(2.0))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { theme.accent } else { theme.border })
        .when(on, |el| el.justify_end())
        .child(div().size(px(16.0)).rounded_full().bg(theme.surface).shadow_sm())
}

/// 纯文字的分段选项（设置中的二三选一）。
pub fn text_segment(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .text_size(px(12.5))
        .cursor_pointer()
        .text_color(if active { theme.on_accent } else { theme.fg })
        .when(active, |el| el.bg(theme.accent))
        .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
        .child(label.into())
}

/// 小徽标（如“暂未支持”）。
pub fn badge(text: impl Into<SharedString>, color: Hsla, theme: &Theme) -> Div {
    div()
        .px(px(6.0))
        .h(px(18.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(4.0))
        .text_size(px(11.0))
        .text_color(color)
        .border_1()
        .border_color(theme.border)
        .child(text.into())
}

/// 滑杆值变化回调：参数为 0–1 的位置，以及拖动是否仍在进行。
pub type SliderChange = std::rc::Rc<dyn Fn(f32, bool, &mut Window, &mut App)>;

/// 滑杆：轨道、已选区间与圆形把手由 canvas 自绘；按下时开始拖动，
/// 拖动期间在窗口级监听移动与松开，因此指针移出轨道仍可继续调整。
/// `dragging` 由调用方保存（同一时刻只有一个滑杆处于拖动）。
pub fn slider(id: impl Into<ElementId>, fraction: f32, dragging: bool, width: f32, theme: &Theme, on_change: SliderChange) -> Stateful<Div> {
    let fraction = fraction.clamp(0.0, 1.0);
    let (track, fill_color, knob, border) = (theme.border, theme.accent, theme.surface, theme.accent);
    div().id(id).w(px(width)).h(px(20.0)).flex_none().cursor_pointer().child(
        gpui::canvas(
            |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                let left = f32::from(bounds.origin.x) + 8.0;
                let span = (f32::from(bounds.size.width) - 16.0).max(1.0);
                let middle = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                let position = move |x: gpui::Pixels| ((f32::from(x) - left) / span).clamp(0.0, 1.0);
                let quad = |x: f32, y: f32, w: f32, h: f32, color: Hsla, radius: f32| {
                    gpui::fill(gpui::Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(w), px(h))), color).corner_radii(px(radius))
                };
                window.paint_quad(quad(left, middle - 2.0, span, 4.0, track, 2.0));
                window.paint_quad(quad(left, middle - 2.0, span * fraction, 4.0, fill_color, 2.0));
                let knob_x = left + span * fraction - 8.0;
                window.paint_quad(quad(knob_x, middle - 8.0, 16.0, 16.0, knob, 8.0).border_color(border).border_widths(px(2.0)));
                let on_down = on_change.clone();
                window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble && event.button == gpui::MouseButton::Left && bounds.contains(&event.position) {
                        on_down(position(event.position.x), true, window, cx);
                    }
                });
                if dragging {
                    let on_move = on_change.clone();
                    window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Bubble && event.pressed_button == Some(gpui::MouseButton::Left) {
                            on_move(position(event.position.x), true, window, cx);
                        }
                    });
                    let on_up = on_change.clone();
                    window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Bubble {
                            on_up(position(event.position.x), false, window, cx);
                        }
                    });
                }
            },
        )
        .size_full(),
    )
}

/// 下拉选择：触发按钮显示当前项；`open` 时在其下方以浮层列出选项。
/// 浮层之下铺一层覆盖整个窗口的透明遮罩，点击遮罩（包括再次点击触发按钮）调用 `on_dismiss`，
/// 避免“点击外部关闭”与触发按钮的点击先后触发、把菜单又打开。选项的点击行为由调用方挂接，
/// 并应调用 `cx.stop_propagation()`，以免冒泡到触发按钮。
#[allow(clippy::too_many_arguments)]
pub fn select(
    id: impl Into<ElementId>,
    current: impl Into<SharedString>,
    open: bool,
    width: f32,
    viewport: gpui::Size<gpui::Pixels>,
    theme: &Theme,
    options: Vec<gpui::AnyElement>,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let trigger = div()
        .id(id)
        .w(px(width))
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(6.0))
        .border_1()
        .border_color(if open { theme.accent } else { theme.border })
        .bg(theme.surface)
        .text_size(px(12.5))
        .cursor_pointer()
        .hover(|style| style.bg(theme.hover))
        .child(div().overflow_hidden().whitespace_nowrap().child(current.into()))
        .child(icon(IconName::ChevronDown, 14.0, theme.muted));
    let overlay = open.then(|| {
        gpui::deferred(
            gpui::anchored().position(gpui::point(px(0.0), px(0.0))).child(
                div()
                    .id("select-overlay")
                    .w(viewport.width)
                    .h(viewport.height)
                    .occlude()
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        on_dismiss(window, cx);
                    }),
            ),
        )
        .with_priority(1)
    });
    let menu = open.then(|| {
        gpui::deferred(
            gpui::anchored().snap_to_window_with_margin(px(8.0)).child(
                div()
                    .id("select-menu")
                    .mt(px(4.0))
                    .w(px(width))
                    .p(px(4.0))
                    .flex()
                    .flex_col()
                    .rounded(px(8.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .occlude()
                    .children(options),
            ),
        )
        .with_priority(2)
    });
    div().flex().flex_col().child(trigger).children(overlay).children(menu)
}

/// 下拉选择中的一个选项行。
pub fn select_option(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(5.0))
        .text_size(px(12.5))
        .cursor_pointer()
        .text_color(if selected { theme.accent_strong } else { theme.fg })
        .when(selected, |el| el.bg(theme.sidebar_active))
        .hover(|style| style.bg(theme.hover))
        .child(label.into())
        .when(selected, |el| el.child(icon(IconName::Check, 14.0, theme.accent)))
}

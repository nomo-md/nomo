//! 颜色与排版令牌，保留既有原生版的明暗配色。主题作为 GPUI 全局状态保存，切换后整窗重绘。

use crate::settings::{Settings, ThemeMode};
use crate::ui::motion::{self, MotionPreference};
use gpui::{App, Font, FontFallbacks, FontStyle, FontWeight, Global, Hsla, WindowAppearance, rgb, rgba};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub app_bg: Hsla,
    pub rail_bg: Hsla,
    pub surface: Hsla,
    pub fg: Hsla,
    pub heading: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub workspace_edge: Hsla,
    pub workspace_shadow_near: Hsla,
    pub workspace_shadow_soft: Hsla,
    pub hover: Hsla,
    pub accent: Hsla,
    pub accent_strong: Hsla,
    pub on_accent: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub sidebar_active: Hsla,
    pub selection: Hsla,
    pub caret: Hsla,
    pub link: Hsla,
    pub quote_border: Hsla,
    pub quote_fg: Hsla,
    pub code_bg: Hsla,
    pub code_border: Hsla,
    pub code_fg: Hsla,
    pub code_keyword: Hsla,
    pub code_string: Hsla,
    pub code_number: Hsla,
    pub code_comment: Hsla,
    pub code_punct: Hsla,
    pub inline_code_bg: Hsla,
    pub inline_code_fg: Hsla,
    pub table_border: Hsla,
    pub table_header_bg: Hsla,
    pub search_match: Hsla,
    pub search_current: Hsla,
    /// `<mark>` 高亮底色。
    pub mark_bg: Hsla,
    /// 文字颜色与背景色调色板（红、橙、黄、绿、蓝、紫、灰），取值参照飞书文档的默认色板。
    pub palette_fg: [Hsla; 7],
    pub palette_bg: [Hsla; 7],
    pub marker: Hsla,
    pub scrollbar: Hsla,
    pub callout_note: Hsla,
    pub callout_tip: Hsla,
    pub callout_important: Hsla,
    pub callout_warning: Hsla,
    pub callout_caution: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            app_bg: c(0xf6f7f9),
            rail_bg: c(0xf3f4f6),
            surface: c(0xffffff),
            fg: c(0x202428),
            heading: c(0x171a1f),
            muted: c(0x68707a),
            border: c(0xdfe3e8),
            workspace_edge: c(0xdfe3e8).opacity(0.76),
            workspace_shadow_near: c(0x16202c).alpha(0.04),
            workspace_shadow_soft: c(0x16202c).alpha(0.06),
            hover: c(0xedf1f2),
            accent: c(0x2f7d6f),
            accent_strong: c(0x174e45),
            on_accent: c(0xffffff),
            warning: c(0x9a6700),
            danger: c(0xb42318),
            sidebar_active: c(0xe7f0ee),
            selection: ca(0x2f7d6f2e),
            caret: c(0x2f7d6f),
            link: c(0x116d8f),
            quote_border: c(0x7f9f97),
            quote_fg: c(0x465753),
            code_bg: c(0xf5f6f8),
            code_border: c(0xe0e2e6),
            code_fg: c(0x383a42),
            code_keyword: c(0x8250df),
            code_string: c(0x0a3069),
            code_number: c(0x0550ae),
            code_comment: c(0x6e7781),
            code_punct: c(0x57606a),
            inline_code_bg: c(0xeef4f2),
            inline_code_fg: c(0x174e45),
            table_border: c(0xdfe3e8),
            table_header_bg: c(0xf0f2f4),
            search_match: ca(0xf5c34a55),
            search_current: ca(0xf59e0bb0),
            mark_bg: ca(0xffe58f99),
            palette_fg: [c(0xd83931), c(0xde7802), c(0xdc9b04), c(0x2ea121), c(0x245bdb), c(0x6425d0), c(0x646a73)],
            palette_bg: [ca(0xfbbfbcaa), ca(0xfed4a4aa), ca(0xf8e6abcc), ca(0xb7edb1aa), ca(0xbacefdaa), ca(0xcdb2faaa), ca(0xdee0e3cc)],
            marker: c(0xa3abb3),
            scrollbar: ca(0x7e8892b8),
            callout_note: c(0x3b82f6),
            callout_tip: c(0x22c55e),
            callout_important: c(0xa855f7),
            callout_warning: c(0xf59e0b),
            callout_caution: c(0xef4444),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            app_bg: c(0x16191d),
            rail_bg: c(0x1b1f24),
            surface: c(0x1f2328),
            fg: c(0xd7dce1),
            heading: c(0xeef1f4),
            muted: c(0x8b949e),
            border: c(0x30363d),
            workspace_edge: c(0x30363d).opacity(0.9),
            workspace_shadow_near: c(0x000000).alpha(0.16),
            workspace_shadow_soft: c(0x000000).alpha(0.22),
            hover: c(0x2a3037),
            accent: c(0x4fb3a0),
            accent_strong: c(0x8fd6c7),
            on_accent: c(0x0d1117),
            warning: c(0xd4a72c),
            danger: c(0xff8a7a),
            sidebar_active: c(0x23332f),
            selection: ca(0x4fb3a040),
            caret: c(0x4fb3a0),
            link: c(0x58a6ff),
            quote_border: c(0x5f7f77),
            quote_fg: c(0xa9b8b4),
            code_bg: c(0x262b31),
            code_border: c(0x353b42),
            code_fg: c(0xd7dce1),
            code_keyword: c(0xd2a8ff),
            code_string: c(0xa5d6ff),
            code_number: c(0x79c0ff),
            code_comment: c(0x8b949e),
            code_punct: c(0xa0a8b0),
            inline_code_bg: c(0x263330),
            inline_code_fg: c(0x8fd6c7),
            table_border: c(0x353b42),
            table_header_bg: c(0x262b31),
            search_match: ca(0xd29b2266),
            search_current: ca(0xf59e0bc0),
            mark_bg: ca(0xbb800966),
            palette_fg: [c(0xf76964), c(0xf8a454), c(0xf5c84c), c(0x51c048), c(0x6a9cf5), c(0xa47ef0), c(0xa3a9b1)],
            palette_bg: [ca(0xd8393150), ca(0xde780250), ca(0xdc9b0450), ca(0x2ea12150), ca(0x245bdb60), ca(0x6425d060), ca(0x646a7360)],
            marker: c(0x6e7781),
            scrollbar: ca(0x8b949eb0),
            callout_note: c(0x58a6ff),
            callout_tip: c(0x3fb950),
            callout_important: c(0xbc8cff),
            callout_warning: c(0xd29922),
            callout_caution: c(0xf85149),
        }
    }

    /// 手动切换深浅色：之后固定为所选模式（写入设置 `themeMode`），不再跟随系统。
    pub fn toggle(cx: &mut App) {
        let dark = !cx.global::<Theme>().dark;
        Settings::set(cx, "themeMode", serde_json::json!(if dark { "dark" } else { "light" }));
    }

    /// 按设置与系统外观应用主题。系统外观变化时由窗口观察者调用；非“跟随系统”模式下忽略外观。
    pub fn apply(appearance: WindowAppearance, cx: &mut App) {
        let dark = match cx.global::<Settings>().theme_mode {
            ThemeMode::System => matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark),
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
        if cx.try_global::<Theme>().is_some_and(|theme| theme.dark == dark) {
            return;
        }
        let target = if dark { Theme::dark() } else { Theme::light() };
        let from = if cx.try_global::<Theme>().is_some() { Theme::ui(cx) } else { target.clone() };
        cx.set_global(target.clone());
        if cx.try_global::<UiThemeTransition>().is_some() {
            let reduced = cx.try_global::<MotionPreference>().is_some_and(|preference| preference.reduced);
            cx.set_global(UiThemeTransition { from, target, started: (!reduced).then(Instant::now) });
        }
        cx.refresh_windows();
    }

    /// 首个窗口的初始主题直接生效，之后的主题切换才启动过渡。
    pub fn arm_ui_transition(cx: &mut App) {
        if cx.try_global::<UiThemeTransition>().is_none() {
            let theme = cx.global::<Theme>().clone();
            cx.set_global(UiThemeTransition { from: theme.clone(), target: theme, started: None });
        }
    }

    pub fn ui_transition_active(cx: &App) -> bool {
        !cx.try_global::<MotionPreference>().is_some_and(|preference| preference.reduced)
            && cx.try_global::<UiThemeTransition>().and_then(|transition| transition.started)
                .is_some_and(|started| started.elapsed() < Duration::from_millis(160))
    }

    pub fn finish_ui_transition(cx: &mut App) {
        if cx.try_global::<UiThemeTransition>().is_some() {
            let target = cx.global::<Theme>().clone();
            cx.set_global(UiThemeTransition { from: target.clone(), target, started: None });
        }
    }

    /// 外壳颜色使用短暂插值；编辑器文档布局继续读取稳定的目标主题。
    pub fn ui(cx: &App) -> Theme {
        let target = cx.global::<Theme>();
        let Some(transition) = cx.try_global::<UiThemeTransition>() else { return target.clone(); };
        let Some(started) = transition.started else { return target.clone(); };
        if transition.target.dark != target.dark
            || cx.try_global::<MotionPreference>().is_some_and(|preference| preference.reduced) {
            return target.clone();
        }
        let elapsed = started.elapsed().as_secs_f32();
        if elapsed >= 0.16 { return target.clone(); }
        let background = motion::ease((elapsed / 0.16).min(1.0));
        let foreground = motion::ease((elapsed / 0.12).min(1.0));
        let mut result = transition.target.clone();
        macro_rules! interpolate {
            ($progress:expr; $($field:ident),+ $(,)?) => {
                $(result.$field = motion::color(transition.from.$field, transition.target.$field, $progress);)+
            };
        }
        interpolate!(background; app_bg, rail_bg, surface, border, workspace_edge, hover, sidebar_active, selection,
            code_bg, code_border, inline_code_bg, table_border, table_header_bg, search_match,
            search_current, mark_bg, scrollbar, workspace_shadow_near, workspace_shadow_soft);
        interpolate!(foreground; fg, heading, muted, accent, accent_strong, on_accent, warning, danger, caret, link,
            quote_border, quote_fg, code_fg, code_keyword, code_string, code_number, code_comment,
            code_punct, inline_code_fg, marker, callout_note, callout_tip, callout_important,
            callout_warning, callout_caution);
        for index in 0..result.palette_fg.len() {
            result.palette_fg[index] = motion::color(transition.from.palette_fg[index], transition.target.palette_fg[index], foreground);
            result.palette_bg[index] = motion::color(transition.from.palette_bg[index], transition.target.palette_bg[index], background);
        }
        result
    }
}

impl Global for Theme {}

/// 固定保存一次过渡的起止颜色；无定时器，窗口销毁时不持有视图或文档。
struct UiThemeTransition {
    from: Theme,
    target: Theme,
    started: Option<Instant>,
}
impl Global for UiThemeTransition {}

/// 字体族。正文与等宽字体按平台选择，等宽字体缺少的汉字回退到正文。
///
/// 正文若以系统 UI 字体为主、苹方为回退，GPUI 0.2.2 的回退只按字族名匹配，汉字会丢失粗体字重。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    Text,
    Mono,
}

/// 字体覆盖（来自设置“字体”项）：空串表示使用默认字体。由 `Settings::derive` 在设置变化时更新。
static FONT_OVERRIDES: std::sync::RwLock<(String, String)> = std::sync::RwLock::new((String::new(), String::new()));

pub fn set_font_overrides(document: &str, mono: &str) {
    if let Ok(mut overrides) = FONT_OVERRIDES.write() {
        *overrides = (document.trim().to_string(), mono.trim().to_string());
    }
}

/// 当前平台的界面字体；正文设置覆盖不影响界面默认字体。
pub fn ui_font_family() -> &'static str {
    if cfg!(target_os = "macos") { ".SystemUIFont" }
    else if cfg!(target_os = "windows") { "Segoe UI" }
    else { "DejaVu Sans" }
}

fn default_fonts() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") { ("PingFang SC", "Menlo") }
    else if cfg!(target_os = "windows") { ("Microsoft YaHei", "Consolas") }
    else { ("Noto Sans CJK SC", "DejaVu Sans Mono") }
}

fn font_names() -> (gpui::SharedString, gpui::SharedString) {
    let (document, mono) = FONT_OVERRIDES.read().map(|names| names.clone()).unwrap_or_default();
    let (default_document, default_mono) = default_fonts();
    let document = if document.is_empty() { std::env::var("NOMO_TEXT_FONT").unwrap_or_else(|_| default_document.into()) } else { document };
    let mono = if mono.is_empty() { default_mono.to_string() } else { mono };
    (document.into(), mono.into())
}

pub fn font_for(family: Family, bold: bool, italic: bool) -> Font {
    let (document, mono) = font_names();
    let (name, fallback) = match family {
        Family::Text => (document, gpui::SharedString::from(ui_font_family())),
        Family::Mono => (mono, document),
    };
    Font {
        family: name,
        features: Default::default(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![fallback.to_string(), default_fonts().0.into()])),
        weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
    }
}

/// 文档排版参数：由设置的字号、行高与缩放比例决定；标题、代码字号按正文字号等比缩放，
/// 默认值（16px、1.75）下与 Web 版各元素的计算样式一致。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metrics {
    pub zoom: f32,
    pub font_size: f32,
    pub line_height: f32,
}

impl Metrics {
    /// 相对默认 16px 正文的比例，再乘以缩放。
    fn scale(&self) -> f32 {
        self.font_size / 16.0 * self.zoom
    }

    pub fn body_size(&self) -> f32 {
        self.font_size * self.zoom
    }

    pub fn body_line_height(&self) -> f32 {
        self.font_size * self.line_height * self.zoom
    }

    pub fn source_size(&self) -> f32 {
        14.0 * self.scale()
    }

    pub fn source_line_height(&self) -> f32 {
        14.0 * self.scale() * (self.line_height - 0.05).max(1.3)
    }

    pub fn code_size(&self) -> f32 {
        14.0 * self.scale()
    }

    pub fn code_line_height(&self) -> f32 {
        14.0 * 1.6 * self.scale()
    }

    /// 标题字号与上下外边距（px），对应 Web 版 h1–h6 的计算样式。
    pub fn heading(&self, level: u8) -> (f32, f32, f32) {
        let (size, top, bottom) = match level {
            1 => (32.0, 8.0, 22.0),
            2 => (26.0, 30.0, 20.0),
            3 => (21.0, 24.0, 16.0),
            4 => (18.0, 20.0, 12.0),
            5 => (16.0, 18.0, 10.0),
            _ => (14.5, 16.0, 10.0),
        };
        let scale = self.scale();
        (size * scale, top * scale, bottom * scale)
    }

    pub fn paragraph_gap(&self) -> f32 {
        16.0 * self.scale()
    }

    pub fn list_indent(&self) -> f32 {
        28.0 * self.scale()
    }
}

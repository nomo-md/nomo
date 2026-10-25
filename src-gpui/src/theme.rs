//! 颜色与排版令牌。浅色值取自 Web 版默认主题（`src/app/services/themeRegistry.ts` 运行时生效的 CSS 变量），
//! 暗色为原型自定的近似值。主题作为 GPUI 全局状态保存，切换后整窗重绘。

use crate::settings::{Settings, ThemeMode};
use gpui::{App, Font, FontFallbacks, FontStyle, FontWeight, Global, Hsla, WindowAppearance, rgb, rgba};

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
    pub hover: Hsla,
    pub accent: Hsla,
    pub accent_strong: Hsla,
    pub on_accent: Hsla,
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
            hover: c(0xedf1f2),
            accent: c(0x2f7d6f),
            accent_strong: c(0x174e45),
            on_accent: c(0xffffff),
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
            hover: c(0x2a3037),
            accent: c(0x4fb3a0),
            accent_strong: c(0x8fd6c7),
            on_accent: c(0x0d1117),
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
        cx.set_global(if dark { Theme::dark() } else { Theme::light() });
        cx.refresh_windows();
    }
}

impl Global for Theme {}

/// 字体族。正文用苹方（其拉丁字形完整，且各字重齐全，汉字粗体才能生效）；代码用 Menlo，汉字回退到苹方。
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

/// 正文与等宽字体名：设置覆盖优先，其次环境变量 `NOMO_TEXT_FONT`（便于对照渲染），默认苹方与 Menlo。
fn font_names() -> (gpui::SharedString, gpui::SharedString) {
    let (document, mono) = FONT_OVERRIDES.read().map(|names| names.clone()).unwrap_or_default();
    let document = if document.is_empty() { std::env::var("NOMO_TEXT_FONT").unwrap_or_else(|_| "PingFang SC".into()) } else { document };
    let mono = if mono.is_empty() { "Menlo".to_string() } else { mono };
    (document.into(), mono.into())
}

pub fn font_for(family: Family, bold: bool, italic: bool) -> Font {
    let (document, mono) = font_names();
    let (name, fallback) = match family {
        Family::Text => (document, gpui::SharedString::from(".SystemUIFont")),
        Family::Mono => (mono, document),
    };
    Font {
        family: name,
        features: Default::default(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![fallback.to_string(), "PingFang SC".into()])),
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

//! 原生颜色、控件与排版令牌；沿用原版主题定义，切换后使可见区布局失效。

use crate::settings::{Settings, ThemeMode};
use crate::ui::motion::{self, MotionPreference};
use gpui::{App, Font, FontFallbacks, FontStyle, FontWeight, Global, Hsla, WindowAppearance, rgb, rgba};
use std::time::{Duration, Instant};

#[path = "theme_catalog.rs"]
pub(crate) mod catalog;

#[derive(Clone, PartialEq)]
pub struct Theme {
    pub id: gpui::SharedString,
    pub definition_stamp: u64,
    pub mermaid: crate::embeds::MermaidPalette,
    pub document_bg: Hsla,
    pub titlebar_bg: Hsla,
    pub titlebar_fg: Hsla,
    pub titlebar_border: Hsla,
    pub accent_fill: Hsla,
    pub dropdown_bg: Hsla,
    pub dropdown_border: Hsla,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub space_md: f32,
    pub control_height: f32,
    pub raised_shadow: gpui::BoxShadow,
    pub floating_shadow: gpui::BoxShadow,
    pub code_function: Hsla,
    pub code_tag: Hsla,
    pub code_parameter: Hsla,
    pub code_inserted: Hsla,
    pub code_deleted: Hsla,
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
    pub success: Hsla,
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
    pub callout_note_fg: Hsla,
    pub callout_tip_fg: Hsla,
    pub callout_important_fg: Hsla,
    pub callout_warning_fg: Hsla,
    pub callout_caution_fg: Hsla,
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
            success: c(0x166534),
            id: "nomo-default".into(), definition_stamp: 0,
            mermaid: crate::embeds::MermaidPalette { mode: 0, colors: [0; 9] },
            titlebar_bg: c(0xf3f4f6), titlebar_fg: c(0x333333), titlebar_border: c(0xe5e5e5),
            document_bg: c(0xffffff), accent_fill: c(0x2f7d6f),
            dropdown_bg: c(0xffffff), dropdown_border: c(0xd8d8d8),
            radius_sm: 4.0, radius_md: 8.0, radius_lg: 12.0, space_md: 12.0, control_height: 34.0,
            raised_shadow: gpui::BoxShadow { inset: false, color: ca(0x16202c14), offset: gpui::point(gpui::px(0.0), gpui::px(1.0)), blur_radius: gpui::px(4.0), spread_radius: gpui::px(0.0) },
            floating_shadow: gpui::BoxShadow { inset: false, color: ca(0x16202c14), offset: gpui::point(gpui::px(0.0), gpui::px(14.0)), blur_radius: gpui::px(34.0), spread_radius: gpui::px(0.0) },
            code_function: c(0x6f42c1),
            code_tag: c(0x22863a),
            code_parameter: c(0xe36209),
            code_inserted: c(0x22863a),
            code_deleted: c(0xb31d28),
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
            code_fg: c(0x24292e),
            code_keyword: c(0xd73a49),
            code_string: c(0x032f62),
            code_number: c(0x005cc5),
            code_comment: c(0x6a737d),
            code_punct: c(0x24292e),
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
            callout_note_fg: c(0x1e40af),
            callout_tip_fg: c(0x166534),
            callout_important_fg: c(0x6b21a8),
            callout_warning_fg: c(0x92400e),
            callout_caution_fg: c(0x991b1b),
        }
    }

    pub fn dark() -> Self {
        Self {
            success: c(0x86efac),
            id: "nomo-default".into(), definition_stamp: 0,
            mermaid: crate::embeds::MermaidPalette { mode: 1, colors: [0; 9] },
            titlebar_bg: c(0x181d23), titlebar_fg: c(0xcccccc), titlebar_border: c(0x2e2e2e),
            document_bg: c(0x1f2328), accent_fill: c(0x2f7d6f),
            dropdown_bg: c(0x1e1e1e), dropdown_border: c(0x3a3a3a),
            radius_sm: 4.0, radius_md: 8.0, radius_lg: 12.0, space_md: 12.0, control_height: 34.0,
            raised_shadow: gpui::BoxShadow { inset: false, color: ca(0x0000002e), offset: gpui::point(gpui::px(0.0), gpui::px(1.0)), blur_radius: gpui::px(4.0), spread_radius: gpui::px(0.0) },
            floating_shadow: gpui::BoxShadow { inset: false, color: ca(0x00000052), offset: gpui::point(gpui::px(0.0), gpui::px(14.0)), blur_radius: gpui::px(34.0), spread_radius: gpui::px(0.0) },
            code_function: c(0xb392f0),
            code_tag: c(0x85e89d),
            code_parameter: c(0xffab70),
            code_inserted: c(0x85e89d),
            code_deleted: c(0xfdaeb7),
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
            quote_border: c(0x6f938b),
            quote_fg: c(0xc8d8d4),
            code_bg: c(0x262b31),
            code_border: c(0x353b42),
            code_fg: c(0xe1e4e8),
            code_keyword: c(0xf97583),
            code_string: c(0x9ecbff),
            code_number: c(0x79b8ff),
            code_comment: c(0x6a737d),
            code_punct: c(0xe1e4e8),
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
            callout_note: c(0x3b82f6),
            callout_tip: c(0x22c55e),
            callout_important: c(0xa855f7),
            callout_warning: c(0xf59e0b),
            callout_caution: c(0xef4444),
            callout_note_fg: c(0x93c5fd),
            callout_tip_fg: c(0x86efac),
            callout_important_fg: c(0xd8b4fe),
            callout_warning_fg: c(0xfcd34d),
            callout_caution_fg: c(0xfca5a5),
        }
    }

    fn from_definition(definition: &catalog::ColorTheme, dark: bool) -> Self {
        let mut theme = if dark { Self::dark() } else { Self::light() };
        let variant = definition.variant(dark);
        theme.id = definition.id.clone().into();
        use std::hash::{Hash, Hasher};
        let mut stamp = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_vec(variant).expect("主题令牌序列化").hash(&mut stamp);
        theme.definition_stamp = stamp.finish();
        theme.mermaid = crate::embeds::MermaidPalette {
            mode: match variant.mermaid.theme.as_str() { "dark" => 1, "base" => 2, _ => 0 },
            colors: ["documentBackground", "surface", "foreground", "border", "mutedForeground", "rail", "chrome", "accent", "heading"].map(|key| {
                let color = gpui::Rgba::from(variant.color(key));
                let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
                (byte(color.r) << 16) | (byte(color.g) << 8) | byte(color.b)
            }),
        };
        theme.app_bg = variant.color("background");
        theme.rail_bg = variant.color("rail");
        theme.surface = variant.color("surface");
        theme.document_bg = variant.color("documentBackground");
        theme.titlebar_bg = variant.color("titlebarBackground");
        theme.titlebar_fg = variant.color("titlebarForeground");
        theme.titlebar_border = variant.color("titlebarBorder");
        theme.fg = variant.color("foreground");
        theme.heading = variant.color("heading");
        theme.muted = variant.color("mutedForeground");
        theme.border = variant.color("border");
        theme.hover = variant.color("hoverBackground");
        theme.accent = variant.color("accent");
        theme.accent_strong = variant.color("accentStrong");
        theme.accent_fill = variant.color("accentFill");
        theme.on_accent = variant.color("onAccent");
        theme.warning = variant.color("warning");
        theme.success = variant.color("success");
        theme.danger = variant.color("danger");
        theme.sidebar_active = variant.color("sidebarActive");
        theme.selection = variant.color("selection");
        theme.caret = variant.color("accent");
        theme.link = variant.color("link");
        theme.quote_border = variant.color("blockquoteBorder");
        theme.quote_fg = variant.color("blockquoteForeground");
        theme.code_bg = variant.color("codeBackground");
        theme.code_border = variant.color("codeBorder");
        theme.table_border = variant.color("tableBorder");
        theme.table_header_bg = variant.color("tableHeaderBackground");
        theme.scrollbar = variant.color("scrollbarThumbIdle");
        theme.dropdown_bg = variant.color("dropdownBackground");
        theme.dropdown_border = variant.color("dropdownBorder");
        theme.callout_note = variant.color("calloutNoteBorder");
        theme.callout_note_fg = variant.color("calloutNoteForeground");
        theme.callout_tip = variant.color("calloutTipBorder");
        theme.callout_tip_fg = variant.color("calloutTipForeground");
        theme.callout_important = variant.color("calloutImportantBorder");
        theme.callout_important_fg = variant.color("calloutImportantForeground");
        theme.callout_warning = variant.color("calloutWarningBorder");
        theme.callout_warning_fg = variant.color("calloutWarningForeground");
        theme.callout_caution = variant.color("calloutCautionBorder");
        theme.callout_caution_fg = variant.color("calloutCautionForeground");
        theme.workspace_edge = theme.border.opacity(if dark { 0.9 } else { 0.76 });
        theme.inline_code_bg = motion::color(theme.document_bg, theme.accent, if dark { 0.12 } else { 0.08 });
        theme.inline_code_fg = theme.accent_strong;
        theme.marker = theme.muted.opacity(0.6);
        theme.radius_sm = variant.length("radiusSm");
        theme.radius_md = variant.length("radiusMd");
        theme.radius_lg = variant.length("radiusLg");
        theme.space_md = variant.length("spaceMd");
        theme.control_height = variant.length("controlHeightMd");
        theme.raised_shadow = variant.shadow("shadowRaised");
        theme.floating_shadow = variant.shadow("shadowFloating");
        // 语法角色沿用既有高亮器；按主题切换其调色板，不重复解析代码。
        let palette = match variant.shiki_theme.as_str() {
            "github-dark" => [0xe1e4e8, 0xf97583, 0x9ecbff, 0x79b8ff, 0x6a737d, 0xe1e4e8, 0xb392f0, 0x85e89d, 0xffab70, 0x85e89d, 0xfdaeb7],
            "gruvbox-light-medium" => [0x3c3836, 0x9d0006, 0x79740e, 0x8f3f71, 0x928374, 0x665c54, 0x427b58, 0x076678, 0x076678, 0x79740e, 0x9d0006],
            "gruvbox-dark-medium" => [0xebdbb2, 0xfb4934, 0xb8bb26, 0xd3869b, 0x928374, 0xa89984, 0x8ec07c, 0x83a598, 0x83a598, 0xb8bb26, 0xfb4934],
            _ => [0x24292e, 0xd73a49, 0x032f62, 0x005cc5, 0x6a737d, 0x24292e, 0x6f42c1, 0x22863a, 0xe36209, 0x22863a, 0xb31d28],
        }.map(c);
        [theme.code_fg, theme.code_keyword, theme.code_string, theme.code_number, theme.code_comment, theme.code_punct,
            theme.code_function, theme.code_tag, theme.code_parameter, theme.code_inserted, theme.code_deleted] = palette;
        theme
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
        let definition = cx.try_global::<catalog::ThemeCatalog>().map(|catalog| catalog.selected(cx.global::<Settings>()));
        let target = if let Some(definition) = definition { Self::from_definition(definition, dark) }
            else if dark { Self::dark() } else { Self::light() };
        if cx.try_global::<Theme>() == Some(&target) { return; }
        if let Some(definition) = definition { set_theme_fonts(definition.variant(dark), cx); }
        // 同明暗下的配色/字体切换也必须重排有界可见区，不能只比较 dark。
        crate::layout::bump_generation();
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
        if transition.target != *target
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
        interpolate!(background; titlebar_bg, titlebar_border, document_bg, accent_fill, dropdown_bg, dropdown_border, app_bg, rail_bg, surface, border, workspace_edge, hover, sidebar_active, selection,
            code_bg, code_border, inline_code_bg, table_border, table_header_bg, search_match,
            search_current, mark_bg, scrollbar, workspace_shadow_near, workspace_shadow_soft);
        interpolate!(foreground; titlebar_fg, fg, heading, muted, accent, accent_strong, on_accent, warning, success, danger, caret, link,
            quote_border, quote_fg, code_fg, code_keyword, code_string, code_number, code_comment,
            code_punct, code_function, code_tag, code_parameter, code_inserted, code_deleted, inline_code_fg, marker, callout_note, callout_tip, callout_important,
            callout_warning, callout_caution, callout_note_fg, callout_tip_fg,
            callout_important_fg, callout_warning_fg, callout_caution_fg);
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
    /// 代码卡片沿用旧版等宽字体优先级，仍尊重用户设置。
    Code,
}

/// 字体覆盖（来自设置“字体”项）：空串表示使用默认字体。由 `Settings::derive` 在设置变化时更新。
static FONT_OVERRIDES: std::sync::RwLock<(String, String)> = std::sync::RwLock::new((String::new(), String::new()));
static THEME_FONTS: std::sync::RwLock<[Vec<String>; 3]> = std::sync::RwLock::new([Vec::new(), Vec::new(), Vec::new()]);
static CODE_FONT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// 只在首次显示代码卡片时查一次字体清单，避免缺失字体被 GPUI 回退为比例字体。
pub fn prepare_code_font(window: &gpui::Window) {
    CODE_FONT.get_or_init(|| {
        let installed = window.text_system().all_font_names();
        ["Cascadia Code", "JetBrains Mono", "Consolas", default_fonts().1].into_iter()
            .find(|candidate| installed.iter().any(|name| name.eq_ignore_ascii_case(candidate)))
            .unwrap_or(default_fonts().1)
    });
}

fn set_theme_fonts(variant: &catalog::Variant, cx: &App) {
    // 只在主题实际切换时查询已安装字体；输入和滚动只读取短字体栈。
    let installed = cx.text_system().all_font_names();
    let stacks = ["fontUi", "fontDocument", "fontMono"].map(|key| {
        catalog::font_stack(&variant.style_tokens[key]).unwrap_or_default().into_iter()
            .filter(|font| installed.iter().any(|name| name.eq_ignore_ascii_case(font))).collect()
    });
    if let Ok(mut fonts) = THEME_FONTS.write() { *fonts = stacks; }
}

pub fn set_font_overrides(document: &str, mono: &str) {
    if let Ok(mut overrides) = FONT_OVERRIDES.write() {
        *overrides = (document.trim().to_string(), mono.trim().to_string());
    }
}

/// 当前平台的界面字体；正文设置覆盖不影响界面默认字体。
pub fn ui_font_family() -> String {
    THEME_FONTS.read().ok().and_then(|fonts| fonts[0].first().cloned()).unwrap_or_else(|| {
        if cfg!(target_os = "macos") { ".SystemUIFont" }
        else if cfg!(target_os = "windows") { "Segoe UI" } else { "DejaVu Sans" }.into()
    })
}

fn default_fonts() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") { ("PingFang SC", "Menlo") }
    else if cfg!(target_os = "windows") { ("Microsoft YaHei", "Consolas") }
    else { ("Noto Sans CJK SC", "DejaVu Sans Mono") }
}

fn font_names() -> (gpui::SharedString, gpui::SharedString) {
    let (document, mono) = FONT_OVERRIDES.read().map(|names| names.clone()).unwrap_or_default();
    let (default_document, default_mono) = default_fonts();
    let themed = THEME_FONTS.read().map(|fonts| (fonts[1].first().cloned(), fonts[2].first().cloned())).unwrap_or_default();
    let document = if document.is_empty() { std::env::var("NOMO_TEXT_FONT").unwrap_or_else(|_| themed.0.unwrap_or_else(|| default_document.into())) } else { document };
    let mono = if mono.is_empty() { themed.1.unwrap_or_else(|| default_mono.to_string()) } else { mono };
    (document.into(), mono.into())
}

pub fn font_for(family: Family, bold: bool, italic: bool) -> Font {
    let (document, mono) = font_names();
    if family == Family::Code {
        let custom = FONT_OVERRIDES.read().is_ok_and(|names| !names.1.is_empty()) || THEME_FONTS.read().is_ok_and(|fonts| !fonts[2].is_empty());
        return Font {
            family: if custom { mono } else { CODE_FONT.get().copied().unwrap_or(default_fonts().1).into() },
            features: Default::default(),
            fallbacks: Some(FontFallbacks::from_fonts(vec!["JetBrains Mono".into(), "Consolas".into(), default_fonts().1.into(), document.to_string()])),
            weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        };
    }
    let (name, fallback) = match family {
        Family::Text => (document, gpui::SharedString::from(ui_font_family())),
        Family::Mono => (mono, document),
        Family::Code => unreachable!(),
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

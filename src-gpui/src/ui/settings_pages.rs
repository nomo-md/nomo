//! 设置分区的专属呈现。预览只使用静态色块，不创建编辑器或后台任务。
use super::*;
use crate::ui::settings_schema::{ABOUT_ITEMS, SHORTCUTS};
use gpui::{Div, Hsla, SharedString};

/// 未接通项缺少旧配置时只显示参考版默认值；这些值不进入 Settings 或配置文件。
pub(super) fn display_value(settings: &Settings, key: &str) -> Value {
    let value = if let Some((root, field)) = key.split_once('.') {
        settings.value(root).get(field).cloned().unwrap_or(Value::Null)
    } else { settings.value(key) };
    if !value.is_null() { return value; }
    match key {
        "documentStyleId" => json!("nomo-modern"),
        "largeDocumentLimit" => json!(500_000),
        "renderMode" => json!("hardware"),
        "imageHandlingSettings.imageInsertStrategy" => json!("copy-assets"),
        "imageHandlingSettings.autoDeleteUnusedLocalImages" => json!(false),
        "imageHandlingSettings.uploadProvider" => json!("picgo"),
        "imageHandlingSettings.picgoServerUrl" => json!("http://127.0.0.1:36677/upload"),
        "imageHandlingSettings.picgoCoreCommand" => json!("picgo"),
        "imageHandlingSettings.picgoCoreConfigPath" | "imageHandlingSettings.defaultImageWidth" => json!(""),
        "imageHandlingSettings.defaultImageAlign" => json!("none"),
        _ => Value::Null,
    }
}

pub(super) fn item_is_visible(item: &Item, settings: &Settings) -> bool {
    let upload = display_value(settings, "imageHandlingSettings.imageInsertStrategy") == json!("upload");
    let core = display_value(settings, "imageHandlingSettings.uploadProvider") == json!("picgo-core");
    match item.key {
        "imageHandlingSettings.uploadProvider" | "imageConnectionTest" => upload,
        "imageHandlingSettings.picgoServerUrl" => upload && !core,
        "imageHandlingSettings.picgoCoreCommand" | "imageHandlingSettings.picgoCoreConfigPath" => upload && core,
        _ => true,
    }
}

pub(super) fn section_heading(title: &'static str, badge: &'static str, theme: &Theme) -> Div {
    div().pb(px(12.0)).flex().items_center().gap(px(8.0)).text_size(px(14.0))
        .font_weight(FontWeight(750.0)).text_color(theme.heading).child(title)
        .when(!badge.is_empty(), |el| el.child(div().px(px(6.0)).py(px(1.0)).rounded_full()
            .border_1().border_color(theme.accent.opacity(0.42)).text_color(theme.accent)
            .text_size(px(10.0)).child(badge)))
}

pub(super) fn readonly_input(value: impl Into<SharedString>, width: f32, theme: &Theme) -> Div {
    div().w(px(width)).h(px(34.0)).px(px(10.0)).flex().items_center().gap(px(8.0))
        .rounded(px(4.0)).border_1().border_color(theme.border).bg(theme.surface)
        .text_size(px(14.0)).text_color(theme.fg).opacity(0.55)
        .cursor(gpui::CursorStyle::OperationNotAllowed)
        .child(div().min_w(px(0.0)).flex_1().overflow_hidden().whitespace_nowrap().child(value.into()))
}

pub(super) fn unavailable_action(key: &str, label: &'static str, theme: &Theme) -> Div {
    div().flex().items_center().gap(px(10.0))
        .when(key != "imageConnectionTest", |el| el.child(div().min_w(px(62.0)).px(px(9.0)).h(px(26.0))
            .flex().items_center().justify_center().rounded_full().border_1().border_color(theme.border)
            .bg(theme.surface.opacity(0.78)).text_size(px(13.0)).font_weight(FontWeight(750.0))
            .text_color(theme.muted).child("不支持")))
        .child(button(eid("unavailable", key), label, None, ButtonKind::Disabled, theme)
            .min_w(px(116.0)).h(px(34.0)).justify_center().rounded(px(4.0))
            .bg(theme.accent).text_color(theme.on_accent).font_weight(FontWeight::BOLD))
}

struct ThemePreview {
    id: &'static str,
    label: &'static str,
    light: [u32; 4],
    dark: [u32; 4],
    radius: f32,
    spacing: f32,
    light_shadow: u32,
    shadow_blur: f32,
}

// 来自固定参考版 themeRegistry.ts；只保存背景、表面、强调色与前景四个预览令牌。
const THEMES: &[ThemePreview] = &[
    ThemePreview { id: "nomo-default", label: "Nomo 默认", light: [0xf6f7f9, 0xffffff, 0x2f7d6f, 0x202428], dark: [0x15181d, 0x1d2229, 0x67b7a5, 0xeef2f5], radius: 8.0, spacing: 12.0, light_shadow: 0x16202c14, shadow_blur: 4.0 },
    ThemePreview { id: "nomo-amber-paper", label: "琥珀纸页", light: [0xf3f0e8, 0xfbf8f1, 0x9a4d10, 0x2b2925], dark: [0x191713, 0x23201b, 0xe3a04c, 0xf2ebdd], radius: 10.0, spacing: 14.0, light_shadow: 0x4a3a2714, shadow_blur: 3.0 },
    ThemePreview { id: "nomo-classic-gray", label: "经典灰", light: [0xf0f2f4, 0xfafbfc, 0x58758e, 0x2b3137], dark: [0x181c20, 0x20262c, 0x7fa0ba, 0xe7ecf0], radius: 6.0, spacing: 12.0, light_shadow: 0x1f23280f, shadow_blur: 0.0 },
    ThemePreview { id: "nomo-github", label: "GitHub", light: [0xf6f8fa, 0xffffff, 0x0969da, 0x1f2328], dark: [0x0d1117, 0x161b22, 0x58a6ff, 0xc9d1d9], radius: 6.0, spacing: 12.0, light_shadow: 0x1f23280f, shadow_blur: 0.0 },
];

fn preview_variant(colors: [u32; 4], radius: f32, spacing: f32, light_shadow: u32, shadow_blur: f32) -> Div {
    let [background, surface, accent, foreground] = colors.map(|value| Hsla::from(gpui::rgb(value)));
    div().relative().flex_1().h_full().bg(background)
        .child(div().absolute().top(px(spacing)).bottom(px(spacing)).left(px(spacing)).right(px(spacing))
            .border_1().border_color(foreground.opacity(0.15)).rounded(px(radius)).bg(surface)
            .shadow(vec![gpui::BoxShadow { inset: false,
                color: gpui::rgba(if background.l < 0.5 { 0x0000002e } else { light_shadow }).into(),
                offset: point(px(0.0), px(1.0)), blur_radius: px(shadow_blur), spread_radius: px(0.0),
            }]))
        .child(div().absolute().left(px(spacing + 6.0)).right(px(spacing + 6.0)).bottom(px(33.0))
            .h(px(3.0)).rounded_full().bg(foreground))
        .child(div().absolute().left(px(spacing + 6.0)).right(px(spacing + 6.0)).bottom(px(20.0))
            .h(px(5.0)).rounded_full().bg(accent))
}

impl SettingsWindow {
    pub(super) fn render_theme_picker(&self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &Context<Self>) -> AnyElement {
        let columns = if f32::from(viewport.width) <= 520.0 { 1 } else { 2 };
        div().flex_none().py(px(12.0)).border_t_1().border_color(theme.border.opacity(0.72)).flex().flex_col().gap(px(12.0))
            .child(div().text_size(px(14.0)).font_weight(FontWeight(650.0)).text_color(theme.heading).child("配色主题"))
            .child(div().flex().flex_col().gap(px(12.0)).children(THEMES.chunks(columns).map(|row| {
                div().flex().gap(px(12.0)).children(row.iter().map(|preview| {
                    // 原生版实际只应用默认主题，不能把保留的旧偏好冒充已应用主题。
                    let active = preview.id == "nomo-default";
                    div().id(eid("theme-card", preview.id)).min_w(px(0.0)).flex_1().flex().flex_col()
                        .overflow_hidden().rounded(px(8.0)).border_1().border_color(if active { theme.accent } else { theme.border })
                        .bg(theme.surface).text_color(theme.fg)
                        .when(active, |el| el.shadow(vec![gpui::BoxShadow { inset: false, color: theme.accent,
                            offset: point(px(0.0), px(0.0)), blur_radius: px(0.0), spread_radius: px(1.0) }])
                            .cursor_pointer().hover(|style| style.border_color(theme.accent))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| Settings::set(cx, "colorThemeId", json!("nomo-default")))))
                        .when(!active, |el| el.cursor(gpui::CursorStyle::OperationNotAllowed)
                            .tooltip(tooltip("此主题尚未接入原生版，当前只展示原版预览。")))
                        .child(div().h(px(74.0)).flex().border_b_1().border_color(theme.border)
                            .child(preview_variant(preview.light, preview.radius, preview.spacing, preview.light_shadow, preview.shadow_blur))
                            .child(preview_variant(preview.dark, preview.radius, preview.spacing, preview.light_shadow, preview.shadow_blur)))
                        .child(div().min_h(px(42.0)).px(px(12.0)).flex().items_center().justify_between().gap(px(10.0))
                            .text_size(px(13.0)).font_weight(FontWeight(650.0)).child(preview.label)
                            .when(active, |el| el.child(div().size(px(18.0)).rounded_full().bg(theme.accent)
                                .flex().items_center().justify_center().child(icon(IconName::Check, 11.0, theme.on_accent)))))
                }))
            })))
            .child(div().flex().flex_wrap().gap(px(8.0))
                .child(button("import-theme", "导入主题…", None, ButtonKind::Disabled, theme)
                    .h(px(28.0)).rounded(px(4.0)).font_weight(FontWeight::NORMAL)
                    .tooltip(tooltip("原生版尚未接入主题文件导入。")))
                .child(button("export-theme", "导出当前主题为模板…", None, ButtonKind::Disabled, theme)
                    .h(px(28.0)).rounded(px(4.0)).font_weight(FontWeight::NORMAL)
                    .tooltip(tooltip("原生版尚未接入主题模板导出。"))))
            .into_any_element()
    }

    pub(super) fn render_font_control(&mut self, item: &'static Item, theme: &Theme, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let field = self.field(item, "跟随主题", cx);
        let populated = !field.read(cx).content.is_empty();
        let key = item.key;
        div().relative().w(px(width)).flex().child(field)
            .when(populated, |el| el.child(div().id(eid("font-reset", key)).absolute().top(px(7.0)).right(px(5.0))
                .size(px(20.0)).rounded(px(2.0)).flex().items_center().justify_center().cursor_pointer()
                .hover(|style| style.bg(theme.hover)).tooltip(tooltip("恢复主题字体"))
                .child(icon(IconName::Close, 14.0, theme.muted))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(field) = this.fields.get(key) {
                        field.update(cx, |field, cx| field.set_text("", cx));
                        window.focus(&field.focus_handle(cx), cx);
                    }
                }))))
            .into_any_element()
    }

    pub(super) fn render_shortcuts(&self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>) -> AnyElement {
        let compact = f32::from(viewport.width) <= COMPACT_WIDTH;
        div().id("settings-shortcuts").flex_none().flex().flex_col()
            .tooltip(tooltip(if cfg!(windows) { "当前构建尚不支持自定义快捷键；Markdown Mini 使用 Ctrl+Alt+M。" } else { "当前构建尚不支持自定义快捷键；Markdown Mini 的原生窗口尚未接入当前平台。" }))
            .children(SHORTCUTS.iter().map(|(label, keys)| {
                let value = if cfg!(target_os = "macos") { format!("⌘{}", keys.replace("Shift+", "⇧")) }
                    else { format!("Ctrl+{keys}") };
                div().min_h(px(54.0)).py(px(10.0)).border_t_1().border_color(theme.border.opacity(0.72))
                    .flex().justify_between()
                    .when(compact, |el| el.flex_col().items_start().gap(px(10.0)))
                    .when(!compact, |el| el.items_center().gap(px(24.0)))
                    .child(div().min_w(px(0.0)).when(!compact, |el| el.flex_1()).flex().flex_col().gap(px(4.0))
                        .child(div().text_size(px(14.0)).font_weight(FontWeight(650.0)).text_color(theme.heading).child(*label))
                        .child(div().text_size(px(13.0)).text_color(theme.muted).child("使用 Ctrl、Shift、Alt 与一个按键组合。")))
                    .child(div().w(px(control_width(viewport))).flex_none().flex().justify_end().child(readonly_input(value, 180.0, theme)))
            })).into_any_element()
    }

    pub(super) fn render_about(&mut self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items = vec![div().w_full().max_w(px(640.0)).flex().flex_col()
            .child(div().mb(px(14.0)).flex().items_center().gap(px(14.0))
                .child(gpui::img("app/128x128.png").size(px(54.0)).flex_none())
                .child(div().flex().items_center().gap(px(9.0))
                    .child(div().text_size(px(18.0)).font_weight(FontWeight::BOLD).text_color(theme.heading).child("Nomo"))
                    .child(div().min_h(px(20.0)).px(px(7.0)).py(px(1.0)).flex().items_center().rounded_full().border_1().border_color(theme.accent.opacity(0.34))
                        .bg(crate::ui::motion::color(theme.surface, theme.accent, 0.07)).text_color(theme.accent)
                        .text_size(px(11.0)).font_weight(FontWeight::BOLD).child(format!("v{}", env!("CARGO_PKG_VERSION"))))))
            .child(about_detail("定位", "本地优先、Markdown-first 的轻量桌面 Markdown 编辑器。", theme))
            .child(about_detail("平台支持", "支持 macOS 与 Windows，并持续完善各平台的原生文件、窗口和快捷键体验。", theme))
            .child(div().mt(px(16.0)).mb(px(8.0)).flex().gap(px(10.0))
                .when(f32::from(viewport.width) <= 520.0, |el| el.flex_col())
                .child(project_link("repository", "GitHub 仓库", "查看源代码、版本发布和项目动态。", IconName::Github, "https://github.com/nomo-md/nomo", theme))
                .child(project_link("issue", "提交 Issue", "报告问题或提出功能建议。", IconName::CircleDot, "https://github.com/nomo-md/nomo/issues/new/choose", theme)))
            .into_any_element()];
        items.extend(ABOUT_ITEMS.iter().map(|item| self.render_item(item, theme, viewport, cx)));
        vec![div().w_full().max_w(px(640.0)).flex().flex_col().children(items).into_any_element()]
    }
}

fn about_detail(label: &'static str, description: &'static str, theme: &Theme) -> Div {
    div().py(px(13.0)).border_t_1().border_color(theme.border.opacity(0.72)).flex().gap(px(12.0))
        .child(div().w(px(68.0)).flex_none().text_size(px(13.0)).font_weight(FontWeight(650.0)).text_color(theme.muted).child(label))
        .child(div().min_w(px(0.0)).flex_1().text_size(px(14.0)).line_height(gpui::relative(1.55)).text_color(theme.fg).child(description))
}

fn project_link(id: &'static str, title: &'static str, description: &'static str, glyph: IconName, url: &'static str, theme: &Theme) -> gpui::Stateful<Div> {
    div().id(eid("project", id)).min_w(px(0.0)).flex_1().min_h(px(76.0)).p(px(12.0)).flex().items_center().gap(px(10.0))
        .rounded(px(8.0)).border_1().border_color(theme.border).bg(crate::ui::motion::color(theme.app_bg, theme.surface, 0.88))
        .cursor_pointer().hover(|style| style.border_color(theme.accent).bg(theme.hover))
        .child(div().size(px(36.0)).flex_none().rounded_full().border_1().border_color(theme.border.opacity(0.72))
            .bg(theme.app_bg).flex().items_center().justify_center().child(icon(glyph, 24.0, if id == "issue" { theme.accent } else { theme.fg })))
        .child(div().min_w(px(0.0)).flex_1().flex().flex_col().gap(px(3.0))
            .child(div().text_size(px(14.0)).font_weight(FontWeight::BOLD).text_color(theme.fg).child(title))
            .child(div().text_size(px(13.0)).line_height(gpui::relative(1.4)).text_color(theme.muted).child(description)))
        .child(div().h_full().flex_none().child(icon(IconName::ExternalLink, 16.0, theme.muted)))
        .on_click(move |_: &ClickEvent, _, cx| cx.open_url(url))
}

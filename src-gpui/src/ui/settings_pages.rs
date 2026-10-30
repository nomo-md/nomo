//! 设置分区的专属呈现。预览只使用静态色块，不创建编辑器或后台任务。
use super::*;
use crate::ui::settings_schema::{ABOUT_ITEMS};
use gpui::{Div, SharedString};

/// 未接通项缺少旧配置时只显示参考版默认值；这些值不进入 Settings 或配置文件。
pub(super) fn display_value(settings: &Settings, key: &str) -> Value {
    let value = if let Some((root, field)) = key.split_once('.') {
        settings.value(root).get(field).cloned().unwrap_or(Value::Null)
    } else { settings.value(key) };
    if !value.is_null() { return value; }
    match key {
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

fn preview_variant(variant: &crate::theme::catalog::Variant) -> Div {
    let [background, surface, accent, foreground] = ["background", "surface", "accent", "foreground"].map(|key| variant.color(key));
    let radius = variant.length("radiusMd").min(20.0);
    let spacing = variant.length("spaceMd").clamp(4.0, 22.0);
    div().relative().flex_1().h_full().bg(background)
        .child(div().absolute().top(px(spacing)).bottom(px(spacing)).left(px(spacing)).right(px(spacing))
            .border_1().border_color(foreground.opacity(0.15)).rounded(px(radius)).bg(surface)
            .shadow(vec![variant.shadow("shadowRaised")]))
        .child(div().absolute().left(px(spacing + 6.0)).right(px(spacing + 6.0)).bottom(px(33.0))
            .h(px(3.0)).rounded_full().bg(foreground))
        .child(div().absolute().left(px(spacing + 6.0)).right(px(spacing + 6.0)).bottom(px(20.0))
            .h(px(5.0)).rounded_full().bg(accent))
}

impl SettingsWindow {
    pub(super) fn render_theme_picker(&self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &Context<Self>) -> AnyElement {
        use crate::theme::catalog::{self, ThemeCatalog};
        let catalog = cx.global::<ThemeCatalog>();
        let settings = cx.global::<Settings>();
        let selected = catalog.selected(settings).id.clone();
        let language = crate::ui::toolbar_i18n::ToolbarLocale::current(cx).code();
        let columns = if content_width(viewport) <= 380.0 { 1 } else { 2 };
        let kind = if catalog.busy { ButtonKind::Disabled } else { ButtonKind::Secondary };
        div().flex_none().py(px(12.0)).border_t_1().border_color(theme.border.opacity(0.72)).flex().flex_col().gap(px(12.0))
            .child(div().text_size(px(14.0)).font_weight(FontWeight(650.0)).text_color(theme.heading).child("配色主题"))
            .child(div().flex().flex_col().gap(px(12.0)).children(catalog.themes.chunks(columns).map(|row| {
                div().flex().gap(px(12.0)).children(row.iter().map(|preview| {
                    let active = preview.id == selected;
                    let id = preview.id.clone();
                    div().id(eid("theme-card", &id)).min_w(px(0.0)).flex_1().flex().flex_col()
                        .overflow_hidden().rounded(px(theme.radius_md)).border_1().border_color(if active { theme.accent } else { theme.border })
                        .bg(theme.surface).text_color(theme.fg).cursor_pointer().hover(|style| style.border_color(theme.accent))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Settings::set(cx, "colorThemeId", json!(id))))
                        .child(div().h(px(74.0)).flex().border_b_1().border_color(theme.border)
                            .child(preview_variant(preview.variant(false))).child(preview_variant(preview.variant(true))))
                        .child(div().min_h(px(42.0)).px(px(12.0)).flex().items_center().justify_between().gap(px(10.0))
                            .text_size(px(13.0)).font_weight(FontWeight(650.0)).child(preview.name(language).to_string())
                            .when(active, |el| el.child(div().size(px(18.0)).rounded_full().bg(theme.accent_fill)
                                .flex().items_center().justify_center().child(icon(IconName::Check, 11.0, theme.on_accent)))))
                }))
            })))
            .child(div().flex().flex_wrap().gap(px(8.0))
                .child(button("import-theme", "导入主题…", None, kind, theme)
                    .when(!catalog.busy, |el| el.on_click(cx.listener(|_, _, _, cx| catalog::import(cx)))))
                .child(button("export-theme", "导出当前主题为模板…", None, kind, theme)
                    .when(!catalog.busy, |el| el.on_click(cx.listener(|_, _, _, cx| catalog::export(cx))))))
            .children(catalog.busy.then(|| div().text_size(px(12.0)).text_color(theme.muted).child("正在处理主题…")))
            .children(catalog.message.clone().map(|message| div().text_size(px(12.0)).text_color(theme.muted).child(message)))
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

//! 左侧栏的大纲页：只消费文档结构索引的有限分页，不复制正文。
use super::*;
use crate::ui::toolbar_i18n::ToolbarLocale;

const SIDEBAR_OUTLINE_ROW_HEIGHT: f32 = 30.0;

impl Workspace {
    /// 侧栏大纲是否需要结构索引；浮动大纲关闭后仍由这里保持自己的查询生命周期。
    pub(super) fn sidebar_outline_requested(&self, cx: &App) -> bool {
        !self.mini.active()
            && cx.global::<Settings>().show_sidebar
            && self.explorer.sidebar_tab == super::explorer::SidebarTab::Outline
            && self.tab().is_some()
    }

    pub(super) fn set_sidebar_tab(
        &mut self,
        tab: super::explorer::SidebarTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.explorer.sidebar_tab == tab { return; }
        self.explorer.sidebar_tab = tab;
        match tab {
            super::explorer::SidebarTab::Files => self.resume_tree(cx),
            super::explorer::SidebarTab::Outline => {
                self.clear_tree_search(cx);
                self.suspend_tree(cx);
                self.explorer.hovered = None;
                if let Some(document) = self.document() {
                    window.focus(&document.focus_handle(cx), cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn render_sidebar_outline(
        &mut self,
        width: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(document) = self.document() else {
            if self.sidebar_outline_document.take().is_some() {
                self.sidebar_outline_scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            }
            return sidebar_outline_message(
                width, theme, ToolbarLocale::current(cx).app_text("sidebar.noDocument"), false, cx,
            );
        };
        let document_id = document.read(cx).status().document_id.clone();
        if self.sidebar_outline_document.as_deref() != Some(document_id.as_str()) {
            self.sidebar_outline_document = Some(document_id);
            self.sidebar_outline_scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        let settings = cx.global::<Settings>().clone();
        document.update(cx, |host, cx| host.set_outline_enabled(true, settings.outline_level, cx));
        let snapshot = document.read(cx).outline_snapshot();
        let scroll = self.sidebar_outline_scroll.0.borrow().base_handle.clone();
        let top = scroll.logical_scroll_top().0;
        document.update(cx, |host, cx| host.request_outline_page(top, cx));
        let locale = ToolbarLocale::current(cx);
        let count = snapshot.count + usize::from(!snapshot.complete && snapshot.count > 0);
        if count == 0 {
            let can_retry = snapshot.error.is_some();
            let text = if can_retry {
                locale.app_text("outline.unavailable")
            } else if snapshot.pending && !snapshot.complete {
                locale.app_text("outline.loading")
            } else if snapshot.incomplete {
                locale.app_text("outline.incomplete")
            } else {
                locale.app_text("outline.empty")
            };
            return sidebar_outline_message(width, theme, text, can_retry, cx);
        }
        let active = snapshot.active_byte;
        let revision = snapshot.revision;
        let page = snapshot.clone();
        let list_theme = theme.clone();
        let list = uniform_list("sidebar-outline-list", count, {
            let theme = list_theme.clone();
            cx.processor(move |_workspace, range: std::ops::Range<usize>, _, cx| {
                range.map(|index| {
                    let Some(heading) = page.row(index).cloned() else {
                        return div().w_full().h(px(SIDEBAR_OUTLINE_ROW_HEIGHT)).pl(px(14.0)).flex().items_center()
                            .text_size(px(12.0)).text_color(theme.muted)
                            .child(ToolbarLocale::current(cx).app_text("outline.loadingRow")).into_any_element();
                    };
                    let selected = active == Some(heading.byte);
                    let fold_heading = heading.clone();
                    let goto_heading = heading.clone();
                    let chevron = if heading.has_children {
                        div().id(("sidebar-outline-fold", index)).size(px(16.0)).flex_none()
                            .flex().items_center().justify_center().rounded(px(3.0)).cursor_pointer()
                            .hover(|style| style.bg(theme.hover))
                            .child(icon(IconName::ChevronDown, 12.0, theme.muted)
                                .with_transformation(Transformation::rotate(radians(
                                    if heading.collapsed { -std::f32::consts::FRAC_PI_2 } else { 0.0 },
                                ))))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |workspace, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(document) = workspace.document() {
                                    document.update(cx, |host, cx| host.set_outline_fold(&fold_heading, cx));
                                }
                            }))
                            .into_any_element()
                    } else {
                        div().w(px(16.0)).flex_none().into_any_element()
                    };
                    let title: String = if heading.text.is_empty() {
                        ToolbarLocale::current(cx).app_text("outline.emptyHeading").into()
                    } else {
                        heading.text.clone()
                    };
                    let hint = ToolbarLocale::current(cx).app_format(
                        "outline.headingLine",
                        &[("title", &title), ("line", &heading.line.to_string())],
                    );
                    div().id(("sidebar-outline-row", index)).w_full().h(px(SIDEBAR_OUTLINE_ROW_HEIGHT))
                        .pr(px(8.0)).flex().items_center().gap(px(5.0))
                        .pl(px(6.0 + (heading.level.saturating_sub(1)) as f32 * 14.0))
                        .rounded(px(4.0)).cursor_pointer().text_size(px(13.0))
                        .text_color(if selected { theme.accent_strong } else { theme.fg })
                        .when(selected, |el| el.bg(theme.sidebar_active).font_weight(FontWeight::SEMIBOLD))
                        .hover(|style| style.bg(theme.hover))
                        .child(chevron)
                        .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().truncate().child(title))
                        .tooltip(tooltip(hint))
                        .on_click(cx.listener(move |workspace, _, _, cx| {
                            if let Some(document) = workspace.document() {
                                document.update(cx, |host, cx| host.navigate_outline_entry(&goto_heading, revision, cx));
                            }
                        }))
                        .into_any_element()
                }).collect()
            })
        }).track_scroll(&self.sidebar_outline_scroll).pt(px(4.0)).pb(px(9.0)).size_full();
        let partial = snapshot.incomplete.then(|| div().px(px(14.0)).pb(px(4.0)).text_size(px(11.0)).text_color(theme.muted)
            .child(locale.app_text("outline.partial")));
        div().id("sidebar-outline").w(px(width)).h_full().min_h(px(0.0)).flex_none().flex().flex_col()
            .children(partial).child(div().flex_1().min_h(px(0.0)).child(list)).into_any_element()
    }
}

fn sidebar_outline_message(
    width: f32,
    theme: &Theme,
    text: &'static str,
    can_retry: bool,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let mut message = div().id("sidebar-outline-message").w(px(width)).h_full().min_h(px(0.0)).flex_none()
        .flex().flex_col().items_start().gap(px(6.0)).px(px(14.0)).pt(px(16.0))
        .text_size(px(12.0)).text_color(theme.muted).child(text);
    if can_retry {
        message = message.child(div().id("sidebar-outline-retry").cursor_pointer().text_color(theme.accent_strong)
            .child(ToolbarLocale::current(cx).app_text("outline.retry"))
            .on_click(cx.listener(|workspace, _, _, cx| {
                if let Some(document) = workspace.document() {
                    document.update(cx, |host, cx| host.retry_outline(cx));
                }
                cx.notify();
            })));
    }
    message.into_any_element()
}

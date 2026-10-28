//! 旧版快捷插入面板。会话与正文属于编辑器，面板只绘制并发送选择动作。
use crate::nomo_mark::quick_insert::{CATEGORIES, QuickInsertSession};
use crate::theme::{Theme, Family, font_for, ui_font_family};
use super::{popup::{popup_with_list, PopupAnchor, PopupPlacement, reduced_motion}, toolbar_i18n::ToolbarLocale};
use gpui::{AnyElement, App, Bounds, BoxShadow, IntoElement, MouseButton, Pixels, Window, div, point, px, prelude::*};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub(crate) enum PanelAction { Choose(usize), Step(usize, bool), Dismiss }

pub(crate) struct QuickInsertPanel {
    pub session: QuickInsertSession,
    pub anchor: Bounds<Pixels>,
    pub on_action: Rc<dyn Fn(PanelAction, &mut Window, &mut App)>,
}
impl QuickInsertPanel {
    pub fn render(self, cx: &App) -> AnyElement {
        let theme=Theme::ui(cx);
        let locale=ToolbarLocale::current(cx);
        let choose=self.on_action.clone();
        let outside=self.on_action.clone();
        let mut list=div().id("quick-insert-list").min_h_0().max_h(px(252.0)).flex_shrink(1.0).overflow_y_scroll().track_scroll(&self.session.scroll);
        for (index,category) in CATEGORIES.iter().enumerate() {
            let active=index==self.session.category;
            let group:gpui::SharedString=format!("quick-insert-row-{index}").into();
            let selected=category.variants[self.session.variants[index]].0;
            let selected=if selected.starts_with("quick_") || selected.contains('_') {locale.text(selected)}else{selected};
            let callback=choose.clone();
            let row_color=if active {theme.accent_strong}else{theme.fg};
            let mut row=div().id(("quick-insert-row",index)).group(group.clone()).flex().items_center().min_h(px(36.0)).pr(px(4.0)).rounded(px(4.0))
                .text_color(row_color).when(active,|row|row.bg(theme.surface.blend(theme.accent.opacity(0.10))))
                .hover(|style| if active {style}else{style.bg(theme.rail_bg)} )
                .child(div().id(("quick-insert-option",index)).flex_1().min_w_0().flex().items_center().gap(px(10.0)).min_h(px(36.0)).py(px(7.0)).px(px(8.0)).cursor_pointer()
                    .child(gpui::svg().path(format!("icons/quick-{}.svg",category.icon)).size(px(16.0)).flex_none().text_color(if active {row_color}else{theme.muted}))
                    .child(div().min_w_0().truncate().child(locale.text(category.key)))
                    .on_click(move|_,window,cx|callback(PanelAction::Choose(index),window,cx)));
            if category.variants.len()>1 {
                for forward in [false,true] {
                    let callback=choose.clone();
                    row=row.child(div().id((if forward {"quick-next"}else{"quick-previous"},index)).w(px(24.0)).h(px(28.0)).flex_none().flex().items_center().justify_center().rounded(px(2.0)).cursor_pointer()
                        .opacity(if active {1.0}else{0.0}).group_hover(group.clone(),|style|style.opacity(1.0))
                        .hover(|style|style.bg(theme.accent.opacity(0.14)))
                        .child(gpui::svg().path(if forward {"icons/quick-next.svg"}else{"icons/quick-previous.svg"}).size(px(13.0)))
                        .on_click(move|_,window,cx|{cx.stop_propagation();callback(PanelAction::Step(index,forward),window,cx);}));
                    if !forward { row=row.child(div().min_w(px(48.0)).max_w(px(90.0)).truncate().text_center().font_family(font_for(Family::Mono,false,false).family)
                        .text_size(px(13.0)).text_color(if active {row_color}else{theme.muted}).child(selected)); }
                }
            }
            list=list.child(row);
        }
        let panel=div().id("quick-insert-panel").w(px(286.0)).flex().flex_col().p(px(6.0)).border_1().border_color(theme.border).rounded(px(12.0))
            .bg(theme.surface).text_color(theme.fg).font_family(ui_font_family()).text_size(px(14.0)).line_height(gpui::relative(1.4))
            .shadow(vec![BoxShadow{color:theme.fg.opacity(0.12),offset:point(px(0.0),px(8.0)),blur_radius:px(28.0),spread_radius:px(0.0),inset:false}])
            .on_mouse_down(MouseButton::Left,|_,_,cx|cx.stop_propagation())
            .on_mouse_down_out(move|_,window,cx|outside(PanelAction::Dismiss,window,cx))
            .child(div().flex_none().flex().items_center().justify_between().pt(px(5.0)).px(px(8.0)).pb(px(8.0)).text_size(px(13.0)).text_color(theme.muted)
                .child(locale.text("quick_title"))
                .child(div().min_w(px(20.0)).px(px(5.0)).border_1().border_color(theme.border).rounded(px(2.0)).text_center().font_family(font_for(Family::Mono,false,false).family).child(self.session.trigger.clone())))
            .child(list)
            .child(div().flex_none().mt(px(5.0)).pt(px(8.0)).px(px(7.0)).pb(px(3.0)).border_t_1().border_color(theme.border).text_size(px(12.0)).text_color(theme.muted)
                .child(self.session.error.clone().unwrap_or_else(||locale.text("quick_keys").into())));
        gpui::deferred(popup_with_list("quick-insert-motion",PopupAnchor::Bounds(self.anchor),PopupPlacement::BelowLeft,panel,reduced_motion(cx))).with_priority(2).into_any_element()
    }
}

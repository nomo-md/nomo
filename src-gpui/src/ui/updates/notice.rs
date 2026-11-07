//! 旧版更新角落通知：仅显示期间持有发布元数据、焦点句柄和可取消的 12 秒计时。
use super::*;
use crate::{theme::Theme,ui::{components::{icon,tooltip},motion,popup::reduced_motion}};
use gpui::{Animation,AnimationExt,Context,FocusHandle,KeyDownEvent,Render,prelude::*,div,px,relative};

pub(super) struct UpdateNotice {
    release:Option<Release>,
    summary:String,
    controls:[FocusHandle;3],
    focus:FocusHandle,
    return_focus:Option<FocusHandle>,
    hovered:bool,
    focused:bool,
    timer:Option<Task<()>>,
}
impl UpdateNotice {
    pub(super) fn new(release:Release,cx:&mut Context<Self>)->Self {
        let locale=ToolbarLocale::current(cx);
        let item=strip_packages(release.body.as_deref().unwrap_or("")).lines().map(str::trim).find(|line| {
            let mut chars=line.chars();matches!(chars.next(),Some('-'|'*'|'+'))&&chars.next().is_some_and(char::is_whitespace)&&!chars.as_str().trim().is_empty()
        }).unwrap_or("");
        let plain=item.trim_start_matches(['-','*','+']).chars().filter(|value|!"`*_~[]#".contains(*value)).collect::<String>();
        let mut summary=plain.split_whitespace().collect::<Vec<_>>().join(" ");
        if summary.is_empty(){summary=locale.text("softwareUpdateNoticeSummary").into();}
        if summary.chars().count()>86{summary=summary.chars().take(85).collect::<String>().trim_end().to_owned()+"…";}
        let mut notice=Self{release:Some(release),summary,controls:std::array::from_fn(|_|cx.focus_handle()),focus:cx.focus_handle(),return_focus:None,hovered:false,focused:false,timer:None};
        notice.schedule(cx);notice
    }
    fn schedule(&mut self,cx:&mut Context<Self>) {
        self.timer=None;
        if self.hovered||self.focused{return;}
        self.timer=Some(cx.spawn(async move |this,cx| {
            cx.background_executor().timer(Duration::from_secs(12)).await;
            let _=this.update(cx,|notice,cx| {
                if notice.hovered||notice.focused{return;}
                if cx.try_global::<Updates>().and_then(|state|state.notice.as_ref()).is_some_and(|(_,view)|*view==cx.entity()) {
                    cx.global_mut::<Updates>().notice=None;cx.refresh_windows();
                }
            });
        }));
    }
    fn activate(&mut self,index:usize,window:&mut Window,cx:&mut Context<Self>) {
        if window.has_active_prompt(){return;}
        let Some(release)=self.release.take()else{return;};
        self.timer=None;
        if let Some(focus)=self.return_focus.take(){window.focus(&focus,cx);}
        cx.global_mut::<Updates>().notice=None;
        if index==0{Settings::set(cx,"softwareUpdateDismissedVersion",serde_json::json!(release.tag_name.trim_start_matches('v')));}
        if index==2{show_release(release,false,window,cx);}
        cx.refresh_windows();
    }
    fn key(&mut self,index:usize,event:&KeyDownEvent,window:&mut Window,cx:&mut Context<Self>) {
        match event.keystroke.key.as_str(){
            "enter"|"space"=>self.activate(index,window,cx),
            "escape"=>self.activate(1,window,cx),
            "tab"=>{
                let next=if event.keystroke.modifiers.shift{index.checked_sub(1)}else{(index<2).then_some(index+1)};
                if let Some(next)=next{window.focus(&self.controls[next],cx);}else if let Some(focus)=&self.return_focus{window.focus(focus,cx);}else{window.focus_next(cx);}
                cx.notify();window.refresh();
            },
            _=>return,
        }
        window.prevent_default();cx.stop_propagation();
    }
    fn button(&self,index:usize,label:&'static str,theme:&Theme,window:&Window,cx:&Context<Self>)->gpui::Stateful<gpui::Div> {
        let primary=index==2;
        div().id(("update-notice-action",index)).relative().track_focus(&self.controls[index]).tab_index(0).h(px(32.0)).px(px(13.0)).rounded(px(7.0)).flex().items_center().justify_center().text_size(px(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).cursor_pointer().border_1()
            .border_color(if primary{theme.accent}else{gpui::transparent_black()}).bg(if primary{theme.accent}else{gpui::transparent_black()}).text_color(if primary{gpui::white()}else{theme.muted})
            .hover(|style|style.bg(if primary{motion::color(theme.accent,gpui::black(),0.14)}else{theme.rail_bg}))
            .when(self.controls[index].is_focused(window),|el|el.child(div().absolute().top(px(-4.0)).bottom(px(-4.0)).left(px(-4.0)).right(px(-4.0)).rounded(px(9.0)).border_2().border_color(theme.accent)))
            .on_mouse_down(gpui::MouseButton::Left,cx.listener(move |notice,_,window,cx|window.focus(&notice.controls[index],cx)))
            .on_click(cx.listener(move |notice,_,window,cx|{notice.activate(index,window,cx);cx.stop_propagation();}))
            .on_key_down(cx.listener(move |notice,event,window,cx|notice.key(index,event,window,cx))).child(label)
    }
}
impl Render for UpdateNotice {
    fn render(&mut self,window:&mut Window,cx:&mut Context<Self>)->impl IntoElement {
        let focused=self.focus.is_focused(window)||self.controls.iter().any(|focus|focus.is_focused(window));
        if focused!=self.focused{self.focused=focused;self.schedule(cx);}
        if !focused{self.return_focus=window.focused(cx);}
        let Some(release)=&self.release else{return div().into_any_element();};
        let theme=Theme::ui(cx);let locale=ToolbarLocale::current(cx);
        let small=window.viewport_size().width<=px(520.0);let right=if small{14.0}else{22.0};let bottom=if small{38.0}else{44.0};
        let close=div().id("update-notice-close").relative().track_focus(&self.controls[0]).tab_index(0).size(px(26.0)).rounded(px(6.0)).flex_none().flex().items_center().justify_center().cursor_pointer().hover(|style|style.bg(theme.rail_bg))
            .when(self.controls[0].is_focused(window),|el|el.child(div().absolute().top(px(-4.0)).bottom(px(-4.0)).left(px(-4.0)).right(px(-4.0)).rounded(px(8.0)).border_2().border_color(theme.accent)))
            .tooltip(tooltip(locale.text("softwareUpdateDismissVersion")))
            .on_mouse_down(gpui::MouseButton::Left,cx.listener(|notice,_,window,cx|window.focus(&notice.controls[0],cx)))
            .on_click(cx.listener(|notice,_,window,cx|{notice.activate(0,window,cx);cx.stop_propagation();}))
            .on_key_down(cx.listener(|notice,event,window,cx|notice.key(0,event,window,cx)))
            .child(icon(IconName::Close,15.0,theme.muted));
        let notice=div().id("software-update-notice").track_focus(&self.focus).tab_index(0).tab_stop(false).absolute().right(px(right)).bottom(px(bottom)).w(px(390.0)).max_w((window.viewport_size().width-px(28.0)).max(px(0.0))).rounded(px(12.0)).border_1().border_color(motion::color(theme.border,theme.accent,0.28)).bg(theme.surface).occlude().shadow_lg()
            .on_mouse_down(gpui::MouseButton::Left,cx.listener(|notice,_,window,cx|{if !notice.controls.iter().any(|focus|focus.is_focused(window)){if !notice.focus.is_focused(window){notice.return_focus=window.focused(cx);}window.focus(&notice.focus,cx);}cx.stop_propagation();}))
            .on_key_down(cx.listener(|notice,event:&KeyDownEvent,window,cx|{match event.keystroke.key.as_str(){"tab"=>{window.focus(&notice.controls[if event.keystroke.modifiers.shift{2}else{0}],cx);cx.notify();window.refresh();},"escape"=>notice.activate(1,window,cx),_=>return}window.prevent_default();cx.stop_propagation();}))
            .on_hover(cx.listener(|notice,hovered,_,cx|{notice.hovered=*hovered;notice.schedule(cx);}))
            .child(div().h(px(3.0)).rounded_t(px(12.0)).bg(theme.accent))
            .child(div().flex().items_start().gap(px(12.0)).px(px(16.0)).pt(px(16.0)).pb(px(13.0))
                .child(div().size(px(42.0)).flex_none().flex().items_center().justify_center().rounded(px(10.0)).bg(theme.sidebar_active).child(icon(IconName::Download,21.0,theme.accent_strong)))
                .child(div().min_w_0().flex_1()
                    .child(div().flex().items_center().gap(px(8.0)).mb(px(4.0)).text_size(px(11.0)).font_weight(gpui::FontWeight::BOLD).text_color(theme.muted).child(locale.text("updateCheck"))
                        .child(div().px(px(6.0)).py(px(2.0)).rounded(px(999.0)).bg(theme.sidebar_active).text_color(theme.accent_strong).text_size(px(10.0)).font_family(crate::theme::font_for(crate::theme::Family::Mono,false,false).family).child(release.tag_name.clone())))
                    .child(div().text_size(px(15.0)).font_weight(gpui::FontWeight::BOLD).text_color(theme.heading).child(locale.text("softwareUpdateNoticeTitle")))
                    .child(div().mt(px(6.0)).max_h(px(37.2)).overflow_hidden().text_size(px(12.0)).line_height(relative(1.55)).text_color(theme.muted).child(self.summary.clone())))
                .child(close))
            .child(div().flex().justify_end().gap(px(8.0)).px(px(16.0)).pb(px(14.0))
                .child(self.button(1,locale.text("softwareUpdateRemindLater"),&theme,window,cx))
                .child(self.button(2,locale.text("softwareUpdateViewDetails"),&theme,window,cx)));
        if reduced_motion(cx){notice.into_any_element()}else{notice.with_animation("software-update-notice-enter",Animation::new(Duration::from_millis(220)).with_easing(motion::ease),move |el,delta|el.opacity(delta).bottom(px(bottom-12.0*(1.0-delta)))).into_any_element()}
    }
}

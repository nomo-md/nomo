//! 旧版保存与删除确认框；只持有提示文字，通过 GPUI PromptHandle 返回选择并恢复焦点。
use super::{components::{IconName, icon}, motion, popup::reduced_motion, toolbar_i18n::ToolbarLocale};
use crate::{settings::Settings, theme::Theme};
use gpui::{
    App, AppContext, BoxShadow, Context, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, KeyDownEvent, MouseButton, PromptButton, PromptLevel, PromptResponse, Render,
    SharedString, Task, Window, div, point, prelude::*, px, relative, rgb,
};
use std::{future::Future, path::Path, time::{Duration, Instant}};
mod secondary;
pub(crate) use secondary::{SecondaryDialog, SecondaryKind, SecondaryAction, Tone, choose};

#[derive(Clone, Copy)]
pub(crate) enum SaveChoice { Save, Discard, Cancel }

enum DialogKind { Unsaved, Delete(SharedString), Message, Secondary(SecondaryDialog) }

#[derive(Default)]
struct NextPrompt(Option<DialogKind>);
impl Global for NextPrompt {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(NextPrompt::default());
    cx.set_prompt_builder(|level, title, message, actions, handle, window, cx| {
        // 元数据只在同步构造提示框时存在，不按文案或按钮数量猜测业务类型。
        let kind = cx.global_mut::<NextPrompt>().0.take().unwrap_or(DialogKind::Message);
        let view = cx.new(|cx| Confirmation::new(kind, level, title, message, actions, cx));
        handle.with_view(view, window, cx)
    });
}

pub(crate) fn confirm_save(title: &str, window: &mut Window, cx: &mut App)
    -> impl Future<Output = SaveChoice> + use<>
{
    let response = if window.has_active_prompt() { None } else {
        let locale = ToolbarLocale::current(cx);
        cx.global_mut::<NextPrompt>().0 = Some(DialogKind::Unsaved);
        Some(window.prompt(PromptLevel::Warning, title, Some(locale.text("dialog_unsaved_message")), &[
            PromptButton::ok(locale.text("dialog_save")),
            PromptButton::new(locale.text("dialog_discard")),
            PromptButton::cancel(locale.text("cancel")),
        ], cx))
    };
    async move {
        match response { Some(response) => match response.await {
            Ok(0) => SaveChoice::Save, Ok(1) => SaveChoice::Discard, _ => SaveChoice::Cancel,
        }, None => SaveChoice::Cancel }
    }
}

pub(crate) fn confirm_delete(path: &Path, window: &mut Window, cx: &mut App)
    -> impl Future<Output = bool> + use<>
{
    let response = if window.has_active_prompt() { None } else {
        let locale = ToolbarLocale::current(cx);
        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        let message = locale.text("dialog_delete_message").replace("{name}", &name);
        cx.global_mut::<NextPrompt>().0 = Some(DialogKind::Delete(path.display().to_string().into()));
        Some(window.prompt(PromptLevel::Warning, locale.text("dialog_delete_title"), Some(&message), &[
            PromptButton::ok(locale.text("dialog_delete")), PromptButton::cancel(locale.text("cancel")),
        ], cx))
    };
    async move { match response { Some(response) => response.await.ok() == Some(0), None => false } }
}

struct Confirmation {
    kind: DialogKind,
    level: PromptLevel,
    title: SharedString,
    message: Option<SharedString>,
    actions: Vec<PromptButton>,
    focus: FocusHandle,
    keyboard_choice: Option<usize>,
    hover: Vec<motion::ScalarMotion>,
    opened: Instant,
    closing: Option<Instant>,
    completion: Option<Task<()>>,
    remember: bool,
    release_notes: Vec<secondary::NoteBlock>,
    release_scroll: gpui::ScrollHandle,
}

impl Confirmation {
    fn new(kind: DialogKind, level: PromptLevel, title: &str, message: Option<&str>, actions: &[PromptButton], cx: &mut Context<Self>) -> Self {
        let release_notes=match &kind {DialogKind::Secondary(dialog)=>dialog.notes.as_deref().map(secondary::parse_notes).unwrap_or_default(),_=>Vec::new()};
        let keyboard_choice=matches!(&kind,DialogKind::Secondary(dialog) if matches!(dialog.kind,SecondaryKind::Update|SecondaryKind::UpdateSuccess)).then_some(actions.len());
        Self { kind, level, title: title.to_owned().into(), message: message.map(|text| text.to_owned().into()),
            actions: actions.to_vec(), focus: cx.focus_handle(), keyboard_choice,
            hover: vec![motion::ScalarMotion::default(); actions.len()],
            opened: Instant::now(), closing: None, completion: None, remember: false, release_notes, release_scroll: gpui::ScrollHandle::new() }
    }

    fn cancel_index(&self) -> Option<usize> {
        if let DialogKind::Secondary(dialog)=&self.kind {return Some(dialog.cancel);}
        self.actions.iter().position(PromptButton::is_cancel).or_else(|| self.actions.len().checked_sub(1))
    }

    fn respond(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.closing.is_some() || (index >= self.actions.len() && self.cancel_index()!=Some(index)) { return; }
        self.closing = Some(Instant::now());
        let index = index | if self.remember { secondary::REMEMBER } else { 0 };
        let duration = Duration::from_millis(if reduced_motion(cx) { 90 } else { 120 });
        self.completion = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            let _ = this.update(cx, |_, cx| cx.emit(PromptResponse(index)));
        }));
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.is_held && self.closing.is_none() {
            match event.keystroke.key.as_str() {
                "escape" => { if let Some(index) = self.cancel_index() { self.respond(index, cx); } }
                "enter" | "space" => {
                    let index = self.keyboard_choice.unwrap_or(0);
                    if self.activate_secondary_control(index,window,cx) {}
                    else { self.respond(index, cx); }
                }
                "tab" | "left" | "right" | "up" | "down" => {
                    let mut order: Vec<_> = (0..self.actions.len()).collect();
                    if let DialogKind::Secondary(dialog)=&self.kind {order=self.secondary_controls(dialog);}
                    if matches!(self.kind, DialogKind::Delete(_)) { order.reverse(); }
                    if !order.is_empty() {
                        let backwards = event.keystroke.modifiers.shift || matches!(event.keystroke.key.as_str(), "left" | "up");
                        let current = self.keyboard_choice.and_then(|choice| order.iter().position(|&index| index == choice));
                        let next = match current {
                            Some(current) if backwards => (current + order.len() - 1) % order.len(),
                            Some(current) => (current + 1) % order.len(),
                            None if backwards => order.len() - 1,
                            None => 0,
                        };
                        self.keyboard_choice = Some(order[next]);
                        self.reveal_secondary_focus();
                        cx.notify();
                        // PromptHandle 中的独立提示视图需同时使窗口失效，空闲时 Tab 也要立即显示焦点。
                        window.refresh();
                    }
                }
                _ => {}
            }
        }
        // 提示框独占键盘，快捷键和普通字符都不能继续送往背后的编辑器。
        window.prevent_default();
        cx.stop_propagation();
    }

    fn action(&self, index: usize, vertical: bool, theme: &Theme, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        if let DialogKind::Secondary(dialog) = &self.kind { return self.secondary_action(dialog, index, theme, cx); }
        let cancel = self.actions[index].is_cancel();
        let unsaved = matches!(self.kind, DialogKind::Unsaved);
        let delete = matches!(self.kind, DialogKind::Delete(_));
        let destructive = !cancel && ((unsaved && index == 1) || delete || matches!(self.level, PromptLevel::Critical));
        let save = unsaved && index == 0;
        let color = if destructive { if delete || matches!(self.level, PromptLevel::Critical) { theme.danger } else { theme.warning } } else if save { theme.accent_strong } else { theme.fg };
        let foreground = if destructive { rgb(0xffffff).into() } else { color };
        let background = if destructive { color } else if save { motion::color(theme.surface, theme.accent, 0.1) }
            else if cancel { gpui::transparent_black() } else { theme.surface };
        let border = if destructive { color } else if save { motion::color(theme.border, theme.accent, 0.42) } else { theme.border };
        let hover_background = if save { motion::color(theme.surface, theme.accent, 0.16) } else { motion::color(background, theme.fg, 0.06) };
        let hover_border = if save { theme.accent } else { motion::color(theme.border, theme.fg, 0.2) };
        let hover = self.hover[index].value();
        let leading = if cancel { IconName::Close } else if save { IconName::Save }
            else if delete { IconName::Trash2 } else if destructive { IconName::AlertTriangle } else { IconName::Check };
        let mut button = div().id(("confirmation-action", index)).relative().h(px(if unsaved { 36.0 } else { 32.0 }))
            .min_w(px(0.0)).px(px(if unsaved { 12.0 } else { 14.0 })).flex().items_center().justify_center()
            .gap(px(if unsaved { 7.0 } else { 6.0 })).rounded(px(4.0)).border_1().border_color(border)
            .bg(if destructive { background } else { motion::color(background, hover_background, hover) })
            .border_color(if destructive { border } else { motion::color(border, hover_border, hover) })
            .opacity(if destructive { 1.0 - 0.12 * hover } else { 1.0 })
            .text_color(foreground).text_size(px(12.5)).cursor_pointer()
            .when(unsaved && !vertical, |el| el.flex_1()).when(vertical, |el| el.w_full())
            .on_hover(cx.listener(move |dialog, hovered, window, cx| {
                if dialog.closing.is_some() { return; }
                dialog.hover[index].set(*hovered, reduced_motion(cx), Duration::from_millis(if unsaved { 140 } else { 150 }));
                cx.notify();
                window.refresh();
            }))
            .child(icon(leading, if unsaved { 15.0 } else { 14.0 }, foreground))
            .child(div().overflow_hidden().whitespace_nowrap().child(self.actions[index].label().clone()))
            .on_click(cx.listener(move |dialog, _, _, cx| { dialog.respond(index, cx); cx.stop_propagation(); }));
        if self.keyboard_choice == Some(index) {
            // 焦点外框不能用实心阴影，透明的取消按钮会透出阴影色。
            button = button.child(div().absolute().top(px(-4.0)).bottom(px(-4.0)).left(px(-4.0)).right(px(-4.0))
                .rounded(px(6.0)).border_2().border_color(theme.accent));
        }
        button
    }
}

impl EventEmitter<PromptResponse> for Confirmation {}
impl Focusable for Confirmation {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl Render for Confirmation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.is_focused(window) { window.focus(&self.focus, cx); }
        let theme = Theme::ui(cx);
        let reduced = reduced_motion(cx);
        let unsaved = matches!(self.kind, DialogKind::Unsaved);
        let delete = matches!(self.kind, DialogKind::Delete(_));
        let viewport = window.viewport_size();
        let inset = window.client_inset().unwrap_or(px(0.0));
        let available_width = (viewport.width - inset * 2.0 - px(32.0)).max(px(0.0));
        let vertical = unsaved && viewport.width <= px(460.0);
        let duration = if reduced { 90.0 } else { 160.0 };
        let (panel_alpha, backdrop_alpha, animating) = if let Some(closing) = self.closing {
            let progress = (closing.elapsed().as_secs_f32() * 1000.0 / if reduced { 90.0 } else { 120.0 }).min(1.0);
            let alpha = 1.0 - motion::ease(progress);
            (alpha, alpha, progress < 1.0)
        } else {
            let elapsed = self.opened.elapsed().as_secs_f32() * 1000.0;
            (motion::ease((elapsed / duration).min(1.0)), motion::ease((elapsed / if reduced { 90.0 } else { 120.0 }).min(1.0)), elapsed < duration)
        };
        if animating || self.hover.iter().any(motion::ScalarMotion::running) || Theme::ui_transition_active(cx) { window.request_animation_frame(); }
        let ui_font = cx.global::<Settings>().string("uiFontFamily");
        let ui_font = if ui_font.trim().is_empty() { crate::theme::ui_font_family().to_owned() } else { ui_font };
        let accent = if delete || matches!(self.level, PromptLevel::Critical) { theme.danger }
            else if matches!(self.level, PromptLevel::Info) { theme.accent } else { theme.warning };
        let badge_size = if unsaved { 34.0 } else { 32.0 };
        let badge = div().size(px(badge_size)).flex_none().flex().items_center().justify_center().rounded(px(4.0))
            .bg(accent.opacity(if unsaved { 0.12 } else { 0.1 }))
            .child(icon(if matches!(self.level, PromptLevel::Info) { IconName::Info } else { IconName::AlertTriangle },
                if unsaved { 18.0 } else { 20.0 }, accent));
        let title = div().text_color(theme.heading).text_size(px(if unsaved { 15.0 } else { 14.5 }))
            .font_weight(if unsaved { gpui::FontWeight::BOLD } else { gpui::FontWeight::SEMIBOLD })
            .line_height(relative(1.35)).when(unsaved, |el| el.mt(px(1.0))).child(self.title.clone());
        let description = self.message.clone().map(|message| div().text_size(px(if unsaved { 12.5 } else { 13.0 }))
            .text_color(if unsaved { theme.muted } else { theme.fg }).line_height(relative(if unsaved { 1.5 } else { 1.6 })).child(message));
        let mut order: Vec<_> = (0..self.actions.len()).collect();
        if delete { order.reverse(); }
        let buttons = order.into_iter().map(|index| self.action(index, vertical, &theme, cx)).collect::<Vec<_>>();
        let mut panel = div().id("confirmation-panel").w(px(if unsaved { 392.0 } else { 380.0 })).max_w(available_width)
            .max_h((viewport.height - inset * 2.0 - px(32.0)).max(px(0.0))).overflow_y_scroll()
            .flex().flex_col().rounded(px(8.0)).border_1().border_color(theme.border).bg(theme.app_bg)
            .text_color(theme.fg).shadow(vec![BoxShadow { color: gpui::black().opacity(if theme.dark { 0.42 } else { 0.22 }),
                offset: point(px(0.0), px(20.0)), blur_radius: px(60.0), spread_radius: px(0.0), inset: false }])
            .opacity(panel_alpha).on_click(|_, _, cx| cx.stop_propagation());
        if let DialogKind::Secondary(dialog) = &self.kind {
            panel = self.secondary_panel(dialog, &theme, viewport, cx).max_w(available_width)
                .max_h((viewport.height - inset * 2.0 - px(32.0)).max(px(0.0))).opacity(panel_alpha);
        } else if unsaved {
            panel = panel.p(px(18.0))
                .child(div().flex().flex_none().gap(px(12.0)).items_start().child(badge)
                    .child(div().flex_1().min_w(px(0.0)).child(title).children(description.map(|description| description.mt(px(4.0))))))
                .child(div().mt(px(18.0)).flex().flex_none().gap(px(10.0)).when(vertical, |el| el.flex_col()).children(buttons));
        } else {
            panel = panel.child(div().flex().flex_none().items_center().gap(px(10.0)).px(px(20.0)).pt(px(16.0)).pb(px(12.0))
                    .border_b_1().border_color(if delete { motion::color(theme.border, theme.danger, 0.25) } else { theme.border })
                    .child(badge).child(div().min_w(px(0.0)).flex_1().child(title)))
                .child(div().flex_none().px(px(20.0)).py(px(16.0)).children(description)
                    .when_some(match &self.kind { DialogKind::Delete(path) => Some(path.clone()), _ => None }, |el, path| {
                        el.child(div().mt(px(8.0)).px(px(10.0)).py(px(8.0)).rounded(px(4.0)).bg(theme.surface)
                            .border_1().border_color(theme.border).text_color(theme.muted).text_size(px(12.0))
                            .font_family(crate::theme::font_for(crate::theme::Family::Mono, false, false).family).child(path))
                    }))
                .child(div().flex_none().px(px(20.0)).pt(px(12.0)).pb(px(16.0)).border_t_1().border_color(theme.border)
                    .flex().justify_end().gap(px(10.0)).flex_wrap().children(buttons));
        }
        let cancel = self.cancel_index();
        let backdrop = match &self.kind { DialogKind::Secondary(dialog) => dialog.backdrop(), _ if delete => 0.4, _ => 0.34 };
        let shade:gpui::Hsla=if matches!(&self.kind,DialogKind::Secondary(dialog) if matches!(dialog.kind,SecondaryKind::Update|SecondaryKind::UpdateSuccess)){rgb(0x111827).into()}else{gpui::black()};
        div().id("confirmation-backdrop").key_context("ConfirmationDialog").track_focus(&self.focus)
            .size_full().absolute().top_0().left_0().p(inset + px(16.0)).flex().items_center().justify_center()
            .font_family(ui_font).bg(shade.opacity(backdrop * backdrop_alpha)).occlude()
            .capture_key_down(cx.listener(Self::on_key))
            .capture_action(|_: &super::workspace::Quit, _, cx| cx.stop_propagation())
            .capture_action(|_: &super::workspace::OpenSettings, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |dialog, _, _, cx| { if let Some(index) = cancel { dialog.respond(index, cx); } cx.stop_propagation(); }))
            .child(panel)
    }
}

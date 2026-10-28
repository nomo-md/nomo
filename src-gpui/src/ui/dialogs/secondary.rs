//! 第二阶段应用弹窗：共享模态、焦点和离场生命周期，布局按旧版组件分别绘制。
use super::*;
use std::sync::Arc;

pub(super) const REMEMBER: usize = 1 << 16;
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SecondaryKind { External, Folder, Close, Recovery, Update, UpdateSuccess }
#[derive(Clone, Copy)]
pub(crate) enum Tone { Normal, Primary, Accent, TextAccent, Quiet, Warning, Danger, DangerSoft }
pub(crate) struct SecondaryAction { pub label: String, pub icon: IconName, pub tone: Tone }
impl SecondaryAction {
    pub(crate) fn new(label: impl Into<String>, icon: IconName, tone: Tone) -> Self { Self { label: label.into(), icon, tone } }
}
pub(crate) struct SecondaryDialog {
    pub kind: SecondaryKind,
    pub title: String,
    pub message: String,
    pub detail: Option<String>,
    pub notes: Option<String>,
    pub actions: Vec<SecondaryAction>,
    pub cancel: usize,
    pub remember: Option<String>,
    pub leading: Vec<String>,
}
impl SecondaryDialog {
    fn width(&self) -> f32 { match self.kind { SecondaryKind::External => 560.0, SecondaryKind::Folder => 420.0, SecondaryKind::Close | SecondaryKind::Recovery => 392.0, _ => 640.0 } }
    pub(super) fn backdrop(&self) -> f32 { if self.kind == SecondaryKind::Folder { 0.45 } else if matches!(self.kind,SecondaryKind::Update|SecondaryKind::UpdateSuccess) {0.32} else { 0.34 } }
}
#[derive(Clone, Copy)]
pub(crate) struct Selection { pub index: usize, pub remember: bool }

pub(crate) fn choose(dialog: SecondaryDialog, window: &mut Window, cx: &mut App) -> impl Future<Output=Selection> + use<> {
    let cancel = dialog.cancel;
    let response = if window.has_active_prompt() { None } else {
        let actions: Vec<_> = dialog.actions.iter().enumerate().map(|(index, action)| if index == cancel { PromptButton::cancel(action.label.clone()) } else { PromptButton::new(action.label.clone()) }).collect();
        let title = dialog.title.clone();
        cx.global_mut::<NextPrompt>().0 = Some(DialogKind::Secondary(dialog));
        Some(window.prompt(PromptLevel::Info, &title, None, &actions, cx))
    };
    async move { let value = match response { Some(response) => response.await.unwrap_or(cancel), None => cancel }; Selection { index: value & !REMEMBER, remember: value & REMEMBER != 0 } }
}

impl Confirmation {
    pub(super) fn reveal_secondary_focus(&self){
        let Some(link)=self.keyboard_choice.and_then(|index|index.checked_sub(self.actions.len()+1)) else{return;};
        let mut offset=0;for(index,block)in self.release_notes.iter().enumerate(){if link<offset+block.link_count(){self.release_scroll.scroll_to_item(index);return;}offset+=block.link_count();}
    }
    pub(super) fn secondary_controls(&self,dialog:&SecondaryDialog)->Vec<usize> {
        let mut order=Vec::new();
        if matches!(dialog.kind,SecondaryKind::Update|SecondaryKind::UpdateSuccess) {
            order.push(self.actions.len());
            let links=self.release_notes.iter().map(NoteBlock::link_count).sum::<usize>();
            order.extend(self.actions.len()+1..self.actions.len()+1+links);
        }
        order.extend(0..self.actions.len());
        if dialog.remember.is_some(){order.push(self.actions.len());}
        order
    }
    pub(super) fn activate_secondary_control(&mut self,index:usize,window:&mut Window,cx:&mut Context<Self>)->bool {
        let DialogKind::Secondary(dialog)=&self.kind else{return false;};
        if index==self.actions.len() && dialog.remember.is_some(){self.remember=!self.remember;cx.notify();window.refresh();return true;}
        if matches!(dialog.kind,SecondaryKind::Update|SecondaryKind::UpdateSuccess) && index>=self.actions.len(){
            if index==self.actions.len(){let cancel=dialog.cancel;self.respond(cancel,cx);}
            else if let Some(link)=self.release_notes.iter().flat_map(|block|block.spans.iter().filter_map(|span|span.4.as_deref())).nth(index-self.actions.len()-1){cx.open_url(link);}
            return true;
        }
        false
    }
    pub(super) fn secondary_action(&self, dialog: &SecondaryDialog, index: usize, theme: &Theme, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let action = &dialog.actions[index];
        let color = match action.tone { Tone::Warning => theme.warning, Tone::Danger | Tone::DangerSoft => theme.danger, _ => theme.accent };
        let solid = matches!(action.tone, Tone::Primary | Tone::Warning | Tone::Danger);
        let accent = matches!(action.tone, Tone::Accent | Tone::DangerSoft);
        let foreground = if solid { rgb(0xffffff).into() } else if matches!(action.tone,Tone::DangerSoft){theme.danger} else if accent || matches!(action.tone,Tone::TextAccent) { theme.accent_strong } else { theme.fg };
        let background = if solid { color } else if accent { motion::color(theme.surface, color, 0.1) } else if matches!(action.tone,Tone::Quiet){gpui::transparent_black()} else { theme.surface };
        let border=if solid{color}else if accent{motion::color(theme.border,color,0.42)}else{theme.border};
        let hover = self.hover[index].value();
        let update = matches!(dialog.kind, SecondaryKind::Update | SecondaryKind::UpdateSuccess);
        let mut button = div().id(("confirmation-action", index)).relative().min_w_0().h(px(if update { 32.0 } else { 36.0 }))
            .px(px(if update { 13.0 } else { 10.0 })).flex().items_center().justify_center().gap(px(6.0))
            .rounded(px(if update { 7.0 } else { 4.0 })).border_1().border_color(if solid{border}else{motion::color(border,if accent{color}else{motion::color(theme.border,theme.fg,0.18)},hover)})
            .bg(if solid { background } else { motion::color(background, if accent{motion::color(theme.surface,color,0.16)}else{theme.hover}, hover) })
            .text_color(foreground).text_size(px(if update {12.0}else if dialog.kind==SecondaryKind::Folder{13.0}else{12.5})).cursor_pointer()
            .opacity(if solid { 1.0 - 0.1 * hover } else { 1.0 })
            .on_hover(cx.listener(move |dialog, hovered, window, cx| { dialog.hover[index].set(*hovered, reduced_motion(cx), Duration::from_millis(140)); cx.notify(); window.refresh(); }))
            .when(dialog.kind!=SecondaryKind::Folder,|el|el.child(icon(action.icon,15.0,foreground))).child(div().overflow_hidden().whitespace_nowrap().child(action.label.clone()))
            .on_click(cx.listener(move |dialog, _, _, cx| { dialog.respond(index, cx); cx.stop_propagation(); }));
        if self.keyboard_choice == Some(index) { button = button.child(div().absolute().top(px(-4.0)).bottom(px(-4.0)).left(px(-4.0)).right(px(-4.0)).rounded(px(6.0)).border_2().border_color(theme.accent)); }
        button
    }

    pub(super) fn secondary_panel(&self, dialog: &SecondaryDialog, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let update = matches!(dialog.kind, SecondaryKind::Update | SecondaryKind::UpdateSuccess);
        let folder = dialog.kind == SecondaryKind::Folder;
        let recovery = dialog.kind == SecondaryKind::Recovery;
        let accent = if dialog.kind == SecondaryKind::External || recovery { theme.warning } else { theme.accent };
        let mut panel = div().id("confirmation-panel").w(px(dialog.width())).flex().flex_col().overflow_y_scroll()
            .rounded(px(if update { 14.0 } else { 8.0 })).border_1().border_color(theme.border).bg(if update { theme.surface } else { theme.app_bg })
            .shadow(vec![BoxShadow { color: gpui::black().opacity(if theme.dark { 0.42 } else { 0.22 }), offset: point(px(0.0), px(20.0)), blur_radius: px(60.0), spread_radius: px(0.0), inset: false }])
            .on_click(|_, _, cx| cx.stop_propagation());
        let title = div().text_size(px(if update { 19.0 } else { 15.0 })).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.heading).child(dialog.title.clone());
        if folder {
            panel = panel.p(px(24.0)).child(title)
                .child(div().mt(px(8.0)).mb(px(20.0)).text_size(px(12.0)).text_color(theme.muted)
                    .font_family(crate::theme::font_for(crate::theme::Family::Mono, false, false).family).child(dialog.message.clone()));
        } else {
            let padding = if update { 22.0 } else { 18.0 };
            let badge=div().size(px(if update {48.0}else{34.0})).flex_none().flex().items_center().justify_center()
                .rounded(px(if update {11.0}else{4.0})).bg(accent.opacity(0.12))
                .when(update,|el|el.child(gpui::img(crate::ui::updates::app_mark(theme.dark)).size(px(48.0))))
                .when(!update,|el|el.child(icon(if dialog.kind==SecondaryKind::External||recovery {IconName::AlertTriangle}else{IconName::Info},18.0,accent)));
            let copy=div().flex_1().min_w_0().child(title).child(div().mt(px(4.0)).text_size(px(if update {11.0}else{12.5})).line_height(relative(1.5)).text_color(theme.muted).child(dialog.message.clone()));
            let header=div().flex().flex_none().items_start().gap(px(if update {14.0}else{12.0})).p(px(padding)).when(update,|el|el.pt(px(20.0)).pb(px(18.0)))
                .when(update,|el|el.border_b_1().border_color(theme.border)).child(badge).child(copy)
                .when(update,|el|el.child(div().id("application-dialog-close").size(px(30.0)).cursor_pointer().flex_none().flex().items_center().justify_center().rounded(px(7.0)).hover(|style|style.bg(theme.hover)).when(self.keyboard_choice==Some(self.actions.len()),|el|el.border_2().border_color(theme.accent)).child(icon(IconName::Close,16.0,theme.muted))
                    .on_click(cx.listener(|dialog,_,_,cx|{if let Some(cancel)=dialog.cancel_index(){dialog.respond(cancel,cx);}cx.stop_propagation();}))));
            panel=panel.child(header);
        }
        if !update {if let Some(detail) = &dialog.detail { panel = panel.child(div().mx(px(18.0)).mb(px(12.0)).p(px(10.0)).rounded(px(4.0)).bg(theme.surface).text_size(px(12.0)).text_color(theme.muted).child(detail.clone())); }}
        if dialog.notes.is_some() {
            // 发布说明有 64 KiB 输入上限；这里不创建文档会话或正文渲染镜像。
            let mut links=0usize;
            let focused=self.keyboard_choice.and_then(|index|index.checked_sub(self.actions.len()+1));
            let paragraphs=self.release_notes.iter().enumerate().map(|(index,block)|{let within=focused.and_then(|index|index.checked_sub(links)).filter(|index|*index<block.link_count());links+=block.link_count();block.render(index,theme,within)}).collect::<Vec<_>>();
            panel = panel.child(div().id("release-notes-scroll").min_h_0().max_h((viewport.height - px(300.0)).max(px(80.0))).overflow_y_scroll().track_scroll(&self.release_scroll).p(px(22.0)).children(paragraphs));
        }
        let columns = if dialog.kind == SecondaryKind::External { if viewport.width <= px(400.0) {1} else if viewport.width <= px(650.0) {2} else {4} }
            else if dialog.kind == SecondaryKind::Close && viewport.width <= px(460.0) {1} else {dialog.actions.len().max(1)};
        let mut actions = div().flex().flex_col().flex_none().gap(px(10.0));
        for (index,path) in dialog.leading.iter().enumerate() {
            panel=panel.child(self.secondary_action(dialog,index,theme,cx).h_auto().min_h(px(36.0)).mb(px(10.0)).py(px(8.0)).flex_col().items_start()
                .child(div().text_size(px(12.0)).text_color(theme.muted).child(path.clone())));
        }
        for indexes in (dialog.leading.len()..dialog.actions.len()).collect::<Vec<_>>().chunks(columns) {
            actions = actions.child(div().flex().gap(px(10.0)).children(indexes.iter().map(|&index| self.secondary_action(dialog,index,theme,cx).flex_1())));
        }
        let mut footer = div().flex_none().when(!folder, |el| el.p(px(if update { 22.0 } else { 18.0 })).pt(px(if update {14.0} else {0.0})))
            .when(update, |el| el.border_t_1().border_color(theme.border).pt(px(14.0))).child(actions);
        if update {footer=div().flex_none().px(px(22.0)).py(px(14.0)).border_t_1().border_color(theme.border).flex().items_center().gap(px(16.0))
            .when(viewport.width<=px(520.0),|el|el.flex_col().items_start())
            .child(div().flex_1().text_size(px(11.0)).line_height(relative(1.5)).text_color(theme.muted).children(dialog.detail.clone()))
            .child(div().flex().gap(px(8.0)).flex_none().children((0..dialog.actions.len()).map(|index|self.secondary_action(dialog,index,theme,cx))));}
        if let Some(label) = &dialog.remember {
            footer = footer.child(div().id("dialog-remember").relative().mt(px(16.0)).flex().items_center().gap(px(8.0)).text_size(px(12.0)).text_color(theme.muted).cursor_pointer()
                .child(div().relative().size(px(16.0)).flex_none().flex().items_center().justify_center().rounded(px(4.0)).border_1().border_color(if self.remember {theme.accent} else {theme.border}).bg(if self.remember {theme.accent} else {theme.app_bg})
                    .when(self.remember, |el| el.child(icon(IconName::Check,12.0,rgb(0xffffff).into())))
                    .when(self.keyboard_choice==Some(self.actions.len()),|el|el.child(div().absolute().top(px(-4.0)).bottom(px(-4.0)).left(px(-4.0)).right(px(-4.0)).rounded(px(6.0)).border_2().border_color(theme.accent))))
                .child(label.clone())
                .on_click(cx.listener(|dialog,_,window,cx| { dialog.remember=!dialog.remember; cx.notify(); window.refresh(); cx.stop_propagation(); })));
        }
        panel.child(footer)
    }
}

pub(super) struct NoteBlock {text:String,heading:bool,code:bool,spans:Vec<(std::ops::Range<usize>,bool,bool,bool,Option<Arc<str>>)>}
impl NoteBlock {
    fn empty()->Self{Self{text:String::new(),heading:false,code:false,spans:Vec::new()}}
    fn link_count(&self)->usize{self.spans.iter().filter(|span|span.4.is_some()).count()}
    fn render(&self,index:usize,theme:&Theme,focused:Option<usize>)->gpui::AnyElement {
        let mut links=Vec::new();let mut ranges=Vec::new();
        let highlights=self.spans.iter().map(|(range,bold,italic,code,link)| {
            let focus=link.is_some()&&focused==Some(links.len());
            if let Some(link)=link{ranges.push(range.clone());links.push(link.clone());}
            (range.clone(),gpui::HighlightStyle {font_weight:bold.then_some(gpui::FontWeight::SEMIBOLD),font_style:italic.then_some(gpui::FontStyle::Italic),background_color:if focus{Some(theme.accent.opacity(0.18))}else{code.then_some(theme.rail_bg)},color:link.as_ref().map(|_|theme.accent),..Default::default()})
        }).collect::<Vec<_>>();
        let text=gpui::InteractiveText::new(("release-note",index),gpui::StyledText::new(self.text.clone()).with_highlights(highlights)).on_click(ranges,move |index,_,cx|{if let Some(link)=links.get(index){cx.open_url(link);}});
        div().mb(px(10.0)).text_size(px(if self.heading{16.0}else{13.0})).line_height(relative(1.65)).text_color(theme.fg)
            .font_weight(if self.heading{gpui::FontWeight::SEMIBOLD}else{gpui::FontWeight::NORMAL})
            .when(self.code,|el|el.p(px(10.0)).rounded(px(4.0)).bg(theme.rail_bg).font_family(crate::theme::font_for(crate::theme::Family::Mono,false,false).family))
            .child(text).into_any_element()
    }
}
pub(super) fn parse_notes(source:&str)->Vec<NoteBlock> {
    use pulldown_cmark::{Event,Tag,TagEnd};
    let mut blocks=Vec::new();let mut block=NoteBlock::empty();let mut bold=0usize;let mut italic=0usize;let mut link=None;
    for event in pulldown_cmark::Parser::new(source).take(8192) {
        match event {
            Event::Start(Tag::Heading{..})=>block.heading=true,
            Event::Start(Tag::CodeBlock(_))=>block.code=true,
            Event::Start(Tag::Item)=>block.text.push_str("• "),
            Event::Start(Tag::Strong)=>bold+=1,
            Event::End(TagEnd::Strong)=>bold=bold.saturating_sub(1),
            Event::Start(Tag::Emphasis)=>italic+=1,
            Event::End(TagEnd::Emphasis)=>italic=italic.saturating_sub(1),
            // 一个链接中的强调片段共享地址，避免长地址按片段重复分配。
            Event::Start(Tag::Link{dest_url,..})=>link=url::Url::parse(&dest_url).ok().filter(|url|url.scheme()=="https").map(|url|Arc::<str>::from(url.as_str())),
            Event::End(TagEnd::Link)=>link=None,
            Event::Code(text)=>{
                let start=block.text.len();block.text.push_str(&text);
                if block.spans.len()<4096{block.spans.push((start..block.text.len(),bold>0,italic>0,true,link.clone()));}
            }
            Event::Text(text)=>{
                let start=block.text.len();block.text.push_str(&text);
                if block.spans.len()<4096{block.spans.push((start..block.text.len(),bold>0,italic>0,false,link.clone()));}
            }
            Event::SoftBreak|Event::HardBreak=>block.text.push('\n'),
            Event::End(TagEnd::Heading(_)|TagEnd::Paragraph|TagEnd::CodeBlock|TagEnd::Item)=>{
                if !block.text.is_empty(){blocks.push(block);block=NoteBlock::empty();}
                if blocks.len()==512{break;}
            }
            _=>{}
        }
    }
    if !block.text.is_empty()&&blocks.len()<512{blocks.push(block);}
    blocks
}

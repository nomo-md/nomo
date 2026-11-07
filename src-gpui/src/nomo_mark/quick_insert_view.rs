//! 快捷插入只在真实输入时创建；光标、正文或视图身份改变即结束本次会话。
use super::*;
use crate::nomo_mark::quick_insert::{self, QuickInsertSession};

impl NomoMark {
    pub(crate) fn dismiss_quick_insert(&mut self, cx:&mut Context<Self>) {
        if self.quick_insert.take().is_some() { cx.emit(ControlEvent::QuickInsertChanged); cx.notify(); }
    }
    pub(crate) fn quick_insert_step(&mut self, category:Option<usize>, forward:bool, cx:&mut Context<Self>) {
        if let Some(session)=&mut self.quick_insert {
            if !session.valid(&self.host.parsed,self.host.selection) {self.dismiss_quick_insert(cx);return;}
            if let Some(category)=category {session.move_variant(category,forward);} else {session.move_category(forward);}
            cx.emit(ControlEvent::QuickInsertChanged);cx.notify();
        }
    }
    pub(crate) fn quick_insert_error(&mut self, message:String, cx:&mut Context<Self>) {
        if let Some(session)=&mut self.quick_insert {session.error=Some(message);cx.emit(ControlEvent::QuickInsertChanged);cx.notify();}
    }
    pub(crate) fn quick_insert_anchor(&self)->Option<Bounds<Pixels>> {
        if !self.quick_insert_layout_ready(){return None;}
        let (_,caret)=self.caret_bounds(self.host.selection.head_byte)?;
        let visible=self.visible_bounds?;
        (caret.bottom()>visible.top() && caret.top()<visible.bottom() && caret.right()>visible.left() && caret.left()<visible.right()).then_some(caret)
    }
    pub(crate) fn quick_insert_layout_ready(&self)->bool {
        self.layout_key.as_ref().is_some_and(|(stamp,..)|stamp.identity==self.host.parsed.identity)
    }
    pub(super) fn quick_insert_input(&self, range:&Range<usize>, text:&str, mark:bool, cx:&App)->bool {
        if mark || self.auxiliary_input || self.effective_mode()!=ViewMode::Semantic || self.host.presentation!=Presentation::Editable {return false;}
        let settings=cx.global::<Settings>();
        let trigger=settings.string("quickInsertTrigger");
        settings.bool("quickInsertEnabled") && text==trigger && quick_insert::valid_trigger(text)
            && (self.host.selection.range().is_empty() || self.host.interaction.marked.as_ref()==Some(range))
            && quick_insert::can_trigger(&self.host.parsed,range.start)
    }
    pub(super) fn open_quick_insert(&mut self,range:Range<usize>,trigger:&str,cx:&mut Context<Self>) {
        if !quick_insert::can_trigger(&self.host.parsed,range.start){return;}
        self.menu_open=false;self.slash_range=None;self.emoji_mode=false;
        let mut session = QuickInsertSession::new(&self.host.parsed,range,trigger.into());
        let diagram = quick_insert::default_diagram_index(&cx.global::<Settings>().string("defaultDiagramType"));
        if let Some(index) = quick_insert::CATEGORIES.iter().position(|category| category.key == "quick_diagram") { session.variants[index] = diagram; }
        self.quick_insert = Some(session);
        cx.emit(ControlEvent::QuickInsertChanged);cx.notify();
    }
    pub(super) fn request_quick_insert(&mut self,cx:&mut Context<Self>)->bool {
        if let Some(session)=&self.quick_insert {
            if session.valid(&self.host.parsed,self.host.selection) {
                cx.emit(ControlEvent::QuickInsertChoose(session.action()));
                return true;
            }
            self.dismiss_quick_insert(cx);
        }
        false
    }
    pub(crate) fn quick_insert_snapshot(&self)->Option<QuickInsertSession> {self.quick_insert.clone()}
}

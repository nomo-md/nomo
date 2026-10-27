//! 同一会话的窗口化视图。双栏各自调度片段；事务和历史仍只有 DocumentHost 一个入口。
use super::*;
use crate::settings::{EditorMode, Settings};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub(super) struct ReadingAnchor { byte: u64, fraction: f32, revision: Option<u64> }

pub(super) struct Pane {
    pub id: u64,
    pub control: Entity<EditorControl>,
    pub mode: ViewMode,
    pub viewport: viewport_ui::Viewport,
    pub scroll: gpui::ScrollHandle,
    pub reveal_selection: bool,
    pub stale: bool,
    pub load_error: Option<String>,
    reading_restore: Option<ReadingAnchor>,
    sync_requested: bool,
}
impl Pane {
    pub fn new(control: Entity<EditorControl>, subscription: Subscription, mode: ViewMode) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self { id:NEXT.fetch_add(1,Ordering::Relaxed), control:control.clone(), mode,
            viewport:viewport_ui::Viewport::new(control,subscription), scroll:gpui::ScrollHandle::new(),
            reveal_selection:false, stale:false, load_error:None, reading_restore:None, sync_requested:false }
    }
}

impl DocumentHost {
    pub(super) fn pane_for(&self, view: &Entity<EditorControl>) -> Option<usize> {
        self.panes.iter().position(|pane|pane.viewport.mounts.iter().any(|mount|mount.view==*view))
    }
    pub(super) fn pane_request_current(&self, pane:usize, id:u64, generation:u64) -> bool {
        !self.closed && !self.hidden && self.panes.get(pane).is_some_and(|p|p.id==id && p.viewport.generation==generation)
    }
    pub(super) fn update_resident_cost(&self, cx:&Context<Self>) {
        for pane in 0..self.panes.len() { self.update_pane_cost(pane,cx); }
    }
    pub(super) fn pause_viewport(&mut self, cx:&Context<Self>) {
        for pane in 0..self.panes.len() { self.pause_pane(pane,cx); }
    }
    pub(super) fn schedule_viewport(&mut self, ms:u64, cx:&mut Context<Self>) {
        for pane in 0..self.panes.len() { self.schedule_pane(pane,ms,cx); }
    }
    pub(super) fn reset_viewport(&mut self, cx:&mut Context<Self>) {
        self.panes[self.active_pane].control=self.control.clone();
        self.reset_pane(self.active_pane,cx);
    }
    pub(super) fn viewport_height(&mut self, view:&Entity<EditorControl>, stamp:&LayoutStamp, height:f32, cx:&mut Context<Self>) {
        if let Some(pane)=self.pane_for(view) {self.pane_height(pane,view,stamp,height,cx);}
    }
    pub(super) fn viewport_edited(&mut self, edits:&[SourceEdit], cx:&mut Context<Self>) {
        self.pane_edited(self.active_pane,edits,cx);
        self.request_pane_sync(self.active_pane);
        for (index,pane) in self.panes.iter_mut().enumerate() {
            if index==self.active_pane {continue;}
            if let Some(anchor)=&mut pane.reading_restore {
                let original=anchor.byte;
                let mut delta=0i128;
                for edit in edits {
                    if original<edit.from_byte {continue;}
                    if original<=edit.to_byte {
                        anchor.byte=(edit.from_byte as i128+delta+edit.inserted_text.len() as i128).max(0) as u64;
                        break;
                    }
                    delta+=edit.inserted_text.len() as i128-(edit.to_byte-edit.from_byte) as i128;
                    anchor.byte=(original as i128+delta).max(0) as u64;
                }
            }
        }
    }
    pub(super) fn sync_selection(&mut self, cx:&mut Context<Self>) {
        // 用户选择比尚未完成的滚动跟随更新；同修订号不能说明旧选择仍有效。
        self.cancel_pane_follow(self.active_pane,cx);
        let selection=self.control.read(cx).host.selection;
        self.request_pane_sync(self.active_pane);
        self.panes[self.active_pane].reading_restore=None;
        for (index,pane) in self.panes.iter().enumerate() {
            if index==self.active_pane || pane.stale {continue;}
            for mount in &pane.viewport.mounts {mount.view.update(cx,|view,cx| {view.restore_selection(selection,cx);});}
        }
    }
    pub(super) fn cancel_pane_follow(&mut self,pane:usize,cx:&mut Context<Self>) {
        if self.panes[pane].reading_restore.is_some() {
            self.pause_pane(pane,cx);
            self.panes[pane].reading_restore=None;
            self.schedule_pane(pane,16,cx);
        }
    }
    pub(super) fn cancel_outline_navigation(&mut self, cx:&mut Context<Self>) {
        for pane in 0..self.panes.len() {
            if self.panes[pane].reading_restore.is_some_and(|anchor| anchor.revision.is_some()) {
                self.cancel_pane_follow(pane, cx);
            }
        }
    }
    fn request_pane_sync(&mut self, source:usize) {
        // 同一帧内两栏都收到用户输入时，以最后的真实意图为准，防止互相回送旧锚点。
        for (index,pane) in self.panes.iter_mut().enumerate() {pane.sync_requested=index==source;}
    }
    fn reading_anchor(&self, pane:usize, cx:&Context<Self>) -> Option<ReadingAnchor> {
        let bounds=self.panes[pane].scroll.bounds();
        let height=f32::from(bounds.size.height);
        if height<=0.0 {return None;}
        let y=f32::from(bounds.origin.y)+height*0.3;
        self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            mount.view.read(cx).source_at_screen_y(y).map(|(byte,line_y)|ReadingAnchor {
                byte, fraction:((line_y-f32::from(bounds.origin.y))/height).clamp(0.0,1.0), revision:None
            })
        })
    }
    pub(super) fn invalidate_other_panes(&mut self, cx:&mut Context<Self>) {
        for pane in 0..self.panes.len() {
            if pane==self.active_pane {continue;}
            let anchor=self.reading_anchor(pane,cx);
            self.pause_pane(pane,cx);
            let state=&mut self.panes[pane];
            state.stale=true;
            state.load_error=None;
            state.reading_restore=state.reading_restore.or(anchor);
            for mount in &state.viewport.mounts {mount.view.update(cx,|v,_|v.host.set_readonly(true));}
            self.schedule_pane(pane,150,cx);
        }
    }
    pub(super) fn refresh_stale_pane(&mut self, pane:usize, cx:&mut Context<Self>) {
        let anchor=self.panes[pane].reading_restore.or_else(||self.reading_anchor(pane,cx));
        let byte=anchor.map_or(self.control.read(cx).host.selection.head_byte,|a|a.byte).min(self.total_bytes);
        self.refresh_pane_at(pane,byte,None,cx);
    }
    pub(super) fn refresh_pane_at(&mut self, pane:usize, byte:u64, selection:Option<DocumentSelection>, cx:&mut Context<Self>) {
        self.pause_pane(pane,cx);
        let state=&mut self.panes[pane];
        state.viewport.loading=true;
        state.load_error=None;
        let generation=state.viewport.generation;
        let id=state.id;
        let request=state.viewport.request.clone();
        let target=state.control.clone();
        let identity=target.read(cx).host.parsed.identity.clone();
        let port=self.port.clone();
        let revision=self.status().revision;
        let selected=selection.unwrap_or(self.control.read(cx).host.selection);
        let task=cx.background_executor().spawn(async move {port.load_group(revision,byte,None,&request)});
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if !host.pane_request_current(pane,id,generation) {return;}
                host.panes[pane].viewport.loading=false;
                if host.panes[pane].control!=target || target.read(cx).host.parsed.identity!=identity {
                    host.schedule_pane(pane,150,cx);return;
                }
                if host.status().revision!=revision {host.panes[pane].stale=true;host.schedule_pane(pane,150,cx);return;}
                let result=result.and_then(|loaded| {
                    target.update(cx,|view,cx| {
                        let generation=view.host.parsed.identity.view_generation+1;
                        view.update_window(loaded.input,generation,loaded.context,cx)?;
                        view.host.set_readonly(pane!=host.active_pane);
                        // 两栏的选区使用完整文档字节坐标；屏外端点无需截成局部坐标。
                        view.restore_selection(selected,cx);
                        Ok::<_,ControlError>(())
                    })
                });
                match result {
                    Ok(())=>{host.panes[pane].stale=false;host.reset_pane(pane,cx);}
                    Err(error)=>{host.panes[pane].stale=true;host.panes[pane].load_error=Some(error.to_string());host.report(error,cx);}
                }
                cx.notify();
            });
        }).detach();
    }
    pub(crate) fn change_mode(&mut self, mode:EditorMode, cx:&mut Context<Self>) -> Result<(),ControlError> {
        self.ready(cx)?;
        let anchor=self.reading_anchor(self.active_pane,cx);
        if mode==EditorMode::Split {
            if self.panes.len()==2 {return Ok(());}
            let existing_mode=self.panes[0].mode;
            let other_mode=if existing_mode==ViewMode::Source {ViewMode::Semantic}else{ViewMode::Source};
            // 只复制当前已加载结构片段；完整正文和历史仍由同一 NativeWindowPort 管理。
            let view=self.control.read(cx);
            let identity=&view.host.parsed.identity;
            let input=self.port.read_window(identity.revision,identity.from_byte..identity.to_byte)?;
            let context=WindowContext::Resolved(view.host.parsed.context.clone());
            static NEXT_WINDOW:AtomicU64=AtomicU64::new(1);
            let window_id=format!("split-{}-{}",identity.window_id,NEXT_WINDOW.fetch_add(1,Ordering::Relaxed));
            let adapter=WindowHostAdapter::new(input,window_id,0,context)?;
            let base_dir=view.base_dir.clone();
            let selected=view.host.selection;
            let control=cx.new(|cx| {let mut view=EditorControl::new(adapter,cx);view.mode=other_mode;view.base_dir=base_dir;view.workspace_hosted=self.embedded;view.host.set_readonly(true);view.host.selection=selected;view});
            let subscription=cx.subscribe(&control,Self::event);
            let mut pane=Pane::new(control,subscription,other_mode);
            pane.reading_restore=anchor;
            self.panes.push(pane);
            self.reset_pane(1,cx);
        } else {
            // 离开双栏立即回收另一栏的窗口、请求及预算，不留下隐藏的重复渲染。
            self.pause_viewport(cx);
            let keep=self.active_pane;
            for (index,pane) in self.panes.iter().enumerate() {
                if index!=keep {for mount in &pane.viewport.mounts {mount.view.update(cx,|v,cx|v.detach(false,cx))?;}}
            }
            let kept=self.panes.remove(keep);
            self.panes.clear();self.panes.push(kept);self.active_pane=0;
            let mode=if mode==EditorMode::Source {ViewMode::Source}else{ViewMode::Semantic};
            self.panes[0].mode=mode;
            for mount in &self.panes[0].viewport.mounts {mount.view.update(cx,|v,cx|v.set_mode(mode,cx))?;}
        }
        self.panes[self.active_pane].reading_restore=anchor;
        self.panes[self.active_pane].reveal_selection=false;
        self.schedule_viewport(16,cx);
        cx.notify();Ok(())
    }
    fn restore_reading(&mut self, pane:usize, cx:&mut Context<Self>) {
        let Some(anchor)=self.panes[pane].reading_restore else{return;};
        if anchor.revision.is_some_and(|revision| revision != self.status().revision) {
            self.cancel_pane_follow(pane, cx);
            return;
        }
        if self.panes[pane].stale || self.panes[pane].viewport.loading || self.panes[pane].load_error.is_some() {return;}
        let bounds=self.panes[pane].scroll.bounds();
        let located=self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            let view=mount.view.read(cx);
            view.caret_bounds(anchor.byte).map(|(_,caret)|(caret,view.measured_height().is_some()))
        });
        if let Some((caret,measured))=located {
            let delta=f32::from(caret.origin.y-bounds.origin.y)-f32::from(bounds.size.height)*anchor.fraction;
            let old=self.panes[pane].scroll.offset();
            // 使用本次 prepaint 后的真实可滚范围；首尾无法满足参考线时在边界结束。
            let max=self.panes[pane].scroll.max_offset().y.max(px(0.0));
            let target=(old.y-px(delta)).clamp(-max,px(0.0));
            if f32::from(target-old.y).abs()>0.5 {
                self.panes[pane].scroll.set_offset(gpui::point(old.x,target));
                self.schedule_pane(pane,16,cx);cx.notify();
            } else if measured && !self.panes[pane].viewport.dirty {self.panes[pane].reading_restore=None;}
        } else if !self.panes[pane].viewport.dirty {
            self.refresh_pane_at(pane,anchor.byte.min(self.total_bytes),None,cx);
        }
    }
    fn pane_prepainted(&mut self, pane:usize, cx:&mut Context<Self>) {
        if self.hidden || self.closed || pane>=self.panes.len() {return;}
        if pane == self.active_pane {
            let bounds = self.panes[pane].scroll.bounds();
            let height = f32::from(bounds.size.height);
            let inset = if self.panes[pane].mode == ViewMode::Source { 0.0 } else { 96.0_f32.min(height.max(0.0) * 0.9) };
            let y = f32::from(bounds.origin.y) + inset;
            let byte = self.panes[pane].viewport.mounts.iter().find_map(|mount| mount.view.read(cx).source_at_screen_y(y).map(|v| v.0));
            if let Some(byte) = byte { self.update_outline_anchor(byte, cx); }
        }
        // 输入法与拖选只暂停跨栏跟随；活动输入区仍须保持候选位置/光标可见。
        if pane==self.active_pane {self.reveal_pane(pane,cx);}
        let interaction=&self.control.read(cx).host.interaction;
        if self.split_dragging || interaction.composing || interaction.draft || interaction.dragging {return;}
        self.restore_reading(pane,cx);
        if self.panes.len()==2 && self.panes[pane].sync_requested && !self.panes[pane].reveal_selection && self.panes[pane].reading_restore.is_none() {
            if let Some(anchor)=self.reading_anchor(pane,cx) {
                self.panes[pane].sync_requested=false;
                let other=1-pane;
                self.panes[other].reading_restore=Some(anchor);
                self.panes[other].reveal_selection=false;
                self.schedule_pane(other,16,cx);cx.notify();
            }
        }
    }
    /// 语义导航只更新阅读锚点；全局选区继续属于文档，不截成窗口起点。
    pub(crate) fn reveal_outline_heading(&mut self, byte: u64, revision: u64, cx: &mut Context<Self>) {
        if self.status().revision != revision || !self.ready_for_session_action(cx) { return; }
        let pane = self.active_pane;
        if self.panes[pane].mode == ViewMode::Source {
            self.navigate_outline_source(byte, revision, cx);
            return;
        }
        self.cancel_pane_follow(pane, cx);
        self.panes[pane].reveal_selection = false;
        self.panes[pane].reading_restore = Some(ReadingAnchor { byte, fraction: 0.08, revision:Some(revision) });
        self.request_pane_sync(pane);
        self.schedule_pane(pane, 16, cx);
        cx.notify();
    }
    fn render_pane(&self, pane:usize, cx:&Context<Self>) -> gpui::AnyElement {
        let host=cx.entity().downgrade();
        let id=self.panes[pane].id;
        let width=f32::from(self.panes[pane].scroll.bounds().size.width);
        let settings=cx.global::<Settings>();
        let avoid_outline=self.panes.len()==1 && settings.show_outline && width>900.0;
        let safe=224.0+(width*0.035).clamp(32.0,160.0);
        let content_width=(width*settings.content_width_percent).min((width-safe).max(0.0));
        let content_left=((width-content_width)/2.0).min((width-safe-content_width).max(0.0));
        let document_body=div().children(self.pane_children(pane,cx))
            .when(avoid_outline,|el|el.w(px(content_width)).ml(px(content_left)))
            .when(!avoid_outline,|el|el.w(gpui::relative(if self.panes.len()==2 {1.0}else{settings.content_width_percent})).mx_auto());
        let content=div().on_children_prepainted(move |_,_,cx| {let _=host.update(cx,|host,cx| {
            if host.panes.get(pane).is_some_and(|p|p.id==id) {host.pane_prepainted(pane,cx);}
        });})
            .id(("document-pane",id)).size_full().min_w_0().min_h_0().overflow_y_scroll()
            .track_scroll(&self.panes[pane].scroll).py_8()
            .px(px(if self.panes.len()==2 {32.0}else{0.0}))
            .on_scroll_wheel(cx.listener(move |host,_,_,cx| {
                host.cancel_pane_follow(pane,cx);
                host.panes[pane].reveal_selection=false;host.panes[pane].reading_restore=None;
                host.request_pane_sync(pane);host.schedule_pane(pane,16,cx);
            })).child(document_body);
        let error=self.panes[pane].load_error.clone().map(|message| {
            div().flex().items_center().px_4().py_2().gap_2().child(div().flex_1().child(message))
                .child(crate::ui::components::button(("retry-pane",id),"重新加载",None,crate::ui::components::ButtonKind::Secondary,cx.global::<Theme>())
                    .on_click(cx.listener(move |host,_,_,cx| {host.panes[pane].load_error=None;host.refresh_stale_pane(pane,cx);})))
        });
        div().size_full().flex().flex_col().children(error).child(content.flex_1()).into_any_element()
    }
    pub(super) fn render_panes(&mut self, cx:&mut Context<Self>) -> gpui::AnyElement {
        if self.panes.len()==1 {return div().flex_1().min_h_0().child(self.render_pane(0,cx)).into_any_element();}
        let source_first=cx.global::<Settings>().split_source_first;
        let left=self.panes.iter().position(|p|(p.mode==ViewMode::Source)==source_first).unwrap_or(0);
        let right=1-left;
        let theme=Theme::ui(cx);
        let divider=div().id("split-divider").w(px(8.0)).h_full().flex_shrink_0().cursor_col_resize()
            .track_focus(&self.split_focus).bg(theme.border).hover(|s|s.bg(theme.accent))
            .on_mouse_down(gpui::MouseButton::Left,cx.listener(|host,_,window,cx| {
                cx.stop_propagation();
                if host.control.read(cx).prepare_unmount().is_err() {return;}
                for pane in 0..host.panes.len() {host.panes[pane].reading_restore=host.reading_anchor(pane,cx);}
                host.split_dragging=true;window.focus(&host.split_focus, cx);
            }))
            .on_key_down(cx.listener(|host,event:&gpui::KeyDownEvent,_,cx| {
                let step=if event.keystroke.modifiers.shift {0.1}else{0.02};
                match event.keystroke.key.as_str() {
                    "left"=>host.split_ratio=(host.split_ratio-step).max(0.25),
                    "right"=>host.split_ratio=(host.split_ratio+step).min(0.75),
                    "home"=>host.split_ratio=0.25,"end"=>host.split_ratio=0.75,_=>return,
                }
                cx.stop_propagation();cx.notify();
            }));
        div().flex_1().min_h_0().flex().min_w_0()
            .on_mouse_move(cx.listener(move |host,event:&gpui::MouseMoveEvent,_,cx| {
                if !host.split_dragging {return;}
                let l=host.panes[left].scroll.bounds();let r=host.panes[right].scroll.bounds();
                let width=f32::from(r.right()-l.left());
                if width>0.0 {host.split_ratio=(f32::from(event.position.x-l.left())/width).clamp(0.25,0.75);cx.notify();}
            }))
            .on_mouse_up(gpui::MouseButton::Left,cx.listener(|host,_,_,cx|{host.split_dragging=false;cx.notify();}))
            .on_mouse_up_out(gpui::MouseButton::Left,cx.listener(|host,_,_,cx|{host.split_dragging=false;cx.notify();}))
            .child(div().w(gpui::relative(self.split_ratio)).min_w_0().h_full().child(self.render_pane(left,cx)))
            .child(divider)
            .child(div().flex_1().min_w_0().h_full().child(self.render_pane(right,cx))).into_any_element()
    }
}

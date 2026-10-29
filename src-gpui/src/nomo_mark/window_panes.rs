//! 同一会话的窗口化视图。双栏各自调度片段；事务和历史仍只有 DocumentHost 一个入口。
use super::*;
use crate::settings::{EditorMode, Settings};
use std::{sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};

/// 单栏且显示悬浮大纲时正文列的左边距与宽度（px）：正文避开右侧大纲卡片；
/// `None` 表示正文按比例居中。编辑区顶部信息行用同一结果与正文左对齐。
pub(crate) fn outline_content_column(width: f32, outline_visible: bool, width_percent: f32) -> Option<(f32, f32)> {
    if !outline_visible || width <= 900.0 { return None; }
    let safe = 224.0 + (width * 0.035).clamp(32.0, 160.0);
    let content_width = (width * width_percent).min((width - safe).max(0.0));
    Some((((width - content_width) / 2.0).min((width - safe - content_width).max(0.0)), content_width))
}

#[derive(Clone, Copy)]
pub(super) struct ReadingAnchor { byte: u64, fraction: f32, revision: Option<u64>, view_only: bool }

pub(super) struct Pane {
    pub id: u64,
    pub control: Entity<NomoMark>,
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
    pub fn new(control: Entity<NomoMark>, subscription: Subscription, mode: ViewMode) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self { id:NEXT.fetch_add(1,Ordering::Relaxed), control:control.clone(), mode,
            viewport:viewport_ui::Viewport::new(control,subscription), scroll:gpui::ScrollHandle::new(),
            reveal_selection:false, stale:false, load_error:None, reading_restore:None, sync_requested:false }
    }
}

impl DocumentHost {
    pub(super) fn pane_for(&self, view: &Entity<NomoMark>) -> Option<usize> {
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
    pub(super) fn viewport_height(&mut self, view:&Entity<NomoMark>, stamp:&LayoutStamp, height:f32, cx:&mut Context<Self>) {
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
                byte, fraction:((line_y-f32::from(bounds.origin.y))/height).clamp(0.0,1.0), revision:None, view_only:false
            })
        })
    }
    /// 缩放前只记录已挂载窗口的阅读位置，不读取全文或创建新 Pane。
    pub(crate) fn prepare_zoom(&mut self, pointer: Option<gpui::Point<gpui::Pixels>>, cx: &mut Context<Self>) {
        if self.hidden || self.closed { return; }
        for pane in 0..self.panes.len() {
            let bounds = self.panes[pane].scroll.bounds();
            let height = f32::from(bounds.size.height);
            if height <= 0.0 { continue; }
            let y = pointer.filter(|point| bounds.contains(point)).map_or(
                f32::from(bounds.origin.y) + height * 0.5, |point| f32::from(point.y),
            );
            let anchor = self.panes[pane].viewport.mounts.iter().find_map(|mount| {
                mount.view.read(cx).source_at_screen_y(y).map(|(byte, line_y)| ReadingAnchor {
                    byte, fraction: ((line_y - f32::from(bounds.origin.y)) / height).clamp(0.0, 1.0),
                    revision: None, view_only: true,
                })
            });
            self.panes[pane].reading_restore = self.panes[pane].reading_restore.or(anchor);
            self.panes[pane].reveal_selection = false;
            self.panes[pane].sync_requested = false;
        }
        cx.notify();
    }
    /// 小窗只调整已挂载 Pane 的纸面留白，复用阅读锚点与窗口预算。
    pub(crate) fn set_mini_layout(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.mini_layout == active { return; }
        let at_top: Vec<_> = self.panes.iter().map(|pane| pane.scroll.offset().y >= px(-0.5)).collect();
        self.prepare_zoom(None, cx);
        // 文档顶部保持顶部，避免缩窄换行后为了追随屏幕中线而把标题滚出视口。
        for (pane, at_top) in self.panes.iter_mut().zip(at_top) {
            if at_top { pane.reading_restore = None; pane.scroll.set_offset(gpui::point(px(0.0), px(0.0))); }
        }
        self.mini_layout = active;
        self.suspend_outline_for_mini(active, cx);
        self.control.update(cx, |view, cx| view.dismiss_quick_insert(cx));
        cx.notify();
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
    pub(super) fn refresh_pending_references(&mut self, pane:usize, cx:&mut Context<Self>) {
        if self.panes[pane].stale || self.panes[pane].viewport.references_failed { return; }
        if !self.panes[pane].viewport.mounts.iter().any(|mount| mount.view.read(cx).host.parsed.context.pending_references) { return; }
        // 每栏至多一个短查询；与全文几何加载分开，不占用视口 loading 状态。
        self.schedule_pane(pane, 150, cx);
        if self.panes[pane].viewport.references_loading || Instant::now() < self.panes[pane].viewport.references_due { return; }
        let revision = self.status().revision;
        let windows: Vec<_> = self.panes[pane].viewport.mounts.iter().filter_map(|mount| {
            let view = mount.view.read(cx);
            (view.host.parsed.context.pending_references && view.host.parsed.identity.revision == revision
                && view.prepare_unmount().is_ok()).then(|| (view.host.parsed.identity.clone(), view.host.parsed.context.clone()))
        }).collect();
        if windows.is_empty() { return; }
        self.panes[pane].viewport.references_loading = true;
        self.panes[pane].viewport.references_due = Instant::now() + Duration::from_millis(150);
        let pane_id = self.panes[pane].id;
        let generation = self.panes[pane].viewport.generation;
        let request = self.panes[pane].viewport.request.clone();
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let mut resolved = Vec::with_capacity(windows.len());
            for (identity, context) in windows {
                let context = port.refresh_references(revision, context, &request)?;
                resolved.push((identity, context));
            }
            Ok::<_, ControlError>(resolved)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if !host.pane_request_current(pane, pane_id, generation) { return; }
                host.panes[pane].viewport.references_loading = false;
                if host.status().revision != revision { return; }
                let resolved = match result {
                    Ok(resolved) => resolved,
                    Err(ControlError::Stale) => return,
                    Err(error) => {
                        host.panes[pane].viewport.references_failed = true;
                        host.report(error, cx);
                        return;
                    }
                };
                let anchor = host.reading_anchor(pane, cx).map(|mut anchor| { anchor.view_only = true; anchor });
                let mut changed = false;
                for (identity, context) in resolved {
                    let Some(view) = host.panes[pane].viewport.mounts.iter()
                        .find(|mount| mount.view.read(cx).host.parsed.identity == identity).map(|mount| mount.view.clone()) else { continue; };
                    match view.update(cx, |view, cx| view.refresh_references(context, cx)) {
                        Ok(updated) => changed |= updated,
                        Err(ControlError::Busy | ControlError::Stale) => {},
                        Err(error) => {
                            host.panes[pane].viewport.references_failed = true;
                            host.report(error, cx);
                        }
                    }
                }
                if changed {
                    if !host.panes[pane].reveal_selection { host.panes[pane].reading_restore = host.panes[pane].reading_restore.or(anchor); }
                    host.update_pane_cost(pane, cx);
                    if pane == host.active_pane {
                        host.refresh_toolbar_marks(cx);
                        if !host.control.read(cx).host.parsed.context.pending_references { host.schedule_metadata(cx); }
                    }
                    cx.notify();
                }
            });
        }).detach();
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
        let resolving_references=target.read(cx).host.parsed.context.pending_references;
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
                    Ok(())=>{
                        host.panes[pane].stale=false;host.reset_pane(pane,cx);
                        if resolving_references && pane==host.active_pane {host.schedule_metadata(cx);}
                    }
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
            let control=cx.new(|cx| {let mut view=NomoMark::new(adapter,cx);view.mode=other_mode;view.base_dir=base_dir;view.workspace_hosted=self.embedded;view.host.set_readonly(true);view.host.selection=selected;view});
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
        if self.panes[pane].stale || (self.panes[pane].viewport.loading && !anchor.view_only) || self.panes[pane].load_error.is_some() {return;}
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
            } else if measured && (!self.panes[pane].viewport.dirty || anchor.view_only) {self.panes[pane].reading_restore=None;}
        } else if !self.panes[pane].viewport.dirty && !anchor.view_only {
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
        self.panes[pane].reading_restore = Some(ReadingAnchor { byte, fraction: 0.08, revision:Some(revision), view_only:false });
        self.request_pane_sync(pane);
        self.schedule_pane(pane, 16, cx);
        cx.notify();
    }
    fn paper_mouse_down(&mut self, pane: usize, event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.modifiers.shift || event.modifiers.control || event.modifiers.platform || event.modifiers.alt
            || self.hidden || self.closed || self.navigating || self.saving || self.close_prompt
            || self.panes[pane].stale || self.panes[pane].load_error.is_some()
            || self.control.read(cx).host.interaction.protected()
        { return; }
        let viewport = &self.panes[pane].viewport;
        let candidate = viewport.mounts.iter().find_map(|mount| {
            let view = mount.view.read(cx);
            let identity = &view.host.parsed.identity;
            let index = if self.production && !viewport.groups.is_empty() {
                let index = viewport.groups.binary_search_by_key(&identity.from_byte, |group| group.from).ok()?;
                if !viewport.groups[index].matches_mount(identity, mount.view == self.panes[pane].control) { return None; }
                Some(index)
            } else { None };
            let bounds = view.content_bounds()?;
            if bounds.contains(&event.position) { return None; }
            let y = event.position.y;
            let above = y < bounds.origin.y;
            let below = y >= bounds.bottom();
            if (above && identity.from_byte != 0)
                || (below && index.is_some_and(|index| index + 1 != viewport.groups.len()))
            { return None; }
            Some(mount.view.clone())
        });
        if let Some(view) = candidate {
            if self.promote(&view, cx) {
                view.update(cx, |view, cx| view.place_caret_at(event.position, window, cx));
            }
        }
    }
    fn render_pane(&self, pane:usize, cx:&Context<Self>) -> gpui::AnyElement {
        let host=cx.entity().downgrade();
        let id=self.panes[pane].id;
        let width=f32::from(self.panes[pane].scroll.bounds().size.width);
        let settings=cx.global::<Settings>();
        let avoid=outline_content_column(width,!self.mini_layout && self.panes.len()==1 && settings.show_outline,settings.content_width_percent);
        let document_body=div().children(self.pane_children(pane,cx))
            .when_some(avoid,|el,(content_left,content_width)|el.w(px(content_width)).ml(px(content_left)))
            .when(avoid.is_none(),|el|el.w(gpui::relative(if self.mini_layout || self.panes.len()==2 {1.0}else{settings.content_width_percent})).mx_auto());
        let zoom_host = cx.entity().downgrade();
        // GPUI 的默认滚动先于冒泡回调执行；在捕获阶段处理缩放，避免一次手势既缩放又滚动。
        // 普通命中区域不遮挡编辑器；弹层的遮挡命中区域仍优先。
        let zoom_capture = gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let host = zoom_host.clone();
                window.on_mouse_event(move |event: &gpui::ScrollEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Capture || !hitbox.should_handle_scroll(window) { return; }
                    let settings = cx.global::<Settings>();
                    if !settings.ctrl_wheel_zoom || !(event.modifiers.control || event.modifiers.platform) { return; }
                    let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
                    let current = (settings.zoom * 100.0).round() as i64;
                    let next = (current + if delta > 0.5 { 5 } else if delta < -0.5 { -5 } else { 0 }).clamp(80, 160);
                    if next != current {
                        let _ = host.update(cx, |host, cx| {
                            if host.panes.get(pane).is_none_or(|state| state.id != id) || host.hidden || host.closed { return; }
                            host.prepare_zoom(Some(event.position), cx);
                            Settings::set(cx, "zoomPercent", serde_json::json!(next));
                        });
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                });
            },
        ).absolute().top_0().left_0().size_full();
        let content=div().on_children_prepainted(move |_,_,cx| {let _=host.update(cx,|host,cx| {
            if host.panes.get(pane).is_some_and(|p|p.id==id) {host.pane_prepainted(pane,cx);}
        });})
            .id(("document-pane",id)).size_full().min_w_0().min_h_0().overflow_y_scroll()
            .track_scroll(&self.panes[pane].scroll)
            .pt(px(if self.mini_layout {48.0}else{32.0})).pb(px(if self.mini_layout {56.0}else{32.0}))
            .px(px(if self.mini_layout {(width*0.05).clamp(18.0,30.0)}else if self.panes.len()==2 {32.0}else{0.0}))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |host, event, window, cx| {
                host.paper_mouse_down(pane, event, window, cx);
            }))
            .on_scroll(cx.listener(move |host,_,_,cx| {
                host.cancel_pane_follow(pane,cx);
                host.panes[pane].reveal_selection=false;host.panes[pane].reading_restore=None;
                host.request_pane_sync(pane);host.schedule_pane(pane,16,cx);
            })).child(document_body);
        let error=self.panes[pane].load_error.clone().map(|message| {
            div().flex().items_center().px_4().py_2().gap_2().child(div().flex_1().child(message))
                .child(crate::ui::components::button(("retry-pane",id),"重新加载",None,crate::ui::components::ButtonKind::Secondary,cx.global::<Theme>())
                    .on_click(cx.listener(move |host,_,_,cx| {host.panes[pane].load_error=None;host.refresh_stale_pane(pane,cx);})))
        });
        let source_only=self.panes[pane].control.read(cx).host.parsed.context.source_only;
        let source_notice=source_only.then(|| div().px_4().py_1().text_size(px(12.0)).text_color(cx.global::<Theme>().muted)
            .child("此结构暂以源码方式编辑"));
        div().size_full().flex().flex_col().children(error).children(source_notice)
            .child(div().relative().flex_1().min_h_0().child(content).child(zoom_capture)).into_any_element()
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

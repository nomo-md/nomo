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
enum ReadingEdge { Top, Bottom }
#[derive(Clone, Copy)]
pub(super) struct ReadingAnchor { byte: u64, fraction: f32, revision: Option<u64>, view_only: bool, edge: Option<ReadingEdge> }

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
    // 只在模式切换首帧使用，完成 prepaint 后释放。
    layout_width: Option<f32>,
}
impl Pane {
    pub fn new(control: Entity<NomoMark>, subscription: Subscription, mode: ViewMode) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self { id:NEXT.fetch_add(1,Ordering::Relaxed), control:control.clone(), mode,
            viewport:viewport_ui::Viewport::new(control,subscription), scroll:gpui::ScrollHandle::new(),
            reveal_selection:false, stale:false, load_error:None, reading_restore:None, sync_requested:false, layout_width:None }
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
        // 位于真正的文首/文末时跟随边缘，不能用中部锚点把尾行挤出视口。
        if self.panes[pane].scroll.offset().y>=px(-0.5) && self.visible_source_bounds(pane,0,cx).is_some() {
            return Some(ReadingAnchor {byte:0,fraction:0.0,revision:None,view_only:false,edge:Some(ReadingEdge::Top)});
        }
        if self.visible_source_bounds(pane,self.total_bytes,cx).is_some() {
            return Some(ReadingAnchor {byte:self.total_bytes,fraction:1.0,revision:None,view_only:false,edge:Some(ReadingEdge::Bottom)});
        }
        let y=f32::from(bounds.origin.y)+height*0.3;
        self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            mount.view.read(cx).source_at_screen_y(y).map(|(byte,line_y)|ReadingAnchor {
                byte, fraction:((line_y-f32::from(bounds.origin.y))/height).clamp(0.0,1.0), revision:None, view_only:false, edge:None
            })
        })
    }
    fn visible_source_bounds(&self, pane:usize, byte:u64, cx:&Context<Self>) -> Option<gpui::Bounds<gpui::Pixels>> {
        let viewport=self.panes[pane].scroll.bounds();
        self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            let (_,bounds)=mount.view.read(cx).caret_bounds(byte)?;
            (bounds.top()>=viewport.top()-px(0.5) && bounds.bottom()<=viewport.bottom()+px(0.5)).then_some(bounds)
        })
    }
    /// 只比较撤销前后已经加载的片段；不复制正文，也不猜测跨窗口的字节映射。
    pub(super) fn history_reading_anchor(&self, loaded:&super::super::session_window::LoadedWindow, selection:Option<DocumentSelection>, cx:&Context<Self>) -> Option<ReadingAnchor> {
        let pane=self.active_pane;
        let mut anchor=self.reading_anchor(pane,cx)?;
        let before=self.control.read(cx);
        let old=&before.host.parsed;
        if old.identity.from_byte!=loaded.input.from_byte {return None;}
        let a=old.source.as_ref();
        let b=loaded.input.text.as_str();
        let mut prefix=a.bytes().zip(b.bytes()).take_while(|(a,b)|a==b).count();
        while !a.is_char_boundary(prefix) || !b.is_char_boundary(prefix) {prefix-=1;}
        let mut suffix=a.as_bytes()[prefix..].iter().rev().zip(b.as_bytes()[prefix..].iter().rev()).take_while(|(a,b)|a==b).count();
        while !a.is_char_boundary(a.len()-suffix) || !b.is_char_boundary(b.len()-suffix) {suffix-=1;}
        let restored=selection?.head_byte.checked_sub(loaded.input.from_byte)? as usize;
        if restored>b.len() {return None;}
        let target=if restored<=prefix {restored} else if restored>=b.len()-suffix {a.len()-(b.len()-restored)} else {prefix};
        // 撤销目标原本不可见时仍沿用文档导航，不能为了保留阅读位置隐藏实际修改。
        self.visible_source_bounds(pane,old.identity.from_byte+target as u64,cx)?;
        if anchor.edge.is_none() {
            // 公式等对象恢复高度时优先固定上方未改动的可见文字，避免跟随对象下方内容挪动画面。
            let viewport=self.panes[pane].scroll.bounds();
            let height=f32::from(viewport.size.height);
            for fraction in [0.0,0.08] {
                let y=f32::from(viewport.top())+height*fraction;
                let located=self.panes[pane].viewport.mounts.iter().find_map(|mount|mount.view.read(cx).source_at_screen_y(y));
                if let Some((byte,line_y))=located {
                    if old.identity.from_byte<=byte && byte<=old.identity.from_byte+prefix as u64
                        && self.visible_source_bounds(pane,byte,cx).is_some() {
                        anchor.byte=byte;
                        anchor.fraction=((line_y-f32::from(viewport.top()))/height).clamp(0.0,1.0);
                        break;
                    }
                }
            }
        }
        anchor.byte=match anchor.edge {
            Some(ReadingEdge::Top)=>0,
            Some(ReadingEdge::Bottom)=>loaded.total_bytes,
            None=> {
                let local=anchor.byte.checked_sub(old.identity.from_byte)? as usize;
                if local<=prefix {anchor.byte}
                else if local>=a.len()-suffix && local<=a.len()
                    && b.len() as i128-a.len() as i128==loaded.total_bytes as i128-self.total_bytes as i128 {
                    loaded.input.from_byte+(b.len()-(a.len()-local)) as u64
                } else {return None;}
            }
        };
        anchor.revision=Some(loaded.input.revision);
        Some(anchor)
    }
    pub(super) fn restore_history_reading(&mut self, anchor:ReadingAnchor) {
        self.panes[self.active_pane].reading_restore=Some(anchor);
        self.panes[self.active_pane].reveal_selection=false;
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
                    revision: None, view_only: true, edge:None,
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
                    Err(ControlError::Stale) => { host.schedule_pane(pane, 150, cx); return; },
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
        if pane == self.active_pane && self.panes[pane].reveal_selection {
            let selection = self.control.read(cx).host.selection;
            self.refresh_pane_at(pane, selection.head_byte.min(self.total_bytes), Some(selection), cx);
            return;
        }
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
                    Err(ControlError::Stale)=>{
                        host.panes[pane].stale=true;
                        host.schedule_pane(pane,crate::nomo_mark::session_window::policy().quiet_ms,cx);
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
        let width=self.panes.iter().map(|pane|f32::from(pane.scroll.bounds().size.width)).sum::<f32>()+if self.panes.len()==2 {8.0}else{0.0};
        if mode==EditorMode::Split {
            if self.panes.len()==2 {return Ok(());}
            let existing_mode=self.panes[0].mode;
            let other_mode=if existing_mode==ViewMode::Source {ViewMode::Semantic}else{ViewMode::Source};
            // 从阅读锚点所在的已挂载片段创建新栏；活动光标可能仍在远处的旧片段。
            let revision=self.control.read(cx).host.parsed.identity.revision;
            let seed=anchor.and_then(|anchor| self.panes[self.active_pane].viewport.mounts.iter().find(|mount| {
                let id=&mount.view.read(cx).host.parsed.identity;
                id.revision==revision && id.from_byte<=anchor.byte && (anchor.byte<id.to_byte || anchor.byte==self.total_bytes && anchor.byte==id.to_byte)
            })).map(|mount| &mount.view).unwrap_or(&self.control);
            let view=seed.read(cx);
            let identity=&view.host.parsed.identity;
            let input=self.port.read_window(identity.revision,identity.from_byte..identity.to_byte)?;
            let context=WindowContext::Resolved(view.host.parsed.context.clone());
            static NEXT_WINDOW:AtomicU64=AtomicU64::new(1);
            let window_id=format!("split-{}-{}",identity.window_id,NEXT_WINDOW.fetch_add(1,Ordering::Relaxed));
            let adapter=WindowHostAdapter::new(input,window_id,0,context)?;
            let base_dir=view.base_dir.clone();
            let selected=self.control.read(cx).host.selection;
            let control=cx.new(|cx| {let mut view=NomoMark::new(adapter,cx);view.mode=other_mode;view.base_dir=base_dir;view.workspace_hosted=self.embedded;view.windowed_navigation=self.production;view.host.set_readonly(true);view.host.selection=selected;view});
            let subscription=cx.subscribe(&control,Self::event);
            let mut pane=Pane::new(control,subscription,other_mode);
            pane.reading_restore=anchor;
            let current=&self.panes[self.active_pane];
            let mut neighbors=Vec::new();
            if !current.viewport.dirty && !current.viewport.groups.is_empty() {
                // 复用同修订的已发布分组；新栏不能先把尾部短片段当作整篇文档。
                pane.viewport.groups=current.viewport.groups.clone();
                pane.viewport.heights=super::super::viewport_model::Heights::new(pane.viewport.groups.iter().map(|group|group.estimate));
                pane.viewport.dirty=false;
                if !current.viewport.budget.pressured() {
                    let visible=current.scroll.bounds();
                    for mount in current.viewport.mounts.iter().filter(|mount|mount.view!=*seed).filter(|mount| {
                        mount.view.read(cx).content_bounds().is_some_and(|bounds|bounds.bottom()>visible.top() && bounds.top()<visible.bottom())
                    }).take(2) {
                        let view=mount.view.read(cx);
                        let id=&view.host.parsed.identity;
                        if id.revision!=revision {continue;}
                        let input=self.port.read_window(revision,id.from_byte..id.to_byte)?;
                        let bytes=input.text.len();
                        let context=WindowContext::Resolved(view.host.parsed.context.clone());
                        let id=format!("split-{}-{}",id.window_id,NEXT_WINDOW.fetch_add(1,Ordering::Relaxed));
                        neighbors.push((WindowHostAdapter::new(input,id,0,context)?,view.base_dir.clone(),bytes));
                    }
                }
            }
            self.panes.push(pane);
            self.update_pane_cost(1,cx);
            // 最多提前创建两个原本可见的邻窗，受既有共享预算约束；屏外预取仍交给调度器。
            for (adapter,base_dir,bytes) in neighbors {
                if !self.panes[1].viewport.budget.reserve(super::super::resident_budget::Cost {views:1,source:bytes,layout:0},false) {break;}
                let view=cx.new(|cx| {let mut view=NomoMark::new(adapter,cx);view.mode=other_mode;view.base_dir=base_dir;view.workspace_hosted=self.embedded;view.windowed_navigation=self.production;view.host.set_readonly(true);view.host.selection=selected;view});
                let subscription=cx.subscribe(&view,Self::event);
                self.panes[1].viewport.mounts.push(viewport_ui::Mounted {view,_subscription:subscription,height:32.0});
                self.update_pane_cost(1,cx);
            }
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
        if width>0.0 {
            let source_first=cx.global::<Settings>().split_source_first;
            for pane in &mut self.panes {
                pane.layout_width=Some(if mode!=EditorMode::Split {width}
                    else if (pane.mode==ViewMode::Source)==source_first {width*self.split_ratio}
                    else {width*(1.0-self.split_ratio)-8.0});
            }
        }
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
        if let Some(edge)=anchor.edge {
            let ready=self.panes[pane].viewport.mounts.iter().any(|mount| {
                let view=mount.view.read(cx);
                let id=&view.host.parsed.identity;
                (match edge {ReadingEdge::Top=>id.from_byte==0,ReadingEdge::Bottom=>id.to_byte==self.total_bytes})
                    && view.measured_height().is_some_and(|height|(height-mount.height).abs()<0.5)
            });
            if !ready {
                if !self.panes[pane].viewport.dirty && !self.panes[pane].viewport.mounts.iter().any(|mount| {
                    let id=&mount.view.read(cx).host.parsed.identity;
                    id.from_byte<=anchor.byte && anchor.byte<=id.to_byte
                }) {self.refresh_pane_at(pane,anchor.byte.min(self.total_bytes),None,cx);}
                return;
            }
            let old=self.panes[pane].scroll.offset();
            let target=match edge {ReadingEdge::Top=>px(0.0),ReadingEdge::Bottom=>-self.panes[pane].scroll.max_offset().y.max(px(0.0))};
            if f32::from(target-old.y).abs()>0.5 {
                self.panes[pane].scroll.set_offset(gpui::point(old.x,target));
                self.schedule_pane(pane,16,cx);cx.notify();
            } else if self.pane_visible_ready(pane,cx) {self.panes[pane].reading_restore=None;}
            return;
        }
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
            } else if measured && (self.pane_visible_ready(pane,cx) || anchor.view_only) {self.panes[pane].reading_restore=None;}
        } else if !self.panes[pane].viewport.dirty && !anchor.view_only {
            self.refresh_pane_at(pane,anchor.byte.min(self.total_bytes),None,cx);
        }
    }
    fn pane_prepainted(&mut self, pane:usize, cx:&mut Context<Self>) {
        if self.hidden || self.closed || pane>=self.panes.len() {return;}
        self.panes[pane].layout_width=None;
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
        self.panes[pane].reading_restore = Some(ReadingAnchor { byte, fraction: 0.08, revision:Some(revision), view_only:false, edge:None });
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
                view.update(cx, |view, cx| {
                    if event.button == gpui::MouseButton::Right { view.open_context_menu(event.position, window, cx); }
                    else { view.place_caret_at(event.position, window, cx); }
                });
            }
        }
    }
    fn render_pane(&self, pane:usize, cx:&Context<Self>) -> gpui::AnyElement {
        let host=cx.entity().downgrade();
        let id=self.panes[pane].id;
        let width=self.panes[pane].layout_width.unwrap_or_else(||f32::from(self.panes[pane].scroll.bounds().size.width));
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
            .on_mouse_down(gpui::MouseButton::Right, cx.listener(move |host, event, window, cx| {
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
    /// 模式切换和历史恢复都在首帧布局前定位；只测量当前驻留窗口。
    fn prepare_reading_layouts(&mut self, window:&mut Window, cx:&mut Context<Self>) {
        for pane in 0..self.panes.len() {
            let Some(anchor)=self.panes[pane].reading_restore else {continue;};
            let width=self.panes[pane].layout_width;
            let history=width.is_none() && self.panes[pane].viewport.groups.is_empty()
                && self.panes[pane].control.read(cx).content_bounds().is_none();
            if width.is_none() && !history {continue;}
            let bounds=self.panes[pane].scroll.bounds();
            let height=if bounds.size.height>px(0.0) {f32::from(bounds.size.height)}
                else {self.panes.iter().map(|pane|f32::from(pane.scroll.bounds().size.height)).fold(0.0_f32,f32::max)};
            if height<=0.0 {continue;}
            let settings=cx.global::<Settings>();
            let content_width=width.map(|width| {
                let content=if self.mini_layout {width-2.0*(width*0.05).clamp(18.0,30.0)}
                    else if self.panes.len()==2 {width-64.0}
                    else if settings.show_outline && width>900.0 {(width*settings.content_width_percent).min((width-224.0-(width*0.035).clamp(32.0,160.0)).max(0.0))}
                    else {width*settings.content_width_percent};
                (content*window.scale_factor()).round()/window.scale_factor()
            });
            let views:Vec<_>=self.panes[pane].viewport.mounts.iter().map(|mount|mount.view.clone()).collect();
            let mut located=None;
            for view in views {
                if let Some((stamp,content_height,y))=view.update(cx,|view,cx|view.prepare_reading_layout(content_width,anchor.byte,window,cx)) {
                    self.pane_height(pane,&view,&stamp,content_height,cx);
                    if let Some(y)=y {located=Some((stamp.identity.from_byte,y));}
                }
            }
            let total=if self.panes[pane].viewport.groups.is_empty() {self.panes[pane].viewport.mounts.first().map_or(0.0,|mount|mount.height)}
                else {self.panes[pane].viewport.heights.total()};
            let (padding_top,padding_bottom)=if self.mini_layout {(48.0,56.0)}else{(32.0,32.0)};
            let max=(total+padding_top+padding_bottom-height).max(0.0);
            let top=match anchor.edge {
                Some(ReadingEdge::Top)=>0.0,
                Some(ReadingEdge::Bottom)=>max,
                None=> {
                    let Some((from,y))=located else {continue;};
                    let prefix=self.panes[pane].viewport.groups.iter().position(|group|group.from==from)
                        .map_or(0.0,|index|self.panes[pane].viewport.heights.top(index));
                    (padding_top+prefix+y-height*anchor.fraction).clamp(0.0,max)
                }
            };
            self.panes[pane].scroll.set_offset(gpui::point(px(0.0),px(-top)));
            self.update_pane_cost(pane,cx);
        }
    }
    pub(super) fn render_panes(&mut self, window:&mut Window, cx:&mut Context<Self>) -> gpui::AnyElement {
        self.prepare_reading_layouts(window,cx);
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

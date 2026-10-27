//! MarkdownViewportHost 的 GPUI 平台连接。沿用已发布结构组、预取距离和回收门禁。
//! 仅一个活动输入控件；相邻已挂载控件可以显示/选择，焦点移交后才开放编辑。
use super::*;
use crate::editor_control::{session_window::WindowRequest, viewport_model::{Group, Heights, ScrollDemand}};
use std::{collections::BTreeSet, time::{Duration, Instant}};
use crate::editor_control::resident_budget::{Cost, ResidentLease};

pub(super) struct Mounted {
    pub view: Entity<EditorControl>,
    _subscription: Subscription,
    pub height: f32,
}
pub(super) struct Viewport {
    pub mounts: Vec<Mounted>,
    pub groups: Vec<Group>,
    pub heights: Heights,
    budget: ResidentLease,
    reserved: Cost,
    pub(super) saved_scroll: gpui::Point<gpui::Pixels>,
    pub(super) request: WindowRequest,
    pub(super) generation: u64,
    pub(super) loading: bool,
    pub(super) dirty: bool,
    failed: BTreeSet<String>,
    demand: ScrollDemand,
    started: Instant,
    last_top: f32,
    pub(super) timer: Option<gpui::Task<()>>,
}
impl Viewport {
    pub fn new(view: Entity<EditorControl>, subscription: Subscription) -> Self {
        Self { mounts: vec![Mounted { view, _subscription: subscription, height:32.0 }], groups:Vec::new(), heights:Heights::default(), budget:ResidentLease::default(), reserved:Cost::default(), saved_scroll:gpui::point(px(0.0),px(0.0)), request:WindowRequest::default(), generation:0, loading:false, dirty:true, failed:BTreeSet::new(), demand:ScrollDemand::default(), started:Instant::now(), last_top:0.0, timer:None }
    }
}
impl Drop for Viewport { fn drop(&mut self) { self.request.cancel(); } }

impl DocumentHost {
    /// 仅在滚动容器及其子控件 prepaint 完成后运行；此时 clamp 和光标几何属于同一帧。
    pub(super) fn reveal_pane(&mut self, pane: usize, cx: &mut Context<Self>) {
        if !self.panes[pane].reveal_selection || self.hidden || self.navigating { return; }
        let view = self.panes[pane].control.read(cx);
        let Some((identity, caret_y, caret_height)) = view.caret_line(view.host.selection.head_byte) else { return; };
        let bounds = self.panes[pane].scroll.bounds();
        let height = f32::from(bounds.size.height);
        if height <= 0.0 { return; }
        let group_top = if self.panes[pane].viewport.groups.is_empty() { 0.0 } else {
            let Ok(index) = self.panes[pane].viewport.groups.binary_search_by_key(&identity.from_byte, |group| group.from) else { return; };
            // 局部乐观编辑会改变 to；同一起点的当前片段仍可用原高度树定位。
            self.panes[pane].viewport.heights.top(index)
        };
        let actual = view.caret_bounds(view.host.selection.head_byte);
        let caret_top = actual.as_ref().map_or(
            group_top + 32.0 + caret_y + f32::from(self.panes[pane].scroll.offset().y),
            |(_, caret)| f32::from(caret.origin.y - bounds.origin.y),
        );
        let caret_bottom = caret_top + caret_height;
        let delta = if caret_top < 16.0 { caret_top - 16.0 }
            else if caret_bottom > height - 16.0 { caret_bottom - height + 16.0 }
            else { 0.0 };
        if !delta.is_finite() { return; }
        if actual.is_some() && caret_top >= 0.0 && caret_bottom <= height && delta.abs() < 0.5 {
            let top = (-f32::from(self.panes[pane].scroll.offset().y) - 32.0).max(0.0);
            let visible_ready = self.panes[pane].viewport.groups.is_empty() ||
                (self.panes[pane].viewport.heights.at(top)..=self.panes[pane].viewport.heights.at(top + height)).all(|index| {
                    let group = &self.panes[pane].viewport.groups[index];
                    !group.semantic || self.panes[pane].viewport.failed.contains(&group.id) || self.panes[pane].viewport.mounts.iter().any(|mount| {
                        let mounted = mount.view.read(cx);
                        let id = &mounted.host.parsed.identity;
                        id.from_byte == group.from && id.to_byte == group.to
                            && mounted.measured_height().is_some_and(|height| (height - mount.height).abs() < 0.5)
                    })
                });
            // 输入确认后还会重取结构和邻窗；它们完成前不能把临时占位布局当作最终位置。
            if view.host.pending_count() == 0 && view.measured_height().is_some()
                && !self.panes[pane].viewport.dirty && !self.panes[pane].viewport.loading && visible_ready {
                self.panes[pane].reveal_selection = false;
            }
            return;
        }
        let before = self.panes[pane].scroll.offset();
        let requested = (before.y - px(delta)).min(px(0.0));
        self.panes[pane].scroll.set_offset(gpui::point(before.x, requested));
        self.schedule_pane(pane, 16, cx);
        cx.notify();
    }
    pub(super) fn pause_pane(&mut self, pane: usize, cx:&Context<Self>) {
        self.panes[pane].viewport.request.cancel(); self.panes[pane].viewport.request=WindowRequest::default();
        self.panes[pane].viewport.generation+=1; self.panes[pane].viewport.loading=false; self.panes[pane].viewport.timer=None;
        self.panes[pane].viewport.reserved=Cost::default(); self.update_pane_cost(pane, cx);
    }
    pub(super) fn update_pane_cost(&self, pane: usize, cx:&Context<Self>) {
        self.panes[pane].viewport.budget.update(self.panes[pane].viewport.mounts.iter().fold(self.panes[pane].viewport.reserved,|cost,m| {let view=m.view.read(cx); Cost {views:cost.views+1,source:cost.source+view.host.parsed.source.len(),layout:cost.layout+view.estimated_bytes()}}));
    }
    pub(super) fn reset_pane(&mut self, pane: usize, cx: &mut Context<Self>) {
        self.panes[pane].viewport.request.cancel();
        self.panes[pane].viewport.generation += 1;
        self.panes[pane].viewport.request = WindowRequest::default();
        self.panes[pane].viewport.loading = false;
        self.panes[pane].viewport.reserved=Cost::default();
        self.panes[pane].viewport.dirty = true;
        self.panes[pane].viewport.failed.clear();
        self.panes[pane].viewport.groups.clear();
        self.panes[pane].viewport.heights = Heights::default();
        self.retain_pane_control(pane, cx);
        if !self.panes[pane].viewport.mounts.iter().any(|m|m.view == self.panes[pane].control) {
            let view=self.panes[pane].control.clone();
            let subscription=cx.subscribe(&view,Self::event);
            self.panes[pane].viewport.mounts.push(Mounted { view, _subscription:subscription, height:32.0 });
        }
        self.update_pane_cost(pane, cx);
        self.schedule_pane(pane, 16, cx);
    }
    fn retain_pane_control(&mut self, pane: usize, cx: &mut Context<Self>) {
        let control=self.panes[pane].control.clone();
        self.panes[pane].viewport.mounts.retain(|mount| {
            if mount.view == control { return true; }
            // 任何未完成交互都不得被预算或滚动回收吞掉。
            mount.view.update(cx, |view,cx| view.detach(false,cx)).is_err()
        });
    }
    pub(super) fn schedule_pane(&mut self, pane: usize, ms: u64, cx: &mut Context<Self>) {
        if !self.production || self.hidden || self.panes[pane].viewport.timer.is_some() { return; }
        let pane_id=self.panes[pane].id;
        self.panes[pane].viewport.timer = Some(cx.spawn(async move |this,cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            let _=this.update(cx, |host,cx| { if host.panes.get(pane).is_some_and(|p|p.id==pane_id) { host.panes[pane].viewport.timer=None; host.pane_tick(pane, cx); } });
        }));
    }
    pub(super) fn pane_edited(&mut self, pane: usize, edits: &[SourceEdit], cx: &mut Context<Self>) {
        if !self.production { return; }
        self.panes[pane].viewport.request.cancel();
        self.panes[pane].viewport.request=WindowRequest::default();
        self.panes[pane].viewport.generation+=1;
        self.panes[pane].viewport.loading=false;
        self.panes[pane].viewport.reserved=Cost::default();
        self.panes[pane].viewport.dirty=true;
        self.panes[pane].viewport.failed.clear();
        // 只有活动控件拥有乐观输入。其他窗口按已确认补丁移动占位，重新查询依赖。
        // 引用定义可能影响远处窗口，不能以范围不相交推断其语义仍然有效。
        self.retain_pane_control(pane, cx);
        self.update_pane_cost(pane, cx);
        let active_from=self.panes[pane].control.read(cx).host.parsed.identity.from_byte;
        for group in &mut self.panes[pane].viewport.groups {
            let before = edits.iter().filter(|edit| edit.to_byte <= group.from && edit.from_byte < group.from).map(|e| e.inserted_text.len() as i128-(e.to_byte-e.from_byte) as i128).sum::<i128>();
            let active=group.from==active_from;
            let within = edits.iter().filter(|edit| edit.from_byte >= group.from && (edit.from_byte < group.to || (active && edit.from_byte==group.to))).map(|e| e.inserted_text.len() as i128-(e.to_byte-e.from_byte) as i128).sum::<i128>();
            group.from=(group.from as i128+before).max(0) as u64;
            group.to=(group.to as i128+before+within).max(group.from as i128) as u64;
        }
        self.panes[pane].viewport.timer=None;
        self.schedule_pane(pane, crate::editor_control::session_window::policy().quiet_ms, cx);
    }
    pub(super) fn promote(&mut self, view: &Entity<EditorControl>, cx: &mut Context<Self>) -> bool {
        let Some(pane)=self.pane_for(view) else { return false; };
        if *view == self.control { self.cancel_pane_follow(pane,cx);return true; }
        if self.navigating || self.saving || self.close_prompt || self.hidden || self.panes[pane].stale || self.panes[pane].viewport.dirty { return false; }
        if let Err(error)=self.control.read(cx).prepare_unmount() {
            self.message=error.to_string(); self.focus_pending=true; return false;
        }
        if view.read(cx).host.parsed.identity.revision != self.status().revision { return false; }
        self.cancel_pane_follow(pane,cx);
        self.control.update(cx, |v,_| v.host.set_readonly(true));
        self.active_pane=pane;
        self.control=view.clone();
        self.panes[pane].control=view.clone();
        self.control.update(cx, |v,_| v.host.set_readonly(false));
        self.refresh_toolbar_marks(cx);
        true
    }
    pub(super) fn pane_height(&mut self, pane: usize, view: &Entity<EditorControl>, stamp: &LayoutStamp, height: f32, cx: &mut Context<Self>) {
        if stamp.identity != view.read(cx).host.parsed.identity || !height.is_finite() || height<1.0 { return; }
        if let Some(mount)=self.panes[pane].viewport.mounts.iter_mut().find(|m| &m.view==view) { mount.height=height; }
        if let Some(index)=self.panes[pane].viewport.groups.iter().position(|g| g.from==stamp.identity.from_byte && g.to==stamp.identity.to_byte) {
            let old_bottom=self.panes[pane].viewport.heights.top(index+1);
            let delta=self.panes[pane].viewport.heights.update(index,height);
            let top=-f32::from(self.panes[pane].scroll.offset().y);
            if delta.abs()>0.1 && old_bottom + 32.0 <= top {
                // 屏外资源就绪/窗口精确测量改变前方高度时维持当前阅读位置。
                self.panes[pane].scroll.set_offset(gpui::point(px(0.0),px(-(top+delta).max(0.0))));
                self.panes[pane].viewport.last_top=(top+delta).max(0.0);
            }
            if delta.abs()>0.1 { cx.notify(); }
        }
        self.schedule_pane(pane, 16,cx);
    }
    fn pane_tick(&mut self, pane: usize, cx: &mut Context<Self>) {
        if !self.production || self.hidden || self.navigating || self.saving || self.close_prompt { return; }
        let top=(-f32::from(self.panes[pane].scroll.offset().y)-32.0).max(0.0);
        let height=f32::from(self.panes[pane].scroll.bounds().size.height).max(1.0);
        let now=self.panes[pane].viewport.started.elapsed().as_secs_f64()*1000.0;
        if (top-self.panes[pane].viewport.last_top).abs()>0.5 { self.panes[pane].viewport.demand.sample(top,height,now); self.panes[pane].viewport.last_top=top; }
        if self.panes[pane].viewport.loading { return; }
        if self.panes[pane].load_error.is_some() { return; }
        if self.panes[pane].stale { self.refresh_stale_pane(pane,cx); return; }
        if self.panes[pane].viewport.dirty {
            if self.panes[pane].control.read(cx).prepare_unmount().is_err() { self.schedule_pane(pane, 300,cx); return; }
            self.refresh_groups(pane, cx);
            return;
        }
        if self.panes[pane].viewport.groups.is_empty() { return; }
        let (before,after,retain_before,retain_after,scrolling)=self.panes[pane].viewport.demand.range(height,now);
        let first=self.panes[pane].viewport.heights.at((top-before).max(0.0));
        let last=self.panes[pane].viewport.heights.at(top+height+after);
        self.update_pane_cost(pane, cx);
        let pressured=self.panes[pane].viewport.budget.pressured();
        let retain_first=self.panes[pane].viewport.heights.at(if pressured {top} else {(top-retain_before).max(0.0)});
        let retain_last=self.panes[pane].viewport.heights.at(top+height+if pressured {0.0} else {retain_after});
        let from=self.panes[pane].viewport.groups[retain_first].from;
        let to=self.panes[pane].viewport.groups[retain_last].to;
        let mounted_before=self.panes[pane].viewport.mounts.len();
        let control=self.panes[pane].control.clone();
        self.panes[pane].viewport.mounts.retain(|mount| {
            let id=&mount.view.read(cx).host.parsed.identity;
            if mount.view==control || (id.to_byte>from && id.from_byte<to) { return true; }
            mount.view.update(cx, |v,cx|v.detach(false,cx)).is_err()
        });
        self.update_pane_cost(pane, cx);
        let visible_first=self.panes[pane].viewport.heights.at(top);
        let visible_last=self.panes[pane].viewport.heights.at(top+height);
        let mut candidates:Vec<_>=(first..=last).collect();
        candidates.sort_by_key(|&i| if i>=visible_first && i<=visible_last { (0, i.abs_diff(visible_first)) } else { (1,i.abs_diff(visible_first)) });
        for index in candidates {
            let group=self.panes[pane].viewport.groups[index].clone();
            if !group.semantic || self.panes[pane].viewport.failed.contains(&group.id) || self.panes[pane].viewport.mounts.iter().any(|m| {let id=&m.view.read(cx).host.parsed.identity;id.from_byte==group.from && id.to_byte==group.to}) { continue; }
            let required=index>=visible_first && index<=visible_last;
            if !required {
                if scrolling || self.panes[pane].control.read(cx).prepare_unmount().is_err() { self.schedule_pane(pane, 300,cx); return; }
            }
            let reservation=Cost {views:1,source:(group.to-group.from) as usize,layout:(group.to-group.from) as usize};
            if !self.panes[pane].viewport.budget.reserve(reservation,required) { break; }
            self.panes[pane].viewport.reserved=reservation;
            self.mount_group(pane, group.clone(),cx);
            return;
        }
        if self.panes[pane].viewport.mounts.len()!=mounted_before { cx.notify(); }
    }
    fn refresh_groups(&mut self, pane: usize, cx: &mut Context<Self>) {
        self.panes[pane].viewport.loading=true;
        let identity=self.panes[pane].control.read(cx).host.parsed.identity.clone();
        let port=self.port.clone();
        let request=self.panes[pane].viewport.request.clone();
        let generation=self.panes[pane].viewport.generation;
        let pane_id=self.panes[pane].id;
        let retained=vec![(identity.from_byte,identity.to_byte)];
        let task=cx.background_executor().spawn(async move {port.window_groups(identity.revision,&retained,&request)});
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if !host.pane_request_current(pane,pane_id,generation) { return; }
                host.panes[pane].viewport.loading=false;
                match result {
                    Ok(groups)=> {
                        let id=&host.panes[pane].control.read(cx).host.parsed.identity;
                        if !groups.is_empty() && !groups.iter().any(|g|g.from==id.from_byte&&g.to==id.to_byte) {
                            // 删除结构边界后旧窗口不再独立，安全结束交互后重取真实完整结构。
                            let byte=host.panes[pane].control.read(cx).host.selection.head_byte;
                            let selection=host.panes[pane].control.read(cx).host.selection;
                            host.refresh_pane_at(pane,byte,Some(selection),cx); return;
                        }
                        let old_top=(-f32::from(host.panes[pane].scroll.offset().y)-32.0).max(0.0);
                        let (anchor,offset)=if host.panes[pane].viewport.groups.is_empty() {(id.from_byte,old_top)} else {let index=host.panes[pane].viewport.heights.at(old_top);(host.panes[pane].viewport.groups[index].from,old_top-host.panes[pane].viewport.heights.top(index))};
                        // 刷新结构索引不等于所有窗口几何失效。回收控件后仍保留已测高度，
                        // 否则每次输入都会把屏外窗口退回估值，长文档阅读位置随之跳变。
                        host.panes[pane].viewport.heights=Heights::new(groups.iter().map(|g| {
                            if let Some(mount)=host.panes[pane].viewport.mounts.iter().find(|m| {let id=&m.view.read(cx).host.parsed.identity;id.from_byte==g.from&&id.to_byte==g.to}) {return mount.height;}
                            if let Ok(index)=host.panes[pane].viewport.groups.binary_search_by_key(&g.from,|old|old.from) {
                                if host.panes[pane].viewport.groups[index].to==g.to {return host.panes[pane].viewport.heights.top(index+1)-host.panes[pane].viewport.heights.top(index);}
                            }
                            g.estimate
                        }));
                        host.panes[pane].viewport.groups=groups;
                        host.panes[pane].viewport.dirty=false;
                        if let Some(index)=host.panes[pane].viewport.groups.iter().position(|g|g.contains(anchor)) {
                            let top=host.panes[pane].viewport.heights.top(index)+offset;
                            host.panes[pane].scroll.set_offset(gpui::point(px(0.0),px(-top)));
                            host.panes[pane].viewport.last_top=top;
                            let now=host.panes[pane].viewport.started.elapsed().as_secs_f64()*1000.0;
                            host.panes[pane].viewport.demand.reset(top,now);
                        }
                        host.schedule_pane(pane, 16,cx);
                    }
                    Err(error)=> { host.panes[pane].viewport.dirty=false; host.message=error.to_string(); }
                }
                cx.notify();
            });
        }).detach();
    }
    fn mount_group(&mut self, pane: usize, group:Group, cx:&mut Context<Self>) {
        self.panes[pane].viewport.loading=true;
        let revision=self.panes[pane].control.read(cx).host.parsed.identity.revision;
        let generation=self.panes[pane].viewport.generation;
        let pane_id=self.panes[pane].id;
        let request=self.panes[pane].viewport.request.clone();
        let port=self.port.clone();
        let started=Instant::now();
        let from=group.from; let to=group.to;
        let task=cx.background_executor().spawn(async move {port.load_group(revision,from,Some(to),&request)});
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if !host.pane_request_current(pane,pane_id,generation) { return; }
                host.panes[pane].viewport.loading=false;
                host.panes[pane].viewport.reserved=Cost::default();
                let result=result.and_then(|loaded| {
                    if loaded.input.from_byte!=from || loaded.input.to_byte!=to {return Err(ControlError::NeedsContext("结构窗口边界已变化".into()));}
                    WindowHostAdapter::new(loaded.input,format!("pane-{pane_id}-{}-{generation}",group.id),generation,loaded.context)
                });
                match result {
                    Ok(mut adapter)=> {
                        adapter.set_readonly(true);
                        let mode=host.panes[pane].mode;
                        let base_dir=host.panes[pane].control.read(cx).base_dir.clone();
                        let embedded=host.embedded;
                        let view=cx.new(|cx| {let mut view=EditorControl::new(adapter,cx);view.mode=mode;view.base_dir=base_dir;view.workspace_hosted=embedded;view});
                        let subscription=cx.subscribe(&view,Self::event);
                        host.panes[pane].viewport.mounts.push(Mounted{view,_subscription:subscription,height:group.estimate});
                        host.panes[pane].viewport.demand.prepared(started.elapsed().as_secs_f64()*1000.0);
                    }
                    Err(error)=> {host.panes[pane].viewport.failed.insert(group.id);host.message=error.to_string();}
                }
                host.update_pane_cost(pane, cx);
                host.schedule_pane(pane, crate::editor_control::session_window::policy().batch_ms,cx);
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn pane_children(&self, pane: usize, cx:&Context<Self>) -> Vec<gpui::AnyElement> {
        if self.hidden { return Vec::new(); }
        if !self.production || self.panes[pane].viewport.groups.is_empty() { return vec![self.pane_mount(self.panes[pane].control.clone(),cx)]; }
        let mut children=Vec::new();
        let mut mounts:Vec<_>=self.panes[pane].viewport.mounts.iter().filter_map(|mount| {
            let id=&mount.view.read(cx).host.parsed.identity;
            let index=self.panes[pane].viewport.groups.binary_search_by_key(&id.from_byte,|g|g.from).ok()?;
            (self.panes[pane].viewport.groups[index].to==id.to_byte).then_some((index,mount))
        }).collect();
        mounts.sort_by_key(|(index,_)|*index);
        let mut bottom=0.0;
        // 滚动热路径只处理有界挂载集合与连续占位，不遍历全部正文/结构组。
        for (index,mount) in mounts {
            let top=self.panes[pane].viewport.heights.top(index);
            if top>bottom {children.push(div().h(px(top-bottom)).flex_shrink_0().into_any_element());}
            let view=mount.view.clone();
            children.push(self.pane_mount(view,cx));
            bottom=self.panes[pane].viewport.heights.top(index+1);
        }
        let total=self.panes[pane].viewport.heights.total();
        if total>bottom {children.push(div().h(px(total-bottom)).flex_shrink_0().into_any_element());}
        children
    }
    fn pane_mount(&self, view:Entity<EditorControl>, cx:&Context<Self>) -> gpui::AnyElement {
        div().child(view.clone()).capture_any_mouse_down(cx.listener(move |host,event:&gpui::MouseDownEvent,window,cx| {
            if event.button==gpui::MouseButton::Left && !host.promote(&view,cx) {
                window.focus(&host.control.focus_handle(cx));cx.stop_propagation();
            }
        })).into_any_element()
    }
}

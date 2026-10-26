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
    generation: u64,
    loading: bool,
    dirty: bool,
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

impl ControlDemo {
    pub(super) fn pause_viewport(&mut self, cx:&Context<Self>) {
        self.viewport.request.cancel(); self.viewport.request=WindowRequest::default();
        self.viewport.generation+=1; self.viewport.loading=false; self.viewport.timer=None;
        self.viewport.reserved=Cost::default(); self.update_resident_cost(cx);
    }
    pub(super) fn update_resident_cost(&self, cx:&Context<Self>) {
        self.viewport.budget.update(self.viewport.mounts.iter().fold(self.viewport.reserved,|cost,m| {let view=m.view.read(cx); Cost {views:cost.views+1,source:cost.source+view.host.parsed.source.len(),layout:cost.layout+view.estimated_bytes()}}));
    }
    pub(super) fn reset_viewport(&mut self, cx: &mut Context<Self>) {
        self.viewport.request.cancel();
        self.viewport.generation += 1;
        self.viewport.request = WindowRequest::default();
        self.viewport.loading = false;
        self.viewport.reserved=Cost::default();
        self.viewport.dirty = true;
        self.viewport.failed.clear();
        self.viewport.groups.clear();
        self.viewport.heights = Heights::default();
        self.retain_active(cx);
        if !self.viewport.mounts.iter().any(|m|m.view == self.control) {
            self.viewport.mounts.push(Mounted { view:self.control.clone(), _subscription:cx.subscribe(&self.control, Self::event), height:32.0 });
        }
        self.update_resident_cost(cx);
        self.schedule_viewport(16, cx);
    }
    fn retain_active(&mut self, cx: &mut Context<Self>) {
        self.viewport.mounts.retain(|mount| {
            if mount.view == self.control { return true; }
            // 任何未完成交互都不得被预算或滚动回收吞掉。
            mount.view.update(cx, |view,cx| view.detach(false,cx)).is_err()
        });
    }
    pub(super) fn schedule_viewport(&mut self, ms: u64, cx: &mut Context<Self>) {
        if !self.production || self.hidden || self.viewport.timer.is_some() { return; }
        self.viewport.timer = Some(cx.spawn(async move |this,cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            let _=this.update(cx, |host,cx| { host.viewport.timer=None; host.viewport_tick(cx); });
        }));
    }
    pub(super) fn viewport_edited(&mut self, edits: &[SourceEdit], cx: &mut Context<Self>) {
        if !self.production { return; }
        self.viewport.request.cancel();
        self.viewport.request=WindowRequest::default();
        self.viewport.generation+=1;
        self.viewport.loading=false;
        self.viewport.reserved=Cost::default();
        self.viewport.dirty=true;
        self.viewport.failed.clear();
        // 只有活动控件拥有乐观输入。其他窗口按已确认补丁移动占位，重新查询依赖。
        // 引用定义可能影响远处窗口，不能以范围不相交推断其语义仍然有效。
        self.retain_active(cx);
        self.update_resident_cost(cx);
        for group in &mut self.viewport.groups {
            let before = edits.iter().filter(|edit| edit.to_byte <= group.from && edit.from_byte < group.from).map(|e| e.inserted_text.len() as i128-(e.to_byte-e.from_byte) as i128).sum::<i128>();
            let active=group.from==self.control.read(cx).host.parsed.identity.from_byte;
            let within = edits.iter().filter(|edit| edit.from_byte >= group.from && (edit.from_byte < group.to || (active && edit.from_byte==group.to))).map(|e| e.inserted_text.len() as i128-(e.to_byte-e.from_byte) as i128).sum::<i128>();
            group.from=(group.from as i128+before).max(0) as u64;
            group.to=(group.to as i128+before+within).max(group.from as i128) as u64;
        }
        self.viewport.timer=None;
        self.schedule_viewport(crate::editor_control::session_window::policy().quiet_ms, cx);
    }
    pub(super) fn promote(&mut self, view: &Entity<EditorControl>, cx: &mut Context<Self>) -> bool {
        if *view == self.control { return true; }
        if self.navigating || self.saving || self.close_prompt || self.hidden || self.viewport.dirty { return false; }
        if !self.viewport.mounts.iter().any(|m| &m.view==view) { return false; }
        if let Err(error)=self.control.read(cx).prepare_unmount() {
            self.message=error.to_string(); self.focus_pending=true; return false;
        }
        if view.read(cx).host.parsed.identity.revision != self.control.read(cx).host.parsed.identity.revision { return false; }
        self.control.update(cx, |v,_| v.host.set_readonly(true));
        self.control=view.clone();
        self.control.update(cx, |v,_| v.host.set_readonly(false));
        true
    }
    pub(super) fn viewport_height(&mut self, view: &Entity<EditorControl>, stamp: &LayoutStamp, height: f32, cx: &mut Context<Self>) {
        if stamp.identity != view.read(cx).host.parsed.identity || !height.is_finite() || height<1.0 { return; }
        if let Some(mount)=self.viewport.mounts.iter_mut().find(|m| &m.view==view) { mount.height=height; }
        if let Some(index)=self.viewport.groups.iter().position(|g| g.from==stamp.identity.from_byte && g.to==stamp.identity.to_byte) {
            let old_bottom=self.viewport.heights.top(index+1);
            let delta=self.viewport.heights.update(index,height);
            let top=-f32::from(self.scroll.offset().y);
            if delta.abs()>0.1 && old_bottom + 32.0 <= top {
                // 屏外资源就绪/窗口精确测量改变前方高度时维持当前阅读位置。
                self.scroll.set_offset(gpui::point(px(0.0),px(-(top+delta).max(0.0))));
                self.viewport.last_top=(top+delta).max(0.0);
            }
            if delta.abs()>0.1 { cx.notify(); }
        }
        self.schedule_viewport(16,cx);
    }
    fn viewport_tick(&mut self, cx: &mut Context<Self>) {
        if !self.production || self.hidden || self.navigating || self.saving || self.close_prompt { return; }
        let top=(-f32::from(self.scroll.offset().y)-32.0).max(0.0);
        let height=f32::from(self.scroll.bounds().size.height).max(1.0);
        let now=self.viewport.started.elapsed().as_secs_f64()*1000.0;
        if (top-self.viewport.last_top).abs()>0.5 { self.viewport.demand.sample(top,height,now); self.viewport.last_top=top; }
        if self.viewport.loading { return; }
        if self.viewport.dirty {
            if self.control.read(cx).prepare_unmount().is_err() { self.schedule_viewport(300,cx); return; }
            self.refresh_groups(cx);
            return;
        }
        if self.viewport.groups.is_empty() { return; }
        let (before,after,retain_before,retain_after,scrolling)=self.viewport.demand.range(height,now);
        let first=self.viewport.heights.at((top-before).max(0.0));
        let last=self.viewport.heights.at(top+height+after);
        self.update_resident_cost(cx);
        let pressured=self.viewport.budget.pressured();
        let retain_first=self.viewport.heights.at(if pressured {top} else {(top-retain_before).max(0.0)});
        let retain_last=self.viewport.heights.at(top+height+if pressured {0.0} else {retain_after});
        let from=self.viewport.groups[retain_first].from;
        let to=self.viewport.groups[retain_last].to;
        let mounted_before=self.viewport.mounts.len();
        self.viewport.mounts.retain(|mount| {
            let id=&mount.view.read(cx).host.parsed.identity;
            if mount.view==self.control || (id.to_byte>from && id.from_byte<to) { return true; }
            mount.view.update(cx, |v,cx|v.detach(false,cx)).is_err()
        });
        self.update_resident_cost(cx);
        let visible_first=self.viewport.heights.at(top);
        let visible_last=self.viewport.heights.at(top+height);
        let mut candidates:Vec<_>=(first..=last).collect();
        candidates.sort_by_key(|&i| if i>=visible_first && i<=visible_last { (0, i.abs_diff(visible_first)) } else { (1,i.abs_diff(visible_first)) });
        for index in candidates {
            let group=&self.viewport.groups[index];
            if !group.semantic || self.viewport.failed.contains(&group.id) || self.viewport.mounts.iter().any(|m| {let id=&m.view.read(cx).host.parsed.identity;id.from_byte==group.from && id.to_byte==group.to}) { continue; }
            let required=index>=visible_first && index<=visible_last;
            if !required {
                if scrolling || self.control.read(cx).prepare_unmount().is_err() { self.schedule_viewport(300,cx); return; }
            }
            let reservation=Cost {views:1,source:(group.to-group.from) as usize,layout:(group.to-group.from) as usize};
            if !self.viewport.budget.reserve(reservation,required) { break; }
            self.viewport.reserved=reservation;
            self.mount_group(group.clone(),cx);
            return;
        }
        if self.viewport.mounts.len()!=mounted_before { cx.notify(); }
    }
    fn refresh_groups(&mut self, cx: &mut Context<Self>) {
        self.viewport.loading=true;
        let identity=self.control.read(cx).host.parsed.identity.clone();
        let port=self.port.clone();
        let request=self.viewport.request.clone();
        let generation=self.viewport.generation;
        let retained=vec![(identity.from_byte,identity.to_byte)];
        let task=cx.background_executor().spawn(async move {port.window_groups(identity.revision,&retained,&request)});
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if host.viewport.generation!=generation { return; }
                host.viewport.loading=false;
                match result {
                    Ok(groups)=> {
                        let id=&host.control.read(cx).host.parsed.identity;
                        if !groups.is_empty() && !groups.iter().any(|g|g.from==id.from_byte&&g.to==id.to_byte) {
                            // 删除结构边界后旧窗口不再独立，安全结束交互后重取真实完整结构。
                            let byte=host.control.read(cx).host.selection.head_byte;
                            let selection=host.control.read(cx).host.selection;
                            host.load_at(byte,Some(selection),cx); return;
                        }
                        let old_top=(-f32::from(host.scroll.offset().y)-32.0).max(0.0);
                        let (anchor,offset)=if host.viewport.groups.is_empty() {(id.from_byte,old_top)} else {let index=host.viewport.heights.at(old_top);(host.viewport.groups[index].from,old_top-host.viewport.heights.top(index))};
                        host.viewport.heights=Heights::new(groups.iter().map(|g|host.viewport.mounts.iter().find(|m| {let id=&m.view.read(cx).host.parsed.identity;id.from_byte==g.from&&id.to_byte==g.to}).map_or(g.estimate,|m|m.height)));
                        host.viewport.groups=groups;
                        host.viewport.dirty=false;
                        if let Some(index)=host.viewport.groups.iter().position(|g|g.contains(anchor)) {
                            let top=host.viewport.heights.top(index)+offset;
                            host.scroll.set_offset(gpui::point(px(0.0),px(-top)));
                            host.viewport.last_top=top;
                            host.viewport.demand.reset(top,host.viewport.started.elapsed().as_secs_f64()*1000.0);
                        }
                        host.schedule_viewport(16,cx);
                    }
                    Err(error)=> { host.viewport.dirty=false; host.message=error.to_string(); }
                }
                cx.notify();
            });
        }).detach();
    }
    fn mount_group(&mut self, group:Group, cx:&mut Context<Self>) {
        self.viewport.loading=true;
        let revision=self.control.read(cx).host.parsed.identity.revision;
        let generation=self.viewport.generation;
        let request=self.viewport.request.clone();
        let port=self.port.clone();
        let started=Instant::now();
        let from=group.from; let to=group.to;
        let task=cx.background_executor().spawn(async move {port.load_group(revision,from,Some(to),&request)});
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if host.viewport.generation!=generation { return; }
                host.viewport.loading=false;
                host.viewport.reserved=Cost::default();
                let result=result.and_then(|loaded| {
                    if loaded.input.from_byte!=from || loaded.input.to_byte!=to {return Err(ControlError::NeedsContext("结构窗口边界已变化".into()));}
                    WindowHostAdapter::new(loaded.input,format!("native-{}-{generation}",group.id),generation,loaded.context)
                });
                match result {
                    Ok(mut adapter)=> {
                        adapter.set_readonly(true);
                        let mode=host.control.read(cx).mode;
                        let base_dir=host.control.read(cx).base_dir.clone();
                        let view=cx.new(|cx| {let mut view=EditorControl::new(adapter,cx);view.mode=mode;view.base_dir=base_dir;view});
                        let subscription=cx.subscribe(&view,Self::event);
                        host.viewport.mounts.push(Mounted{view,_subscription:subscription,height:group.estimate});
                        host.viewport.demand.prepared(started.elapsed().as_secs_f64()*1000.0);
                    }
                    Err(error)=> {host.viewport.failed.insert(group.id);host.message=error.to_string();}
                }
                host.update_resident_cost(cx);
                host.schedule_viewport(crate::editor_control::session_window::policy().batch_ms,cx);
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn viewport_children(&self, cx:&Context<Self>) -> Vec<gpui::AnyElement> {
        if self.hidden { return Vec::new(); }
        if !self.production || self.viewport.groups.is_empty() { return vec![self.control.clone().into_any_element()]; }
        let mut children=Vec::new();
        let mut mounts:Vec<_>=self.viewport.mounts.iter().filter_map(|mount| {
            let id=&mount.view.read(cx).host.parsed.identity;
            let index=self.viewport.groups.binary_search_by_key(&id.from_byte,|g|g.from).ok()?;
            (self.viewport.groups[index].to==id.to_byte).then_some((index,mount))
        }).collect();
        mounts.sort_by_key(|(index,_)|*index);
        let mut bottom=0.0;
        // 滚动热路径只处理有界挂载集合与连续占位，不遍历全部正文/结构组。
        for (index,mount) in mounts {
            let top=self.viewport.heights.top(index);
            if top>bottom {children.push(div().h(px(top-bottom)).flex_shrink_0().into_any_element());}
            let view=mount.view.clone();
            children.push(div().child(view.clone()).capture_any_mouse_down(cx.listener(move |host,event:&gpui::MouseDownEvent,window,cx| {
                if event.button==gpui::MouseButton::Left && !host.promote(&view,cx) {
                    window.focus(&host.control.focus_handle(cx)); cx.stop_propagation();
                }
            })).into_any_element());
            bottom=self.viewport.heights.top(index+1);
        }
        let total=self.viewport.heights.total();
        if total>bottom {children.push(div().h(px(total-bottom)).flex_shrink_0().into_any_element());}
        children
    }
}

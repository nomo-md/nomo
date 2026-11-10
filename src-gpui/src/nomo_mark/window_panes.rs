//! 同一会话的窗口化视图。双栏各自调度片段；事务和历史仍只有 DocumentHost 一个入口。
use super::*;
use crate::settings::{EditorMode, Settings};
use std::{sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};

fn mini_paper_gutter(width: f32) -> f32 { (width * 0.05).clamp(18.0, 30.0) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OutlineLayout {
    Visible { shift_fraction: f32 },
    Hidden,
    ManualOverlay,
}

impl OutlineLayout {
    pub(crate) const PANEL_WIDTH: f32 = 220.0;
    pub(crate) const RIGHT_INSET: f32 = 16.0;
    pub(super) const CENTERED: Self = Self::Visible { shift_fraction: 0.0 };

    pub(super) fn with_manual_open(self, open: bool) -> Self {
        if open { Self::ManualOverlay } else if self == Self::ManualOverlay { Self::Hidden } else { self }
    }

    fn for_area(self, width: f32, percent: f32, split: bool) -> Self {
        // 用户在空间不足时点开的浮层保留到关闭或自然放得下，只影响当前文档。
        let when_narrow = if self == Self::ManualOverlay { Self::ManualOverlay } else { Self::Hidden };
        if split { return if width > 900.0 { Self::CENTERED } else { when_narrow }; }
        let right_margin = (width - width * percent) / 2.0;
        let missing = Self::PANEL_WIDTH + Self::RIGHT_INSET + 12.0 - right_margin;
        let max_shift = (width * 0.03).min(48.0);
        // 重新显示大纲时多等 8px 余量，避免调整正文或侧栏宽度时反复显隐。
        let threshold = if matches!(self, Self::Visible { .. }) { max_shift } else { (max_shift - 8.0).max(0.0) };
        if missing > threshold { when_narrow } else { Self::Visible { shift_fraction: missing.max(0.0) / width } }
    }
}

#[test]
fn outline_content_column_preserves_percentage_and_outline_clearance() {
    use OutlineLayout::{Hidden, ManualOverlay, Visible};
    let initial = OutlineLayout::CENTERED;
    for width in [420.0, 900.0, 1000.0, 1200.0, 1600.0, 4000.0] {
        for percent in 30..=100 {
            let fraction = percent as f32 / 100.0;
            let margin = (width - width * fraction) / 2.0;
            let max_shift = (width * 0.03_f32).min(48.0);
            match initial.for_area(width, fraction, false) {
                Visible { shift_fraction } => {
                    let text_shift = width * shift_fraction;
                    assert!(text_shift <= max_shift + 0.01 && margin >= text_shift);
                    assert!((margin + text_shift) >= 248.0 - 0.01);
                }
                Hidden => assert!(margin + max_shift < 248.0),
                ManualOverlay => panic!("自动布局不能自行打开浮层"),
            }
        }
    }
    assert_eq!(initial.for_area(1200.0, 0.60, false), Visible { shift_fraction: 8.0 / 1200.0 });
    let hidden = initial.for_area(1200.0, 0.75, false);
    assert_eq!(hidden, Hidden);
    assert_eq!(initial.for_area(1600.0, 0.75, false), Visible { shift_fraction: 48.0 / 1600.0 });
    assert_eq!(hidden.for_area(1600.0, 0.75, false), Hidden);
    assert_eq!(hidden.for_area(1600.0, 0.74, false), Visible { shift_fraction: 40.0 / 1600.0 });
    let manual = hidden.with_manual_open(true);
    assert_eq!(manual.for_area(1200.0, 0.90, false), ManualOverlay);
    assert_eq!(manual.for_area(800.0, 0.60, true), ManualOverlay);
    assert_eq!(manual.with_manual_open(false).for_area(1200.0, 0.90, false), Hidden);
    let automatic = manual.for_area(1600.0, 0.60, false);
    assert_eq!(automatic, initial);
    assert_eq!(automatic.for_area(1200.0, 0.90, false), Hidden);
}

#[derive(Clone, Copy)]
enum ReadingEdge { Top, Bottom }
#[derive(Clone, Copy)]
struct OutlineScrollMotion { from: gpui::Pixels, previous: gpui::Pixels, started: Instant, duration: Duration }
#[derive(Clone, Copy)]
pub(super) struct ReadingAnchor { byte: u64, fraction: f32, revision: Option<u64>, view_only: bool, edge: Option<ReadingEdge>, motion: Option<OutlineScrollMotion> }

const SCROLLBAR_EDGE_WIDTH: f32 = 20.0;
const SCROLLBAR_IDLE: Duration = Duration::from_millis(900);

#[derive(Default)]
struct ScrollbarActivity {
    last_offset: Option<gpui::Pixels>,
    visible_until: Option<Instant>,
    hide_timer: Option<gpui::Task<()>>,
}

pub(super) struct Pane {
    pub id: u64,
    pub control: Entity<NomoMark>,
    pub mode: ViewMode,
    pub viewport: viewport_ui::Viewport,
    pub scroll: gpui::ScrollHandle,
    pub(super) scrollbar_drag: Option<f32>,
    scrollbar_activity: ScrollbarActivity,
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
            scrollbar_drag:None, scrollbar_activity:ScrollbarActivity::default(), reveal_selection:false, stale:false, load_error:None, reading_restore:None, sync_requested:false, layout_width:None }
    }

    fn recently_scrolled(&mut self, offset: gpui::Pixels, cx: &mut Context<DocumentHost>) -> bool {
        let activity = &mut self.scrollbar_activity;
        let now = Instant::now();
        if activity.last_offset.replace(offset).is_some_and(|previous| previous != offset) {
            activity.visible_until = Some(now + SCROLLBAR_IDLE);
        }
        let Some(delay) = activity.visible_until.and_then(|end| end.checked_duration_since(now)) else { return false; };
        // 连续滚动只延后截止时间。到期重绘时如果又滚过，再等待余下的时间。
        if activity.hide_timer.is_none() {
            let id = self.id;
            activity.hide_timer = Some(cx.spawn(async move |host, cx| {
                cx.background_executor().timer(delay).await;
                let _ = host.update(cx, |host, cx| {
                    if let Some(pane) = host.panes.iter_mut().find(|pane| pane.id == id) {
                        pane.scrollbar_activity.hide_timer = None;
                        cx.notify();
                    }
                });
            }));
        }
        true
    }

    pub(super) fn clear_scrollbar(&mut self) {
        self.scrollbar_drag = None;
        self.scrollbar_activity = ScrollbarActivity::default();
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
    pub(super) fn cancel_pane_animation(&mut self, pane:usize) {
        if self.panes[pane].reading_restore.is_some_and(|anchor| anchor.motion.is_some()) {
            self.panes[pane].reading_restore=None;
        }
    }
    pub(super) fn animated_outline_byte(&self, pane:usize) -> Option<u64> {
        self.panes[pane].reading_restore.filter(|anchor| anchor.motion.is_some()).map(|anchor| anchor.byte)
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
    pub(crate) fn workspace_reading_position(&self, cx: &gpui::App) -> Option<(u64, f32)> {
        self.panes[self.active_pane].reading_restore.or_else(|| self.reading_anchor(self.active_pane, cx))
            .map(|anchor| (anchor.byte, anchor.fraction))
    }
    pub(crate) fn restore_workspace_position(&mut self, byte:u64, fraction:f32, mut selection:DocumentSelection, cx:&mut Context<Self>) {
        selection.anchor_byte=selection.anchor_byte.min(self.total_bytes);
        selection.head_byte=selection.head_byte.min(self.total_bytes);
        self.control.update(cx,|view,cx|view.restore_session_selection(selection,cx));
        let pane=&mut self.panes[self.active_pane];
        pane.reveal_selection=false;
        pane.reading_restore=Some(ReadingAnchor {byte:byte.min(self.total_bytes),fraction:fraction.clamp(0.0,1.0),revision:None,view_only:false,edge:None,motion:None});
        self.schedule_viewport(16,cx);
    }
    fn reading_anchor(&self, pane:usize, cx:&gpui::App) -> Option<ReadingAnchor> {
        let bounds=self.panes[pane].scroll.bounds();
        let height=f32::from(bounds.size.height);
        if height<=0.0 {return None;}
        // 位于真正的文首/文末时跟随边缘，不能用中部锚点把尾行挤出视口。
        if self.panes[pane].scroll.offset().y>=px(-0.5) && self.visible_source_bounds(pane,0,cx).is_some() {
            return Some(ReadingAnchor {byte:0,fraction:0.0,revision:None,view_only:false,edge:Some(ReadingEdge::Top),motion:None});
        }
        if self.visible_source_bounds(pane,self.total_bytes,cx).is_some() {
            return Some(ReadingAnchor {byte:self.total_bytes,fraction:1.0,revision:None,view_only:false,edge:Some(ReadingEdge::Bottom),motion:None});
        }
        let y=f32::from(bounds.origin.y)+height*0.3;
        self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            mount.view.read(cx).source_at_screen_y(y).map(|(byte,line_y)|ReadingAnchor {
                byte, fraction:((line_y-f32::from(bounds.origin.y))/height).clamp(0.0,1.0), revision:None, view_only:false, edge:None, motion:None
            })
        })
    }
    /// 尾部占位变为真实内容后，首帧按同一正文位置排版，避免估值到实测值的二次跳动。
    pub(super) fn anchor_published_content(&mut self, pane:usize, byte:u64) {
        if self.panes[pane].reading_restore.is_none() && !self.panes[pane].reveal_selection {
            self.panes[pane].reading_restore=Some(ReadingAnchor {byte,fraction:0.0,
                revision:Some(self.status().revision),view_only:true,edge:None,motion:None});
        }
    }
    fn visible_source_bounds(&self, pane:usize, byte:u64, cx:&gpui::App) -> Option<gpui::Bounds<gpui::Pixels>> {
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
            self.cancel_pane_animation(pane);
            let bounds = self.panes[pane].scroll.bounds();
            let height = f32::from(bounds.size.height);
            if height <= 0.0 { continue; }
            let y = pointer.filter(|point| bounds.contains(point)).map_or(
                f32::from(bounds.origin.y) + height * 0.5, |point| f32::from(point.y),
            );
            let anchor = self.panes[pane].viewport.mounts.iter().find_map(|mount| {
                mount.view.read(cx).source_at_screen_y(y).map(|(byte, line_y)| ReadingAnchor {
                    byte, fraction: ((line_y - f32::from(bounds.origin.y)) / height).clamp(0.0, 1.0),
                    revision: None, view_only: true, edge:None, motion:None,
                })
            });
            self.panes[pane].reading_restore = self.panes[pane].reading_restore.or(anchor);
            self.panes[pane].reveal_selection = false;
            self.panes[pane].sync_requested = false;
        }
        cx.notify();
    }
    fn prepare_paper_reflow(&mut self, cx: &mut Context<Self>) {
        let edges: Vec<_> = self.panes.iter().enumerate().map(|(i, pane)| {
            let offset = -pane.scroll.offset().y;
            if offset <= px(0.5) { Some(ReadingEdge::Top) }
            else if offset >= pane.scroll.max_offset().y - px(0.5) && self.visible_source_bounds(i, self.total_bytes, cx).is_some() {
                Some(ReadingEdge::Bottom)
            } else { None }
        }).collect();
        self.prepare_zoom(None, cx);
        // 文首和文末跟随纸面边缘，中间位置跟随原来可见的文字。
        for (pane, edge) in self.panes.iter_mut().zip(edges) {
            match edge {
                Some(ReadingEdge::Top) => { pane.reading_restore = None; pane.scroll.set_offset(gpui::point(px(0.0), px(0.0))); }
                Some(ReadingEdge::Bottom) => pane.reading_restore = Some(ReadingAnchor { byte: self.total_bytes, fraction: 1.0,
                    revision: None, view_only: true, edge, motion: None }),
                None => {}
            }
        }
    }

    /// 小窗只调整已挂载 Pane 的纸面留白，复用阅读锚点与窗口预算。
    pub(crate) fn set_mini_layout(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.mini_layout == active { return; }
        if self.mini_paper_padding.is_none() { self.prepare_paper_reflow(cx); }
        self.mini_paper_padding = None;
        self.mini_layout = active;
        self.suspend_outline_for_mini(active, cx);
        self.control.update(cx, |view, cx| view.dismiss_quick_insert(cx));
        cx.notify();
    }

    fn outline_column_shift(&self, settings: &Settings) -> f32 {
        if settings.show_outline && let OutlineLayout::Visible { shift_fraction } = self.outline_layout { shift_fraction } else { 0.0 }
    }

    pub(crate) fn set_mini_paper_frame(&mut self, frame: Option<(gpui::Pixels, f32)>, cx: &mut Context<Self>) {
        // 原模式切换申请的首帧预量也用此帧宽度，之后走正常可视区布局。
        if let Some((width, _)) = frame && self.panes.len() == 1 && self.panes[0].layout_width.is_some() {
            self.panes[0].layout_width = Some(f32::from(width));
        }
        let padding = frame.map(|(width, progress)| {
            let width = f32::from(width);
            let settings = cx.global::<Settings>();
            let margin = (1.0 - settings.content_width_percent) * 0.5;
            let left = (margin - self.outline_column_shift(settings)).max(0.0);
            let mini = mini_paper_gutter(width);
            let mix = |a: f32, b: f32| px(a + (b - a) * progress);
            gpui::Edges { left: mix(width * left, mini), right: mix(width * (margin * 2.0 - left), mini),
                top: mix(32.0, 48.0), bottom: mix(32.0, 56.0) }
        });
        if self.mini_paper_padding == padding { return; }
        // 开始展开时在小窗里取一次锚点。滚动或输入若主动取消锚点，后续帧不重新捕获。
        if self.mini_paper_padding.is_none() && self.panes.iter().any(|p| p.reading_restore.is_none() && p.scroll.offset().y < px(-0.5)) {
            self.prepare_paper_reflow(cx);
        }
        self.mini_paper_padding = padding;
        cx.notify();
    }

    fn paper_padding(&self, width: f32) -> gpui::Edges<gpui::Pixels> {
        self.mini_paper_padding.unwrap_or_else(|| {
            let gutter = px(if self.mini_layout { mini_paper_gutter(width) } else if self.panes.len() == 2 { 32.0 } else { 0.0 });
            gpui::Edges { left: gutter, right: gutter, top: px(if self.mini_layout { 48.0 } else { 32.0 }),
                bottom: px(if self.mini_layout { 56.0 } else { 32.0 }) }
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
        let restore_reading=state.reading_restore.is_some();
        let port=self.port.clone();
        let revision=self.status().revision;
        let selected=selection.unwrap_or(self.control.read(cx).host.selection);
        let task=cx.background_executor().spawn(async move {
            let loaded=port.load_group(revision,byte,None,&request)?;
            // 阅读导航先准备完整占位坐标，再发布目标片段；不把片段边界临时当作文档边界。
            let groups=if restore_reading {
                Some(port.window_groups(revision,&[(loaded.input.from_byte,loaded.input.to_byte)],&request)?)
            } else {None};
            Ok::<_,ControlError>((loaded,groups))
        });
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|host,cx| {
                if !host.pane_request_current(pane,id,generation) {return;}
                host.panes[pane].viewport.loading=false;
                if host.panes[pane].control!=target || target.read(cx).host.parsed.identity!=identity {
                    host.schedule_pane(pane,150,cx);return;
                }
                if host.status().revision!=revision {host.panes[pane].stale=true;host.schedule_pane(pane,150,cx);return;}
                let result=result.and_then(|(loaded,groups)| {
                    target.update(cx,|view,cx| {
                        let generation=view.host.parsed.identity.view_generation+1;
                        view.update_window(loaded.input,generation,loaded.context,cx)?;
                        view.host.set_readonly(pane!=host.active_pane);
                        // 两栏的选区使用完整文档字节坐标；屏外端点无需截成局部坐标。
                        view.restore_selection(selected,cx);
                        Ok::<_,ControlError>(groups)
                    })
                });
                match result {
                    Ok(groups)=>{
                        host.panes[pane].stale=false;host.reset_pane(pane,cx);
                        if let Some(groups)=groups { host.replace_pane_groups(pane,groups,cx); }
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
        for pane in 0..self.panes.len() { self.cancel_pane_animation(pane); }
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
            let mut adapter=WindowHostAdapter::new(input,window_id,0,context)?;
            adapter.set_editing_locked(self.editing_locked());
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
            for (mut adapter,base_dir,bytes) in neighbors {
                if !self.panes[1].viewport.budget.reserve(super::super::resident_budget::Cost {views:1,source:bytes,layout:0},false) {break;}
                adapter.set_editing_locked(self.editing_locked());
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
        if self.panes[pane].stale || self.panes[pane].load_error.is_some() {
            if let Some(anchor)=&mut self.panes[pane].reading_restore { anchor.motion=None; }
            return;
        }
        if self.panes[pane].viewport.loading && !anchor.view_only && anchor.motion.is_none() {return;}
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
            } else if self.mini_paper_padding.is_none() && self.pane_visible_ready(pane,cx) {self.panes[pane].reading_restore=None;}
            return;
        }
        let located=self.panes[pane].viewport.mounts.iter().find_map(|mount| {
            let view=mount.view.read(cx);
            let id=&view.host.parsed.identity;
            // 分组交界属于后一片段；前一片段的 EOF 光标不是该标题的屏幕位置。
            if anchor.byte<id.from_byte || anchor.byte>=id.to_byte && !(anchor.byte==self.total_bytes && anchor.byte==id.to_byte) { return None; }
            view.caret_bounds(anchor.byte).map(|(_,caret)|(caret,view.measured_height().is_some()))
        });
        if let Some((caret,measured))=located {
            let delta=f32::from(caret.origin.y-bounds.origin.y)-f32::from(bounds.size.height)*anchor.fraction;
            let old=self.panes[pane].scroll.offset();
            // 使用本次 prepaint 后的真实可滚范围；首尾无法满足参考线时在边界结束。
            let max=self.panes[pane].scroll.max_offset().y.max(px(0.0));
            let target=(old.y-px(delta)).clamp(-max,px(0.0));
            if let Some(mut motion)=anchor.motion {
                // 屏外窗口高度修正会重基滚动位置，插值起点也同步移动，避免倒跳。
                motion.from+=old.y-motion.previous;
                let progress=(motion.started.elapsed().as_secs_f32()/motion.duration.as_secs_f32()).min(1.0);
                if progress<1.0 && f32::from(target-motion.from).abs()>=1.0 {
                    // 与旧版大纲一致的 easeOutCubic；目标随本帧实际布局校正。
                    let eased=1.0-(1.0-progress).powi(3);
                    let next=(motion.from+(target-motion.from)*eased).clamp(-max,px(0.0));
                    self.panes[pane].scroll.set_offset(gpui::point(old.x,next));
                    motion.previous=next;
                    if let Some(anchor)=&mut self.panes[pane].reading_restore { anchor.motion=Some(motion); }
                    self.schedule_pane(pane,16,cx);
                    return;
                }
                if let Some(anchor)=&mut self.panes[pane].reading_restore { anchor.motion=None; }
            }
            if f32::from(target-old.y).abs()>0.5 {
                self.panes[pane].scroll.set_offset(gpui::point(old.x,target));
                self.schedule_pane(pane,16,cx);cx.notify();
            } else if measured && self.mini_paper_padding.is_none() && (self.pane_visible_ready(pane,cx) || anchor.view_only) {
                self.panes[pane].reading_restore=None;
            }
        } else {
            // 目标被预算回收或布局失效后走既有加载定位，不继续穿过占位内容做动画。
            if let Some(anchor)=&mut self.panes[pane].reading_restore { anchor.motion=None; }
            if !self.panes[pane].viewport.dirty && !anchor.view_only {
                self.refresh_pane_at(pane,anchor.byte.min(self.total_bytes),None,cx);
            }
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
        if self.split_dragging || interaction.composing || interaction.draft || interaction.dragging {
            self.cancel_pane_animation(pane);
            return;
        }
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
        self.cancel_pane_follow(pane, cx);
        let state=&self.panes[pane];
        let loaded=(!state.stale && !state.viewport.dirty && state.load_error.is_none())
            .then(|| state.viewport.mounts.iter().find_map(|mount| {
                let view=mount.view.read(cx);
                let id=&view.host.parsed.identity;
                if id.revision!=revision || byte<id.from_byte || byte>=id.to_byte { return None; }
                view.measured_height()?;
                view.caret_bounds(byte)?;
                Some(mount.view.clone())
            })).flatten();
        if self.panes[pane].mode == ViewMode::Source {
            let source=loaded.as_ref().and_then(|source| {
                let view=source.read(cx);
                let id=&view.host.parsed.identity;
                let tail=view.host.parsed.source.get((byte-id.from_byte) as usize..)?;
                let end=tail.find(['\r','\n']).or_else(|| (id.to_byte==self.total_bytes).then_some(tail.len()))?;
                Some((source.clone(),byte+end as u64))
            });
            let Some((source,end))=source else {
                self.navigate_outline_source(byte, revision, cx);
                return;
            };
            if !self.promote(&source,cx) { return; }
            source.update(cx,|view,cx| view.restore_session_selection(DocumentSelection::caret(end),cx));
            self.selection_changed(true, cx);
            self.focus_pending=true;
        }
        self.position_outline_heading(byte,revision,loaded.is_some(),cx);
    }
    pub(super) fn position_outline_heading(&mut self, byte:u64, revision:u64, animate:bool, cx:&mut Context<Self>) {
        let pane=self.active_pane;
        let motion=animate.then(|| OutlineScrollMotion {
            from:self.panes[pane].scroll.offset().y, previous:self.panes[pane].scroll.offset().y, started:Instant::now(),
            duration:Duration::from_millis(if crate::ui::popup::reduced_motion(cx) {140}else{260}),
        });
        let fraction=if self.panes[pane].mode==ViewMode::Source {
            (40.0/f32::from(self.panes[pane].scroll.bounds().size.height).max(1.0)).min(0.9)
        } else {0.08};
        self.panes[pane].reveal_selection = false;
        self.panes[pane].reading_restore = Some(ReadingAnchor { byte, fraction, revision:Some(revision), view_only:false, edge:None, motion });
        self.request_pane_sync(pane);
        self.schedule_pane(pane, 16, cx);
        cx.notify();
    }
    fn paper_input_ready(&self, pane: usize, cx: &gpui::App) -> bool {
        !self.hidden && !self.closed && !self.navigating && !self.saving && !self.close_prompt
            && self.panes.get(pane).is_some_and(|pane| !pane.stale && pane.load_error.is_none())
            && !self.control.read(cx).host.interaction.protected()
    }

    fn paper_mouse_down(&mut self, pane: usize, event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.modifiers.shift || event.modifiers.control || event.modifiers.platform || event.modifiers.alt
            || !self.paper_input_ready(pane, cx)
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

    fn paragraph_gutter_target(
        &self, pane: usize, id: u64, view: &Entity<NomoMark>, y: gpui::Pixels, cx: &gpui::App,
    ) -> Option<(std::ops::Range<usize>, std::ops::Range<gpui::Pixels>)> {
        let pane_state = self.panes.get(pane)?;
        if pane_state.id != id || pane_state.scrollbar_drag.is_some() || self.split_dragging
            || !self.paper_input_ready(pane, cx)
        {
            return None;
        }
        let target = view.read(cx);
        if *view != self.control && (pane_state.viewport.dirty
            || target.host.parsed.identity.revision != self.status().revision
            || self.control.read(cx).prepare_unmount().is_err()) {
            return None;
        }
        target.paragraph_at_y(y)
    }

    pub(super) fn paragraph_gutter(&self, pane: usize, view: &Entity<NomoMark>, cx: &Context<Self>) -> impl IntoElement {
        let id = self.panes[pane].id;
        let scroll = self.panes[pane].scroll.clone();
        let zoom = cx.global::<Settings>().zoom;
        let owner = cx.entity().downgrade();
        let view = view.downgrade();
        gpui::canvas(
            move |bounds, window, _| {
                let strip = bounds.intersect(&scroll.bounds()).intersect(&window.content_mask().bounds);
                (!strip.is_empty() && strip.size.width >= px(10.0 * zoom))
                    .then(|| window.insert_hitbox(strip, gpui::HitboxBehavior::Normal))
            },
            move |_, strip, window, cx| {
                let Some(strip) = strip else { return; };
                let position = window.mouse_position();
                let hovered = window.is_window_hovered() && window.is_window_active()
                    && strip.contains(&position) && strip.is_hovered(window)
                    && !window.modifiers().modified() && !cx.has_active_drag() && !window.has_active_prompt();
                if hovered && let Some(view) = view.upgrade()
                    && let Ok(Some((_, band))) = owner.read_with(cx, |host, cx| {
                        host.paragraph_gutter_target(pane, id, &view, position.y, cx)
                    })
                {
                    let top = band.start.max(strip.top());
                    let bottom = band.end.min(strip.bottom());
                    let height = (bottom - top - px(4.0 * zoom)).min(px(16.0 * zoom));
                    if height > px(0.0) {
                        let y = (position.y - height / 2.0).clamp(top, bottom - height);
                        let marker = gpui::Bounds::new(
                            gpui::point(strip.right() - px(4.0 * zoom), y), gpui::size(px(2.0 * zoom), height),
                        );
                        crate::layout::paint_box(marker, cx.global::<Theme>().accent, zoom, None, window);
                        window.set_cursor_style(gpui::CursorStyle::PointingHand, &strip);
                    }
                }
                let repaint = window.current_view();
                let hover_strip = strip.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Capture { return; }
                    let over = hover_strip.contains(&event.position) && hover_strip.is_hovered(window)
                        && event.pressed_button.is_none() && !event.modifiers.modified()
                        && window.is_window_active() && window.is_window_hovered()
                        && !window.has_active_prompt() && !cx.has_active_drag();
                    if over != hovered || over && event.position.y != position.y { cx.notify(repaint); }
                });
                window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble || event.button != gpui::MouseButton::Left
                        || event.modifiers.modified() || !strip.contains(&event.position) || !strip.is_hovered(window)
                        || window.has_active_prompt() || cx.has_active_drag()
                    { return; }
                    let Some(view) = view.upgrade() else { return; };
                    let _ = owner.update(cx, |host, cx| {
                        // 单击按最新排版重新命中，滚动和片段替换不会沿用悬停的字节坐标。
                        let Some((source, _)) = host.paragraph_gutter_target(pane, id, &view, event.position.y, cx) else {
                            return;
                        };
                        if host.promote(&view, cx) {
                            view.update(cx, |view, cx| view.select_paragraph(source, window, cx));
                            cx.stop_propagation();
                        }
                    });
                });
            },
        ).absolute().top_0().bottom_0().left(px(-24.0 * zoom)).w(px(20.0 * zoom))
    }

    fn drag_pane_scrollbar(&mut self, pane:usize, y:gpui::Pixels, track:gpui::Bounds<gpui::Pixels>, thumb_height:f32, cx:&mut Context<Self>) {
        let Some(grab) = self.panes[pane].scrollbar_drag else { return; };
        let travel = f32::from(track.size.height) - thumb_height;
        if travel <= 0.0 { return; }
        let fraction = ((f32::from(y - track.top()) - grab * thumb_height) / travel).clamp(0.0, 1.0);
        let scroll = &self.panes[pane].scroll;
        scroll.set_offset(gpui::point(scroll.offset().x, -scroll.max_offset().y * fraction));
        self.panes[pane].reveal_selection = false;
        self.panes[pane].reading_restore = None;
        self.request_pane_sync(pane);
        self.schedule_pane(pane, 16, cx);
        cx.notify();
    }
    pub(super) fn pane_scrollbar(&self, pane:usize, cx:&Context<Self>) -> impl IntoElement {
        let id = self.panes[pane].id;
        let scroll = self.panes[pane].scroll.clone();
        let owner = cx.entity().downgrade();
        let observer = owner.clone();
        let color = cx.global::<Theme>().scrollbar;
        gpui::canvas(
            move |bounds, window, cx| {
                let max = f32::from(scroll.max_offset().y);
                let visible = f32::from(scroll.bounds().size.height);
                let track = gpui::Bounds::new(bounds.origin + gpui::point(px(0.0), px(2.0)),
                    gpui::size(bounds.size.width, (bounds.size.height - px(4.0)).max(px(0.0))));
                let height = f32::from(track.size.height);
                let offset = scroll.offset().y;
                let (scrolling, dragging) = observer.update(cx, |host, cx| {
                    if host.hidden || host.closed { return None; }
                    let state = host.panes.get_mut(pane).filter(|state| state.id == id)?;
                    if max <= 0.0 || visible <= 0.0 || height <= 0.0 {
                        state.clear_scrollbar();
                        return None;
                    }
                    if !window.is_window_active() { state.scrollbar_drag = None; }
                    Some((state.recently_scrolled(offset, cx), state.scrollbar_drag.is_some()))
                }).ok().flatten()?;
                let thumb_height = (height * visible / (visible + max)).clamp(24.0_f32.min(height), height);
                let top = (-f32::from(offset) / max).clamp(0.0, 1.0) * (height - thumb_height);
                let thumb = gpui::Bounds::new(track.origin + gpui::point(px(3.0), px(top)), gpui::size(px(6.0), px(thumb_height)));
                Some((window.insert_hitbox(bounds, gpui::HitboxBehavior::BlockMouseExceptScroll), track, thumb, scrolling, dragging))
            },
            move |bounds, geometry, window, _| {
                let released = owner.clone();
                window.on_mouse_event(move |event:&gpui::MouseUpEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble || event.button != gpui::MouseButton::Left { return; }
                    let _ = released.update(cx, |host, cx| {
                        if host.panes.get(pane).is_some_and(|state| state.id == id) && host.panes[pane].scrollbar_drag.take().is_some() {
                            cx.stop_propagation(); cx.notify();
                        }
                    });
                });
                let Some((hitbox, track, thumb, scrolling, dragging)) = geometry else { return; };
                // 感应带只决定绘制。真正拦截点击的仍是原来的 12px 轨道。
                let edge = gpui::Bounds::new(gpui::point(bounds.right() - px(SCROLLBAR_EDGE_WIDTH), bounds.top()),
                    gpui::size(px(SCROLLBAR_EDGE_WIDTH), bounds.size.height));
                let near = window.is_window_hovered() && edge.contains(&window.mouse_position());
                if dragging { window.set_window_cursor_style(gpui::CursorStyle::ClosedHand); }
                else { window.set_cursor_style(gpui::CursorStyle::OpenHand, &hitbox); }
                if scrolling || near || dragging { crate::layout::paint_box(thumb, color, 3.0, None, window); }
                let pressed = owner.clone();
                window.on_mouse_event(move |event:&gpui::MouseDownEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble || event.button != gpui::MouseButton::Left || !hitbox.is_hovered(window) { return; }
                    let _ = pressed.update(cx, |host, cx| {
                        if host.panes.get(pane).is_none_or(|state| state.id != id) || host.hidden || host.closed { return; }
                        host.cancel_pane_follow(pane, cx);
                        let height = f32::from(thumb.size.height);
                        let grab = if event.position.y >= thumb.top() && event.position.y <= thumb.bottom() {
                            f32::from(event.position.y - thumb.top()) / height
                        } else { 0.5 };
                        host.panes[pane].scrollbar_drag = Some(grab);
                        host.drag_pane_scrollbar(pane, event.position.y, track, height, cx);
                        cx.stop_propagation();
                    });
                });
                let moved = owner.clone();
                let view_id = window.current_view();
                window.on_mouse_event(move |event:&gpui::MouseMoveEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        if near != (window.is_window_hovered() && edge.contains(&event.position)) { cx.notify(view_id); }
                        return;
                    }
                    if !dragging && event.pressed_button != Some(gpui::MouseButton::Left) { return; }
                    let _ = moved.update(cx, |host, cx| {
                        if host.panes.get(pane).is_some_and(|state| state.id == id && state.scrollbar_drag.is_some()) && !host.hidden && !host.closed {
                            if event.pressed_button != Some(gpui::MouseButton::Left) {
                                host.panes[pane].scrollbar_drag = None;
                                cx.notify();
                                return;
                            }
                            host.drag_pane_scrollbar(pane, event.position.y, track, f32::from(thumb.size.height), cx);
                            cx.stop_propagation();
                        }
                    });
                });
            },
        ).absolute().right_0().top_0().w(px(12.0)).h_full()
    }
    pub(crate) fn set_manual_outline_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.outline_layout = self.outline_layout.with_manual_open(open);
        cx.notify();
    }

    pub(crate) fn outline_can_show(&self) -> bool { self.outline_layout != OutlineLayout::Hidden }

    pub(crate) fn update_outline_layout(&mut self, width: f32, percent: f32, cx: &mut Context<Self>) {
        let previous = if cx.global::<Settings>().show_outline { self.outline_layout } else { self.outline_layout.with_manual_open(false) };
        let layout = previous.for_area(width, percent, self.panes.len() == 2);
        if layout != self.outline_layout {
            self.outline_layout = layout;
            cx.notify();
        }
    }

    fn render_pane(&self, pane:usize, cx:&Context<Self>) -> gpui::AnyElement {
        let host=cx.entity().downgrade();
        let id=self.panes[pane].id;
        let width=self.panes[pane].layout_width.unwrap_or_else(||f32::from(self.panes[pane].scroll.bounds().size.width));
        let settings=cx.global::<Settings>();
        let padding = self.paper_padding(width);
        let single_column = !self.mini_layout && self.panes.len() == 1;
        let mut document_body = div().children(self.pane_children(pane,cx))
            .w(gpui::relative(if single_column { settings.content_width_percent } else { 1.0 })).mx_auto();
        let shift = self.outline_column_shift(settings);
        if single_column && shift > 0.0 {
            let left = (1.0 - settings.content_width_percent) / 2.0 - shift;
            document_body = document_body.ml(gpui::relative(left.max(0.0)));
        }
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
        let content=div().on_children_prepainted(move |_,window,cx| {let _=host.update(cx,|host,cx| {
            if host.panes.get(pane).is_some_and(|p|p.id==id) {
                host.pane_prepainted(pane,cx);
                if !host.hidden && !host.closed && host.animated_outline_byte(pane).is_some() { window.request_animation_frame(); }
            }
        });})
            .id(("document-pane",id)).size_full().min_w_0().min_h_0().overflow_y_scroll()
            .track_scroll(&self.panes[pane].scroll)
            .pt(padding.top).pb(padding.bottom).pl(padding.left).pr(padding.right)
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
                cx.notify();
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
            .child(div().relative().flex_1().min_h_0().child(content).child(zoom_capture).child(self.pane_scrollbar(pane,cx))).into_any_element()
    }
    /// 模式切换、阅读导航及新邻窗在首帧前测量；只处理当前驻留窗口。
    fn prepare_reading_layouts(&mut self, window:&mut Window, cx:&mut Context<Self>) {
        for pane in 0..self.panes.len() {
            let Some(anchor)=self.panes[pane].reading_restore else {continue;};
            let width=self.panes[pane].layout_width;
            let unmeasured=self.panes[pane].viewport.mounts.iter().any(|mount| mount.view.read(cx).content_bounds().is_none());
            if width.is_none() && !unmeasured {continue;}
            let bounds=self.panes[pane].scroll.bounds();
            let height=if bounds.size.height>px(0.0) {f32::from(bounds.size.height)}
                else {self.panes.iter().map(|pane|f32::from(pane.scroll.bounds().size.height)).fold(0.0_f32,f32::max)};
            if height<=0.0 {continue;}
            let width=width.or_else(|| (bounds.size.width>px(0.0)).then_some(f32::from(bounds.size.width)));
            let settings=cx.global::<Settings>();
            let padding=self.paper_padding(width.unwrap_or_default());
            let content_width=width.map(|width| {
                let content=if self.mini_layout || self.panes.len()==2 {width-f32::from(padding.left+padding.right)}
                    else {width * settings.content_width_percent};
                (content*window.scale_factor()).round()/window.scale_factor()
            });
            let views:Vec<_>=self.panes[pane].viewport.mounts.iter().map(|mount|mount.view.clone()).collect();
            let mut located=None;
            for view in views {
                if let Some((stamp,content_height,y))=view.update(cx,|view,cx|view.prepare_reading_layout(content_width,anchor.byte,window,cx)) {
                    self.pane_height(pane,&view,&stamp,content_height,cx);
                    let id=&stamp.identity;
                    if id.from_byte<=anchor.byte && (anchor.byte<id.to_byte || anchor.byte==self.total_bytes && anchor.byte==id.to_byte) {
                        if let Some(y)=y {located=Some((id.from_byte,y));}
                    }
                }
            }
            self.update_pane_cost(pane,cx);
            // 已加载的大纲目标继续由逐帧插值定位，预量邻窗不能提前跳到动画终点。
            if anchor.motion.is_some() {continue;}
            let total=if self.panes[pane].viewport.groups.is_empty() {self.panes[pane].viewport.mounts.first().map_or(0.0,|mount|mount.height)}
                else {self.panes[pane].viewport.heights.total()};
            let (padding_top,padding_bottom)=(f32::from(padding.top),f32::from(padding.bottom));
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

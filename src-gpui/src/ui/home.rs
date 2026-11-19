//! 文档主页：轻量目录入口、收藏、最近文件与按可见行请求的位图预览。
pub(crate) mod library;
mod thumbnail;
mod browser;
mod navigation;
pub(crate) mod folder_card;
pub(crate) mod document_card;

use crate::{theme::Theme, ui::components::{ButtonKind, IconName, button, icon, small_icon_button, tooltip}};
use crate::ui::{motion, popup::reduced_motion};
use crate::ui::components::{SIDEBAR_ROW_HOVER, sidebar_row};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement, MouseButton, ParentElement, Render, RenderImage, SharedString, Styled, Subscription, Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list};
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
use library::{Change, Entry, Library, Snapshot};
use std::{cell::RefCell, collections::{HashMap, VecDeque}, path::{Path, PathBuf}, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use browser::{Browser, Node};
use crate::ui::path_drag::{ItemMotion, PathDrag, PathDragPreview};

const MEMORY_CACHE_BYTES: usize = 24 * 1024 * 1024;
const BITMAP_BYTES: usize = (thumbnail::WIDTH * thumbnail::HEIGHT * 4) as usize;
// 每张位图另留 1 KiB 给标题和指纹，避免超宽窗口内的预览相互淘汰并反复加载。
const MAX_VISIBLE_PREVIEWS: usize = MEMORY_CACHE_BYTES / (BITMAP_BYTES + 1024);
const CARD_PADDING: f32 = 16.0;
const CARD_TITLE_HEIGHT: f32 = 40.0;
const CARD_PREVIEW_GAP: f32 = 12.0;
const CARD_FOOTER_HEIGHT: f32 = 18.0;
const CARD_LIFT: f32 = 3.0;
const PREVIEW_FADE: Duration = Duration::from_millis(140);
const MIN_CARD_WIDTH: f32 = 240.0;
const MAX_CARD_WIDTH: f32 = 300.0;
const GRID_GAP: f32 = 16.0;
const GRID_SHADOW_INSET: f32 = 12.0;
pub(super) const COMPACT_WINDOW_WIDTH: f32 = 850.0;

fn card_height(width: f32) -> f32 {
    CARD_PADDING * 2.0 + 2.0 + CARD_TITLE_HEIGHT + CARD_PREVIEW_GAP * 2.0 + CARD_FOOTER_HEIGHT
        + (width - CARD_PADDING * 2.0 - 2.0) * thumbnail::HEIGHT as f32 / thumbnail::WIDTH as f32
}

#[derive(Clone)]
pub(crate) enum HomeEvent {
    Open(PathBuf), OpenFavoriteFolder(PathBuf), NewFile, OpenFile, OpenFolder, Resume, Restore,
    Place { item: PathDrag, placement: crate::ui::tree_drag::Placement },
}

enum Preview { Ready { image: Arc<RenderImage>, title: SharedString, fingerprint: String }, Failed(UiMessage) }
struct CachedPreview { preview: Preview, used: u64, stale: bool, fade_started: Option<Instant> }

impl CachedPreview {
    fn fade_progress(&self, now: Instant) -> f32 {
        self.fade_started.map_or(1.0, |started| {
            (now.saturating_duration_since(started).as_secs_f32() / PREVIEW_FADE.as_secs_f32()).min(1.0)
        })
    }
}

#[derive(Default)]
struct CardMotion {
    hovered: bool,
    pressed: bool,
    favorite: Option<bool>,
    document: Option<Rc<RefCell<document_card::DocumentMotion>>>,
    folder: Option<Rc<RefCell<folder_card::FolderFx>>>,
}

impl CardMotion {
    fn set_hover(&mut self, hovered: bool, now: Instant, reduced: bool) {
        self.hovered = hovered;
        if !hovered {
            self.pressed = false;
        }
        if let Some(folder) = &self.folder {
            folder.borrow_mut().hover(hovered && !reduced);
        } else {
            self.retarget_document(now, reduced);
        }
    }

    fn retarget_document(&mut self, now: Instant, reduced: bool) {
        self.document.get_or_insert_with(Default::default).borrow_mut()
            .retarget(self.hovered, self.pressed, now, reduced);
    }

    fn running(&self) -> bool {
        self.document.as_ref().is_some_and(|motion| motion.borrow().running())
            || self.folder.as_ref().is_some_and(|motion| motion.borrow().running())
    }
}

#[derive(Clone, Copy)]
struct GridLayout {
    columns: usize,
    card_width: f32,
}

impl GridLayout {
    fn for_width(available: f32, sidebar: motion::SidebarFrame) -> Self {
        let columns_for_width = |width: f32| {
            // 先让现有列长到上限，再在新列能维持最小宽度时平分空间。
            let span = width + GRID_GAP;
            let preferred = (span / (MAX_CARD_WIDTH + GRID_GAP)).ceil();
            let fit = (span / (MIN_CARD_WIDTH + GRID_GAP)).floor();
            preferred.min(fit).max(1.0) as usize
        };
        let width_with_sidebar = (available - sidebar.full_width).max(MIN_CARD_WIDTH);
        let columns_with_sidebar = columns_for_width(width_with_sidebar);
        // 动画期间保持展开态列数，只让现有列跟随侧栏占位连续伸缩。
        let columns = if sidebar.target_visible || sidebar.progress > 0.0 {
            columns_with_sidebar
        } else {
            columns_for_width(available)
        };
        let layout_width = (available - sidebar.occupied_width()).max(MIN_CARD_WIDTH);
        let card_width = ((layout_width - (columns - 1) as f32 * GRID_GAP)
            / columns as f32).min(MAX_CARD_WIDTH);
        Self { columns, card_width }
    }
}

#[cfg(test)]
mod grid_layout_tests {
    use super::{GRID_GAP, GridLayout, MAX_CARD_WIDTH, MIN_CARD_WIDTH};
    use crate::ui::motion::SidebarFrame;

    #[test]
    fn closing_sidebar_expands_existing_columns_to_the_live_width() {
        let available = 1040.0;
        let sidebar = SidebarFrame {
            full_width: 250.0,
            progress: 0.5,
            target_visible: false,
        };

        let layout = GridLayout::for_width(available, sidebar);

        assert_eq!(layout.columns, 3);
        let occupied = layout.card_width * layout.columns as f32
            + GRID_GAP * (layout.columns - 1) as f32;
        assert!((occupied - (available - sidebar.occupied_width())).abs() < 0.01);
    }

    #[test]
    fn fully_closed_sidebar_adds_columns_after_the_spring_finishes() {
        let available = 1040.0;
        let layout = GridLayout::for_width(available, SidebarFrame {
            full_width: 250.0,
            progress: 0.0,
            target_visible: false,
        });

        assert_eq!(layout.columns, 4);
        let occupied = layout.card_width * layout.columns as f32
            + GRID_GAP * (layout.columns - 1) as f32;
        assert!((occupied - available).abs() < 0.01);
    }

    #[test]
    fn wide_grid_fills_the_row_beyond_six_columns() {
        let available = 2488.0;
        let layout = GridLayout::for_width(available, SidebarFrame::default());

        assert_eq!(layout.columns, 8);
        let occupied = layout.card_width * layout.columns as f32
            + GRID_GAP * (layout.columns - 1) as f32;
        assert!((occupied - available).abs() < 0.01);
    }

    #[test]
    fn cards_grow_before_shrinking_to_fit_another_column() {
        let sidebar = SidebarFrame::default();
        let layout = GridLayout::for_width(600.0, sidebar);
        assert_eq!((layout.columns, layout.card_width), (2, 292.0));

        let third_column_width = 3.0 * MIN_CARD_WIDTH + 2.0 * GRID_GAP;
        let layout = GridLayout::for_width(third_column_width - 1.0, sidebar);
        assert_eq!((layout.columns, layout.card_width), (2, MAX_CARD_WIDTH));

        let layout = GridLayout::for_width(third_column_width, sidebar);
        assert_eq!((layout.columns, layout.card_width), (3, MIN_CARD_WIDTH));

        let layout = GridLayout::for_width(third_column_width + 48.0, sidebar);
        assert_eq!((layout.columns, layout.card_width), (3, MIN_CARD_WIDTH + 16.0));
    }
}

#[derive(Default)]
struct CardPosition {
    motion: Rc<RefCell<ItemMotion>>,
    frame: u64,
    reflow_started: Option<Instant>,
}

impl CardPosition {
    fn running(&self) -> bool {
        self.reflow_started.is_some() || self.motion.borrow().running()
    }
}

pub(crate) struct Home {
    focus: FocusHandle,
    library: Entity<Library>,
    snapshot: Arc<Snapshot>,
    _subscription: Subscription,
    active: bool,
    pub open_tabs: usize,
    pub restore_layout: bool,
    pub(super) sidebar_frame: motion::SidebarFrame,
    pub title_opacity: f32,
    sidebar_scroll: UniformListScrollHandle,
    grid_scroll: UniformListScrollHandle,
    preview_theme: thumbnail::Palette,
    previews: HashMap<PathBuf, CachedPreview>,
    // 只保留可见卡片的交互状态；路径与事件回调共用 Arc，不持有缩略图或编辑会话。
    card_motion: HashMap<Arc<Path>, CardMotion>,
    bytes: usize,
    clock: u64,
    visible: Vec<PathBuf>,
    pending: VecDeque<PathBuf>,
    scheduled: bool,
    worker: Option<Task<()>>,
    running: Option<(PathBuf, Arc<AtomicBool>)>,
    browser: Browser,
    dragging: Option<PathBuf>,
    grid_positions: HashMap<PathBuf, CardPosition>,
    grid_frame: u64,
    grid_time: Instant,
    grid_layout: Option<GridLayout>,
    grid_drag: crate::ui::tree_drag::TreeDrag,
    grid_drag_rows: Option<Arc<[Node]>>,
    visible_folders: Vec<PathBuf>,
    tree_drag: crate::ui::tree_drag::TreeDrag,
    tree_drag_rows: Option<Arc<[Node]>>,
}

impl EventEmitter<HomeEvent> for Home {}
impl Focusable for Home { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }

impl Home {
    pub(crate) fn new(library: Entity<Library>, active: bool, cx: &mut Context<Self>) -> Self {
        let snapshot = library.read(cx).snapshot.clone();
        let mut browser = Browser::default();
        browser.rebuild(&snapshot);
        let subscription = cx.observe(&library, |home, library, cx| {
            if (home.tree_drag.committing || home.grid_drag.committing) && !library.read(cx).is_busy() { home.order_finished(cx); }
            let next = library.read(cx).snapshot.clone();
            for state in home.card_motion.values_mut() { state.favorite = None; }
            if Arc::ptr_eq(&home.snapshot, &next) { cx.notify(); return; }
            let retired: Vec<_> = home.previews.keys().filter(|path| {
                let old = home.snapshot.recent.iter().find(|entry| &entry.path == *path);
                let new = next.recent.iter().find(|entry| &entry.path == *path);
                old.map(|entry| (entry.opened,entry.revision)) != new.map(|entry| (entry.opened,entry.revision))
            }).cloned().collect();
            for path in retired {
                if next.recent.iter().any(|entry| entry.path == path) {
                    if let Some(preview) = home.previews.get_mut(&path) { preview.stale = true; }
                } else { home.remove_preview(&path, cx); }
            }
            if let Some((path, cancel)) = &home.running {
                if home.snapshot.recent.iter().find(|entry| &entry.path == path).map(|entry| (entry.opened,entry.revision))
                    != next.recent.iter().find(|entry| &entry.path == path).map(|entry| (entry.opened,entry.revision)) {
                    cancel.store(true, Ordering::Release);
                }
            }
            home.snapshot = next;
            home.sync_browser(cx);
            cx.notify();
        });
        cx.on_release(|home, cx| { home.clear_previews(cx); home.browser.suspend(); }).detach();
        Self { focus: cx.focus_handle(), library, snapshot, _subscription: subscription, active, open_tabs: 0, restore_layout:false, sidebar_frame: Default::default(), title_opacity: 1.0,
            sidebar_scroll: UniformListScrollHandle::new(), grid_scroll: UniformListScrollHandle::new(),
            preview_theme: thumbnail::Palette::new(cx.global::<Theme>(), ToolbarLocale::current(cx)),
            previews: HashMap::new(), card_motion: HashMap::new(), bytes: 0, clock: 0, visible: Vec::new(), pending: VecDeque::new(), scheduled: false, worker: None, running: None,
            browser, dragging: None,
            grid_positions: HashMap::new(), grid_frame: 0, grid_time: Instant::now(), grid_layout: None,
            grid_drag: Default::default(), grid_drag_rows: None,
            visible_folders: Vec::new(), tree_drag: Default::default(), tree_drag_rows: None }
    }

    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active { return; }
        self.active = active;
        if active { self.sync_browser(cx); self.refresh(cx); } else {
            self.clear_previews(cx); self.card_motion = HashMap::new(); self.browser.suspend();
            self.dragging = None;
            self.grid_layout = None;
            self.grid_positions.clear(); self.grid_drag = Default::default(); self.grid_drag_rows = None;
            self.visible_folders.clear(); self.tree_drag = Default::default(); self.tree_drag_rows = None;
        }
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        // 窗口激活后只在后台校验可见卡片；旧位图保留到文件或资源指纹确实改变。
        if let Some((_, cancel)) = &self.running { cancel.store(true, Ordering::Release); }
        for preview in self.previews.values_mut() { preview.stale = true; }
        self.browser.error = None;
        self.refresh_directories(cx);
        self.library.update(cx, |library, cx| library.change(Change::Refresh, cx));
        cx.notify();
    }

    fn clear_previews(&mut self, cx: &mut App) {
        if let Some((_, cancel)) = &self.running { cancel.store(true, Ordering::Release); }
        self.pending.clear();
        self.visible.clear();
        for (_, preview) in self.previews.drain() { if let Preview::Ready { image, .. } = preview.preview { cx.drop_image(image, None); } }
        self.bytes = 0;
    }

    fn remove_preview(&mut self, path: &Path, cx: &mut App) {
        if let Some(CachedPreview { preview: Preview::Ready { image, title, fingerprint }, .. }) = self.previews.remove(path) {
            self.bytes = self.bytes.saturating_sub(BITMAP_BYTES + title.len() + fingerprint.len());
            cx.drop_image(image, None);
        }
    }

    fn request_visible(&mut self, mut paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        paths.truncate(MAX_VISIBLE_PREVIEWS);
        self.clock = self.clock.wrapping_add(1);
        let reduced = reduced_motion(cx);
        for path in &paths {
            if let Some(preview) = self.previews.get_mut(path) {
                preview.used = self.clock;
                if reduced || preview.fade_progress(self.grid_time) >= 1.0 { preview.fade_started = None; }
            }
        }
        if self.visible == paths && (self.worker.is_some() || paths.iter().all(|path| self.previews.get(path).is_some_and(|p| !p.stale))) { return; }
        self.visible = paths;
        if self.scheduled { return; }
        self.scheduled = true;
        let weak = cx.entity().downgrade();
        // 真实行完成 prepaint 后登记范围，下一帧启动任务，避免在布局阶段改变实体树。
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |home, cx| {
                home.scheduled = false;
                if !home.active { return; }
                if let Some((path, cancel)) = &home.running {
                    if !home.visible.contains(path) { cancel.store(true, Ordering::Release); }
                }
                home.pending = home.visible.iter().filter(|path| home.previews.get(*path).is_none_or(|p| p.stale)).cloned().collect();
                home.start_next(cx);
            });
        });
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.worker.is_some() { return; }
        let Some(path) = self.pending.pop_front() else { return; };
        if self.previews.get(&path).is_some_and(|p| !p.stale) { self.start_next(cx); return; }
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some((path.clone(), cancel.clone()));
        let background_path = path.clone();
        let background_cancel = cancel.clone();
        let palette = self.preview_theme.clone();
        let render_palette = palette.clone();
        let known = self.previews.get(&path).and_then(|entry| match &entry.preview {
            Preview::Ready { fingerprint, .. } => Some(fingerprint.clone()), Preview::Failed(_) => None,
        });
        let task = cx.background_executor().spawn(async move { thumbnail::load(&background_path, &render_palette, known.as_deref(), &background_cancel) });
        self.worker = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |home, cx| {
                home.worker = None;
                home.running = None;
                let mut changed = false;
                if home.active && home.preview_theme == palette && !cancel.load(Ordering::Acquire)
                    && home.visible.contains(&path) {
                    let replacement = match result {
                        Ok(None) => None,
                        Ok(Some(thumbnail::Thumbnail { mut pixels, title, fingerprint })) => {
                            // GPUI 的 RenderImage 使用 BGRA，磁盘缓存仍保留标准 PNG。
                            for pixel in pixels.pixels_mut() { pixel.0.swap(0,2); }
                            home.bytes += BITMAP_BYTES + title.len() + fingerprint.len();
                            Some(Preview::Ready { image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])), title: title.into(), fingerprint })
                        },
                        Err(error) => Some(Preview::Failed(error.context("preview.failed"))),
                    };
                    if let Some(preview) = replacement {
                        changed = true;
                        // 只从骨架过渡到新图；刷新已有预览时直接替换，避免闪回骨架。
                        let fade_started = (!reduced_motion(cx) && matches!(&preview, Preview::Ready { .. })
                            && !home.previews.contains_key(&path)).then(Instant::now);
                        home.remove_preview(&path, cx);
                        home.clock = home.clock.wrapping_add(1);
                        home.previews.insert(path, CachedPreview { preview, used: home.clock, stale: false, fade_started });
                    } else if let Some(preview) = home.previews.get_mut(&path) {
                        preview.stale = false;
                    }
                    while home.bytes > MEMORY_CACHE_BYTES || home.previews.len() > library::RECENT_LIMIT {
                        let oldest = home.previews.iter().min_by_key(|(_, entry)| entry.used).map(|(path, _)| path.clone());
                        let Some(oldest) = oldest else { break; };
                        home.remove_preview(&oldest, cx);
                    }
                }
                home.pending.retain(|path| home.visible.contains(path) && home.previews.get(path).is_none_or(|p| p.stale));
                home.start_next(cx);
                if changed || (home.worker.is_none() && home.active && home.visible.iter().any(|path| home.previews.get(path).is_none_or(|p| p.stale))) {
                    cx.notify();
                }
            });
        }));
    }

    fn set_card_hover(&mut self, path: Arc<Path>, hovered: bool, cx: &mut Context<Self>) {
        if !self.active { return; }
        if hovered && self.dragging.is_some() { return; }
        let reduced = reduced_motion(cx);
        let now = Instant::now();
        if hovered {
            for (other, state) in &mut self.card_motion {
                if other != &path && state.hovered { state.set_hover(false, now, reduced); }
            }
            self.card_motion.entry(path).or_default().set_hover(true, now, reduced);
        } else if let Some(state) = self.card_motion.get_mut(&path) {
            state.set_hover(false, now, reduced);
        } else { return; }
        cx.notify();
    }

    fn set_card_pressed(&mut self, path: Arc<Path>, pressed: bool, cx: &mut Context<Self>) {
        if !self.active { return; }
        let state = if pressed { self.card_motion.entry(path).or_default() }
            else if let Some(state) = self.card_motion.get_mut(&path) { state } else { return; };
        if state.pressed == pressed { return; }
        state.pressed = pressed;
        let reduced = reduced_motion(cx);
        if let Some(folder) = &state.folder {
            if pressed && !reduced {
                folder.borrow_mut().press();
            }
        } else {
            state.retarget_document(Instant::now(), reduced);
        }
        cx.notify();
    }

    fn card_favorite(&self, path: &Path, directory: bool) -> document_card::FavoriteVisual {
        let state = self.card_motion.get(path);
        let active = state.and_then(|state| state.favorite).unwrap_or_else(|| {
            if directory { self.snapshot.is_folder_favorite(path) } else { self.snapshot.is_favorite(path) }
        });
        let (scale, burst) = state.and_then(|state| state.document.as_ref())
            .map_or((1.0, None), |motion| motion.borrow().favorite_frame(self.grid_time));
        document_card::FavoriteVisual { active, visible: false, scale, burst }
    }

    fn set_favorite(&mut self, path: PathBuf, directory: bool, favorite: bool, cx: &mut Context<Self>) {
        let change = if directory {
            Change::SetFolderFavorite { path, favorite }
        } else {
            Change::SetFavorite { path, favorite }
        };
        self.library.update(cx, |library, cx| library.change(change, cx));
    }

    fn toggle_favorite(&mut self, path: Arc<Path>, directory: bool, cx: &mut Context<Self>) {
        let favorite = !self.card_favorite(path.as_ref(), directory).active;
        let reduced = reduced_motion(cx);
        let state = self.card_motion.entry(path.clone()).or_default();
        state.favorite = Some(favorite);
        state.document.get_or_insert_with(Default::default).borrow_mut().press_favorite(favorite, Instant::now(), reduced);
        self.set_favorite(path.to_path_buf(), directory, favorite, cx);
        cx.notify();
    }

    fn favorite_button(
        path: Arc<Path>, directory: bool, favorite: document_card::FavoriteVisual,
        reveal: f32, theme: &Theme, cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let tooltip_key = match (directory, favorite.active) {
            (true, true) => "home.unfavoriteFolder",
            (true, false) => "home.favoriteFolder",
            (false, true) => "home.unfavorite",
            (false, false) => "home.favorite",
        };
        div().id((gpui::ElementId::Path(path.clone()), "home-favorite-toggle"))
            .absolute().size(px(document_card::FAVORITE_SIZE))
            .cursor_pointer().rounded(px(6.0)).opacity(reveal)
            .hover(|style| style.bg(theme.hover))
            .when_some(favorite.burst, |button, t| button.child(document_card::favorite_burst(t, theme)))
            .child(document_card::favorite_icon(favorite, theme))
            .tooltip(tooltip(ToolbarLocale::current(cx).app_text(tooltip_key)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |home, _, _, cx| {
                cx.stop_propagation();
                home.toggle_favorite(path.clone(), directory, cx);
            }))
    }

    fn update_grid_layout(&mut self, layout: GridLayout, rows: usize, row_height: f32) -> bool {
        let Some(previous) = self.grid_layout.replace(layout) else { return false; };
        if previous.columns == layout.columns { return false; }
        if previous.card_width != layout.card_width {
            self.grid_positions.clear();
            return false;
        }

        let scroll = self.grid_scroll.0.borrow();
        let offset = scroll.base_handle.offset();
        let old_top = -f32::from(offset.y);
        let mut scroll_delta = 0.0;
        if self.dragging.is_none() && old_top > GRID_SHADOW_INSET
            && let Some(size) = scroll.last_item_size
        {
            // 按换列前的阅读位置换算滚动量，来回切换时不累积整行的偏差。
            let old_row = (old_top - GRID_SHADOW_INSET) / row_height;
            let new_row = old_row * previous.columns as f32 / layout.columns as f32;
            let max_top = (rows as f32 * row_height - f32::from(size.item.height)).max(0.0);
            let new_top = (new_row * row_height + GRID_SHADOW_INSET).min(max_top);
            scroll.base_handle.set_offset(gpui::point(offset.x, px(-new_top)));
            scroll_delta = new_top - old_top;
        }
        for position in self.grid_positions.values() {
            position.motion.borrow_mut().reflow_from_current(scroll_delta, self.grid_time);
        }
        true
    }

    fn card(&self, node: &Node, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entry = &node.entry;
        let reduced = reduced_motion(cx);
        let card_state = self.card_motion.get(entry.path.as_path());
        let (elevation, hint) = card_state.and_then(|state| state.document.as_ref())
            .map(|motion| motion.borrow().values()).unwrap_or_default();
        let favorite = self.card_favorite(&entry.path, false);
        let cached = self.previews.get(&entry.path);
        let preview = cached.map(|entry| &entry.preview);
        let title = match preview {
            Some(Preview::Ready { title, .. }) => title.clone(),
            _ => document_name(&entry.path),
        };
        let cover = document_card::DocumentCover {
            title: title.clone(), font_family: self.preview_theme.font_family.clone().into(),
            image: match preview { Some(Preview::Ready { image, .. }) => Some(image.clone()), _ => None },
            error: match preview { Some(Preview::Failed(error)) => Some(error.render(ToolbarLocale::current(cx)).into()), _ => None },
            open_label: ToolbarLocale::current(cx).app_text("home.openDocument").into(),
            favorite,
            thumbnail_opacity: if reduced { 1.0 } else { cached.map_or(1.0, |preview| motion::ease(preview.fade_progress(self.grid_time))) },
        };
        let height = card_height(width);
        let path: Arc<Path> = entry.path.as_path().into();
        let hover_path = path.clone();
        let press_path = path.clone();
        let release_path = path.clone();
        let outside_path = path.clone();
        let paper = cover.render(width, height, elevation, hint, reduced, theme);
        // 感应区域覆盖浮起前后的卡片，避免边缘因视觉位移而反复触发 hover。
        let placeholder = self.dragging.as_ref() == Some(&entry.path) && !self.grid_drag.committing && !self.tree_drag.committing;
        let hovered = card_state.is_some_and(|state| state.hovered);
        let hover_progress = if reduced { f32::from(hovered) } else { hint };
        let reveal = if favorite.active { 1.0 } else { hover_progress };
        let card = div().id((gpui::ElementId::Path(path.clone()), "home-card")).relative()
            .w(px(width)).h(px(height + CARD_LIFT)).flex_none().cursor_pointer()
            .when(placeholder, |el| el.bg(theme.hover).rounded(px(14.0)))
            .tooltip(tooltip(title)).child(div().opacity(if placeholder { 0.0 } else { 1.0 }).child(paper))
            .when(!placeholder && (hovered || reveal > 0.0), |card| {
                let lift = if reduced { 0.0 } else { CARD_LIFT * elevation };
                card.child(Self::favorite_button(path.clone(), false, favorite, reveal, theme, cx)
                    .right(px(CARD_PADDING + document_card::FAVORITE_RIGHT))
                    .top(px(CARD_PADDING + CARD_LIFT - lift + document_card::FAVORITE_TOP)))
            })
            .on_hover(cx.listener(move |home, hovered, _, cx| home.set_card_hover(hover_path.clone(), *hovered, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(press_path.clone(), true, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(release_path.clone(), false, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(outside_path.clone(), false, cx)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(path.to_path_buf()))));
        self.draggable(card, node, Some((width, height)), Some(document_card::DocumentCover {
            favorite: document_card::FavoriteVisual { visible: favorite.active, ..favorite },
            thumbnail_opacity: 1.0, ..cover
        }), 0.0, cx).into_any_element()
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reduced = reduced_motion(cx);
        // 侧栏收起后，已提交的排序仍需等模型更新再结束手势。
        self.tree_drag.check_commit(
            self.browser.tree_revision as usize,
            || Self::drag_rows(&self.browser.rows),
            reduced,
        );
        if self.browser.error.is_some() && self.tree_drag.committing {
            self.tree_drag.end(reduced);
        }
        if !cx.has_active_drag() && !self.tree_drag.committing && !self.grid_drag.committing {
            self.release_drag(cx);
        }
        if !self.tree_drag.active() {
            self.tree_drag_rows = None;
        }
        let now = Instant::now();
        self.grid_time = now;
        self.card_motion.retain(|path, state| {
            // 统一推进一次，虚拟列表的测量和可见行绘制读取同一姿态。
            if let Some(document) = &state.document {
                document.borrow_mut().advance(now, reduced);
            }
            state.folder.is_some() || state.hovered || state.pressed || state.running()
                || self.dragging.as_deref() == Some(path.as_ref())
        });
        let previous_frame = self.grid_frame;
        self.grid_frame = self.grid_frame.wrapping_add(1);
        self.grid_positions.retain(|path, position| position.frame == previous_frame || self.dragging.as_ref() == Some(path));
        let sidebar_width = self.sidebar_frame.occupied_width();
        if self.active && (self.card_motion.values().any(CardMotion::running)
            || self.grid_positions.values().any(CardPosition::running)
            || (sidebar_width > 0.0 && self.tree_drag.running()))
        {
            window.request_animation_frame();
        }
        let preview_theme = thumbnail::Palette::new(cx.global::<Theme>(), ToolbarLocale::current(cx));
        if self.preview_theme != preview_theme { self.preview_theme = preview_theme; self.clear_previews(cx); }
        let theme = Theme::ui(cx);
        let viewport_width = f32::from(window.viewport_size().width);
        let content_padding = if viewport_width < COMPACT_WINDOW_WIDTH { 18.0 } else { 24.0 };
        let available = viewport_width - (content_padding + GRID_SHADOW_INSET) * 2.0;
        let grid_layout = GridLayout::for_width(available, self.sidebar_frame);
        let GridLayout { columns, card_width } = grid_layout;
        let row_height = card_height(card_width) + CARD_LIFT + GRID_GAP;
        let grid = self.browser.grid.clone();
        self.grid_drag.check_commit(self.browser.revision as usize, || Self::drag_rows(&grid), reduced_motion(cx));
        if self.browser.error.is_some() && self.grid_drag.committing { self.grid_drag.end(reduced_motion(cx)); }
        if !self.grid_drag.active() { self.grid_drag_rows = None; }
        let grid = self.grid_drag_rows.clone().unwrap_or(grid);
        let layout = self.grid_drag.layout();
        let dragged_index = self.dragging.as_ref().and_then(|path| grid.iter().position(|node| &node.entry.path == path));
        let count = layout.len(grid.len());
        let rows = count.div_ceil(columns);
        let reflow = self.update_grid_layout(grid_layout, rows, row_height);
        let sidebar = (sidebar_width > 0.0).then(|| {
            let mut sidebar_theme = theme.clone();
            sidebar_theme.hover = gpui::rgba(SIDEBAR_ROW_HOVER).into();
            let list = self.sidebar(self.sidebar_frame.full_width - 16.0, &sidebar_theme, cx);
            let panel = div().flex().flex_col()
                .child(div().id("home-sidebar-list-region").w_full().flex_1().min_h(px(0.0)).px(px(8.0)).pb(px(10.0))
                    .overflow_hidden().child(list));
            self.sidebar_frame.clip(panel)
        });
        let library = self.library.read(cx);
        let error = self.browser.error.as_ref().or(library.error.as_ref()).map(|message| message.render(ToolbarLocale::current(cx)));
        let loading = !library.loaded;
        let content = if rows == 0 {
            self.card_motion.clear();
            self.visible_folders.clear(); self.grid_positions.clear();
            self.cancel_hidden_directory_scan();
            div().size_full().flex().flex_col().items_center().justify_center().gap(px(14.0))
                .child(div().size(px(58.0)).rounded(px(16.0)).bg(theme.sidebar_active).flex().items_center().justify_center()
                    .child(icon(IconName::BookOpenText, 28.0, theme.accent)))
                .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).child(ToolbarLocale::current(cx).app_text(
                    if loading { "home.loading" } else { "home.start" })))
                .child(div().text_size(px(13.0)).text_color(theme.muted).child(ToolbarLocale::current(cx).app_text("home.empty")))
                .child(button("home-first-open", ToolbarLocale::current(cx).app_text("home.openFile"), Some(IconName::FileText), ButtonKind::Secondary, &theme)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::OpenFile))))
                .into_any_element()
        } else {
            let grid_theme = theme.clone();
            let list = uniform_list("home-document-grid", rows, cx.processor(move |home, range: std::ops::Range<usize>, window, cx| {
                let start = (range.start * columns).min(count);
                let end = (range.end * columns).min(count);
                let visible: Vec<_> = (start..end).filter_map(|index| layout.original(index).or(dragged_index).and_then(|index| grid.get(index))).collect();
                let paths: Vec<_> = visible.iter().filter(|node| !node.directory && home.dragging.as_ref() != Some(&node.entry.path)).map(|node| node.entry.path.clone()).collect();
                let folders: Vec<_> = visible.iter().filter(|node| node.directory).map(|node| node.entry.path.clone()).collect();
                let first_visible = range.start;
                let items = range.map(|row| {
                    let first = row * columns;
                    let last = (first + columns).min(count);
                    let paths = paths.clone();
                    let folders = folders.clone();
                    let weak = cx.entity().downgrade();
                    div().w_full().h(px(row_height)).px(px(GRID_SHADOW_INSET)).flex().gap(px(GRID_GAP))
                        .children((first..last).filter_map(|index| {
                            let node = grid.get(layout.original(index).or(dragged_index)?)?;
                            let card = if node.directory { home.folder_card(node, card_width, &grid_theme, cx) } else { home.card(node, card_width, &grid_theme, cx) };
                            let card = home.animate_card(&node.entry.path, index, grid_layout, row_height, reflow, card, reduced_motion(cx), &grid_theme);
                            Some(div().w(px(card_width)).h(px(card_height(card_width) + CARD_LIFT)).flex_none().child(card))
                        }).collect::<Vec<_>>())
                        .when(row == first_visible, |row| row.on_children_prepainted(move |_, window, cx| {
                            let _ = weak.update(cx, |home, cx| {
                                // 只在真实可见行预绘制时更新可见集合；测量回调不能清掉其他卡片的状态。
                                home.visible_folders = folders.clone();
                                home.cancel_hidden_directory_scan();
                                home.card_motion.retain(|path, _| paths.iter().chain(&folders).any(|visible| visible.as_path() == path.as_ref()) || home.dragging.as_deref() == Some(path.as_ref()));
                                if !reduced_motion(cx) && paths.iter().any(|path| home.previews.get(path)
                                    .is_some_and(|preview| preview.fade_progress(home.grid_time) < 1.0)) {
                                    window.request_animation_frame();
                                }
                                // 每个文件夹只请求前页；后续纸张完全由原生线条绘制。
                                let mut previews = paths.clone();
                                for folder in &folders {
                                    if let Some(document) = home.browser.cover_document(folder) {
                                        if !previews.contains(&document.path) { previews.push(document.path.clone()); }
                                    }
                                }
                                home.request_visible(previews, window, cx);
                                // 文件夹封面只读取可见卡片的直接子项，不递归扫描内容。
                                let weak = cx.entity().downgrade();
                                let folders: Vec<_> = folders.iter().filter(|path| home.browser.needs_directory(path)).cloned().collect();
                                if !folders.is_empty() { window.on_next_frame(move |_, cx| {
                                    let _ = weak.update(cx, |home, cx| { for path in folders { home.queue_directory(path, false, cx); } });
                                }); }
                            });
                        }))
                }).collect::<Vec<_>>();
                if home.grid_positions.values().any(CardPosition::running) { window.request_animation_frame(); }
                items
            })).w_full().h_full().py(px(GRID_SHADOW_INSET)).track_scroll(&self.grid_scroll);
            div().id("home-grid-sort").size_full().child(list)
                .on_drag_move(cx.listener(move |home, event: &gpui::DragMoveEvent<PathDrag>, _, cx| {
                    if home.tree_drag.committing || home.grid_drag.committing { return; }
                    if event.bounds.contains(&event.event.position) {
                        let started = !home.grid_drag.active();
                        home.begin_grid_drag(event.drag(cx), cx);
                        let scroll = home.grid_scroll.0.borrow().base_handle.offset().y;
                        let x = (f32::from(event.event.position.x - event.bounds.left()) - GRID_SHADOW_INSET).max(0.0);
                        let y = (f32::from(event.event.position.y - event.bounds.top() - scroll) - GRID_SHADOW_INSET).max(0.0);
                        let column = ((x / (card_width + GRID_GAP)).floor() as usize).min(columns - 1);
                        let index = (y / row_height).floor() as usize * columns + column;
                        let after = x - column as f32 * (card_width + GRID_GAP) >= card_width * 0.5;
                        let changed = home.grid_drag.reorder_at(index, after, reduced_motion(cx));
                        if changed || (started && home.grid_drag.active()) { cx.notify(); }
                    } else if home.grid_drag.active() {
                        home.grid_drag.end(reduced_motion(cx)); cx.notify();
                    }
                }))
                .on_drop(cx.listener(|home, item: &PathDrag, _, cx| {
                    if home.tree_drag.committing || home.grid_drag.committing { return; }
                    if let Some(placement) = home.grid_drag.placement() {
                        home.land_grid(item, true);
                        home.tree_drag.end(reduced_motion(cx));
                        home.grid_drag.commit(reduced_motion(cx));
                        cx.emit(HomeEvent::Place { item: item.clone(), placement }); cx.notify();
                    }
                })).into_any_element()
        };
        div().id("document-home").track_focus(&self.focus).w_full().flex_1().min_h(px(0.0)).flex().bg(theme.rail_bg)
            .on_key_down(cx.listener(|home, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && cx.has_active_drag() {
                    home.release_drag(cx); cx.stop_active_drag(window); cx.notify();
                }
            }))
            .children(sidebar)
            .child(div().relative().flex_1().min_w(px(0.0)).min_h(px(0.0)).h_full().flex().flex_col()
                .child(div().flex_1().min_w(px(0.0)).min_h(px(0.0)).bg(theme.surface).overflow_hidden()
                    .rounded_tl(px(12.0)).border_t_1().border_l_1().border_color(theme.workspace_edge)
                    .px(px(content_padding)).pt(px(14.0)).flex().flex_col().gap(px(12.0))
                    .child(div().w_full().flex_none().flex().items_center().gap(px(12.0))
                        .child(div().flex_none().flex().items_center().gap(px(6.0))
                            .child(div().mr(px(4.0)).text_size(px(18.0)).font_weight(FontWeight::BOLD).opacity(self.title_opacity)
                                .child(ToolbarLocale::current(cx).app_text("home.title")))
                            .child(small_icon_button("home-refresh", IconName::RefreshCw, false, &theme).size(px(30.0))
                                .tooltip(tooltip(ToolbarLocale::current(cx).app_text("home.refresh")))
                                .on_click(cx.listener(|home, _, _, cx| home.refresh(cx))))
                            .when(!self.snapshot.folders.is_empty() || !self.snapshot.recent.is_empty(), |row| row.child(small_icon_button("home-clear", IconName::Trash2, false, &theme).size(px(30.0))
                                .tooltip(tooltip(ToolbarLocale::current(cx).app_text("home.clearEntries")))
                                .on_click(cx.listener(|home, _, _, cx| home.library.update(cx, |library, cx| library.change(Change::ClearHome, cx)))))))
                        .child(div().flex_1().min_w(px(0.0)).flex().flex_wrap().items_center().justify_end().gap(px(6.0))
                            .when(self.restore_layout, |row| row.child(button("home-restore-layout", ToolbarLocale::current(cx).text("tab_restore_layout"), Some(IconName::Columns), ButtonKind::Ghost, &theme)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::Restore)))))
                            .when(self.open_tabs > 0, |row| row.child(button("home-resume", ToolbarLocale::current(cx).app_text("home.resume"), Some(IconName::SquarePen), ButtonKind::Ghost, &theme)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::Resume)))))
                            .child(button("home-open-folder", ToolbarLocale::current(cx).app_text("home.openFolder"), Some(IconName::FolderOpen), ButtonKind::Ghost, &theme)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::OpenFolder))))
                            .child(button("home-open", ToolbarLocale::current(cx).app_text("home.openFile"), Some(IconName::FileText), ButtonKind::Ghost, &theme)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::OpenFile))))
                            .child(button("home-new", ToolbarLocale::current(cx).app_text("home.new"), Some(IconName::Plus), ButtonKind::Primary, &theme)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::NewFile))))))
                    .children(error.map(|error| div().text_size(px(12.0)).text_color(theme.danger).child(error)))
                    .child(div().flex_1().min_h(px(0.0)).overflow_hidden().child(content))))
    }
}

fn document_file_name(path: &Path) -> &std::ffi::OsStr {
    let markdown = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"));
    let name = if markdown { path.file_stem() } else { path.file_name() };
    name.unwrap_or_else(|| path.as_os_str())
}

fn document_name(path: &Path) -> SharedString {
    document_file_name(path).to_string_lossy().chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect::<String>().into()
}

pub(crate) fn entry_name(path: &Path) -> SharedString {
    path.file_name().unwrap_or_else(|| path.as_os_str()).to_string_lossy().chars().map(|ch|if ch.is_control(){' '}else{ch}).collect::<String>().into()
}

/// 工作区侧栏的返回主页入口。
pub(crate) fn nav_home_row(active: bool, theme: &Theme, cx: &App) -> gpui::Stateful<gpui::Div> {
    let color = if active { theme.accent } else { theme.muted };
    sidebar_row().id("sidebar-home").flex_none().px(px(12.0))
        .text_color(color).font_weight(FontWeight::SEMIBOLD).when(active, |el| el.bg(theme.sidebar_active))
        .when(!active, |el| el.cursor_pointer().hover(|style| style.bg(gpui::rgba(SIDEBAR_ROW_HOVER))))
        .child(icon(IconName::Home, 14.0, color)).child(ToolbarLocale::current(cx).app_text("home.title"))
}

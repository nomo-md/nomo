//! 文档主页：轻量目录入口、最近文件与按可见行请求的位图预览。
pub(crate) mod library;
mod thumbnail;

use crate::{theme::Theme, ui::components::{ButtonKind, IconName, button, icon, small_icon_button, tooltip}};
use crate::ui::{motion, popup::reduced_motion};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement, MouseButton, ParentElement, Render, RenderImage, SharedString, Styled, Subscription, Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list};
use library::{Change, Entry, Library, Snapshot};
use std::{collections::{HashMap, VecDeque}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};

const MEMORY_CACHE_BYTES: usize = 24 * 1024 * 1024;
const BITMAP_BYTES: usize = (thumbnail::WIDTH * thumbnail::HEIGHT * 4) as usize;
const CARD_PADDING: f32 = 16.0;
const CARD_TITLE_HEIGHT: f32 = 40.0;
const CARD_PREVIEW_GAP: f32 = 12.0;
const CARD_FOOTER_HEIGHT: f32 = 18.0;
const CARD_LIFT: f32 = 3.0;
const CARD_ENTER: Duration = Duration::from_millis(260);
const CARD_LEAVE: Duration = Duration::from_millis(340);
const CARD_PRESS: Duration = Duration::from_millis(80);
const CARD_HINT: Duration = Duration::from_millis(160);
const GRID_GAP: f32 = 16.0;
const GRID_SHADOW_INSET: f32 = 12.0;

fn card_height(width: f32) -> f32 {
    CARD_PADDING * 2.0 + 2.0 + CARD_TITLE_HEIGHT + CARD_PREVIEW_GAP * 2.0 + CARD_FOOTER_HEIGHT
        + (width - CARD_PADDING * 2.0 - 2.0) * thumbnail::HEIGHT as f32 / thumbnail::WIDTH as f32
}

#[derive(Clone)]
pub(crate) enum HomeEvent { Open(PathBuf), NewFile, OpenFile, OpenFolder, AddFolder, Resume }

enum Preview { Ready { image: Arc<RenderImage>, title: SharedString }, Failed(SharedString) }
struct CachedPreview { preview: Preview, used: u64 }

#[derive(Default)]
struct CardMotion {
    hovered: bool,
    pressed: bool,
    hover: motion::ScalarMotion,
    press: motion::ScalarMotion,
    hint: motion::ScalarMotion,
}

impl CardMotion {
    fn set_hover(&mut self, hovered: bool, reduced: bool) {
        self.hovered = hovered;
        self.hover.set(hovered, reduced, if hovered { CARD_ENTER } else { CARD_LEAVE });
        self.hint.set(hovered, reduced, CARD_HINT);
        if !hovered { self.pressed = false; self.press.set(false, reduced, CARD_PRESS); }
    }

    fn running(&self) -> bool { self.hover.running() || self.press.running() || self.hint.running() }
}

pub(crate) struct Home {
    focus: FocusHandle,
    library: Entity<Library>,
    snapshot: Arc<Snapshot>,
    _subscription: Subscription,
    active: bool,
    pub open_tabs: usize,
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
}

impl EventEmitter<HomeEvent> for Home {}
impl Focusable for Home { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }

impl Home {
    pub(crate) fn new(library: Entity<Library>, active: bool, cx: &mut Context<Self>) -> Self {
        let snapshot = library.read(cx).snapshot.clone();
        let subscription = cx.observe(&library, |home, library, cx| {
            let next = library.read(cx).snapshot.clone();
            let retired: Vec<_> = home.previews.keys().filter(|path| {
                let old = home.snapshot.recent.iter().find(|entry| &entry.path == *path);
                let new = next.recent.iter().find(|entry| &entry.path == *path);
                old.map(|entry| (entry.opened,entry.revision)) != new.map(|entry| (entry.opened,entry.revision))
            }).cloned().collect();
            for path in retired { home.remove_preview(&path, cx); }
            if let Some((path, cancel)) = &home.running {
                if home.snapshot.recent.iter().find(|entry| &entry.path == path).map(|entry| (entry.opened,entry.revision))
                    != next.recent.iter().find(|entry| &entry.path == path).map(|entry| (entry.opened,entry.revision)) {
                    cancel.store(true, Ordering::Release);
                }
            }
            home.snapshot = next;
            home.card_motion.retain(|path, _| home.snapshot.recent.iter().any(|entry| entry.path.as_path() == path.as_ref()));
            cx.notify();
        });
        cx.on_release(|home, cx| home.clear_previews(cx)).detach();
        Self { focus: cx.focus_handle(), library, snapshot, _subscription: subscription, active, open_tabs: 0,
            sidebar_scroll: UniformListScrollHandle::new(), grid_scroll: UniformListScrollHandle::new(),
            preview_theme: thumbnail::Palette::new(cx.global::<Theme>()),
            previews: HashMap::new(), card_motion: HashMap::new(), bytes: 0, clock: 0, visible: Vec::new(), pending: VecDeque::new(), scheduled: false, worker: None, running: None }
    }

    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active { return; }
        self.active = active;
        if active { self.refresh(cx); } else { self.clear_previews(cx); self.card_motion = HashMap::new(); }
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.clear_previews(cx);
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
        if let Some(CachedPreview { preview: Preview::Ready { image, title }, .. }) = self.previews.remove(path) {
            self.bytes = self.bytes.saturating_sub(BITMAP_BYTES + title.len());
            cx.drop_image(image, None);
        }
    }

    fn request_visible(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.clock = self.clock.wrapping_add(1);
        for path in &paths { if let Some(preview) = self.previews.get_mut(path) { preview.used = self.clock; } }
        if self.visible != paths {
            self.card_motion.retain(|path, _| paths.iter().any(|visible| visible.as_path() == path.as_ref()));
        }
        if self.visible == paths && (self.worker.is_some() || paths.iter().all(|path| self.previews.contains_key(path))) { return; }
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
                home.pending = home.visible.iter().filter(|path| !home.previews.contains_key(*path)).cloned().collect();
                home.start_next(cx);
            });
        });
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.worker.is_some() { return; }
        let Some(path) = self.pending.pop_front() else { return; };
        if self.previews.contains_key(&path) { self.start_next(cx); return; }
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some((path.clone(), cancel.clone()));
        let background_path = path.clone();
        let background_cancel = cancel.clone();
        let palette = self.preview_theme.clone();
        let render_palette = palette.clone();
        let task = cx.background_executor().spawn(async move { thumbnail::load(&background_path, &render_palette, &background_cancel) });
        self.worker = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |home, cx| {
                home.worker = None;
                home.running = None;
                if home.active && home.preview_theme == palette && !cancel.load(Ordering::Acquire)
                    && home.snapshot.recent.iter().any(|entry| entry.path == path) {
                    home.remove_preview(&path, cx);
                    let preview = match result {
                        Ok(thumbnail::Thumbnail { mut pixels, title }) => {
                            // GPUI 的 RenderImage 使用 BGRA，磁盘缓存仍保留标准 PNG。
                            for pixel in pixels.pixels_mut() { pixel.0.swap(0,2); }
                            home.bytes += BITMAP_BYTES + title.len();
                            Preview::Ready { image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])), title: title.into() }
                        },
                        Err(error) => Preview::Failed(error.into()),
                    };
                    home.clock = home.clock.wrapping_add(1);
                    home.previews.insert(path, CachedPreview { preview, used: home.clock });
                    while home.bytes > MEMORY_CACHE_BYTES || home.previews.len() > library::RECENT_LIMIT {
                        let oldest = home.previews.iter().min_by_key(|(_, entry)| entry.used).map(|(path, _)| path.clone());
                        let Some(oldest) = oldest else { break; };
                        home.remove_preview(&oldest, cx);
                    }
                    cx.notify();
                }
                home.pending.retain(|path| home.visible.contains(path) && !home.previews.contains_key(path));
                home.start_next(cx);
                cx.notify();
            });
        }));
    }

    fn sidebar_row(&self, entry: &Entry, folder: bool, index: usize, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let path = entry.path.clone();
        let remove_path = path.clone();
        let color = theme.muted;
        let label = if folder { entry_name(&path) } else { document_name(&path) };
        div().id((if folder { "home-folder" } else { "home-recent" }, index)).relative().w_full().min_w(px(0.0)).h(px(32.0)).px(px(10.0))
            .group("home-sidebar-row")
            .flex().items_center().gap(px(8.0)).rounded(px(7.0)).cursor_pointer().hover(|s| s.bg(theme.hover))
            .tooltip(tooltip(label.clone()))
            .child(icon(if folder { IconName::Folder } else { IconName::FileText },16.0,color))
            .child(div().flex_1().min_w(px(0.0)).text_size(px(13.0)).truncate().child(label))
            .child(small_icon_button((if folder { "home-remove-folder" } else { "home-remove-recent" },index),IconName::Close,false,theme)
                .absolute().right(px(5.0)).top(px(5.0)).bg(theme.hover)
                .invisible().group_hover("home-sidebar-row", |style| style.visible())
                .tooltip(tooltip(if folder { "移除文件夹入口（保留磁盘文件）" } else { "从最近记录移除" }))
                .on_click(cx.listener(move |home, _, _, cx| { cx.stop_propagation(); home.library.update(cx, |library,cx| library.change(Change::Remove { path: remove_path.clone(), folder },cx)); })))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(path.clone())))).into_any_element()
    }

    fn set_card_hover(&mut self, path: Arc<Path>, hovered: bool, cx: &mut Context<Self>) {
        if !self.active { return; }
        let reduced = reduced_motion(cx);
        if hovered {
            for (other, state) in &mut self.card_motion {
                if other != &path && state.hovered { state.set_hover(false, reduced); }
            }
            self.card_motion.entry(path).or_default().set_hover(true, reduced);
        } else if let Some(state) = self.card_motion.get_mut(&path) {
            state.set_hover(false, reduced);
        } else { return; }
        cx.notify();
    }

    fn set_card_pressed(&mut self, path: Arc<Path>, pressed: bool, cx: &mut Context<Self>) {
        if !self.active { return; }
        let state = if pressed { self.card_motion.entry(path).or_default() }
            else if let Some(state) = self.card_motion.get_mut(&path) { state } else { return; };
        if state.pressed == pressed { return; }
        state.pressed = pressed;
        state.press.set(pressed, reduced_motion(cx), CARD_PRESS);
        cx.notify();
    }

    fn card(&mut self, entry: &Entry, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let reduced = reduced_motion(cx);
        let (hover, press, hint) = if let Some(state) = self.card_motion.get_mut(entry.path.as_path()) {
            state.hover.retime(reduced, if state.hovered { CARD_ENTER } else { CARD_LEAVE });
            state.press.retime(reduced, CARD_PRESS);
            state.hint.retime(reduced, CARD_HINT);
            (state.hover.value(), state.press.value(), state.hint.value())
        } else { (0.0, 0.0, 0.0) };
        let lift = if reduced { 0.0 } else { (CARD_LIFT * hover - 2.0 * press).max(0.0) };
        let elevation = (hover - 0.7 * press).max(0.0);
        let near_shadow = gpui::Hsla { a: (theme.workspace_shadow_near.a * (1.0 + 0.75 * elevation)).min(1.0), ..theme.workspace_shadow_near };
        let soft_shadow = gpui::Hsla { a: (theme.workspace_shadow_soft.a * (1.0 + 0.67 * elevation)).min(1.0), ..theme.workspace_shadow_soft };
        let preview = self.previews.get(&entry.path).map(|entry| &entry.preview);
        let title = match preview {
            Some(Preview::Ready { title, .. }) => title.clone(),
            _ => document_name(&entry.path),
        };
        let height = card_height(width);
        let inner_width = width - CARD_PADDING * 2.0 - 2.0;
        let inner_height = inner_width * thumbnail::HEIGHT as f32 / thumbnail::WIDTH as f32;
        let image = match preview {
            Some(Preview::Ready { image, .. }) => gpui::img(image.clone()).w(px(inner_width)).h(px(inner_height)).rounded(px(3.0)).into_any_element(),
            Some(Preview::Failed(error)) => div().w(px(inner_width)).h(px(inner_height)).flex().flex_col().items_center().justify_center().gap(px(8.0))
                .child(icon(IconName::FileText,24.0,theme.muted))
                .child(div().text_size(px(12.0)).text_color(theme.muted).text_center().line_clamp(3).child(error.clone())).into_any_element(),
            None => div().w(px(inner_width)).h(px(inner_height)).flex().flex_col().gap(px(10.0))
                .children([0.92, 1.0, 0.76, 0.88, 0.62].map(|fraction| div().w(px(inner_width * fraction)).h(px(5.0)).rounded(px(3.0))
                    .bg(motion::color(theme.surface, theme.border, 0.45)))).into_any_element(),
        };
        let path: Arc<Path> = entry.path.as_path().into();
        let hover_path = path.clone();
        let press_path = path.clone();
        let release_path = path.clone();
        let outside_path = path.clone();
        let paper = div().absolute().top(px(CARD_LIFT - lift)).left_0().w(px(width)).h(px(height))
            .rounded(px(14.0)).border_1().border_color(motion::color(theme.surface, theme.border, 0.3)).bg(theme.surface)
            .shadow(vec![
                gpui::BoxShadow { inset: false, color: near_shadow,
                    offset: gpui::point(px(0.0),px(1.0 + 2.0 * elevation)), blur_radius: px(2.0 + 5.0 * elevation), spread_radius: px(0.0) },
                gpui::BoxShadow { inset: false, color: soft_shadow,
                    offset: gpui::point(px(0.0),px(2.0 + 7.0 * elevation)), blur_radius: px(7.0 + 12.0 * elevation), spread_radius: px(0.0) },
            ])
            .p(px(CARD_PADDING)).flex().flex_col().gap(px(CARD_PREVIEW_GAP))
            .child(div().w_full().h(px(CARD_TITLE_HEIGHT)).flex_none().text_size(px(14.0)).line_height(px(20.0))
                .font_family(self.preview_theme.font_family.clone()).font_weight(FontWeight::BOLD).text_color(theme.heading).line_clamp(2).child(title.clone()))
            .child(div().w(px(inner_width)).h(px(inner_height)).flex_none().overflow_hidden().child(image))
            // 入口始终占位，淡入时不会挤压预览或改变虚拟列表行高。
            .child(div().relative().top(px(if reduced { 0.0 } else { 3.0 * (1.0 - hint) })).opacity(hint)
                .w_full().h(px(CARD_FOOTER_HEIGHT)).flex_none().flex().items_center().justify_between()
                .text_size(px(11.0)).line_height(px(CARD_FOOTER_HEIGHT)).text_color(theme.muted)
                .child("打开文档")
                .child(icon(IconName::ArrowUp, 13.0, theme.muted)
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::FRAC_PI_4)))));
        // 感应区域覆盖浮起前后的卡片，避免边缘因视觉位移而反复触发 hover。
        div().id((gpui::ElementId::Path(path.clone()), "home-card")).relative()
            .w(px(width)).h(px(height + CARD_LIFT)).flex_none().cursor_pointer()
            .tooltip(tooltip(title)).child(paper)
            .on_hover(cx.listener(move |home, hovered, _, cx| home.set_card_hover(hover_path.clone(), *hovered, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(press_path.clone(), true, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(release_path.clone(), false, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(outside_path.clone(), false, cx)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(path.to_path_buf())))).into_any_element()
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.card_motion.retain(|_, state| state.hovered || state.pressed || state.running());
        // 快速掠过多张卡片时也只合并请求一帧，静止后不再重绘。
        if self.active && self.card_motion.values().any(CardMotion::running) { window.request_animation_frame(); }
        let preview_theme = thumbnail::Palette::new(cx.global::<Theme>());
        if self.preview_theme != preview_theme {
            self.preview_theme = preview_theme;
            self.clear_previews(cx);
        }
        let theme = Theme::ui(cx);
        let snapshot = self.snapshot.clone();
        let narrow = f32::from(window.viewport_size().width) < 850.0;
        let sidebar_width = if narrow { 208.0 } else { 240.0 };
        let content_padding = if narrow { 18.0 } else { 24.0 };
        let available = (f32::from(window.viewport_size().width) - sidebar_width - content_padding * 2.0 - GRID_SHADOW_INSET * 2.0).max(188.0);
        let columns = ((available + GRID_GAP) / (188.0 + GRID_GAP)).floor().clamp(1.0,6.0) as usize;
        let card_width = ((available - (columns-1) as f32 * GRID_GAP) / columns as f32).min(300.0);
        let row_height = card_height(card_width) + CARD_LIFT + GRID_GAP;
        let rows = snapshot.recent.len().div_ceil(columns);
        let sidebar_bg = motion::color(theme.rail_bg, theme.surface, 0.35);
        let mut sidebar_theme = theme.clone();
        sidebar_theme.hover = motion::color(sidebar_bg, theme.fg, 0.035);
        sidebar_theme.sidebar_active = motion::color(sidebar_bg, theme.accent, if theme.dark { 0.14 } else { 0.08 });
        let sidebar_active = sidebar_theme.sidebar_active;
        let sidebar_snapshot = snapshot.clone();
        let sidebar = uniform_list("home-sidebar-list", snapshot.folders.len() + snapshot.recent.len() + 2,
            cx.processor(move |home, range: std::ops::Range<usize>, _, cx| range.map(|index| {
                if index == 0 || index == sidebar_snapshot.folders.len()+1 {
                    let folder = index == 0;
                    div().w_full().h(px(32.0)).px(px(10.0)).flex().items_center().justify_between().text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(sidebar_theme.muted)
                        .child(if folder { "文件夹" } else { "最近文档" })
                        .child(small_icon_button(if folder { "home-add-folder" } else { "home-open-recent" },IconName::Plus,false,&sidebar_theme)
                            .tooltip(tooltip(if folder { "添加文件夹" } else { "打开文件" }))
                            .on_click(cx.listener(move |_,_,_,cx|cx.emit(if folder { HomeEvent::AddFolder } else { HomeEvent::OpenFile }))))
                        .into_any_element()
                } else if index <= sidebar_snapshot.folders.len() {
                    home.sidebar_row(&sidebar_snapshot.folders[index-1],true,index,&sidebar_theme,cx)
                } else {
                    home.sidebar_row(&sidebar_snapshot.recent[index-sidebar_snapshot.folders.len()-2],false,index,&sidebar_theme,cx)
                }
            }).collect::<Vec<_>>())).w_full().h_full().track_scroll(&self.sidebar_scroll);
        let library = self.library.read(cx);
        let error = library.error.clone();
        let loaded = library.loaded;
        let content = if rows == 0 {
            div().size_full().flex().flex_col().items_center().justify_center().gap(px(14.0))
                .child(div().size(px(58.0)).rounded(px(16.0)).bg(theme.sidebar_active).flex().items_center().justify_center().child(icon(IconName::BookOpenText,28.0,theme.accent)))
                .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).child(if loaded { "从这里开始写作" } else { "正在读取最近记录…" }))
                .child(div().text_size(px(13.0)).text_color(theme.muted).child("打开过的文件会显示在这里。"))
                .child(button("home-first-open","打开文件",Some(IconName::FileText),ButtonKind::Secondary,&theme).on_click(cx.listener(|_,_,_,cx|cx.emit(HomeEvent::OpenFile))))
                .into_any_element()
        } else {
            let grid_theme = theme.clone();
            uniform_list("home-document-grid",rows,cx.processor(move |home,range: std::ops::Range<usize>,_,cx| {
                let start = range.start * columns;
                let end = (range.end * columns).min(snapshot.recent.len());
                let paths: Vec<_> = snapshot.recent[start.min(end)..end].iter().map(|entry|entry.path.clone()).collect();
                let first_visible = range.start;
                range.map(|row| {
                    let first = row * columns;
                    let last = (first + columns).min(snapshot.recent.len());
                    let paths = paths.clone();
                    let weak = cx.entity().downgrade();
                    div().w_full().h(px(row_height)).px(px(GRID_SHADOW_INSET)).flex().gap(px(GRID_GAP))
                        .children((first..last).map(|index|home.card(&snapshot.recent[index],card_width,&grid_theme,cx)).collect::<Vec<_>>())
                        .when(row == first_visible, |row| row.on_children_prepainted(move |_, window, cx| {
                            let _ = weak.update(cx, |home, cx| home.request_visible(paths.clone(), window, cx));
                        }))
                }).collect::<Vec<_>>()
            })).w_full().h_full().py(px(GRID_SHADOW_INSET)).track_scroll(&self.grid_scroll).into_any_element()
        };
        div().id("document-home").track_focus(&self.focus).w_full().flex_1().min_h(px(0.0)).flex().bg(theme.surface)
            .child(div().w(px(sidebar_width)).h_full().flex_none().px(px(12.0)).py(px(14.0)).flex().flex_col().gap(px(10.0))
                .bg(sidebar_bg).border_r_1().border_color(motion::color(sidebar_bg, theme.border, 0.45))
                .child(div().w_full().h(px(34.0)).flex_none().px(px(10.0)).flex().items_center().gap(px(9.0)).rounded(px(8.0)).bg(sidebar_active).text_color(theme.accent_strong)
                    .child(icon(IconName::Home,17.0,theme.accent)).child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child("主页")))
                .child(div().id("home-sidebar-list-region")
                    .w_full().flex_1().min_h(px(0.0)).overflow_hidden().child(sidebar)))
            .child(div().relative().flex_1().min_w(px(0.0)).min_h(px(0.0)).h_full().flex().flex_col()
                .child(div().flex_1().min_w(px(0.0)).min_h(px(0.0)).bg(theme.surface).overflow_hidden()
                    .px(px(content_padding)).pt(px(14.0)).flex().flex_col().gap(px(12.0))
                    .child(div().w_full().flex_none().flex().items_center().gap(px(12.0))
                        .child(div().flex_none().text_size(px(18.0)).font_weight(FontWeight::BOLD).child("主页"))
                        .child(div().flex_1().min_w(px(0.0)).flex().flex_wrap().items_center().justify_end().gap(px(6.0))
                            .when(self.open_tabs>0,|row|row.child(button("home-resume","继续编辑",Some(IconName::SquarePen),ButtonKind::Ghost,&theme).flex_none()
                                .on_click(cx.listener(|_,_,_,cx|cx.emit(HomeEvent::Resume)))))
                            .child(button("home-open-folder","打开文件夹",Some(IconName::FolderOpen),ButtonKind::Ghost,&theme).flex_none()
                                .on_click(cx.listener(|_,_,_,cx|cx.emit(HomeEvent::OpenFolder))))
                            .child(button("home-open","打开文件",Some(IconName::FileText),ButtonKind::Ghost,&theme).flex_none()
                                .on_click(cx.listener(|_,_,_,cx|cx.emit(HomeEvent::OpenFile))))
                            .child(button("home-new","新建",Some(IconName::Plus),ButtonKind::Primary,&theme).flex_none()
                                .on_click(cx.listener(|_,_,_,cx|cx.emit(HomeEvent::NewFile))))))
                    .child(div().w_full().h(px(28.0)).flex_none().flex().items_center().justify_between().gap(px(8.0))
                        .child(div().flex().items_center().gap(px(8.0))
                            .child(div().px(px(9.0)).py(px(4.0)).rounded(px(6.0)).bg(theme.hover).text_size(px(12.0)).font_weight(FontWeight::MEDIUM).child("最近打开"))
                            .child(div().text_size(px(11.0)).text_color(theme.muted).child(self.snapshot.recent.len().to_string())))
                        .child(div().flex().flex_none().items_center().gap(px(8.0))
                            .child(small_icon_button("home-refresh",IconName::RefreshCw,false,&theme).tooltip(tooltip("刷新记录和预览"))
                                .on_click(cx.listener(|home,_,_,cx|home.refresh(cx))))
                            .when(rows>0,|row|row.child(small_icon_button("home-clear",IconName::Trash2,false,&theme)
                                .tooltip(tooltip("清空最近记录（保留磁盘文件）"))
                                .on_click(cx.listener(|home,_,_,cx|home.library.update(cx,|library,cx|library.change(Change::ClearRecent,cx))))))))
                    .children(error.map(|error|div().text_size(px(12.0)).text_color(theme.danger).child(error)))
                    .child(div().flex_1().min_h(px(0.0)).overflow_hidden().child(content))))
    }
}

fn document_name(path: &Path) -> SharedString {
    let markdown = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"));
    let name = if markdown { path.file_stem() } else { path.file_name() }.unwrap_or_else(|| path.as_os_str());
    name.to_string_lossy().chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect::<String>().into()
}

pub(crate) fn entry_name(path: &Path) -> SharedString {
    path.file_name().unwrap_or_else(|| path.as_os_str()).to_string_lossy().chars().map(|ch|if ch.is_control(){' '}else{ch}).collect::<String>().into()
}

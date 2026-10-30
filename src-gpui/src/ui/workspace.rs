//! 窗口根视图：标题栏标签、侧栏文件树、工具栏、编辑区（语义 / 源码 / 分栏）、大纲浮卡、搜索与浮动统计。
//!
//! 每个标签持有一个窗口化 DocumentHost；完整正文、历史与保存归共享文档会话服务。

use crate::editor::ViewMode;
use crate::nomo_mark::{document_host::{DocumentHost, DocumentEvent, EditTarget, OutlineHeading as Heading}, native_port::NativeWindowPort, session_window::WindowRequest};
use crate::text_document::markdown_session::MarkdownDocumentSessionManager;
use crate::settings::{EditorMode, Settings, StatsMetric};
use crate::theme::Theme;
use crate::ui::components::{ButtonKind, IconName, button, floating_card, icon, slider, small_icon_button, tooltip};
use crate::ui::file_tree::FileTree;
use crate::ui::home::{self, Home, HomeEvent, library::{Library, Change as LibraryChange}};
use crate::nomo_mark::{link_edit::LinkDraft, protocol::ControlError};
use crate::ui::editor_popovers::{EditorPopover, EditorPopoverEvent, EditorPopoverKind};
use crate::ui::text_field::{FieldEvent, TextField};
use crate::ui::chrome::{Chrome, ChromeState, ChromeEvent, ChromeCapability, ChromeCommand};
use crate::ui::workspace_surface::{TAB_RAIL_HEIGHT, WorkspaceSurface};
use crate::ui::motion::{self, MotionPreference, ScalarMotion};
use crate::ui::{dialogs::{self, SaveChoice}, toast::Toast};
use crate::ui::popup::{popup, PopupAnchor, PopupPlacement};
use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, PathPromptOptions, Render, ScrollHandle, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Transformation, UniformListScrollHandle, Window,
    actions, div, ease_in_out, prelude::*, px, radians, uniform_list,
};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

mod explorer;
mod explorer_actions;
mod explorer_ui;
mod explorer_search;
mod outline;
mod toolbar;
mod status;
mod application_dialogs;
mod mini;
mod document_transition;
mod open_requests;
mod recovery_ui;
mod export;
use open_requests::{OpenRequests, OpenRequest, AfterOpen};
use explorer::ExplorerState;

actions!(
    workspace,
    [
        NewFile, NewWindow, OpenFile, OpenFolder, Save, SaveAs, ExportHtml, ExportPdf, ExportWord, CloseTab, NextTab, PrevTab, Find, FindReplace, ToggleFind, ToggleReplace, FindNext,
        FindPrev, ToggleSource, ModeSemantic, ModeSource, ModeSplit, ToggleOutline, ToggleSidebar, ToggleToolbar,
        ToggleTheme, FollowSystemTheme, ToggleTypography, Underline, Highlight, ZoomIn, ZoomOut, ZoomReset, Bold, Italic, Strike, InlineCode, Link,
        Heading1, Heading2, Heading3, Heading4, Heading5, Heading6, ClearFormatting, Paragraph, Quote, BulletList, OrderedList, TaskList, CodeBlock, Table,
        MathBlock, BlockComment, EditFrontMatter, Rule, PromoteHeading, DemoteHeading, OpenSettings, ShowHome, RecoverDocuments, ToggleMini, Quit
    ]
);

pub(crate) struct Tab {
    document: Entity<DocumentHost>,
    pub(crate) mode: EditorMode,
    preview: bool,
    opened_at: Instant,
    dirty: bool,
    dirty_animation_started: Option<Instant>,
    hover: ScalarMotion,
    close_hover: ScalarMotion,
    active_color: ScalarMotion,
    hidden_hover: ScalarMotion,
    autosave: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

struct PendingSave {
    document: Entity<DocumentHost>,
    path: PathBuf,
    follow_current: bool,
    close_after: bool,
}

struct OpeningDocument {
    id: u64,
    generation: u64,
    path: Option<PathBuf>,
    request: WindowRequest,
    preview: bool,
    anchor: Option<String>,
}

const TREE_ROW_HEIGHT: f32 = 30.0;
const TREE_SELECTION_DURATION: Duration = Duration::from_millis(180);
const TREE_CHEVRON_DURATION: Duration = Duration::from_millis(200);

#[derive(Clone, PartialEq, Eq)]
struct TreeSelection {
    path: PathBuf,
    index: usize,
    depth: usize,
}

impl TreeSelection {
    fn position(&self) -> (f32, f32) {
        (32.0 + self.depth as f32 * 12.0, self.index as f32 * TREE_ROW_HEIGHT + 8.0)
    }
}

#[derive(Clone)]
struct TreeSelectionAnimation {
    id: usize,
    started: Instant,
    from: (f32, f32),
    to: (f32, f32),
}

fn tree_selection_ease(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

impl TreeSelectionAnimation {
    fn position(&self) -> (f32, f32) {
        let progress = (self.started.elapsed().as_secs_f32() / TREE_SELECTION_DURATION.as_secs_f32()).min(1.0);
        let progress = tree_selection_ease(progress);
        (
            self.from.0 + (self.to.0 - self.from.0) * progress,
            self.from.1 + (self.to.1 - self.from.1) * progress,
        )
    }
}

fn tree_selection_marker(index: usize, depth: usize, animation: Option<&TreeSelectionAnimation>, color: gpui::Hsla) -> AnyElement {
    let target = (32.0 + depth as f32 * 12.0, 8.0);
    let marker = div().absolute().w(px(2.0)).h(px(14.0)).rounded_full().bg(color).opacity(0.88);
    if let Some(animation) = animation.filter(|animation| animation.started.elapsed() < TREE_SELECTION_DURATION) {
        let from = (animation.from.0, animation.from.1 - index as f32 * TREE_ROW_HEIGHT);
        let id = animation.id;
        marker.with_animation(
            ("tree-selection", id),
            Animation::new(TREE_SELECTION_DURATION).with_easing(tree_selection_ease),
            move |marker, progress| marker
                .left(px(from.0 + (target.0 - from.0) * progress))
                .top(px(from.1 + (target.1 - from.1) * progress)),
        ).into_any_element()
    } else {
        marker.left(px(target.0)).top(px(target.1)).into_any_element()
    }
}

#[derive(Clone)]
struct TreeChevronAnimation {
    path: PathBuf,
    id: usize,
    started: Instant,
    from: f32,
    to: f32,
}

impl TreeChevronAnimation {
    fn rotation(&self) -> f32 {
        let progress = (self.started.elapsed().as_secs_f32() / TREE_CHEVRON_DURATION.as_secs_f32()).min(1.0);
        self.from + (self.to - self.from) * ease_in_out(progress)
    }
}

fn tree_chevron(expanded: bool, path: Option<&Path>, animation: Option<&TreeChevronAnimation>, color: gpui::Hsla) -> AnyElement {
    let target = if expanded { 0.0 } else { -std::f32::consts::FRAC_PI_2 };
    let arrow = icon(IconName::ChevronDown, 12.0, color);
    if let Some(animation) = animation.filter(|animation| {
        path.is_some_and(|path| path == animation.path) && animation.started.elapsed() < TREE_CHEVRON_DURATION
    }) {
        let (id, from, to) = (animation.id, animation.from, animation.to);
        arrow.with_animation(
            ("tree-chevron", id),
            Animation::new(TREE_CHEVRON_DURATION).with_easing(ease_in_out),
            move |arrow, progress| arrow.with_transformation(Transformation::rotate(radians(from + (to - from) * progress))),
        ).into_any_element()
    } else {
        arrow.with_transformation(Transformation::rotate(radians(target))).into_any_element()
    }
}

#[derive(Clone)]
struct DocumentServices {manager:MarkdownDocumentSessionManager,_lease:std::sync::Arc<crate::text_document::markdown_session::SessionLease>}
impl gpui::Global for DocumentServices {}

pub(crate) struct SearchPanel {
    pub(crate) query: Entity<TextField>,
    pub(crate) replacement: Entity<TextField>,
    show_replace: bool,
    case_sensitive: bool,
    whole_word: bool,
    backwards: bool,
    wrap_around: bool,
    _subscriptions: Vec<Subscription>,
}

/// 浮层只保留窗口身份/选区和弱文档引用；丢弃时释放输入框及订阅。
struct PendingPopover {
    document: gpui::WeakEntity<DocumentHost>,
    target: EditTarget,
    link: Option<LinkDraft>,
    view: Entity<EditorPopover>,
    _subscription: Subscription,
}

pub struct Workspace {
    focus: FocusHandle,
    home_visible: bool,
    home: Entity<Home>,
    library: Entity<Library>,
    _home_subscription: Subscription,
    _library_subscription: Subscription,
    chrome: Entity<Chrome>,
    _chrome_subscription: Subscription,
    _motion_activation: Subscription,
    _motion_observer: Subscription,
    #[cfg(target_os = "linux")]
    motion_query: Option<Task<()>>,
    services: Option<MarkdownDocumentSessionManager>,
    owner: String,
    opening: Vec<OpeningDocument>,
    open_requests: OpenRequests,
    pending_saves: Vec<PendingSave>,
    export_job: Option<export::ExportJob>,
    pending_document_transition: Option<document_transition::PendingDocumentTransition>,
    pending_tree_click: Option<Task<()>>,
    deferred_open: Option<(PathBuf, bool, Option<String>, u64)>,
    tree_animation: Option<TreeChevronAnimation>,
    tree_animation_id: usize,
    tree_selection: Option<TreeSelection>,
    tree_selection_animation: Option<TreeSelectionAnimation>,
    tree_selection_animation_id: usize,
    close_prompt: bool,
    closing_window: bool,
    waiting_library_close: bool,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    tab_scroll: ScrollHandle,
    toolbar_scroll: ScrollHandle,
    toolbar_trigger_bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    toolbar_popup_anchors: [Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>; 5],
    toolbar_popup: Option<toolbar::ToolbarPopup>,
    toolbar_popup_focus: FocusHandle,
    toolbar_target: Option<bool>,
    toolbar_motion: Option<(Instant, usize, f32, f32, Duration)>,
    toolbar_motion_id: usize,
    toolbar_motion_task: Option<Task<()>>,
    toolbar_mode_target: Option<f32>,
    toolbar_mode_motion: Option<(Instant, usize, f32, f32)>,
    toolbar_mode_motion_id: usize,
    split_guide_visible: bool,
    tab_overflow: Rc<Cell<bool>>,
    tab_menu_open: bool,
    tab_menu_button_bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    tab_action_hover: ScalarMotion,
    surface: WorkspaceSurface,
    status_ui: status::StatusUi,
    mini: mini::MiniState,
    tree: Option<FileTree>,
    explorer: ExplorerState,
    pub(crate) search: Option<SearchPanel>,
    popover: Option<PendingPopover>,
    outline_scroll: UniformListScrollHandle,
    outline_ui: outline::OutlineUi,
    tree_scroll: UniformListScrollHandle,
    /// 待显示的短提示；渲染时消费，同一条消息再次出现也会重新计时。
    notice: Option<SharedString>,
    failure_notice: Option<SharedString>,
    last_notice: Option<SharedString>,
    toast: Option<Entity<Toast>>,
    /// 工具栏内容宽度滑杆是否正在拖动。
    width_dragging: bool,
    application_dialogs: application_dialogs::ApplicationDialogs,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Workspace {
    pub fn new(paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home_visible = paths.is_empty();
        let library = home::library::shared(cx);
        // 编辑页独占连体标签轮廓；主页使用平面左右分栏。
        let surface = WorkspaceSurface::default();
        let home = cx.new(|cx| Home::new(library.clone(), home_visible, cx));
        let home_subscription = cx.subscribe_in(&home, window, |workspace, _, event, window, cx| match event {
            HomeEvent::Open(path) => workspace.open_path(path, window, cx),
            HomeEvent::NewFile => workspace.new_file(window, cx),
            HomeEvent::OpenFile => workspace.prompt_open(false, window, cx),
            HomeEvent::OpenFolder => workspace.prompt_open(true, window, cx),
            HomeEvent::AddFolder => workspace.prompt_home_folder(window, cx),
            HomeEvent::Resume => workspace.activate(workspace.active, window, cx),
        });
        let mut last_library_error = None;
        let library_subscription = cx.observe_in(&library, window, move |workspace, library, window, cx| {
            let error = library.read(cx).error.as_ref();
            let error_key = error.map(|error| {
                let mut hash = DefaultHasher::new();
                error.hash(&mut hash);
                hash.finish()
            });
            // 任务结束也会 notify；持续的同一个后台错误只提示一次，恢复后允许再次提示。
            if !workspace.home_visible && error_key != last_library_error {
                if let Some(error) = error { workspace.failure_notice = Some(error.clone().into()); }
            }
            last_library_error = error_key;
            if workspace.waiting_library_close && !library.read(cx).is_busy() {
                workspace.waiting_library_close = false;
                if workspace.tabs.is_empty() && workspace.opening.is_empty() { window.remove_window(); }
            }
            cx.notify();
        });
        // 一个应用共享索引/搜索/统计服务；进程目录避免第二实例清理活动缓存。
        let service = if let Some(service)=cx.try_global::<DocumentServices>() {Ok(service.manager.clone())} else {
            let root = crate::settings::config_path().and_then(|p|p.parent().map(|p|p.join("gpui-sessions")))
                .unwrap_or_else(||std::env::temp_dir().join("nomo-gpui-sessions"))
                .join(format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
            crate::text_document::markdown_session::SessionLease::acquire(&root).and_then(|lease| MarkdownDocumentSessionManager::new(root).map(|manager| {cx.set_global(DocumentServices{manager:manager.clone(),_lease:std::sync::Arc::new(lease)});manager}))
        };
        let service_error=service.as_ref().err().map(|e|format!("无法初始化文档会话：{e}").into());
        let this=cx.entity().downgrade();
        window.on_window_should_close(cx,move |window,cx| this.update(cx,|workspace,cx| workspace.request_close_window(window,cx)).unwrap_or(true));
        let chrome=cx.new(Chrome::new);
        let chrome_subscription=cx.subscribe_in(&chrome,window,|workspace,_,event,window,cx|workspace.chrome_event(event,window,cx));
        let motion_activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if window.is_window_active() {
                workspace.refresh_system_motion(cx);
                if let Some(document)=workspace.document(){let _=document.update(cx,|host,cx|host.set_active(true,cx));}
                if workspace.home_visible { workspace.home.update(cx, |home, cx| home.refresh(cx)); }
            }
        });
        let motion_observer = cx.observe_global::<MotionPreference>(|workspace, cx| {
            let reduced = cx.global::<MotionPreference>().reduced;
            workspace.set_motion_preference(reduced);
            cx.notify();
        });
        Theme::arm_ui_transition(cx);
        let mut workspace = Self {
            focus: cx.focus_handle(),
            home_visible, home, library,
            _home_subscription: home_subscription,
            _library_subscription: library_subscription,
            chrome,
            _chrome_subscription: chrome_subscription,
            _motion_activation: motion_activation,
            _motion_observer: motion_observer,
            #[cfg(target_os = "linux")]
            motion_query: None,
            services: service.ok(),
            owner: format!("workspace-{:?}",cx.entity_id()),
            opening: Vec::new(), open_requests: OpenRequests::default(), pending_saves: Vec::new(), export_job: None, pending_document_transition: None, pending_tree_click: None, deferred_open: None,
            tree_animation: None, tree_animation_id: 0,
            tree_selection: None, tree_selection_animation: None, tree_selection_animation_id: 0,
            close_prompt:false, closing_window:false, waiting_library_close:false,
            tabs: Vec::new(),
            active: 0,
            tab_scroll: ScrollHandle::new(),
            toolbar_scroll: ScrollHandle::new(),
            toolbar_trigger_bounds: Rc::new(Cell::new(None)),
            toolbar_popup_anchors: std::array::from_fn(|_| Rc::new(Cell::new(None))),
            toolbar_popup: None,
            toolbar_popup_focus: cx.focus_handle(),
            toolbar_target: None,
            toolbar_motion: None,
            toolbar_motion_id: 0,
            toolbar_motion_task: None,
            toolbar_mode_target: None,
            toolbar_mode_motion: None,
            toolbar_mode_motion_id: 0,
            split_guide_visible: false,
            tab_overflow: Rc::new(Cell::new(false)),
            tab_menu_open: false,
            tab_menu_button_bounds: Rc::new(Cell::new(None)),
            tab_action_hover: ScalarMotion::default(),
            surface,
            status_ui: status::StatusUi::new(cx),
            tree: None,
            explorer: ExplorerState::default(),
            search: None,
            popover: None,
            outline_scroll: UniformListScrollHandle::new(),
            outline_ui: outline::OutlineUi::new(window, cx),
            tree_scroll: UniformListScrollHandle::new(),
            notice: None,
            failure_notice: service_error,
            last_notice: None,
            toast: None,
            width_dragging: false,
            application_dialogs: Default::default(),
            mini: Default::default(),
        };
        workspace.refresh_system_motion(cx);
        workspace.init_tree_search(window, cx);
        for path in paths {
            workspace.enqueue_open(Some(path), false, None, false, cx);
        }
        workspace.drain_open_requests(window, cx);
        window.focus(&workspace.focus, cx);
        workspace.start_application_dialogs(window, cx);
        workspace
    }

    fn set_motion_preference(&mut self, reduced: bool) {
        self.explorer.reduce_motion = reduced;
        self.outline_ui.set_reduced_motion(reduced);
        self.surface.set_reduced_motion(reduced);
        self.tab_action_hover.retime(reduced, Duration::from_millis(150));
        for tab in &mut self.tabs {
            tab.hover.retime(reduced, Duration::from_millis(150));
            tab.close_hover.retime(reduced, Duration::from_millis(150));
            tab.active_color.retime(reduced, Duration::from_millis(150));
            tab.hidden_hover.retime(reduced, Duration::from_millis(120));
        }
        if reduced {
            self.tree_animation = None;
            self.tree_selection_animation = None;
            self.explorer.scroll_animation = None;
        }
    }

    fn apply_system_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        self.set_motion_preference(reduced);
        if reduced { Theme::finish_ui_transition(cx); }
        if cx.global::<MotionPreference>().reduced != reduced {
            cx.set_global(MotionPreference { reduced });
        }
        cx.notify();
    }

    fn refresh_system_motion(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "windows")]
        self.apply_system_motion(explorer::system_reduce_motion(), cx);
        #[cfg(target_os = "macos")]
        self.apply_system_motion(motion::macos_reduce_motion(), cx);
        #[cfg(target_os = "linux")]
        {
            let query = cx.background_executor().spawn(motion::linux_reduce_motion());
            self.motion_query = Some(cx.spawn(async move |workspace, cx| {
                let result = query.await;
                let _ = workspace.update(cx, |workspace, cx| match result {
                    Ok(reduced) => workspace.apply_system_motion(reduced, cx),
                    Err(error) => eprintln!("{error}"),
                });
            }));
        }
    }

    pub(crate) fn tab(&self) -> Option<&Tab> {
        if self.home_visible { None } else { self.tabs.get(self.active) }
    }

    pub(crate) fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing_window || window.has_active_prompt() { return; }
        self.next_open_intent();
        self.open_requests.pending.clear();
        self.pending_tree_click.take();
        self.deferred_open = None;
        for opening in &self.opening { opening.request.cancel(); }
        if self.home_visible { return; }
        if let Some(document) = self.document() {
            if !self.prepare_document_transition(&document, false, window, cx, |workspace, window, cx| workspace.show_home(window, cx)) { return; }
        }
        if !self.leave_mini(window, cx) { return; }
        if let Some(document) = self.document() {
            if let Err(error) = document.update(cx, |host, cx| host.set_active(false, cx)) {
                self.notice = Some(error.to_string().into()); cx.notify(); return;
            }
        }
        for opening in &self.opening { opening.request.cancel(); }
        self.pending_tree_click.take();
        self.deferred_open = None;
        self.popover = None;
        self.toolbar_popup = None;
        self.tab_menu_open = false;
        self.search = None;
        self.close_status(cx);
        self.outline_ui.clear_document();
        self.home_visible = true;
        self.suspend_tree(cx);
        self.surface.clear();
        self.home.update(cx, |home, cx| home.set_active(true, cx));
        window.set_window_title("Nomo");
        window.focus(&self.home.focus_handle(cx), cx);
        cx.notify();
    }

    fn prompt_home_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let response = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("添加文件夹".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = response.await {
                let _ = this.update(cx, |workspace, cx| {
                    for path in paths {
                        let path = path.canonicalize().unwrap_or(path);
                        workspace.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(path), cx));
                    }
                });
            }
        }).detach();
    }

    fn document(&self) -> Option<Entity<DocumentHost>> { self.tab().map(|tab|tab.document.clone()) }

    pub fn open_path(&mut self,path:&Path,window:&mut Window,cx:&mut Context<Self>) {
        if self.waiting_library_close { self.waiting_library_close=false; self.cancel_close(cx); }
        self.pending_tree_click.take();
        self.deferred_open=None;
        self.enqueue_open(Some(path.to_path_buf()), false, None, true, cx);
        self.drain_open_requests(window, cx);
    }

    fn cancel_preview_opens(&self, except: Option<&Path>) {
        for opening in &self.opening {
            if opening.preview && opening.path.as_deref() != except {
                opening.request.cancel();
            }
        }
    }

    fn open_path_with_preview(&mut self,path:&Path,preview:bool,window:&mut Window,cx:&mut Context<Self>) {
        self.open_path_with_anchor(path,preview,None,window,cx);
    }

    fn open_markdown_link(&mut self,path:&Path,anchor:Option<String>,window:&mut Window,cx:&mut Context<Self>) {
        self.pending_tree_click.take();
        self.deferred_open=None;
        self.open_path_with_anchor(path,false,anchor,window,cx);
    }

    fn open_path_with_anchor(&mut self,path:&Path,preview:bool,anchor:Option<String>,window:&mut Window,cx:&mut Context<Self>) {
        self.enqueue_open(Some(path.to_path_buf()), preview, anchor, false, cx);
        self.drain_open_requests(window, cx);
    }

    fn open_path_now(&mut self,path:&Path,preview:bool,anchor:Option<String>,generation:u64,window:&mut Window,cx:&mut Context<Self>) {
        if !self.leave_mini(window,cx) {return;}
        if self.explorer.mutating.iter().any(|busy| path.starts_with(busy)) {self.notice=Some("文件操作正在进行，请稍后打开。".into());cx.notify();return;}
        if path.is_dir() {
            self.cancel_preview_opens(None);
            let root=path.canonicalize().unwrap_or_else(|_|path.to_path_buf());
            if generation==self.open_requests.generation {
                if self.home_visible && !self.tabs.is_empty() {
                    let document=self.tabs[self.active].document.clone();
                    let retry=root.clone();
                    if !self.prepare_document_transition(&document,false,window,cx,move |workspace,window,cx|workspace.open_path_now(&retry,false,None,generation,window,cx)) {return;}
                    self.activate_after_open(self.active,generation,AfterOpen::default(),window,cx);
                    if self.home_visible {return;}
                }
                self.home_visible = false;
                self.home.update(cx, |home, cx| home.set_active(false, cx));
            }
            self.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(root.clone()), cx));
            Settings::set(cx, "sidebarHidden", serde_json::json!(false));
            self.replace_tree(root,cx);
            cx.notify();
            return;
        }
        let path=path.canonicalize().unwrap_or_else(|_|path.to_path_buf());
        if self.explorer.mutating.iter().any(|busy| path.starts_with(busy)) {self.notice=Some("文件操作正在进行，请稍后打开。".into());cx.notify();return;}
        let preview=preview && cx.global::<Settings>().bool("filePreviewEnabled");
        self.cancel_preview_opens(Some(&path));
        if let Some(index)=self.tabs.iter().position(|tab|tab.document.read(cx).path().as_ref()==Some(&path)) {
            if !preview {self.tabs[index].preview=false;}
            self.activate_after_open(index,generation,AfterOpen {anchor,..Default::default()},window,cx);
            return;
        }
        if let Some(opening)=self.opening.iter_mut().find(|opening|opening.path.as_ref()==Some(&path) && opening.request.check().is_ok()) {
            opening.generation = generation;
            if !preview {opening.preview=false;}
            if anchor.is_some() {opening.anchor=anchor;}
            return;
        }
        // 同一路径的过期请求必须先释放会话，才能重新打开。
        if self.opening.iter().any(|opening|opening.path.as_ref()==Some(&path)) {
            self.deferred_open=Some((path,preview,anchor,generation));
            return;
        }
        self.open_document_from(Some(path),preview,anchor,application_dialogs::OpenSource::File,generation,window,cx);
    }

    fn open_tree_file(&mut self,path:PathBuf,click_count:usize,window:&mut Window,cx:&mut Context<Self>) {
        self.pending_tree_click.take();
        self.deferred_open=None;
        let path=path.canonicalize().unwrap_or(path);
        self.cancel_preview_opens(Some(&path));
        if click_count>=2 {
            self.open_path_with_preview(&path,false,window,cx);
        } else {
            self.pending_tree_click=Some(cx.spawn_in(window,async move |this,cx| {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let _=this.update_in(cx,|workspace,window,cx|workspace.open_path_with_preview(&path,true,window,cx));
            }));
        }
    }

    fn sync_tree_selection(&mut self, animate: bool, cx: &App) {
        let next = self.tab().and_then(|tab| tab.document.read(cx).path()).and_then(|path| {
            let rows = self.tree.as_ref()?.rows();
            let insertion = self.tree_create_insertion(&rows);
            rows.iter().enumerate()
                .find(|(_, row)| !row.entry.is_dir && (row.entry.path == path || row.entry.resolved_path.as_ref() == Some(&path)))
                .map(|(index, row)| {
                    let index = index + 1 + usize::from(insertion.is_some_and(|insert| insert <= index + 1));
                    TreeSelection { path, index, depth: row.depth }
                })
        });
        let changed = self.tree_selection != next;
        if animate && !changed { return; }

        let previous = self.tree_selection.take();
        let previous_animation = self.tree_selection_animation.take();
        let (offset, height) = {
            let scroll = self.tree_scroll.0.borrow();
            (f32::from(scroll.base_handle.offset().y), f32::from(scroll.base_handle.bounds().size.height))
        };
        let visible = |index: usize| {
            let top = index as f32 * TREE_ROW_HEIGHT + offset;
            height > 0.0 && top < height && top + TREE_ROW_HEIGHT > 0.0
        };

        if animate && changed {
            if let (Some(previous), Some(next)) = (&previous, &next) {
                if !self.explorer.reduce_motion && visible(previous.index) && visible(next.index) {
                    let from = previous_animation.as_ref()
                        .filter(|animation| animation.started.elapsed() < TREE_SELECTION_DURATION)
                        .map(TreeSelectionAnimation::position)
                        .unwrap_or_else(|| previous.position());
                    self.tree_selection_animation_id = self.tree_selection_animation_id.wrapping_add(1);
                    self.tree_selection_animation = Some(TreeSelectionAnimation {
                        id: self.tree_selection_animation_id,
                        started: Instant::now(),
                        from,
                        to: next.position(),
                    });
                }
            }
            if let Some(next) = &next {
                if height > 0.0 { self.tree_scroll.scroll_to_item(next.index, ScrollStrategy::Top); }
            }
        }
        self.tree_selection = next;
    }

    fn animate_tree_chevron(&mut self,path:PathBuf,expanded:bool) {
        if self.explorer.reduce_motion { self.tree_animation=None; return; }
        let from=self.tree_animation.as_ref()
            .filter(|animation|animation.path==path && animation.started.elapsed()<TREE_CHEVRON_DURATION)
            .map(TreeChevronAnimation::rotation)
            .unwrap_or(if expanded {-std::f32::consts::FRAC_PI_2} else {0.0});
        let to=if expanded {0.0} else {-std::f32::consts::FRAC_PI_2};
        self.tree_animation_id=self.tree_animation_id.wrapping_add(1);
        self.tree_animation=Some(TreeChevronAnimation {path,id:self.tree_animation_id,started:Instant::now(),from,to});
    }

    fn open_document_from(&mut self,path:Option<PathBuf>,preview:bool,anchor:Option<String>,source:application_dialogs::OpenSource,generation:u64,window:&mut Window,cx:&mut Context<Self>) {
        let Some(manager)=self.services.clone() else {return;};
        if let Some(current)=self.document() {
            let (path, anchor, source) = (path.clone(), anchor.clone(), source.clone());
            if !self.prepare_document_transition(&current, false, window, cx, move |workspace, window, cx| {
                workspace.open_document_from(path, preview, anchor, source, generation, window, cx);
            }) { return; }
        }
        if !self.begin_recovery_attempt(&source,cx) {return;}
        let request=WindowRequest::default();
        let open_profile=crate::open_profile::OpenProfile::begin();
        let loading_profile=open_profile.clone();
        // 查询服务按 owner 取消任务；每文档独立 owner 防止后台标签误取消活动文档。
        static NEXT_DOCUMENT: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let id=NEXT_DOCUMENT.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let owner=match &source { application_dialogs::OpenSource::Reload(document,_) | application_dialogs::OpenSource::DiskRecovery(_,Some(document),_) | application_dialogs::OpenSource::Recovery(_,false,Some(document),_) => document.read(cx).port().owner, _ => format!("{}:document-{}",self.owner,id) };
        self.opening.push(OpeningDocument {id,generation,path:path.clone(),request:request.clone(),preview,anchor});
        let requested=path.clone();
        let recovery=match &source {application_dialogs::OpenSource::Recovery(candidate,copy,_,_)=>Some((candidate.clone(),*copy)),_=>None};
        let fresh=matches!(&source,application_dialogs::OpenSource::Reload(_,_)|application_dialogs::OpenSource::DiskRecovery(_,_,_));
        self.last_notice=Some("正在打开文档…".into());
        let task=cx.background_executor().spawn(async move {
            request.check()?;
            let port=match recovery {
                Some((candidate,copy)) => { let status=manager.adopt_recovery(&owner,&candidate,copy).map_err(|error|ControlError::HostFailure(error.to_string()))?; NativeWindowPort {manager,owner,document_id:status.document_id} },
                None if fresh => { let status=manager.open_with_policy(&owner,path.clone(),None,true).map_err(|error|ControlError::HostFailure(error.to_string()))?; NativeWindowPort {manager,owner,document_id:status.document_id} },
                _ => NativeWindowPort::open_in(manager,owner,path.clone(),path.is_none().then(String::new))?,
            };
            if let Some(profile)=&loading_profile {profile.stage("session_ready_ms");}
            let loaded=port.status().and_then(|status|port.load_initial_group(status.revision,&request));
            if let (Some(profile),Ok(loaded))=(&loading_profile,&loaded) {
                profile.stage("window_ready_ms");
                if let Ok(progress)=port.manager.query_structure(&port.owner,loaded.index.clone(),crate::text_document::StructureQuery::Progress) {
                    profile.value("indexed_bytes_at_window",progress.indexed_bytes);
                    profile.value("document_bytes",progress.total_bytes);
                }
            }
            Ok::<_,ControlError>((port,loaded))
        });
        cx.spawn_in(window,async move |this,cx| {
            let (result,cleanup)=match task.await {
                Ok((port,Ok(loaded)))=>(Ok((port.clone(),loaded)),Some(port)),
                Ok((port,Err(error)))=>(Err(error),Some(port)),
                Err(error)=>(Err(error),None),
            };
            let completed_path=requested.clone();
            let accepted=this.update_in(cx,|workspace,window,cx| {
                let Some(opening)=workspace.opening.iter().find(|opening|opening.id==id) else {return false;};
                if opening.request.check().is_err() {return false;}
                let preview=opening.preview && cx.global::<Settings>().bool("filePreviewEnabled");
                let anchor=opening.anchor.clone();
                let generation=opening.generation;
                match result {
                    Ok((port,loaded))=> {
                        let mode=match &source {application_dialogs::OpenSource::Reload(previous,_)|application_dialogs::OpenSource::DiskRecovery(_,Some(previous),_)|application_dialogs::OpenSource::Recovery(_,false,Some(previous),_)=>workspace.tabs.iter().find(|tab|tab.document==*previous).map(|tab|tab.mode).unwrap_or(cx.global::<Settings>().default_mode),_=>cx.global::<Settings>().default_mode};
                        let status=match port.status(){Ok(status)=>status,Err(e)=>{workspace.notice=Some(e.to_string().into());return false;}};
                        let adapter=match crate::nomo_mark::host::WindowHostAdapter::new(loaded.input,"workspace-window",0,loaded.context) {
                            Ok(adapter)=>adapter,Err(e)=>{workspace.notice=Some(e.to_string().into());return false;}
                        };
                        let mut reload_selection=None;
                        if let application_dialogs::OpenSource::Recovery(_,false,Some(previous),revision)=&source {
                            let Some(index)=workspace.tabs.iter().position(|tab|tab.document==*previous) else {return false;};
                            if previous.read(cx).status().revision!=*revision || previous.read(cx).is_dirty(cx) {
                                workspace.failure_notice=Some("当前文档已有新修改，草稿仍保留，请使用另存草稿。".into());return false;
                            }
                            workspace.cancel_export_for(previous);
                            if let Err(error)=previous.update(cx,|host,cx|host.close_document(false,cx)){workspace.failure_notice=Some(error.to_string().into());return false;}
                            application_dialogs::clear_external_after_save(workspace,previous,cx);
                            workspace.forget_document_transition(previous);
                            workspace.pending_saves.retain(|pending|pending.document!=*previous);
                            workspace.tabs.remove(index);workspace.active=workspace.active.saturating_sub(usize::from(index<workspace.active)).min(workspace.tabs.len().saturating_sub(1));
                        }
                        let replacement=match &source {application_dialogs::OpenSource::Reload(previous,revision)=>Some((previous,*revision,true)),application_dialogs::OpenSource::DiskRecovery(_,Some(previous),revision)=>Some((previous,*revision,false)),_=>None};
                        if let Some((previous,revision,discard))=replacement {
                            let Some(index)=workspace.tabs.iter().position(|tab|tab.document==*previous) else {return false;};
                            if previous.read(cx).status().revision!=revision {workspace.failure_notice=Some("文档已有新修改，重载已取消。".into());return false;}
                            reload_selection=Some(previous.read(cx).selection(cx));
                            workspace.cancel_export_for(previous);
                            if let Err(error)=previous.update(cx,|host,cx|host.close_document(discard,cx)) {workspace.failure_notice=Some(error.to_string().into());return false;}
                            application_dialogs::clear_external_after_save(workspace,previous,cx);
                            workspace.forget_document_transition(previous);
                            workspace.pending_saves.retain(|pending|pending.document!=*previous);
                            workspace.tabs.remove(index); workspace.active=workspace.active.saturating_sub(usize::from(index<workspace.active)).min(workspace.tabs.len().saturating_sub(1));
                            workspace.search=None; workspace.popover=None;
                        }
                        let document=cx.new(|cx|DocumentHost::workspace(port,adapter,status,if mode==EditorMode::Source {ViewMode::Source}else{ViewMode::Semantic},open_profile,cx));
                        application_dialogs::mark_recovered_change(workspace,&document,&source,cx);
                        if mode==EditorMode::Split {if let Err(e)=document.update(cx,|host,cx|host.change_mode(mode,cx)) {workspace.notice=Some(e.to_string().into());return false;}}
                        let subscriptions=vec![
                            cx.subscribe_in(&document,window,|workspace,document,event,window,cx| {
                                match event {
                                    DocumentEvent::Changed=>{
                                        if document.read(cx).is_dirty(cx) {
                                            if let Some(tab)=workspace.tabs.iter_mut().find(|tab|tab.document==*document) {tab.preview=false;}
                                        }
                                        workspace.schedule_autosave(document.clone(),window,cx);
                                    }
                                    DocumentEvent::Failure(message)=>workspace.failure_notice=Some(message.clone().into()),
                                    DocumentEvent::Notice(message)=>workspace.notice=Some(message.clone().into()),
                                    DocumentEvent::OpenMarkdownLink {path,anchor}=>workspace.open_markdown_link(path,anchor.clone(),window,cx),
                                    DocumentEvent::TocSettled(warning)=>{
                                        if let Some(warning)=warning {
                                            let locale=crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
                                            workspace.notice=Some(format!("{}: {warning}",locale.text("toc_warning")).into());
                                        }
                                        if let Some(index)=workspace.pending_saves.iter().position(|pending| pending.document==*document) {
                                            let pending=workspace.pending_saves.remove(index);
                                            if warning.is_some() && document.read(cx).has_failed_input(cx) {
                                                let locale=crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
                                                workspace.notice=Some(format!("{}: {}",locale.text("save_blocked"),warning.as_deref().unwrap_or("")).into());
                                                workspace.cancel_close(cx);
                                            } else {
                                                workspace.save_path(pending.document,pending.path,pending.follow_current,pending.close_after,window,cx);
                                            }
                                        }
                                    },
                                    DocumentEvent::Saved=>{workspace.refresh_tree(cx);workspace.sync_tree_selection(false,cx);},
                                    DocumentEvent::Status|DocumentEvent::ToolbarState=>{},
                                }
                                cx.notify();
                            }),
                            cx.observe(&document,|_,_,cx|cx.notify()),
                        ];
                        let _=document.update(cx,|host,cx|host.set_active(false,cx));
                        if let Some(path)=document.read(cx).path() {workspace.library.update(cx,|library,cx|library.change(LibraryChange::Opened(path),cx));}
                        if workspace.tree.is_none() {if let Some(parent)=document.read(cx).path().as_ref().or(requested.as_ref()).and_then(|p|p.parent()){workspace.replace_tree(parent.to_path_buf(),cx);}}
                        if preview {for tab in &mut workspace.tabs {tab.preview=false;}}
                        let dirty=document.read(cx).is_dirty(cx);
                        workspace.tabs.push(Tab {document,mode,preview,opened_at:Instant::now(),dirty,dirty_animation_started:dirty.then(Instant::now),hover:ScalarMotion::default(),close_hover:ScalarMotion::default(),active_color:ScalarMotion::default(),hidden_hover:ScalarMotion::default(),autosave:None,_subscriptions:subscriptions});
                        let save_as=matches!(&source, application_dialogs::OpenSource::Recovery(_,true,_,_));
                        workspace.activate_after_open(workspace.tabs.len()-1,generation,AfterOpen {anchor,selection:reload_selection,save_as},window,cx);
                        true
                    }
                    Err(e)=>{workspace.failure_notice=Some(format!("打开失败：{e}").into());cx.notify();false}
                }
            }).unwrap_or(false);
            let mut recovery_finished=accepted;
            if accepted {if let application_dialogs::OpenSource::DiskRecovery(candidate,_,_)=&source {
                let candidate=candidate.clone();
                let manager=cleanup.as_ref().map(|port|port.manager.clone());
                if let Some(manager)=manager {
                    if let Err(error)=cx.background_executor().spawn(async move{manager.dismiss_recovery(&candidate)}).await {
                        recovery_finished=false;
                        let _=this.update_in(cx,|workspace,_,cx|{workspace.failure_notice=Some(format!("磁盘版本已打开，旧草稿清理失败：{error}").into());cx.notify();});
                    }
                }
            }}
            let cleanup_error=if !accepted {
                let _=this.update_in(cx,|workspace,_,_| {
                    if let Some(opening)=workspace.opening.iter().find(|opening|opening.id==id) {opening.request.cancel();}
                });
                if let Some(port)=cleanup {cx.background_executor().spawn(async move {port.release_unmounted()}).await.err()} else {None}
            } else {None};
            let _=this.update_in(cx,|workspace,window,cx| {
                let mut completed_path=completed_path.clone();
                if let Some(index)=workspace.opening.iter().position(|opening|opening.id==id) {
                    completed_path=workspace.opening[index].path.clone().or(completed_path);
                    workspace.opening.remove(index);
                }
                let retry=completed_path.as_ref().is_some_and(|completed|workspace.deferred_open.as_ref().is_some_and(|(path,_,_,_)|path==completed));
                if let Some(error)=cleanup_error {
                    if retry {workspace.deferred_open=None;}
                    workspace.notice=Some(format!("清理未完成的文档会话失败：{error}").into());
                    cx.notify();
                } else if retry {
                    if let Some((path,preview,anchor,generation))=workspace.deferred_open.take() {
                        workspace.open_path_now(&path,preview,anchor,generation,window,cx);
                    }
                }
                workspace.finish_recovery_attempt(&source,recovery_finished,cx);
                workspace.drain_open_requests(window,cx);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn activate(&mut self,index:usize,window:&mut Window,cx:&mut Context<Self>) {
        let generation=self.next_open_intent();
        self.activate_after_open(index,generation,AfterOpen::default(),window,cx);
    }

    fn activate_after_open(&mut self,index:usize,generation:u64,after:AfterOpen,window:&mut Window,cx:&mut Context<Self>) {
        if generation!=self.open_requests.generation {return;}
        if index>=self.tabs.len() {return;}
        let document=self.tabs[index].document.clone();
        if let Some(current)=self.document() {
            let target = self.tabs[index].document.downgrade();
            let after=after.clone();
            if !self.prepare_document_transition(&current, false, window, cx, move |workspace, window, cx| {
                if let Some(index) = target.upgrade().and_then(|target| workspace.tabs.iter().position(|tab| tab.document == target)) {
                    workspace.activate_after_open(index, generation, after, window, cx);
                }
            }) { return; }
        }
        if self.document().as_ref() != Some(&document) {
            let target = document.downgrade();
            let after=after.clone();
            if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
                if let Some(index) = target.upgrade().and_then(|target| workspace.tabs.iter().position(|tab| tab.document == target)) {
                    workspace.activate_after_open(index, generation, after, window, cx);
                }
            }) { return; }
        }
        if !self.leave_mini(window,cx) {return;}
        if index!=self.active {
            if let Some(current)=self.document() {if let Err(e)=current.update(cx,|host,cx|host.set_active(false,cx)) {self.notice=Some(e.to_string().into());cx.notify();return;}}
        }
        if let Err(e)=document.update(cx,|host,cx|host.set_active(true,cx)) {self.notice=Some(e.to_string().into());cx.notify();return;}
        let from_home = self.home_visible;
        self.home_visible = false;
        self.home.update(cx, |home, cx| home.set_active(false, cx));
        if from_home { self.resume_tree(cx); }
        if let Some(path) = document.read(cx).path() {
            self.library.update(cx, |library, cx| library.change(LibraryChange::Opened(path), cx));
        }
        self.popover=None;
        self.toolbar_popup=None;
        self.close_status(cx);
        self.active=index;
        self.tab_menu_open=false;
        self.tab_scroll.scroll_to_item(index);
        self.sync_tree_selection(true,cx);
        window.set_window_title(&format!("{} — Nomo",document.read(cx).title()));
        if !window.has_active_prompt() { window.focus(&document.focus_handle(cx), cx); }
        self.last_notice=None;
        self.run_search(cx);
        if let Some(selection)=after.selection {
            let byte=selection.head_byte.min(document.read(cx).status().byte_length);
            document.update(cx,|host,cx|host.navigate(byte,Some(crate::nomo_mark::protocol::DocumentSelection::caret(byte)),cx));
        } else if let Some(anchor)=after.anchor {document.update(cx,|host,cx|host.anchor(anchor,cx));}
        if after.save_as {self.save_document(&document,true,false,window,cx);}
        cx.notify();
    }
    pub(crate) fn set_mode(&mut self,mode:EditorMode,window:&mut Window,cx:&mut Context<Self>) {
        let Some(document)=self.document() else{return;};
        let target = document.downgrade();
        if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
            if workspace.document().as_ref() == target.upgrade().as_ref() { workspace.set_mode(mode, window, cx); }
        }) { return; }
        if !self.leave_mini(window,cx) {return;}
        match document.update(cx,|host,cx|host.change_mode(mode,cx)) {
            Ok(())=>{self.popover=None;self.toolbar_popup=None;self.close_status(cx);self.tabs[self.active].mode=mode;
                let preference=match mode { EditorMode::Semantic=>"semantic", EditorMode::Source=>"source", EditorMode::Split=>"split" };
                Settings::set(cx, "editorMode", serde_json::json!(preference));
                window.focus(&document.focus_handle(cx), cx);}
            Err(e)=>self.notice=Some(e.to_string().into()),
        }
        cx.notify();
    }
    fn close_tab(&mut self,index:usize,window:&mut Window,cx:&mut Context<Self>) {
        if self.close_prompt || window.has_active_prompt() {return;}
        let Some(tab)=self.tabs.get(index) else{return;};
        let document=tab.document.clone();
        let target = document.downgrade();
        if !self.prepare_document_transition(&document, true, window, cx, move |workspace, window, cx| {
            if let Some(index) = target.upgrade().and_then(|target| workspace.tabs.iter().position(|tab| tab.document == target)) {
                workspace.close_tab(index, window, cx);
            }
        }) { return; }
        if !self.leave_mini(window,cx) {return;}
        if !document.read(cx).is_dirty(cx) {self.remove_tab(&document,false,window,cx);return;}
        self.close_prompt=true;
        let response=dialogs::confirm_save(&document.read(cx).title(),window,cx);
        cx.spawn_in(window,async move |this,cx| {
            let choice=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                workspace.close_prompt=false;
                match choice {
                    SaveChoice::Save=>workspace.save_document(&document,false,true,window,cx),
                    SaveChoice::Discard=>workspace.remove_tab(&document,true,window,cx),
                    SaveChoice::Cancel=>{workspace.cancel_close(cx);}
                }
                cx.notify();
            });
        }).detach();
    }
    fn remove_tab(&mut self,document:&Entity<DocumentHost>,discard:bool,window:&mut Window,cx:&mut Context<Self>) {
        let Some(index)=self.tabs.iter().position(|tab|tab.document==*document) else{return;};
        self.cancel_export_for(document);
        if let Err(e)=document.update(cx,|host,cx|host.close_document(discard,cx)) {self.notice=Some(e.to_string().into());self.cancel_close(cx);cx.notify();return;}
        application_dialogs::clear_external_after_save(self,document,cx);
        self.forget_document_transition(document);
        if self.popover.as_ref().and_then(|p|p.document.upgrade()).as_ref()==Some(document) {self.popover=None;}
        self.pending_saves.retain(|pending| &pending.document != document);
        self.tabs.remove(index);
        if index<self.active {self.active-=1;}
        self.active=self.active.min(self.tabs.len().saturating_sub(1));
        if self.tabs.is_empty() {
            self.sync_tree_selection(false,cx);
            self.toolbar_popup=None;
            self.tab_menu_open=false;
            self.search=None;
            self.close_status(cx);
            self.outline_ui.clear_document();
            self.surface.clear();
            window.set_window_title("Nomo");
            if self.closing_window {
                if self.request_close_window(window,cx) {window.remove_window();}
            } else if !self.home_visible && !window.has_active_prompt() {
                window.focus(&self.focus,cx);
            }
        }
        else {if !self.home_visible || self.closing_window {self.activate(self.active,window,cx);}if self.closing_window {self.close_tab(self.active,window,cx);}}
        cx.notify();
    }
    pub(crate) fn request_close_window(&mut self,window:&mut Window,cx:&mut Context<Self>)->bool {
        if window.has_active_prompt() {return false;}
        if self.closing_window && !self.tabs.is_empty() {return false;}
        if let Some(document)=self.document() {
            if !self.prepare_document_transition(&document, true, window, cx, |workspace, window, cx| {
                if workspace.request_close_window(window, cx) { window.remove_window(); }
            }) { return false; }
        }
        if self.mini.active() { self.toggle_mini(window,cx); return false; }
        if !self.application_dialogs.close_approved {self.choose_close_behavior(window,cx);return false;}
        if !self.opening.is_empty() || !self.open_requests.pending.is_empty() {self.notice=Some("文档仍在打开，请稍后关闭".into());cx.notify();return false;}
        if self.tabs.is_empty() {
            if self.library.read(cx).is_busy() {
                self.waiting_library_close=true;
                self.notice=Some("正在保存主页记录…".into());cx.notify();return false;
            }
            return true;
        }
        if !self.closing_window {self.closing_window=true;self.close_tab(self.active,window,cx);}
        false
    }
    fn cancel_close(&mut self,cx:&mut Context<Self>){self.closing_window=false;self.application_dialogs.close_approved=false;crate::cancel_quit(cx);}
    pub(crate) fn request_quit(&mut self,window:&mut Window,cx:&mut Context<Self>)->bool {
        if window.has_active_prompt(){crate::cancel_quit(cx);return false;}
        if self.closing_window && !self.tabs.is_empty() {return false;}
        if let Some(document)=self.document() {
            if !self.prepare_document_transition(&document, true, window, cx, |workspace, window, cx| {
                if workspace.request_quit(window, cx) { window.remove_window(); }
            }) { return false; }
        }
        if !self.leave_mini(window,cx) {crate::cancel_quit(cx);return false;}
        if self.tabs.iter().any(|tab|tab.document.read(cx).is_dirty(cx)){self.reveal_window(window,cx);}
        self.application_dialogs.close_approved=true;
        self.request_close_window(window,cx)
    }
    fn prompt_open(&mut self,directories:bool,window:&mut Window,cx:&mut Context<Self>) {
        if !self.leave_mini(window,cx) {return;}
        let response=cx.prompt_for_paths(PathPromptOptions {files:!directories,directories,multiple:!directories,prompt:Some("打开".into())});
        cx.spawn_in(window,async move |this,cx| {
            if let Ok(Ok(Some(paths)))=response.await {let _=this.update_in(cx,|workspace,window,cx| {workspace.open_paths(paths,window,cx);});}
        }).detach();
    }
    fn save_document(&mut self,document:&Entity<DocumentHost>,save_as:bool,close_after:bool,window:&mut Window,cx:&mut Context<Self>) {
        if !save_as && self.resolve_external_before_save(document,window,cx){return;}
        let existing=document.read(cx).path();
        if !save_as {if let Some(path)=existing.clone() {self.save_path(document.clone(),path,true,close_after,window,cx);return;}}
        let response=match document.update(cx,|host,cx| {
            host.set_save_directory(self.tree.as_ref().map(|tree|tree.root.as_path()));
            host.prompt_save_path(window,cx)
        }) {
            Ok(response)=>response,
            Err(error)=>{self.failure_notice=Some(error.to_string().into());self.cancel_close(cx);cx.notify();return;}
        };
        let document=document.clone();
        cx.spawn_in(window,async move |this,cx| {
            let result=response.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                match result {
                    Ok(Some(path))=>workspace.save_path(document,path,false,close_after,window,cx),
                    Ok(None)=>workspace.cancel_close(cx),
                    Err(error)=>{workspace.failure_notice=Some(error.to_string().into());workspace.cancel_close(cx);cx.notify();}
                }
            });
        }).detach();
    }
    fn save_path(&mut self,document:Entity<DocumentHost>,path:PathBuf,follow_current:bool,close_after:bool,window:&mut Window,cx:&mut Context<Self>) {
        if !self.tabs.iter().any(|tab|tab.document==document) {return;}
        if !document.update(cx,|host,cx|host.toc_ready_for_save(cx)) {
            if let Some(pending)=self.pending_saves.iter_mut().find(|pending|pending.document==document) {
                pending.close_after |= close_after;
            } else {
                self.pending_saves.push(PendingSave {document,path,follow_current,close_after});
            }
            return;
        }
        let (port, captured)=match document.update(cx,|host,cx|host.begin_save(cx)) {Ok(value)=>value,Err(e)=>{self.failure_notice=Some(e.to_string().into());self.cancel_close(cx);cx.notify();return;}};
        let snapshot=cx.global::<Settings>().snapshot_before_save;
        let task=cx.background_executor().spawn(async move {
            port.save_captured(captured, if follow_current { None } else { Some(path) }, snapshot)
        });
        cx.spawn_in(window,async move |this,cx| {
            let result=task.await;
            let _=this.update_in(cx,|workspace,window,cx| {
                let success=document.update(cx,|host,cx|host.finish_save(result,cx));
                if success {
                    self::application_dialogs::clear_external_after_save(workspace,&document,cx);
                    if let Some(path) = document.read(cx).path() {
                        workspace.library.update(cx, |library, cx| library.change(LibraryChange::Saved(path), cx));
                    }
                    workspace.refresh_tree(cx);
                    workspace.sync_tree_selection(false,cx);
                    if workspace.document().as_ref()==Some(&document) {window.set_window_title(&format!("{} — Nomo",document.read(cx).title()));}
                    if close_after {if document.read(cx).is_dirty(cx) {workspace.cancel_close(cx);workspace.notice=Some("保存期间有新修改，文档保持打开".into());}else{workspace.remove_tab(&document,false,window,cx);}}
                }else{workspace.cancel_close(cx);}
                cx.notify();
            });
        }).detach();
    }
    fn schedule_autosave(&mut self,document:Entity<DocumentHost>,window:&mut Window,cx:&mut Context<Self>) {
        if self.external_save_blocked(&document,cx){return;}
        let Some(delay)=cx.global::<Settings>().autosave else{return;};
        if document.read(cx).path().is_none() {return;}
        let Some(tab)=self.tabs.iter_mut().find(|tab|tab.document==document) else{return;};
        tab.autosave=Some(cx.spawn_in(window,async move |this,cx| {
            cx.background_executor().timer(delay).await;
            let _=this.update_in(cx,|workspace,window,cx| {if document.read(cx).is_dirty(cx){workspace.save_document(&document,false,false,window,cx);}});
        }));
    }

    // ---- 搜索 ----

    fn toggle_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.as_ref().is_some_and(|search| search.show_replace == replace) {
            self.close_search(window, cx);
        } else {
            self.open_search(replace, window, cx);
        }
    }

    pub(crate) fn open_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.popover=None;
        let Some(buffer) = self.document() else { return };
        let selected = buffer.read(cx).selected_text(cx);
        let was_open = self.search.is_some();
        if self.search.is_none() {
            let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
            let query = cx.new(|cx| TextField::new(locale.text("find"), cx));
            let replacement = cx.new(|cx| TextField::new(locale.text("replace_with"), cx));
            let subscriptions = vec![
                cx.subscribe_in(&query, window, |workspace, _, event, window, cx| workspace.on_query_event(event, window, cx)),
                cx.subscribe_in(&replacement, window, |workspace, _, event, window, cx| match event {
                    FieldEvent::Submit { .. } => workspace.replace_one(window, cx),
                    FieldEvent::Cancel => workspace.close_search(window, cx),
                    FieldEvent::Changed => {}
                }),
            ];
            self.search = Some(SearchPanel { query, replacement, show_replace: replace, case_sensitive: false, whole_word: false, backwards: false, wrap_around: true, _subscriptions: subscriptions });
        }
        let search = self.search.as_mut().expect("created");
        search.show_replace = replace;
        let query = search.query.clone();
        if !was_open && !selected.is_empty() && !selected.contains('\n') && selected.len() < 200 {
            query.update(cx, |field, cx| field.set_text(&selected, cx));
        } else {
            let text = query.read(cx).content.clone();
            query.update(cx, |field, cx| field.set_text(&text, cx));
        }
        window.focus(&query.focus_handle(cx), cx);
        cx.notify();
    }

    fn on_query_event(&mut self, event: &FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FieldEvent::Changed => self.run_search(cx),
            FieldEvent::Submit { backwards } => { let reverse=self.search.as_ref().is_some_and(|search| search.backwards); self.step_match(!(*backwards ^ reverse), window, cx); },
            FieldEvent::Cancel => self.close_search(window, cx),
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let (Some(search), Some(buffer)) = (&self.search, self.document()) else { return };
        let query = search.query.read(cx).content.clone();
        let case = search.case_sensitive;
        let whole_word = search.whole_word;
        buffer.update(cx, |buffer, cx| buffer.set_search(query, case, whole_word, cx));
    }

    pub(crate) fn step_match(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        let wrap = self.search.as_ref().is_none_or(|search| search.wrap_around);
        if let Some(document)=self.document() { document.update(cx,|host,cx|host.step_match(forward,wrap,cx)); }
    }
    fn replace_one(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(search),Some(document))=(&self.search,self.document()) else {return;};
        let replacement=search.replacement.read(cx).content.clone();
        document.update(cx,|host,cx|host.replace_current(&replacement,cx));
    }
    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let (Some(search),Some(document))=(&self.search,self.document()) else {return;};
        let replacement=search.replacement.read(cx).content.clone();
        document.update(cx,|host,cx|host.replace_all(replacement,cx));
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        if let Some(tab) = self.tab() {
            let buffer = tab.document.clone();
            let primary = tab.document.clone();
            buffer.update(cx, |buffer, cx| buffer.set_search(String::new(), false, false, cx));
            window.focus(&primary.focus_handle(cx), cx);
        }
        cx.notify();
    }

    // ---- 大纲 ----

    // ---- 格式命令 ----

    fn with_document(&mut self, window: &mut Window, cx: &mut Context<Self>, update: impl FnOnce(&mut DocumentHost, &mut Context<DocumentHost>)) {
        let Some(tab) = self.tab() else { return };
        let buffer = tab.document.clone();
        let primary = tab.document.clone();
        buffer.update(cx, update);
        window.focus(&primary.focus_handle(cx), cx);
    }

    pub(crate) fn zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        let current = (cx.global::<Settings>().zoom * 100.0).round() as i64;
        let next = if delta == 0.0 { 100 } else { current + (delta * 100.0).round() as i64 };
        self.set_status_zoom(next, true, cx);
    }

    fn open_inline_math(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        let target = match document.read(cx).edit_target(cx) { Ok(target) => target, Err(error) => { self.notice=Some(error.to_string().into());cx.notify();return; } };
        if let Err(error) = document.update(cx, |host, cx| host.begin_inline_math(&target, window, cx)) { self.notice=Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into());cx.notify(); }
    }

    fn open_inline_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        if let Err(error) = document.update(cx, |host, cx| host.begin_inline_comment(window, cx)) {
            self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()); cx.notify();
        }
    }

    fn open_block_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        let target = document.clone();
        if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
            if workspace.document().as_ref() == Some(&target) { workspace.open_block_comment(window, cx); }
        }) { return; }
        if let Err(error) = document.update(cx, |host, cx| host.begin_block_comment(window, cx)) {
            self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()); cx.notify();
        }
    }

    fn open_front_matter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        let target = document.clone();
        if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
            if workspace.document().as_ref() == Some(&target) { workspace.open_front_matter(window, cx); }
        }) { return; }
        if let Err(error) = document.update(cx, |host, cx| host.edit_front_matter(window, cx)) {
            self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()); cx.notify();
        }
    }

    fn open_editor_popover(&mut self,kind:EditorPopoverKind,window:&mut Window,cx:&mut Context<Self>) {
        self.open_editor_popover_at(kind, None, window, cx);
    }
    fn open_editor_popover_at(&mut self,kind:EditorPopoverKind,toolbar_position:Option<gpui::Point<gpui::Pixels>>,window:&mut Window,cx:&mut Context<Self>) {
        if toolbar_position.is_some() && self.popover.as_ref().is_some_and(|pending| pending.view.read(cx).kind() == kind) {
            self.popover = None;
            if let Some(document) = self.document() { window.focus(&document.focus_handle(cx), cx); }
            cx.notify();
            return;
        }
        let Some(document)=self.document() else{return;};
        let draft=if kind==EditorPopoverKind::Link {
            document.read(cx).link_draft(cx).map(|(target,link)|(target,Some(link)))
        }else {document.read(cx).edit_target(cx).map(|target|(target,None))};
        let (target,link)=match draft {Ok(value)=>value,Err(error)=>{self.notice=Some(error.to_string().into());cx.notify();return;}};
        let anchor = if let Some(position) = toolbar_position {
            let index = if kind == EditorPopoverKind::Table { toolbar::TABLE_ANCHOR } else { toolbar::LINK_ANCHOR };
            let bounds = self.toolbar_popup_anchors[index].clone();
            self.toolbar_trigger_bounds.set(bounds.get());
            PopupAnchor::Tracked(bounds, position)
        } else {
            self.toolbar_trigger_bounds.set(None);
            PopupAnchor::Bounds(document.read(cx).selection_anchor(cx).unwrap_or_else(||
                gpui::Bounds::new(gpui::point(px(24.0), px(110.0)), gpui::size(px(0.0), px(0.0)))))
        };
        let view=cx.new(|cx| {
            let mut view=match &link {
                Some(draft)=>EditorPopover::new_link(draft.text.clone(),draft.href.clone(),draft.active,cx),
                None=>EditorPopover::new_table(cx),
            };
            view.set_popup_anchor(anchor);
            view
        });
        let subscription=cx.subscribe_in(&view,window,|workspace,view,event,window,cx|workspace.editor_popover_event(&view,event,window,cx));
        window.focus(&view.focus_handle(cx), cx);
        self.toolbar_popup=None;
        self.popover=Some(PendingPopover {document:document.downgrade(),target,link,view,_subscription:subscription});
        self.last_notice=None;
        cx.notify();
    }
    fn editor_popover_event(&mut self,view:&Entity<EditorPopover>,event:&EditorPopoverEvent,window:&mut Window,cx:&mut Context<Self>) {
        let Some(pending)=self.popover.as_ref().filter(|pending|pending.view==*view) else{return;};
        let document=pending.document.upgrade();
        let is_current=document.is_some() && document==self.document();
        if matches!(event,EditorPopoverEvent::Dismiss) {
            self.popover=None;
            if is_current {if let Some(document)=document {window.focus(&document.focus_handle(cx), cx);}}
            cx.notify();return;
        }
        let result=if !is_current {Err(ControlError::Stale)}else if let Some(document)=document.as_ref() {
            document.update(cx,|host,cx|match event {
                EditorPopoverEvent::ApplyLink{text,href}=>pending.link.as_ref().ok_or(ControlError::InvalidRange).and_then(|draft|host.apply_link(&pending.target,draft,text,Some(href),cx)),
                EditorPopoverEvent::RemoveLink=>pending.link.as_ref().ok_or(ControlError::InvalidRange).and_then(|draft|host.apply_link(&pending.target,draft,&draft.text,None,cx)),
                EditorPopoverEvent::InsertTable{rows,columns}=>host.insert_table(&pending.target,*rows,*columns,cx),
                EditorPopoverEvent::Dismiss=>Ok(()),
            })
        }else{Err(ControlError::Stale)};
        match result {
            Ok(())=>{self.popover=None;if let Some(document)=document {window.focus(&document.focus_handle(cx), cx);}},
            Err(error)=>view.update(cx,|view,cx|view.set_error(Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()),cx)),
        }
        cx.notify();
    }

    // ---- 渲染 ----

    fn chrome_event(&mut self, event:&ChromeEvent, window:&mut Window, cx:&mut Context<Self>) {
        match event {
            ChromeEvent::OpenRecent { path, .. } => self.open_path(path, window, cx),
            ChromeEvent::RemoveRecent(path) => self.library.update(cx, |library, cx| library.change(LibraryChange::Remove { path: path.clone(), folder: false }, cx)),
            ChromeEvent::ClearRecent => self.library.update(cx, |library, cx| library.change(LibraryChange::ClearRecent, cx)),
            ChromeEvent::CloseWindow=>{if self.request_close_window(window,cx){window.remove_window();}},
            ChromeEvent::Quit=>cx.defer(crate::quit_workspaces),
            ChromeEvent::NewWindow=>crate::open_workspace(Vec::new(),cx),
            ChromeEvent::Command(ChromeCommand::Heading(level))=>self.with_document(window,cx,|host,cx|host.set_heading(*level,cx)),
            ChromeEvent::Command(ChromeCommand::AdjustHeading { increase })=>self.with_document(window,cx,|host,cx|host.command(crate::nomo_mark::commands::EditIntent::AdjustHeading { increase: *increase },cx)),
            ChromeEvent::Command(ChromeCommand::ClearFormatting)=>self.with_document(window,cx,|host,cx|host.command(crate::nomo_mark::commands::EditIntent::ClearFormatting,cx)),
            ChromeEvent::Command(ChromeCommand::MathInline)=>self.open_inline_math(window,cx),
            ChromeEvent::Command(ChromeCommand::InlineComment)=>self.open_inline_comment(window,cx),
            ChromeEvent::Command(ChromeCommand::BlockComment)=>self.open_block_comment(window,cx),
            ChromeEvent::Export(format)=>self.export_document(*format,window,cx),
            ChromeEvent::FrontMatter=>self.open_front_matter(window,cx),
            ChromeEvent::Command(ChromeCommand::Callout)=>self.with_document(window,cx,|host,cx|host.insert_block("> [!NOTE]\n> ",12,cx)),
            ChromeEvent::Command(ChromeCommand::Details)=>self.with_document(window,cx,|host,cx|host.command(crate::nomo_mark::commands::EditIntent::InsertDetails,cx)),
            ChromeEvent::Command(ChromeCommand::Footnote)=>self.with_document(window,cx,|host,cx|host.insert_footnote(cx)),
            ChromeEvent::Command(ChromeCommand::Toc)=>self.with_document(window,cx,|host,cx|host.insert_toc(cx)),
            ChromeEvent::Command(ChromeCommand::Diagram(kind))=>{let template=toolbar::diagram_template(*kind, &cx.global::<Settings>().string("defaultDiagramType"));self.with_document(window,cx,|host,cx|host.command(crate::nomo_mark::commands::EditIntent::InsertDiagram(template.into()),cx));},
            ChromeEvent::LinkPicker=>self.open_editor_popover(EditorPopoverKind::Link,window,cx),
            ChromeEvent::TablePicker=>self.open_editor_popover(EditorPopoverKind::Table,window,cx),
            ChromeEvent::ResetOutlinePosition=>self.reset_outline_position(cx),
            ChromeEvent::ToggleMini=>self.toggle_mini(window,cx),
            ChromeEvent::Unavailable(_)=>{self.notice=Some("此功能尚未提供".into());cx.notify();},
            _=>{self.notice=Some("此功能尚未接入原生页面".into());cx.notify();},
        }
    }
    fn render_hidden_tabs(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.tab_menu_open {
            for tab in &mut self.tabs { tab.hidden_hover.reset(); }
            return None;
        }
        let button_bounds = self.tab_menu_button_bounds.get()?;
        let viewport = self.tab_scroll.bounds();
        let offset = self.tab_scroll.offset().x;
        let reduced = self.explorer.reduce_motion;
        let rows: Vec<AnyElement> = self.tabs.iter().enumerate().filter_map(|(index, tab)| {
            let bounds = self.tab_scroll.bounds_for_item(index)?;
            if bounds.left() + offset >= viewport.left() && bounds.right() + offset <= viewport.right() { return None; }
            let document = tab.document.read(cx);
            let title = document.title();
            let dirty = document.is_dirty(cx);
            let tab_id = tab.document.entity_id();
            let hover = tab.hidden_hover.value();
            let active = index == self.active;
            let row = div().id(("hidden-tab", index)).h(px(30.0)).px(px(8.0)).flex().flex_none()
                .items_center().gap(px(8.0)).rounded(px(5.0)).cursor_pointer()
                .text_color(if active { theme.heading } else { motion::color(theme.muted, theme.fg, hover) })
                .bg(gpui::Hsla::from(gpui::rgba(0x80808014)).opacity(hover))
                .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                .child(icon(IconName::FileText, 13.0, theme.accent))
                .child(div().flex_1().min_w(px(0.0)).truncate()
                    .when(tab.preview, |el| el.italic().opacity(0.85)).child(title))
                .children(dirty.then(|| div().size(px(6.0)).rounded_full().bg(theme.accent)))
                .on_hover(cx.listener(move |workspace, hovered: &bool, _, cx| {
                    let reduced = workspace.explorer.reduce_motion;
                    if let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.document.entity_id() == tab_id) {
                        tab.hidden_hover.set(*hovered, reduced, Duration::from_millis(120));
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    workspace.activate(index, window, cx);
                }));
            Some(row.into_any_element())
        }).collect();
        let trigger_bounds = self.tab_menu_button_bounds.clone();
        let menu = div().id("hidden-tabs-menu").w(px(240.0)).max_h(px(320.0)).p(px(4.0))
            .flex().flex_col().overflow_y_scroll().rounded(px(8.0)).bg(theme.surface)
            .border_1().border_color(theme.border).shadow_lg().occlude()
            .on_mouse_down_out(cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| {
                if !trigger_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) {
                    workspace.tab_menu_open = false;
                    cx.notify();
                }
            }))
            .children(rows);
        Some(gpui::deferred(popup("hidden-tabs-enter",
            PopupAnchor::Tracked(self.tab_menu_button_bounds.clone(), button_bounds.bottom_right()),
            PopupPlacement::BelowRight, menu, reduced)).with_priority(5).into_any_element())
    }

    fn render_tabs(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_count = self.tabs.len();
        let reduced = self.explorer.reduce_motion;
        let surface = self.surface.clone();
        let tab_scroll = self.tab_scroll.clone();
        if tab_count == 0 { surface.clear(); }
        let tabs: Vec<AnyElement> = self
            .tabs
            .iter_mut()
            .enumerate()
            .map(|(index, tab)| {
                let buffer = tab.document.read(cx);
                let active = index == self.active;
                let title = buffer.title();
                let dirty = buffer.is_dirty(cx);
                if tab.dirty != dirty {
                    tab.dirty = dirty;
                    tab.dirty_animation_started = dirty.then(Instant::now);
                }
                let enter_duration = Duration::from_millis(if reduced { 90 } else { 180 });
                let animate = tab.opened_at.elapsed() < enter_duration;
                let tab_id = tab.document.entity_id();
                let hover = tab.hover.value();
                let close_hover = tab.close_hover.value();
                let active_color = tab.active_color.value();
                let tab_text = motion::color(motion::color(theme.muted, theme.fg, hover), theme.heading, active_color);
                let tab_icon = motion::color(theme.muted, theme.accent, active_color);
                let dirty_indicator = dirty.then(|| {
                    let dot = div().size(px(6.0)).flex_none().rounded_full().bg(theme.accent);
                    let duration = Duration::from_millis(if reduced { 80 } else { 120 });
                    if tab.dirty_animation_started.is_some_and(|started| started.elapsed() < duration) {
                        dot.with_animation(("tab-dirty", tab_id), Animation::new(duration).with_easing(tree_selection_ease),
                            move |dot, progress| dot.opacity(if reduced { 0.78 + 0.22 * progress } else { progress })).into_any_element()
                    } else { dot.into_any_element() }
                });
                let close = div().id(("close", index)).size(px(14.0)).flex().flex_none()
                    .items_center().justify_center().rounded_full().cursor_pointer()
                    .bg(gpui::Hsla::from(gpui::rgba(0x80808033)).opacity(close_hover))
                    .child(icon(IconName::Close, 12.0, motion::color(theme.muted, theme.palette_fg[0], close_hover)))
                    .opacity(if active { 1.0 } else { hover })
                    .on_hover(cx.listener(move |workspace, hovered: &bool, _, cx| {
                        let reduced = workspace.explorer.reduce_motion;
                        if let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.document.entity_id() == tab_id) {
                            tab.close_hover.set(*hovered, reduced, Duration::from_millis(150));
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        workspace.close_tab(index, window, cx);
                    }));
                let tab_element = div()
                    .id(("tab", index))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .min_w(px(120.0))
                    .max_w(px(180.0))
                    .h(px(30.0))
                    .px(px(12.0))
                    .rounded_t(px(12.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .text_color(tab_text)
                    .bg(gpui::Hsla::from(gpui::rgba(0x8080800f)).opacity(hover * (1.0 - active_color)))
                    .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                    .children(active.then(|| surface.tab_marker(tab_id, index, tab_count, tab_scroll.clone())))
                    .child(icon(IconName::FileText, 14.0, tab_icon))
                    .child(div().flex_1().min_w(px(0.0)).truncate()
                        .when(tab.preview, |el| el.italic().opacity(0.85)).child(title))
                    .children(dirty_indicator)
                    .child(close)
                    .on_hover(cx.listener(move |workspace, hovered: &bool, _, cx| {
                        let reduced = workspace.explorer.reduce_motion;
                        if let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.document.entity_id() == tab_id) {
                            tab.hover.set(*hovered, reduced, Duration::from_millis(150));
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                        workspace.pending_tree_click.take();
                        workspace.deferred_open=None;
                        workspace.cancel_preview_opens(None);
                        if event.click_count()>=2 {
                            if let Some(tab)=workspace.tabs.get_mut(index) {tab.preview=false;}
                        }
                        workspace.activate(index, window, cx);
                    }));
                if animate {
                    tab_element.with_animation(
                        ("tab-enter", tab_id),
                        Animation::new(enter_duration).with_easing(tree_selection_ease),
                        move |tab, progress| if reduced { tab.opacity(0.78 + 0.22 * progress) }
                            else { tab.relative().top(px((1.0-progress)*5.0)).opacity(progress) },
                    ).into_any_element()
                } else {
                    tab_element.into_any_element()
                }
            })
            .collect();
        let overflow = self.tab_overflow.get();
        let hidden_tabs = self.render_hidden_tabs(theme, cx);
        let overflow_state = self.tab_overflow.clone();
        let measured_scroll = self.tab_scroll.clone();
        let weak_workspace = cx.entity().downgrade();
        let button_bounds = self.tab_menu_button_bounds.clone();
        let overflow_marker = gpui::canvas(
            move |bounds, _, _| button_bounds.set(Some(bounds)),
            |_, _, _, _| {},
        ).absolute().top_0().left_0().size_full();
        let action_hover = self.tab_action_hover.value();
        let action_button = |id, name| div().id(id).size(px(28.0)).flex().flex_none()
            .items_center().justify_center().rounded(px(5.0)).cursor_pointer()
            .bg(gpui::Hsla::from(gpui::rgba(0x8080801a)).opacity(action_hover))
            .child(icon(name, 14.0, motion::color(theme.muted, theme.fg, action_hover)))
            .on_hover(cx.listener(|workspace, hovered: &bool, _, cx| {
                workspace.tab_action_hover.set(*hovered, workspace.explorer.reduce_motion, Duration::from_millis(150));
                cx.notify();
            }));
        let action = if overflow {
            action_button("tab-overflow", IconName::ChevronDown)
                .relative().child(overflow_marker).children(hidden_tabs)
                .tooltip(tooltip("显示隐藏的标签"))
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.tab_menu_open = !workspace.tab_menu_open;
                    cx.notify();
                })).into_any_element()
        } else {
            action_button("new-tab", IconName::Plus)
                .tooltip(tooltip("新建 ⌘N"))
                .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.new_file(window, cx)))
                .into_any_element()
        };
        div()
            .id("tabs")
            .h(px(TAB_RAIL_HEIGHT))
            .w_full()
            .flex()
            .flex_none()
            .items_end()
            .pl(px(18.0))
            .child(
                div().on_children_prepainted(move |_, window, _| {
                    let overflow = measured_scroll.max_offset().x > px(0.0);
                    if overflow_state.replace(overflow) != overflow {
                        let weak_workspace = weak_workspace.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = weak_workspace.update(cx, |workspace, cx| {
                                workspace.tab_action_hover.reset();
                                if !workspace.tab_overflow.get() { workspace.tab_menu_open = false; }
                                cx.notify();
                            });
                        });
                    }
                })
                    .id("tab-scroll")
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .items_end()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
            .child(div().ml(px(4.0)).mb(px(2.0)).size(px(28.0)).flex().flex_none()
                .items_center().justify_center().child(action))
    }

    fn toggle_toolbar(&mut self, cx: &mut Context<Self>) {
        if cx.global::<Settings>().show_toolbar { self.popover = None; self.toolbar_popup = None; self.width_dragging = false; }
        Settings::toggle(cx, "toolbarHidden");
        cx.notify();
    }

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.leave_mini(window,cx) {return;}
        if self.waiting_library_close { self.waiting_library_close=false; self.cancel_close(cx); }
        self.pending_tree_click.take();
        self.deferred_open=None;
        self.cancel_preview_opens(None);
        self.enqueue_open(None,false,None,false,cx);
        self.drain_open_requests(window,cx);
    }

    fn render_search(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.search.as_ref()?;
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        search.query.update(cx, |field, cx| field.set_placeholder(locale.text("find"), cx));
        search.replacement.update(cx, |field, cx| field.set_placeholder(locale.text("replace_with"), cx));
        let buffer = self.document()?;
        let state = buffer.read(cx).search();
        let status = if state.pending {
            "…".to_string()
        } else if let Some(error) = &state.error {
            error.clone()
        } else if state.total == 0 {
            if search.query.read(cx).content.is_empty() { String::new() } else { locale.text("no_results").into() }
        } else {
            format!("{}/{}", state.current.map_or(0, |index| index + 1), state.total)
        };
        let case = search.case_sensitive;
        let whole_word = search.whole_word;
        let backwards = search.backwards;
        let wrap_around = search.wrap_around;
        let show_replace = search.show_replace;
        let available_width = self.outline_ui.bounds.get().map_or(900.0, |bounds| f32::from(bounds.size.width));
        let panel_width = (available_width - 24.0).clamp(240.0, 400.0);
        let panel_right = if available_width >= 700.0 { 272.0 } else { 12.0 };
        let replace_row = show_replace.then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(search.replacement.clone())
                .child(small_icon_button("replace-one", IconName::Replace, false, theme).tooltip(tooltip(locale.text("replace_current"))).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.replace_one(window, cx))))
                .child(small_icon_button("replace-all", IconName::ReplaceAll, false, theme).tooltip(tooltip(locale.text("replace_all"))).on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.replace_all(cx))))
                .child(div().w(px(22.0)))
        });
        Some(
            floating_card("search-panel", theme)
                .top(px(10.0))
                .right(px(panel_right))
                .w(px(panel_width))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(icon(IconName::Search, 14.0, theme.muted))
                        .child(search.query.clone())
                        .child(div().min_w(px(52.0)).text_size(px(12.0)).text_color(theme.muted).child(status))
                        .child(small_icon_button("case", IconName::CaseSensitive, case, theme).tooltip(tooltip(locale.text("match_case"))).on_click(cx.listener(|w, _: &ClickEvent, _, cx| {
                            if let Some(search) = &mut w.search {
                                search.case_sensitive = !search.case_sensitive;
                            }
                            w.run_search(cx);
                            cx.notify();
                        })))
                        .child(small_icon_button("prev", IconName::ArrowUp, false, theme).tooltip(tooltip(format!("{} ⇧⏎", locale.text("previous")))).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.step_match(false, window, cx))))
                        .child(small_icon_button("next", IconName::ArrowDown, false, theme).tooltip(tooltip(format!("{} ⏎", locale.text("next")))).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.step_match(true, window, cx))))
                        .child(small_icon_button("toggle-replace", IconName::Replace, show_replace, theme).tooltip(tooltip(locale.tooltip("replace", if cfg!(target_os = "macos") { "⌘⌥F" } else { "Ctrl+H" }))).on_click(cx.listener(|w, _: &ClickEvent, _, cx| {
                            if let Some(search) = &mut w.search {
                                search.show_replace = !search.show_replace;
                            }
                            cx.notify();
                        })))
                        .child(small_icon_button("close-search", IconName::Close, false, theme).tooltip(tooltip(format!("{} Esc", locale.text("close")))).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.close_search(window, cx)))),
                )
                .children(replace_row)
                .child(div().flex().items_center().gap(px(12.0)).text_size(px(12.0)).text_color(theme.muted)
                    .child(div().id("search-backwards").tab_index(0).cursor_pointer().child(format!("{} {}", if backwards { "☑" } else { "☐" }, locale.text("reverse")))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(search)=&mut w.search { search.backwards=!search.backwards; } cx.notify(); })))
                    .child(div().id("search-whole-word").tab_index(0).cursor_pointer().child(format!("{} {}", if whole_word { "☑" } else { "☐" }, locale.text("whole_word")))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(search)=&mut w.search { search.whole_word=!search.whole_word; } w.run_search(cx); cx.notify(); })))
                    .child(div().id("search-wrap").tab_index(0).cursor_pointer().child(format!("{} {}", if wrap_around { "☑" } else { "☐" }, locale.text("wrap")))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(search)=&mut w.search { search.wrap_around=!search.wrap_around; } cx.notify(); }))))
                .into_any_element(),
        )
    }

    fn render_empty(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(div().size(px(44.0)).rounded(px(10.0)).bg(theme.hover).flex().items_center().justify_center().child(icon(IconName::FileText, 20.0, theme.accent_strong)))
            .child(div().text_size(px(17.0)).font_weight(FontWeight::BOLD).text_color(theme.heading).child("未打开文档"))
            .child(div().text_size(px(13.0)).text_color(theme.muted).child("从左侧选择 Markdown 文件，或新建一个文档开始编辑。"))
            .children(self.last_notice.clone().map(|notice|div().max_w(px(560.0)).text_size(px(13.0)).text_color(theme.fg).child(notice)))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .child(button("empty-new", "新建文件", Some(IconName::SquarePen), ButtonKind::Primary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.new_file(window, cx))))
                    .child(button("empty-open", "打开文件", Some(IconName::FileText), ButtonKind::Secondary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.prompt_open(false, window, cx))))
                    .child(button("empty-folder", "打开文件夹", Some(IconName::FolderOpen), ButtonKind::Secondary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.prompt_open(true, window, cx)))),
            )
    }
}



/// 插入代码块，语言取设置“默认代码块语言”。
fn insert_code_block(buffer: &mut DocumentHost, cx: &mut Context<DocumentHost>) {
    let language = cx.global::<Settings>().default_code_language.clone();
    buffer.command(crate::nomo_mark::commands::EditIntent::InsertCodeBlock(language), cx);
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::frame_profile::frame_begin();
        match crate::frame_profile::autoscroll_step() {
            Some(true) => window.on_next_frame(|window, cx| {
                crate::frame_profile::input();
                window.dispatch_action(Box::new(crate::editor::PageDown), cx);
            }),
            Some(false) => window.request_animation_frame(),
            None => {}
        }
        let mini = self.mini.active();
        // 确认框期间保留待显示消息，避免 Toast 在被遮挡时已经计时结束。
        if !window.has_active_prompt() {
            let failure = self.failure_notice.take();
            let is_failure = failure.is_some();
            if let Some(message) = failure.or_else(|| self.notice.take()) {
                self.last_notice = Some(message.clone());
                if is_failure { self.notice = None; }
                let duration = Duration::from_millis(if is_failure { 3500 } else { 1500 });
                if let Some(toast) = &self.toast { toast.update(cx, |toast, cx| toast.show(message, duration, cx)); }
                else { self.toast = Some(cx.new(|cx| Toast::new(message, duration, cx))); }
            }
        }
        let reduced = self.explorer.reduce_motion;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.active_color.set(index == self.active, reduced, Duration::from_millis(150));
        }
        let animate_ui = Theme::ui_transition_active(cx) || self.tab_action_hover.running()
            || self.tabs.iter().any(|tab| tab.hover.running() || tab.close_hover.running()
                || tab.active_color.running() || tab.hidden_hover.running());
        let theme = Theme::ui(cx);
        let settings = cx.global::<Settings>().clone();
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let ui_font = Some(settings.string("uiFontFamily")).filter(|name| !name.trim().is_empty()).unwrap_or_else(|| crate::theme::ui_font_family().into());
        let mode = self.tab().map(|tab| tab.mode);
        if mode.is_none() { self.outline_ui.clear_document(); }
        let status=self.document().map(|document|document.read(cx).status().clone());
        // 编辑页始终保留标签栏，单个文档也通过标签显示名称与修改状态。
        let show_tabs = !self.home_visible && !mini;
        self.surface.set_rail_hidden(!show_tabs);
        let recent = self.library.read(cx).snapshot.recent.iter().take(10).map(|entry| crate::ui::chrome::ChromeRecentEntry {
            path: entry.path.clone(), label: home::entry_name(&entry.path), folder: false, missing: false,
        }).collect();
        self.chrome.update(cx,|chrome,cx|chrome.set_state(ChromeState {
            can_show_home: !self.home_visible,
            has_document:status.is_some(), can_undo:status.as_ref().is_some_and(|s|s.can_undo),
            can_redo:status.as_ref().is_some_and(|s|s.can_redo),mode:mode.unwrap_or(EditorMode::Semantic),
            sidebar_visible:settings.show_sidebar, toolbar_visible:settings.show_toolbar,
            outline_visible:settings.show_outline,split_available:true,
            toolbar_shortcut: crate::ui::shortcuts::current("toggle-toolbar", cx).into(),
            recent,
            capabilities:vec![ChromeCapability::Recent,ChromeCapability::NewWindow,ChromeCapability::HeadingLevels,ChromeCapability::HeadingAdjustment,ChromeCapability::ClearFormatting,ChromeCapability::LinkPicker,ChromeCapability::TablePicker,ChromeCapability::MathInline,ChromeCapability::Callout,ChromeCapability::Details,ChromeCapability::InlineComment,ChromeCapability::BlockComment,ChromeCapability::FrontMatter,ChromeCapability::Diagrams,ChromeCapability::Footnote,ChromeCapability::Toc,ChromeCapability::UnavailableNotice,ChromeCapability::ResetOutlinePosition]
                .into_iter().chain([ChromeCapability::ExportHtml, ChromeCapability::ExportPdf, ChromeCapability::ExportWord].into_iter().filter(|_| self.export_job.is_none())).chain((cfg!(windows) && status.is_some() && !window.is_fullscreen()).then_some(ChromeCapability::Mini)).collect(),
            ..Default::default()
        },cx));
        let tabs = show_tabs.then(|| self.render_tabs(&theme, cx).into_any_element());
        let sidebar = if self.home_visible || mini { None } else { self.render_sidebar_animated(settings.show_sidebar, &theme, window, cx) };
        let main: AnyElement = if self.home_visible {
            let count = self.tabs.len();
            let sidebar_width = self.explorer.sidebar_width.unwrap_or(250.0)
                .min((f32::from(window.viewport_size().width) - 420.0).max(180.0));
            self.home.update(cx, |home, _| { home.open_tabs = count; home.sidebar_width = sidebar_width; });
            self.home.clone().into_any_element()
        } else { match mode {
            None => self.render_empty(&theme, cx).into_any_element(),
            Some(mode) => {
                let toolbar = if mini { None } else { self.render_toolbar_animated(settings.show_toolbar, &theme, mode, cx) };
                let tab = self.tab().expect("active tab");
                tab.document.update(cx, |host, _| host.set_save_directory(self.tree.as_ref().map(|tree|tree.root.as_path())));
                let editors=div().size_full().child(tab.document.clone()).into_any_element();
                let outline = if mini { None } else { self.render_outline_panel(&theme, window, cx) };
                let measured = self.outline_ui.bounds.clone();
                let weak = cx.entity().downgrade();
                let outline_measure = gpui::canvas(move |bounds, window, _| {
                    if measured.replace(Some(bounds)) != Some(bounds) {
                        let weak = weak.clone();
                        window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |_, cx| cx.notify()); });
                    }
                }, |_, _, _, _| {}).absolute().top_0().left_0().size_full();
                let search = self.render_search(&theme, cx);
                let split_guide = (mode == EditorMode::Split && self.split_guide_visible).then(|| {
                    gpui::canvas(|_, _, _| (), |bounds, _, window, _| {
                        let origin = bounds.origin + gpui::point(px(0.0), px(f32::from(bounds.size.height) * 0.3));
                        window.paint_quad(gpui::fill(gpui::Bounds::new(origin, gpui::size(bounds.size.width, px(1.0))), gpui::rgba(0xef4444b8)));
                    }).absolute().size_full()
                });
                let reveal_toolbar = (!mini && !settings.show_toolbar && self.toolbar_motion.is_none()).then(|| {
                    div().id("reveal-toolbar").absolute().top_0().right(px(16.0)).w(px(32.0)).h(px(18.0))
                        .flex().items_center().justify_center().rounded_b(px(4.0)).border_1()
                        .border_color(theme.border).bg(theme.rail_bg).cursor_pointer()
                        .hover(|style| style.bg(theme.hover))
                        .tooltip(tooltip(locale.tooltip("show-toolbar", "⌘⇧B")))
                        .child(icon(IconName::ChevronDown, 15.0, theme.muted))
                        .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| workspace.toggle_toolbar(cx)))
                });
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .children(toolbar)
                    .child(div().relative().flex_1().min_h(px(0.0)).children((!mini).then_some(outline_measure)).child(editors).children(outline).children(search).children(split_guide).children(reveal_toolbar))
                    .into_any_element()
            }
        }};
        let popover=self.popover.as_ref().map(|pending|gpui::deferred(pending.view.clone()).with_priority(4));
        let toolbar_popup = self.render_toolbar_popup(&theme, cx);
        let outline_menu = self.render_outline_menu(&theme, cx);
        let outline_drag_preview = self.render_outline_drag_preview(&theme);
        let toast = self.toast.clone().filter(|_|!window.has_active_prompt()).map(|toast| gpui::deferred(toast).with_priority(9));
        let update_notice=if window.has_active_prompt(){None}else{crate::ui::updates::render_notice(window,cx).map(|notice|gpui::deferred(notice).with_priority(10))};
        let status = if mini { None } else { self.render_status_controls(&theme, window, cx) };
        let status_popup = self.render_status_popup(&theme, cx);
        let mini_bar = self.render_mini_bar(cx);
        div()
            .id("workspace")
            .relative()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.rail_bg)
            .text_color(theme.fg)
            .font_family(ui_font)
            .capture_key_down(|_, _, _| crate::frame_profile::input())
            .on_mouse_move(cx.listener(|workspace, event: &gpui::MouseMoveEvent, _, cx| workspace.outline_mouse_move(event, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|workspace, _, window, cx| workspace.outline_mouse_up(window, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|workspace, event: &gpui::MouseUpEvent, window, cx| workspace.outline_mouse_up_out(event, window, cx)))
            .on_action(cx.listener(|w, _: &NewFile, window, cx| w.new_file(window, cx)))
            .on_action(cx.listener(|w, _: &ShowHome, window, cx| w.show_home(window, cx)))
            .on_action(cx.listener(|w, _: &RecoverDocuments, window, cx| w.show_recovery_list(window, cx)))
            .on_action(cx.listener(|w, _: &ToggleMini, window, cx| w.toggle_mini(window, cx)))
            .on_action(cx.listener(|w, _: &OpenFile, window, cx| w.prompt_open(false, window, cx)))
            .on_action(cx.listener(|w, _: &OpenFolder, window, cx| w.prompt_open(true, window, cx)))
            .on_action(cx.listener(|w, _: &Save, window, cx| {
                if let Some(buffer) = w.document() {
                    w.save_document(&buffer,false,false,window,cx);
                }
            }))
            .on_action(cx.listener(|w, _: &ExportHtml, window, cx| w.export_document(crate::document_export::Format::Html, window, cx)))
            .on_action(cx.listener(|w, _: &ExportPdf, window, cx| w.export_document(crate::document_export::Format::Pdf, window, cx)))
            .on_action(cx.listener(|w, _: &ExportWord, window, cx| w.export_document(crate::document_export::Format::Word, window, cx)))
            .on_action(cx.listener(|w, _: &SaveAs, window, cx| {
                if let Some(buffer) = w.document() {
                    w.save_document(&buffer,true,false,window,cx);
                }
            }))
            .on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                if w.home_visible { return; }
                let index = w.active;
                w.close_tab(index, window, cx);
            }))
            .on_action(cx.listener(|w, _: &NextTab, window, cx| {
                if !w.tabs.is_empty() {
                    let next = (w.active + 1) % w.tabs.len();
                    w.activate(next, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &PrevTab, window, cx| {
                if !w.tabs.is_empty() {
                    let previous = (w.active + w.tabs.len() - 1) % w.tabs.len();
                    w.activate(previous, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &Find, window, cx| w.open_search(false, window, cx)))
            .on_action(cx.listener(|w, _: &FindReplace, window, cx| w.open_search(true, window, cx)))
            .on_action(cx.listener(|w, _: &ToggleFind, window, cx| w.toggle_search(false, window, cx)))
            .on_action(cx.listener(|w, _: &ToggleReplace, window, cx| w.toggle_search(true, window, cx)))
            .on_action(cx.listener(|w, _: &FindNext, window, cx| w.step_match(true, window, cx)))
            .on_action(cx.listener(|w, _: &FindPrev, window, cx| w.step_match(false, window, cx)))
            .on_action(cx.listener(|w, _: &ToggleSource, window, cx| {
                let next = if w.tab().map(|tab| tab.mode) == Some(EditorMode::Source) { EditorMode::Semantic } else { EditorMode::Source };
                w.set_mode(next, window, cx);
            }))
            .on_action(cx.listener(|w, _: &ModeSemantic, window, cx| w.set_mode(EditorMode::Semantic, window, cx)))
            .on_action(cx.listener(|w, _: &ModeSource, window, cx| w.set_mode(EditorMode::Source, window, cx)))
            .on_action(cx.listener(|w, _: &ModeSplit, window, cx| w.set_mode(EditorMode::Split, window, cx)))
            .on_action(cx.listener(|_, _: &ToggleOutline, _, cx| Settings::toggle(cx, "outlineVisible")))
            .on_action(cx.listener(|_, _: &ToggleSidebar, _, cx| Settings::toggle(cx, "sidebarHidden")))
            .on_action(cx.listener(|w, _: &ToggleToolbar, _, cx| w.toggle_toolbar(cx)))
            .on_action(cx.listener(|_, _: &ToggleTheme, _, cx| Theme::toggle(cx)))
            .on_action(cx.listener(|_, _: &FollowSystemTheme, _, cx| Settings::set(cx, "themeMode", serde_json::json!("system"))))
            .on_action(cx.listener(|w, _: &Underline, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("<u>", "</u>", cx))))
            .on_action(cx.listener(|w, _: &Highlight, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("<mark>", "</mark>", cx))))
            .on_action(cx.listener(|_, _: &ToggleTypography, _, cx| Settings::toggle(cx, "typographyEnabled")))
            .on_action(cx.listener(|w, _: &ZoomIn, _, cx| w.zoom(0.05, cx)))
            .on_action(cx.listener(|w, _: &ZoomOut, _, cx| w.zoom(-0.05, cx)))
            .on_action(cx.listener(|w, _: &ZoomReset, _, cx| w.zoom(0.0, cx)))
            .on_action(cx.listener(|w, _: &Bold, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("**", "**", cx))))
            .on_action(cx.listener(|w, _: &Italic, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("*", "*", cx))))
            .on_action(cx.listener(|w, _: &Strike, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("~~", "~~", cx))))
            .on_action(cx.listener(|w, _: &InlineCode, window, cx| w.with_document(window, cx, |b, cx| b.wrap_selection("`", "`", cx))))
            .on_action(cx.listener(|w, _: &Link, window, cx| w.open_editor_popover(EditorPopoverKind::Link,window,cx)))
            .on_action(cx.listener(|w, _: &Heading1, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(1, cx))))
            .on_action(cx.listener(|w, _: &Heading2, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(2, cx))))
            .on_action(cx.listener(|w, _: &Heading3, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(3, cx))))
            .on_action(cx.listener(|w, _: &Heading4, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(4, cx))))
            .on_action(cx.listener(|w, _: &Heading5, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(5, cx))))
            .on_action(cx.listener(|w, _: &Heading6, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(6, cx))))
            .on_action(cx.listener(|w, _: &PromoteHeading, window, cx| w.with_document(window, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::AdjustHeading { increase: true }, cx))))
            .on_action(cx.listener(|w, _: &DemoteHeading, window, cx| w.with_document(window, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::AdjustHeading { increase: false }, cx))))
            .on_action(cx.listener(|w, _: &ClearFormatting, window, cx| w.with_document(window,cx,|b,cx|b.command(crate::nomo_mark::commands::EditIntent::ClearFormatting,cx))))
            .on_action(cx.listener(|_, _: &NewWindow, _, cx|crate::open_workspace(Vec::new(),cx)))
            .on_action(cx.listener(|w, _: &Paragraph, window, cx| w.with_document(window, cx, |b, cx| b.set_heading(0, cx))))
            .on_action(cx.listener(|w, _: &Quote, window, cx| w.with_document(window, cx, |b, cx| b.toggle_line_prefix("> ", cx))))
            .on_action(cx.listener(|w, _: &BulletList, window, cx| w.with_document(window, cx, |b, cx| b.toggle_line_prefix("- ", cx))))
            .on_action(cx.listener(|w, _: &OrderedList, window, cx| w.with_document(window, cx, |b, cx| b.toggle_line_prefix("1. ", cx))))
            .on_action(cx.listener(|w, _: &TaskList, window, cx| w.with_document(window, cx, |b, cx| b.toggle_line_prefix("- [ ] ", cx))))
            .on_action(cx.listener(|w, _: &CodeBlock, window, cx| w.with_document(window, cx, |b, cx| insert_code_block(b, cx))))
            .on_action(cx.listener(|w, _: &Table, window, cx| w.open_editor_popover(EditorPopoverKind::Table,window,cx)))
            .on_action(cx.listener(|w, _: &MathBlock, window, cx| w.with_document(window, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::InsertMathBlock, cx))))
            .on_action(cx.listener(|w, _: &BlockComment, window, cx| w.open_block_comment(window, cx)))
            .on_action(cx.listener(|w, _: &EditFrontMatter, window, cx| w.open_front_matter(window, cx)))
            .on_action(cx.listener(|w, _: &Rule, window, cx| w.with_document(window, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::ToggleRule, cx))))
            .on_action(cx.listener(|w, _: &crate::editor::Undo, _, cx| {if let Some(doc)=w.document(){doc.update(cx,|h,cx|h.undo_redo(false,cx));}}))
            .on_action(cx.listener(|w, _: &crate::editor::Redo, _, cx| {if let Some(doc)=w.document(){doc.update(cx,|h,cx|h.undo_redo(true,cx));}}))
            .on_action(cx.listener(|_, _: &Quit, _, cx|cx.defer(crate::quit_workspaces)))
            .when(!mini, |el| el.child(self.chrome.clone()))
            .children(mini_bar)
            .children(popover)
            .children(toolbar_popup)
            .children(outline_menu)
            .children(outline_drag_preview)
            .children(toast)
            .children(update_notice)
            .children(status_popup)
            .child(
                if mini { div().flex_1().min_h_0().relative().child(main).into_any_element() }
                else if self.home_visible { main } else {
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .children(sidebar)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(self.surface.background(theme.clone(), animate_ui))
                            .children(tabs)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .min_w(px(0.0))
                                    .bg(theme.surface)
                                    .rounded_tl(px(12.0))
                                    .overflow_hidden()
                                    .child(main),
                            )
                            .child(self.surface.outline(theme))
                            .children(status),
                    ).into_any_element()
                },
            )
            // 帧采样终点：作为根节点最后一个子元素在常规绘制末尾执行（延迟弹层除外）。
            .child(gpui::canvas(|_, _, _| (), |_, _, _, _| crate::frame_profile::frame_end()).absolute().size_0())
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let context = Some("Workspace");
    cx.bind_keys([
        crate::ui::key_binding("secondary-alt-shift-h", ShowHome, context, cx),
        crate::ui::key_binding("secondary-shift-n", NewWindow, context, cx),
        crate::ui::key_binding("secondary-shift-o", OpenFolder, context, cx),
        crate::ui::key_binding("secondary-shift-s", SaveAs, context, cx),
        crate::ui::key_binding("secondary-w", CloseTab, context, cx),
        KeyBinding::new("ctrl-tab", NextTab, context),
        KeyBinding::new("ctrl-shift-tab", PrevTab, context),
        crate::ui::key_binding("secondary-f", ToggleFind, context, cx),
        crate::ui::key_binding("secondary-alt-f", ToggleReplace, context, cx),
        crate::ui::key_binding("secondary-g", FindNext, context, cx),
        crate::ui::key_binding("secondary-shift-g", FindPrev, context, cx),
        crate::ui::key_binding("secondary-1", Heading1, context, cx),
        crate::ui::key_binding("secondary-2", Heading2, context, cx),
        crate::ui::key_binding("secondary-3", Heading3, context, cx),
        crate::ui::key_binding("secondary-4", Heading4, context, cx),
        crate::ui::key_binding("secondary-5", Heading5, context, cx),
        crate::ui::key_binding("secondary-6", Heading6, context, cx),
        crate::ui::key_binding("secondary-alt-o", ToggleOutline, context, cx),
        crate::ui::key_binding("secondary-z", crate::editor::Undo, Some("Outline"), cx),
        crate::ui::key_binding("secondary-shift-z", crate::editor::Redo, Some("Outline"), cx),
        KeyBinding::new("ctrl-y", crate::editor::Redo, Some("Outline")),
        crate::ui::key_binding("secondary-alt-s", ToggleSidebar, context, cx),
        crate::ui::key_binding("secondary-=", PromoteHeading, context, cx),
        crate::ui::key_binding("secondary-+", ZoomIn, context, cx),
        crate::ui::key_binding("secondary--", DemoteHeading, context, cx),
        crate::ui::key_binding("secondary-0", Paragraph, context, cx),
        crate::ui::key_binding("secondary-b", Bold, context, cx),
        crate::ui::key_binding("secondary-i", Italic, context, cx),
        KeyBinding::new("alt-shift-5", Strike, context),
        crate::ui::key_binding("secondary-shift-x", TaskList, context, cx),
        crate::ui::key_binding("secondary-shift-q", Quote, context, cx),
        crate::ui::key_binding("secondary-shift-[", OrderedList, context, cx),
        crate::ui::key_binding("secondary-shift-]", BulletList, context, cx),
        crate::ui::key_binding("secondary-u", Underline, context, cx),
        crate::ui::key_binding("secondary-shift-h", Rule, context, cx),
        crate::ui::key_binding("secondary-`", InlineCode, context, cx),
        crate::ui::key_binding("secondary-shift-c", InlineCode, context, cx),
        crate::ui::key_binding("secondary-alt-1", Heading1, context, cx),
        crate::ui::key_binding("secondary-alt-2", Heading2, context, cx),
        crate::ui::key_binding("secondary-alt-3", Heading3, context, cx),
        crate::ui::key_binding("secondary-alt-0", Paragraph, context, cx),
        crate::ui::key_binding("secondary-q", Quit, None, cx),
        crate::ui::key_binding("secondary-,", OpenSettings, None, cx),
    ]);
    if !cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new("ctrl-h", ToggleReplace, context)]);
    }
}

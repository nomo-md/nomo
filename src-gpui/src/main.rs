//! Nomo GPUI 原生入口。
//!
//! 用法：`nomo [文件或目录 ...]`。目录作为侧栏根目录打开，文件各开一个标签。

#![cfg_attr(all(target_os = "windows", not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod snapshots;
mod cache;
mod clipboard;
#[cfg(test)]
mod document;
mod document_export;
mod editor;
mod nomo_mark;
mod open_profile;
mod frame_profile;
#[cfg(feature = "automation")]
mod automation;
#[cfg(windows)]
mod open_instance;
#[cfg(windows)]
mod windows_integration;
mod embeds;
mod images;
mod image_import;
mod kp;
mod layout;
mod markdown;
mod markdown_templates;
mod settings;
mod legacy_records;
mod theme;
mod ui;
mod window_placement;

#[allow(dead_code)] // 仅接入现有会话 API，其余原版服务保留原实现。
mod text_document;
mod models;
mod file_system;
mod app_logger {
    pub fn warn(scope: &str, message: &str) { eprintln!("[{scope}] {message}"); }
}

// 会话既有异步测试共用一个 Tokio 运行时。
#[cfg(test)]
mod async_runtime {
    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime"))
    }
    pub fn block_on<F: std::future::Future>(future:F)->F::Output { runtime().block_on(future) }
    pub fn spawn<F:std::future::Future+Send+'static>(future:F)->tokio::task::JoinHandle<F::Output>
        where F::Output:Send+'static { runtime().spawn(future) }
}

use gpui::{App, AppContext, Bounds, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};
use std::path::PathBuf;
use ui::workspace::{self, Workspace};
use window_placement::WindowPlacement;

#[derive(Clone, Copy)]
enum QuitReason { Normal, #[cfg(windows)] Install }
#[derive(Clone, Copy)]
struct PendingQuit {
    reason: QuitReason,
    preferred_window: Option<gpui::AnyWindowHandle>,
    placement: Option<WindowPlacement>,
}
struct QuitRequested(Option<PendingQuit>);
impl gpui::Global for QuitRequested {}
struct LastActiveWorkspace(gpui::AnyWindowHandle);
impl gpui::Global for LastActiveWorkspace {}

pub(crate) fn cancel_quit(cx:&mut App){cx.set_global(QuitRequested(None));}
pub(crate) fn quit_workspaces(cx:&mut App){
    request_quit(QuitReason::Normal,cx);
}
#[cfg(windows)]
pub(crate) fn quit_to_install(cx:&mut App){
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        if !handle.update(cx,|workspace,window,cx| workspace.can_quit_to_install(window,cx)).unwrap_or(false) { return; }
    }
    request_quit(QuitReason::Install,cx);
}
fn preferred_workspace(cx: &App) -> Option<gpui::WindowHandle<Workspace>> {
    let windows = cx.windows();
    let last_active = cx.try_global::<LastActiveWorkspace>().map(|last| last.0);
    cx.active_window().into_iter().chain(last_active)
        .filter(|handle| windows.contains(handle))
        .find_map(|handle| handle.downcast::<Workspace>())
        .or_else(|| windows.into_iter().find_map(|handle| handle.downcast::<Workspace>()))
}

fn request_quit(reason: QuitReason,cx:&mut App){
    // 批量关闭会改变焦点。先选定窗口，确认关闭时再读取它的最终位置。
    let preferred_window = preferred_workspace(cx).map(gpui::AnyWindowHandle::from);
    cx.set_global(QuitRequested(Some(PendingQuit { reason, preferred_window, placement: None })));
    for handle in cx.windows().into_iter().filter_map(|window|window.downcast::<Workspace>()) {
        if cx.global::<QuitRequested>().0.is_none() { break; }
        let _=handle.update(cx,|workspace,window,cx|{if workspace.request_quit(window,cx){window.remove_window();}});
    }
    finish_quit(cx);
}
fn finish_quit(cx:&mut App){
    let Some(PendingQuit { reason: _reason, placement, .. }) = cx.try_global::<QuitRequested>().and_then(|request| request.0) else { return; };
    if cx.windows().iter().any(|window|window.downcast::<Workspace>().is_some()){return;}
    cancel_quit(cx);
    if let Some(placement) = placement { placement.save(); }
    #[cfg(windows)]
    if matches!(_reason, QuitReason::Install) && let Err(error)=ui::updates::launch_after_close(cx) {
        match try_open_workspace(Vec::new(),cx) {
            Ok(handle)=>{ let _=handle.update(cx,|_,window,cx|ui::updates::show_error(error,window,cx)); }
            Err(e)=>open_instance::show_error(&format!("{error}\n{e}")),
        }
        return;
    }
    for handle in cx.windows(){let _=handle.update(cx,|_,window,_|window.remove_window());}
    cx.quit();
}

fn save_closing_workspace_placement(workspace: &Workspace, window: &gpui::Window, cx: &mut App) {
    let handle = window.window_handle();
    let last_window = !cx.windows().iter().any(|other| *other != handle && other.downcast::<Workspace>().is_some());
    if let Some(mut request) = cx.try_global::<QuitRequested>().and_then(|request| request.0) {
        if request.preferred_window == Some(handle) || (request.placement.is_none() && last_window) {
            request.placement = workspace.window_placement(window, cx);
            cx.set_global(QuitRequested(Some(request)));
        }
    } else if last_window && let Some(placement) = workspace.window_placement(window, cx) {
        placement.save();
    }
}

fn main() {
    #[cfg(feature = "nomo-mark-demo")]
    let nomo_mark_demo = std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark-demo" | "--editor-control-demo"));
    #[cfg(feature = "nomo-mark-demo")]
    let nomo_mark_file = std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark" | "--editor-control"));
    let paths: Vec<PathBuf> = std::env::args().skip(1).filter(|argument| !matches!(argument.as_str(), "--nomo-mark-demo" | "--nomo-mark" | "--editor-control-demo" | "--editor-control")).map(PathBuf::from).collect();
    let development_entry = cfg!(feature = "nomo-mark-demo") && std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark" | "--nomo-mark-demo" | "--editor-control" | "--editor-control-demo"));
    #[cfg(feature = "automation")]
    if let Err(error) = automation::initialize(&paths) {
        eprintln!("自动化隔离校验失败：{error}");
        std::process::exit(2);
    }
    #[cfg(windows)]
    let instance=if !development_entry {
        match open_instance::start(&paths) {
            Ok(open_instance::Startup::Forwarded)=>return,
            Ok(open_instance::Startup::Primary(instance))=>Some(instance),
            Err(error)=>{open_instance::show_error(&error);return;}
        }
    }else{None};
    #[cfg(not(windows))]
    let application = gpui_platform::application();
    #[cfg(windows)]
    let application = gpui::Application::with_platform(std::rc::Rc::new(
        gpui_windows::WindowsPlatform::new(false).expect("初始化 Windows 平台"),
    ));
    let application = application.with_assets(ui::components::Assets);
    // 平台可能在启动完成前送达文件 URL；只暂存尚未交给 Workspace 的路径。
    let open_state = std::rc::Rc::new(std::cell::RefCell::new((None::<gpui::AsyncApp>, Vec::<PathBuf>::new())));
    if !development_entry {
        let state = open_state.clone();
        application.on_open_urls(move |urls| {
            let paths: Vec<PathBuf> = urls.into_iter().filter_map(|raw| {
                match url::Url::parse(&raw).ok().and_then(|url| url.to_file_path().ok()) {
                    Some(path) => Some(path),
                    None => { eprintln!("忽略非本地文件打开请求"); None }
                }
            }).collect();
            let context = {
                let mut state = state.borrow_mut();
                match &state.0 {
                    Some(context) => context.clone(),
                    None => { state.1.extend(paths); return; }
                }
            };
            if !paths.is_empty() {
                context.update(|cx| open_paths(paths, cx));
            }
        });
        application.on_reopen(|cx| open_paths(Vec::new(), cx));
    }
    application.run(move |cx: &mut App| {
        // 先放浅色主题占位，窗口创建后按系统外观立即校正。
        cx.set_global(theme::Theme::light());
        cx.set_global(ui::motion::MotionPreference::default());
        cx.set_global(settings::Settings::load());
        theme::catalog::init(cx);
        cx.set_global(images::ImageStore::default());
        cx.set_global(embeds::EmbedStore::default());
        ui::reload_key_bindings(cx);
        ui::dialogs::init(cx);
        cx.on_action(|_: &workspace::Quit,cx|quit_workspaces(cx));
        cx.on_window_closed(|cx,_|{
            // 结束本轮关闭通知后再联动设置窗口，让它也完整走过关闭与释放回调。
            cx.defer(ui::settings_window::close_if_orphaned);
            ui::updates::prune_closed_windows(cx);
            finish_quit(cx);
        }).detach();
        cx.on_action(|_: &workspace::OpenSettings, cx| ui::settings_window::open(cx));
        set_menus(cx);

        #[cfg(feature = "nomo-mark-demo")]
        if nomo_mark_demo || nomo_mark_file {
            cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
            let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
            cx.open_window(WindowOptions { app_id: Some("com.nomo.desktop".into()), window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {title:Some("NomoMark · 开发".into()),..Default::default()}),..Default::default()},
                |window,cx| {theme::Theme::apply(window.appearance(),cx);cx.new(|cx| if nomo_mark_file { nomo_mark::demo::NomoMarkDemo::file(paths.first().cloned(), window, cx) } else { nomo_mark::demo::NomoMarkDemo::new(window,cx) })}).expect("打开 NomoMark 开发窗口");
            cx.activate(true);return;
        }

        // 系统直接请求退出时仍可读取当前窗口。应用内退出已在关闭路径保存。
        cx.on_app_quit(|cx| {
            if let Some(handle) = preferred_workspace(cx)
                && let Ok(Some(placement)) = handle.update(cx, |workspace, window, cx| workspace.window_placement(window, cx))
            {
                placement.save();
            }
            std::future::ready(())
        }).detach();
        let mut paths = paths;
        paths.append(&mut open_state.borrow_mut().1);
        open_workspace(paths, cx);
        open_state.borrow_mut().0 = Some(cx.to_async());
        #[cfg(windows)]
        if let Some(mut instance)=instance {
            let mut receiver=instance.receiver.take().expect("instance receiver");
            cx.set_global(instance);
            cx.spawn(async move |cx| {
                while let Some(paths)=receiver.recv().await {
                    cx.update(|cx| {
                        // 无参数再次启动明确创建主页窗口；平台 reopen 仍保留唤醒语义。
                        if paths.is_empty() {
                            open_workspace(Vec::new(),cx);
                            cx.activate(true);
                        } else {
                            open_paths(paths,cx);
                        }
                    });
                }
            }).detach();
        }
        cx.activate(true);
    });
}

fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    let handle=cx.active_window().and_then(|window|window.downcast::<Workspace>())
        .or_else(||cx.windows().into_iter().find_map(|window|window.downcast::<Workspace>()));
    if let Some(handle)=handle {
        if let Err(error)=handle.update(cx,|workspace,window,cx| {
            if paths.is_empty() {workspace.reveal_window(window,cx);}
            else {
                // 先唤醒接收窗口，再让文档路由决定最终目标，避免把源窗口重新置前。
                workspace.reveal_window(window,cx);
                workspace.open_paths(paths,window,cx);
            }
        }) {eprintln!("打开文档窗口失败：{error}");}
    }else{open_workspace(paths,cx);}
    cx.activate(true);
}

fn open_workspace(paths: Vec<PathBuf>, cx: &mut App) {
    try_open_workspace(paths,cx).expect("打开窗口失败");
}
fn try_open_workspace(paths: Vec<PathBuf>, cx: &mut App) -> Result<gpui::WindowHandle<Workspace>,String> {
    let first_workspace = !cx.windows().iter().any(|window| window.downcast::<Workspace>().is_some());
    let placement = if first_workspace { WindowPlacement::restore(cx) } else { None };
    let (window_bounds, display_id) = placement.map_or_else(
        || (WindowBounds::Windowed(Bounds::centered(None, size(px(1280.0), px(820.0)), cx)), None),
        |(bounds, display)| (bounds, Some(display)),
    );
    let options = WindowOptions {
        app_id: Some("com.nomo.desktop".into()),
        window_bounds: Some(window_bounds),
        display_id,
        titlebar: Some(TitlebarOptions { title: Some("Nomo".into()), appears_transparent: true, traffic_light_position: Some(point(px(14.0), px(13.0))) }),
        window_min_size: Some(size(px(640.0), px(420.0))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        theme::Theme::apply(window.appearance(), cx);
        // 跟随系统深浅色：外观变化时重新应用主题（手动指定模式时 apply 会忽略外观）。
        window.observe_window_appearance(|window, cx| theme::Theme::apply(window.appearance(), cx)).detach();
        cx.new(|cx| Workspace::new(paths, window_bounds.get_bounds(), window, cx))
    })
    .map_err(|error|error.to_string())
}

fn set_menus(cx: &mut App) {
    use workspace::*;
    let locale = ui::toolbar_i18n::ToolbarLocale::current(cx);
    cx.set_menus(vec![
        Menu { disabled: false, name: "Nomo".into(), items: vec![MenuItem::action(locale.app_text("menu.openSettings"), OpenSettings), MenuItem::separator(), MenuItem::action(locale.app_text("menu.quit"), Quit)] },
        Menu { disabled: false,
            name: locale.app_text("menu.file").into(),
            items: vec![
                MenuItem::action(locale.app_text("menu.newFile"), NewFile),
                MenuItem::action(locale.app_text("menu.openFile"), OpenFile),
                MenuItem::action(locale.app_text("menu.openFolder"), OpenFolder),
                MenuItem::action(locale.app_text("menu.recoverDocuments"), RecoverDocuments),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.save"), Save),
                MenuItem::action(locale.app_text("menu.saveAs"), SaveAs),
                MenuItem::action(locale.app_text("menu.exportHtml"), ExportHtml),
                MenuItem::action(locale.app_text("menu.exportPdf"), ExportPdf),
                MenuItem::action(locale.app_text("menu.exportWord"), ExportWord),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.closeTab"), CloseTab),
            ],
        },
        Menu { disabled: false,
            name: locale.app_text("menu.edit").into(),
            items: vec![
                MenuItem::action(locale.app_text("menu.undo"), editor::Undo),
                MenuItem::action(locale.app_text("menu.redo"), editor::Redo),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.cut"), editor::Cut),
                MenuItem::action(locale.app_text("menu.copy"), editor::Copy),
                MenuItem::action(locale.app_text("menu.paste"), editor::Paste),
                MenuItem::action(locale.app_text("menu.selectAll"), editor::SelectAll),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.find"), Find),
                MenuItem::action(locale.app_text("menu.findReplace"), FindReplace),
                MenuItem::action(locale.app_text("menu.findNext"), FindNext),
                MenuItem::action(locale.app_text("menu.findPrev"), FindPrev),
            ],
        },
        Menu { disabled: false,
            name: locale.app_text("menu.format").into(),
            items: vec![
                MenuItem::action(locale.app_text("menu.bold"), Bold),
                MenuItem::action(locale.app_text("menu.italic"), Italic),
                MenuItem::action(locale.app_text("menu.strike"), Strike),
                MenuItem::action(locale.app_text("menu.inlineCode"), InlineCode),
                MenuItem::action(locale.app_text("menu.link"), Link),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.heading1"), Heading1),
                MenuItem::action(locale.app_text("menu.heading2"), Heading2),
                MenuItem::action(locale.app_text("menu.heading3"), Heading3),
                MenuItem::action(locale.app_text("menu.paragraph"), Paragraph),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.quote"), Quote),
                MenuItem::action(locale.app_text("menu.bulletList"), BulletList),
                MenuItem::action(locale.app_text("menu.orderedList"), OrderedList),
                MenuItem::action(locale.app_text("menu.taskList"), TaskList),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.codeBlock"), CodeBlock),
                MenuItem::action(locale.app_text("menu.table"), Table),
                MenuItem::action(locale.app_text("menu.mathBlock"), MathBlock),
                MenuItem::action(locale.app_text("menu.blockComment"), BlockComment),
                MenuItem::action(locale.app_text("menu.editFrontMatter"), EditFrontMatter),
                MenuItem::action(locale.app_text("menu.rule"), Rule),
            ],
        },
        Menu { disabled: false,
            name: locale.app_text("menu.view").into(),
            items: vec![
                MenuItem::action(locale.app_text("menu.modeSemantic"), ModeSemantic),
                MenuItem::action(locale.app_text("menu.modeSource"), ModeSource),
                MenuItem::action(locale.app_text("menu.modeSplit"), ModeSplit),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.toggleOutline"), ToggleOutline),
                MenuItem::action(locale.app_text("menu.toggleSidebar"), ToggleSidebar),
                MenuItem::action(locale.app_text("menu.toggleToolbar"), ToggleToolbar),
                MenuItem::action(locale.app_text("menu.toggleTypography"), ToggleTypography),
                MenuItem::action(locale.app_text("menu.toggleTheme"), ToggleTheme),
                MenuItem::action(locale.app_text("menu.followSystemTheme"), FollowSystemTheme),
                MenuItem::separator(),
                MenuItem::action(locale.app_text("menu.zoomIn"), ZoomIn),
                MenuItem::action(locale.app_text("menu.zoomOut"), ZoomOut),
                MenuItem::action(locale.app_text("menu.zoomReset"), ZoomReset),
            ],
        },
    ]);
}

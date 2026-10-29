//! Nomo GPUI 原生入口。
//!
//! 用法：`nomo [文件或目录 ...]`。目录作为侧栏根目录打开，文件各开一个标签。

#![cfg_attr(all(target_os = "windows", not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod buffer;
mod cache;
mod clipboard;
mod document;
mod editor;
mod nomo_mark;
mod open_profile;
mod frame_profile;
mod embeds;
mod images;
mod image_import;
mod kp;
mod layout;
mod markdown;
mod markdown_templates;
mod settings;
mod theme;
mod ui;

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

struct QuitRequested(bool);
impl gpui::Global for QuitRequested {}
pub(crate) fn cancel_quit(cx:&mut App){cx.set_global(QuitRequested(false));}
pub(crate) fn quit_workspaces(cx:&mut App){
    cx.set_global(QuitRequested(true));
    for handle in cx.windows().into_iter().filter_map(|window|window.downcast::<Workspace>()) {
        let _=handle.update(cx,|workspace,window,cx|{if workspace.request_quit(window,cx){window.remove_window();}});
    }
    finish_quit(cx);
}
fn finish_quit(cx:&mut App){
    if !cx.try_global::<QuitRequested>().is_some_and(|request|request.0)||cx.windows().iter().any(|window|window.downcast::<Workspace>().is_some()){return;}
    cancel_quit(cx);
    for handle in cx.windows(){let _=handle.update(cx,|_,window,_|window.remove_window());}
    cx.quit();
}

fn main() {
    #[cfg(feature = "nomo-mark-demo")]
    let nomo_mark_demo = std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark-demo" | "--editor-control-demo"));
    #[cfg(feature = "nomo-mark-demo")]
    let nomo_mark_file = std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark" | "--editor-control"));
    let paths: Vec<PathBuf> = std::env::args().skip(1).filter(|argument| !matches!(argument.as_str(), "--nomo-mark-demo" | "--nomo-mark" | "--editor-control-demo" | "--editor-control")).map(PathBuf::from).collect();
    #[cfg(not(windows))]
    let application = gpui_platform::application();
    #[cfg(windows)]
    let application = gpui::Application::with_platform(std::rc::Rc::new(
        gpui_windows::WindowsPlatform::new(false).expect("初始化 Windows 平台"),
    ));
    let application = application.with_assets(ui::components::Assets);
    // 平台可能在启动完成前送达文件 URL；只暂存尚未交给 Workspace 的路径。
    let open_state = std::rc::Rc::new(std::cell::RefCell::new((None::<gpui::AsyncApp>, Vec::<PathBuf>::new())));
    let development_entry = cfg!(feature = "nomo-mark-demo") && std::env::args().any(|argument| matches!(argument.as_str(), "--nomo-mark" | "--nomo-mark-demo" | "--editor-control" | "--editor-control-demo"));
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
        cx.set_global(images::ImageStore::default());
        cx.set_global(embeds::EmbedStore::default());
        editor::bind_keys(cx);
        nomo_mark::view::bind_keys(cx);
        ui::text_field::bind_keys(cx);
        ui::dialogs::init(cx);
        workspace::bind_keys(cx);
        cx.on_action(|_: &workspace::Quit,cx|quit_workspaces(cx));
        cx.on_window_closed(|cx,_|{ui::updates::prune_closed_windows(cx);finish_quit(cx);}).detach();
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

        let mut paths = paths;
        paths.append(&mut open_state.borrow_mut().1);
        open_workspace(paths, cx);
        open_state.borrow_mut().0 = Some(cx.to_async());
        cx.activate(true);
    });
}

fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    if let Some(handle) = cx.windows().into_iter().find_map(|window| window.downcast::<Workspace>()) {
        if let Err(error) = handle.update(cx, |workspace, window, cx| {
            if paths.is_empty() { workspace.show_home(window, cx); }
            for path in paths { workspace.open_path(&path, window, cx); }
            workspace.reveal_window(window,cx);
        }) { eprintln!("打开文档窗口失败：{error}"); }
    } else {
        open_workspace(paths, cx);
    }
    cx.activate(true);
}

fn open_workspace(paths: Vec<PathBuf>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
    let options = WindowOptions {
        app_id: Some("com.nomo.desktop".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("Nomo".into()), appears_transparent: true, traffic_light_position: Some(point(px(14.0), px(13.0))) }),
        window_min_size: Some(size(px(640.0), px(420.0))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        theme::Theme::apply(window.appearance(), cx);
        // 跟随系统深浅色：外观变化时重新应用主题（手动指定模式时 apply 会忽略外观）。
        window.observe_window_appearance(|window, cx| theme::Theme::apply(window.appearance(), cx)).detach();
        cx.new(|cx| Workspace::new(paths, window, cx))
    })
    .expect("打开窗口失败");
}

fn set_menus(cx: &mut App) {
    use workspace::*;
    cx.set_menus(vec![
        Menu { disabled: false, name: "Nomo".into(), items: vec![MenuItem::action("设置…", OpenSettings), MenuItem::separator(), MenuItem::action("退出 Nomo", Quit)] },
        Menu { disabled: false,
            name: "文件".into(),
            items: vec![
                MenuItem::action("新建", NewFile),
                MenuItem::action("打开…", OpenFile),
                MenuItem::action("打开文件夹…", OpenFolder),
                MenuItem::separator(),
                MenuItem::action("保存", Save),
                MenuItem::action("另存为…", SaveAs),
                MenuItem::separator(),
                MenuItem::action("关闭标签", CloseTab),
            ],
        },
        Menu { disabled: false,
            name: "编辑".into(),
            items: vec![
                MenuItem::action("撤销", editor::Undo),
                MenuItem::action("重做", editor::Redo),
                MenuItem::separator(),
                MenuItem::action("剪切", editor::Cut),
                MenuItem::action("复制", editor::Copy),
                MenuItem::action("粘贴", editor::Paste),
                MenuItem::action("全选", editor::SelectAll),
                MenuItem::separator(),
                MenuItem::action("查找", Find),
                MenuItem::action("查找并替换", FindReplace),
                MenuItem::action("查找下一个", FindNext),
                MenuItem::action("查找上一个", FindPrev),
            ],
        },
        Menu { disabled: false,
            name: "格式".into(),
            items: vec![
                MenuItem::action("加粗", Bold),
                MenuItem::action("斜体", Italic),
                MenuItem::action("删除线", Strike),
                MenuItem::action("行内代码", InlineCode),
                MenuItem::action("链接", Link),
                MenuItem::separator(),
                MenuItem::action("一级标题", Heading1),
                MenuItem::action("二级标题", Heading2),
                MenuItem::action("三级标题", Heading3),
                MenuItem::action("正文", Paragraph),
                MenuItem::separator(),
                MenuItem::action("引用", Quote),
                MenuItem::action("无序列表", BulletList),
                MenuItem::action("有序列表", OrderedList),
                MenuItem::action("任务列表", TaskList),
                MenuItem::separator(),
                MenuItem::action("代码块", CodeBlock),
                MenuItem::action("表格", Table),
                MenuItem::action("公式块", MathBlock),
                MenuItem::action("分隔线", Rule),
            ],
        },
        Menu { disabled: false,
            name: "视图".into(),
            items: vec![
                MenuItem::action("语义模式", ModeSemantic),
                MenuItem::action("源码模式", ModeSource),
                MenuItem::action("分栏模式", ModeSplit),
                MenuItem::separator(),
                MenuItem::action("文档大纲", ToggleOutline),
                MenuItem::action("侧栏", ToggleSidebar),
                MenuItem::action("工具栏", ToggleToolbar),
                MenuItem::action("精细排版", ToggleTypography),
                MenuItem::action("切换深色 / 浅色", ToggleTheme),
                MenuItem::action("主题跟随系统", FollowSystemTheme),
                MenuItem::separator(),
                MenuItem::action("放大", ZoomIn),
                MenuItem::action("缩小", ZoomOut),
                MenuItem::action("实际大小", ZoomReset),
            ],
        },
    ]);
}

//! Nomo GPUI 原生原型入口。
//!
//! 用法：`nomo-gpui [文件或目录 ...]`。目录作为侧栏根目录打开，文件各开一个标签。

mod buffer;
mod cache;
mod document;
mod editor;
mod editor_control;
mod embeds;
mod images;
mod kp;
mod layout;
mod markdown;
#[path = "../../src-tauri/src/markdown_templates.rs"]
mod markdown_templates;
mod settings;
mod theme;
mod ui;

#[cfg(feature = "editor-control-demo")]
#[path = "../../src-tauri/src/text_document/native.rs"]
#[allow(dead_code)] // 仅接入现有会话 API，其余原版服务保留原实现。
mod text_document;
#[cfg(feature = "editor-control-demo")]
#[path = "../../src-tauri/src/markdown_encoding.rs"]
mod models;
#[cfg(feature = "editor-control-demo")]
#[path = "../../src-tauri/src/file_system/atomic_write.rs"]
mod file_system;
#[cfg(feature = "editor-control-demo")]
mod app_logger {
    pub fn warn(scope: &str, message: &str) { eprintln!("[{scope}] {message}"); }
}

// 原版后端的既有异步测试使用 Tauri 的 Tokio 包装；开发宿主测试复用同一运行时语义。
#[cfg(all(test,feature = "editor-control-demo"))]
extern crate self as tauri;
#[cfg(all(test,feature = "editor-control-demo"))]
mod async_runtime {
    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime"))
    }
    pub fn block_on<F: std::future::Future>(future:F)->F::Output { runtime().block_on(future) }
    pub fn spawn<F:std::future::Future+Send+'static>(future:F)->tokio::task::JoinHandle<F::Output>
        where F::Output:Send+'static { runtime().spawn(future) }
}

use gpui::{App, AppContext, Application, Bounds, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};
use std::path::PathBuf;
use ui::workspace::{self, Workspace};

fn main() {
    #[cfg(feature = "editor-control-demo")]
    let control_demo = std::env::args().any(|argument| argument == "--editor-control-demo");
    #[cfg(feature = "editor-control-demo")]
    let native_editor = std::env::args().any(|argument| argument == "--editor-control");
    let paths: Vec<PathBuf> = std::env::args().skip(1).filter(|argument| argument != "--editor-control-demo" && argument != "--editor-control").map(PathBuf::from).collect();
    Application::new().with_assets(ui::components::Assets).run(move |cx: &mut App| {
        // 先放浅色主题占位，窗口创建后按系统外观立即校正。
        cx.set_global(theme::Theme::light());
        cx.set_global(settings::Settings::load());
        cx.set_global(images::ImageStore::default());
        cx.set_global(embeds::EmbedStore::default());
        editor::bind_keys(cx);
        editor_control::view::bind_keys(cx);
        ui::text_field::bind_keys(cx);
        workspace::bind_keys(cx);
        cx.on_action(|_: &workspace::Quit, cx| cx.quit());
        cx.on_action(|_: &workspace::OpenSettings, cx| ui::settings_window::open(cx));
        set_menus(cx);

        #[cfg(feature = "editor-control-demo")]
        if control_demo || native_editor {
            cx.on_window_closed(|cx| { if cx.windows().is_empty() { cx.quit(); } }).detach();
            let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
            cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {title:Some("Nomo · 控件开发".into()),..Default::default()}),..Default::default()},
                |window,cx| {theme::Theme::apply(window.appearance(),cx);cx.new(|cx| if native_editor { editor_control::demo::ControlDemo::file(paths.first().cloned(), window, cx) } else { editor_control::demo::ControlDemo::new(window,cx) })}).expect("打开控件开发窗口");
            cx.activate(true);return;
        }

        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
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
        cx.activate(true);
    });
}

fn set_menus(cx: &mut App) {
    use workspace::*;
    cx.set_menus(vec![
        Menu { name: "Nomo".into(), items: vec![MenuItem::action("设置…", OpenSettings), MenuItem::separator(), MenuItem::action("退出 Nomo", Quit)] },
        Menu {
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
        Menu {
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
        Menu {
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
        Menu {
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

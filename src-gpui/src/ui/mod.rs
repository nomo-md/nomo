pub mod chrome;
pub mod components;
pub(crate) mod dialogs;
pub mod editor_popovers;
pub mod emoji_picker;
pub mod file_tree;
pub(crate) mod home;
pub mod icon_assets;
pub mod motion;
pub(crate) mod mini;
pub(crate) mod popup;
pub(crate) mod quick_insert;
pub mod settings_schema;
pub mod settings_window;
pub(crate) mod shortcuts;
pub mod text_field;
pub(crate) mod toast;
pub(crate) mod tray;
pub(crate) mod updates;
pub(crate) mod toolbar_i18n;
pub mod window_chrome;
pub mod workspace;
pub mod workspace_surface;

/// gpui-pre 的静态 KeyBinding::new 不再解析 secondary，须根据实际平台加载。
pub(crate) fn key_binding<A: gpui::Action>(
    keys: &str,
    action: A,
    context: Option<&str>,
    cx: &gpui::App,
) -> gpui::KeyBinding {
    let context = context.map(|value| {
        gpui::KeyBindingContextPredicate::parse(value).expect("内置快捷键上下文").into()
    });
    gpui::KeyBinding::load(keys, Box::new(action), context, false, None, cx)
        .expect("内置平台快捷键")
}

/// 设置提交时完整替换应用拥有的键表，旧自定义组合立即失效。
pub(crate) fn reload_key_bindings(cx: &mut gpui::App) {
    cx.clear_key_bindings();
    crate::editor::bind_keys(cx);
    crate::nomo_mark::view::bind_keys(cx);
    text_field::bind_keys(cx);
    workspace::bind_keys(cx);
    shortcuts::bind_keys(cx);
}

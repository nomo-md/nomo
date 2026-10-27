pub mod chrome;
pub mod components;
pub mod editor_popovers;
pub mod emoji_picker;
pub mod file_tree;
pub mod icon_assets;
pub mod motion;
pub mod settings_schema;
pub mod settings_window;
pub mod text_field;
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

use super::{Menu, MenuAction, MenuItem, ToolbarLocale, action};
use crate::settings::{EditorMode, Settings, ThemeMode};
use gpui::{Action, App, Global};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewMenuState {
    pub mode: Option<EditorMode>,
    pub editing_locked: bool,
    pub split_available: bool,
    pub outline_visible: bool,
    pub split_guide: Option<bool>,
    pub sidebar_visible: bool,
    toolbar_visible: bool,
    code_line_numbers: bool,
    zoom_control_visible: bool,
    stats_visible: bool,
    lint_indicator: Option<bool>,
    edit_lock_button_visible: bool,
    typography: bool,
    follows_system_theme: bool,
}

impl Global for ViewMenuState {}

impl ViewMenuState {
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        Self {
            sidebar_visible: settings.show_sidebar,
            toolbar_visible: settings.show_toolbar,
            code_line_numbers: settings.code_line_numbers,
            zoom_control_visible: settings.zoom_control_visible,
            stats_visible: settings.stats_visible,
            lint_indicator: settings.markdown_lint_enabled.then_some(settings.lint_indicator_visible),
            edit_lock_button_visible: settings.edit_lock_button_visible,
            typography: settings.typography,
            follows_system_theme: settings.theme_mode == ThemeMode::System,
            ..Self::default()
        }
    }

    pub(super) fn items(self, menu: Menu, locale: ToolbarLocale) -> Vec<MenuItem> {
        let document = self.mode.is_some();
        let action = |key, shortcut, action: fn() -> Box<dyn Action>, enabled| {
            MenuItem::new(locale.app_text(key), shortcut, MenuAction::Dispatch(action), enabled)
        };
        let separator = MenuItem::separator;
        match menu {
            Menu::View => vec![
                action("menu.modeSemantic", "", || Box::new(action::ModeSemantic), document)
                    .checked(self.mode == Some(EditorMode::Semantic)),
                action("menu.modeSource", "", || Box::new(action::ModeSource), document)
                    .checked(self.mode == Some(EditorMode::Source)),
                action("menu.modeSplit", "", || Box::new(action::ModeSplit), document && self.split_available)
                    .checked(self.mode == Some(EditorMode::Split)),
                separator(),
                action("menu.toggleSidebar", "", || Box::new(action::ToggleSidebar), true).checked(self.sidebar_visible),
                action("menu.toggleToolbar", "", || Box::new(action::ToggleToolbar), true).checked(self.toolbar_visible),
                action("menu.toggleOutline", "Ctrl + Alt + O", || Box::new(action::ToggleOutline), document).checked(self.outline_visible),
                action("outline.resetPosition", "", || Box::new(action::ResetOutlinePosition), true),
                MenuItem::submenu(Menu::ViewText, locale),
                MenuItem::submenu(Menu::ViewCorner, locale),
                separator(),
                MenuItem::submenu(Menu::ViewAppearance, locale),
            ],
            Menu::ViewText => vec![
                action("menu.splitGuide", "", || Box::new(action::ToggleSplitGuide), self.split_guide.is_some())
                    .checked(self.split_guide.unwrap_or(false)),
                action("settings.codeBlockLineNumbersVisible.label", "", || Box::new(action::ToggleCodeLineNumbers), true)
                    .checked(self.code_line_numbers),
            ],
            Menu::ViewCorner => vec![
                action("menu.zoomControl", "", || Box::new(action::ToggleZoomControl), true).checked(self.zoom_control_visible),
                action("menu.writingStats", "", || Box::new(action::ToggleWritingStats), true).checked(self.stats_visible),
                action("menu.lintIndicator", "", || Box::new(action::ToggleLintIndicator), self.lint_indicator.is_some())
                    .checked(self.lint_indicator.unwrap_or(false)),
                action("menu.editLockButton", "", || Box::new(action::ToggleEditLockButton), true).checked(self.edit_lock_button_visible),
            ],
            Menu::ViewAppearance => vec![
                action("menu.zoomIn", "Ctrl + +", || Box::new(action::ZoomIn), true),
                action("menu.zoomOut", "", || Box::new(action::ZoomOut), true),
                action("menu.zoomReset", "", || Box::new(action::ZoomReset), true),
                separator(),
                action("chrome.switchTheme", "", || Box::new(action::ToggleTheme), true),
                action("menu.followSystemTheme", "", || Box::new(action::FollowSystemTheme), true).checked(self.follows_system_theme),
                action("menu.toggleTypography", "", || Box::new(action::ToggleTypography), true).checked(self.typography),
            ],
            _ => unreachable!("only display menus use this item list"),
        }
    }

    pub(super) fn edit_lock_item(self, locale: ToolbarLocale) -> MenuItem {
        MenuItem::new(locale.app_text("documentLock.lock"), "",
            MenuAction::Dispatch(|| Box::new(action::ToggleEditLock)), self.mode.is_some())
            .checked(self.editing_locked)
    }

    fn native_item(self, entry: MenuItem, locale: ToolbarLocale) -> gpui::MenuItem {
        if entry.separator { return gpui::MenuItem::separator(); }
        match entry.action {
            MenuAction::Dispatch(action) => gpui::MenuItem::Action {
                name: entry.label,
                action: action(),
                os_action: None,
                checked: entry.checked.unwrap_or(false),
                disabled: !entry.enabled,
            },
            MenuAction::Submenu(menu) => gpui::MenuItem::submenu(self.native_menu(menu, locale)),
            _ => unreachable!("native display menus dispatch workspace actions"),
        }
    }

    fn native_menu(self, menu: Menu, locale: ToolbarLocale) -> gpui::Menu {
        gpui::Menu::new(menu.label(locale))
            .items(self.items(menu, locale).into_iter().map(|item| self.native_item(item, locale)))
    }
}

fn native_state(cx: &App) -> ViewMenuState {
    cx.try_global::<ViewMenuState>().copied().unwrap_or_else(|| ViewMenuState::from_settings(cx.global::<Settings>()))
}

pub(crate) fn native_view_menu(cx: &App) -> gpui::Menu {
    native_state(cx).native_menu(Menu::View, ToolbarLocale::current(cx))
}

pub(crate) fn native_edit_lock_item(cx: &App) -> gpui::MenuItem {
    let locale = ToolbarLocale::current(cx);
    let state = native_state(cx);
    state.native_item(state.edit_lock_item(locale), locale)
}

#[cfg(target_os = "macos")]
impl ViewMenuState {
    pub(crate) fn sync_native(self, cx: &mut App) {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::{MainThreadMarker, NSString};

        if cx.try_global::<Self>() == Some(&self) { return; }
        let Some(mtm) = MainThreadMarker::new() else { return; };
        let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return; };
        let locale = ToolbarLocale::current(cx);
        // 原地更新勾选。重建系统菜单会重复向 GPUI 注册同一批动作。
        if let Some(view) = menu.itemWithTitle(&NSString::from_str(Menu::View.label(locale))).and_then(|item| item.submenu()) {
            self.sync_native_checks(Menu::View, &view, locale);
        }
        if let Some(item) = menu.itemWithTitle(&NSString::from_str(Menu::Edit.label(locale)))
            .and_then(|item| item.submenu())
            .and_then(|edit| edit.itemWithTitle(&NSString::from_str(locale.app_text("documentLock.lock"))))
        {
            item.setState(isize::from(self.editing_locked));
        }
        cx.set_global(self);
    }

    fn sync_native_checks(self, menu: Menu, native: &objc2_app_kit::NSMenu, locale: ToolbarLocale) {
        use objc2_foundation::NSString;

        for entry in self.items(menu, locale) {
            if entry.checked.is_none() && !matches!(entry.action, MenuAction::Submenu(_)) { continue; }
            let Some(item) = native.itemWithTitle(&NSString::from_str(entry.label.as_ref())) else { continue; };
            if let Some(checked) = entry.checked {
                item.setState(isize::from(checked));
            } else if let MenuAction::Submenu(group) = entry.action && let Some(submenu) = item.submenu() {
                self.sync_native_checks(group, &submenu, locale);
            }
        }
    }
}

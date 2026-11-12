//! 文档导航的组合与显隐生命周期；主页和编辑会话不参与布局切换。
use super::*;

const SPLIT_DIVIDER: f32 = 6.0;
const SPLIT_SHADOW_DURATION: Duration = Duration::from_millis(110);
const MIN_SECTION: f32 = 128.0;
const TAB_HEIGHT: f32 = 36.0;
const TAB_INSET: f32 = 3.0;
const TAB_MARGIN: f32 = 16.0;
const TAB_CONTENT_DURATION: Duration = Duration::from_millis(160);

pub(super) struct NavigationState {
    layout: Option<DocumentNavigationLayout>,
    pub tree_visible: bool,
    bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    split_bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    split_grab_offset: Option<f32>,
    split_ratio: Option<f32>,
    split_shadow: motion::ScalarMotion,
    divider_focus: FocusHandle,
    tab_motion: Option<(motion::Spring<1>, Instant)>,
    tab_content_started: Option<Instant>,
}

impl NavigationState {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        Self { layout: None, tree_visible: false, bounds: Rc::new(Cell::new(None)),
            split_bounds: Rc::new(Cell::new(None)), split_grab_offset: None, split_ratio: None,
            split_shadow: motion::ScalarMotion::default(),
            divider_focus: cx.focus_handle(), tab_motion: None, tab_content_started: None }
    }

    fn tab_progress(&mut self, outline: bool, reduced: bool, window: &Window) -> (f32, f32) {
        let now = Instant::now();
        if reduced || self.tab_content_started.is_some_and(|started| now.duration_since(started) >= TAB_CONTENT_DURATION) {
            self.tab_content_started = None;
        }
        let target = if outline { 1.0 } else { 0.0 };
        let (spring, last) = self.tab_motion.get_or_insert_with(|| {
            (motion::Spring::new([target], 480.0, 36.0), now)
        });
        let elapsed = now.saturating_duration_since(std::mem::replace(last, now));
        if spring.target != [target] {
            // 途中切换只改目标，保留最后绘制的位置与速度，不重新起步或排队。
            spring.target = [target];
            if !reduced {
                // 快速切回时沿用正在恢复的透明度和位移，避免反复闪暗。
                self.tab_content_started.get_or_insert(now);
            }
        } else if !reduced && spring.running() {
            spring.step(elapsed.as_secs_f32());
        }
        if reduced { spring.snap(); }
        let content_progress = self.tab_content_started.map_or(1.0, |started| {
            motion::ease(now.duration_since(started).as_secs_f32() / TAB_CONTENT_DURATION.as_secs_f32())
        });
        if spring.running() || self.tab_content_started.is_some() { window.request_animation_frame(); }
        (spring.p[0], content_progress)
    }
}

fn split_ratio(settings: &Settings) -> f32 {
    settings.value("documentNavigationSplitRatio").as_f64().filter(|ratio| ratio.is_finite())
        .map_or(0.55, |ratio| (ratio as f32).clamp(0.1, 0.9))
}

/// 极短窗口均分可用空间；正常窗口保留可用的导航区域，比例不随窗口缩小改写。
fn split_heights(height: f32, ratio: f32) -> (f32, f32) {
    let available = (height - SPLIT_DIVIDER).max(0.0);
    let minimum = MIN_SECTION.min(available / 2.0);
    let top = (available * ratio).clamp(minimum, (available - minimum).max(minimum));
    (top, available - top)
}

impl Workspace {
    pub(super) fn tree_panel_visible(&self, settings: &Settings) -> bool {
        !self.home_visible && !self.mini.active() && self.explorer.sidebar_visible(settings)
            && match settings.document_navigation_layout {
                DocumentNavigationLayout::RightOutline => true,
                DocumentNavigationLayout::LeftTabs => settings.string("documentNavigationTab") != "outline",
                DocumentNavigationLayout::LeftSplit => true,
            }
    }

    pub(super) fn sync_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>().clone();
        let changed = self.navigation.layout != Some(settings.document_navigation_layout);
        if changed || settings.document_navigation_layout != DocumentNavigationLayout::LeftTabs
            || self.home_visible || self.mini.active() || !self.explorer.sidebar_visible(&settings) {
            // 隐藏后丢弃短时动效，重新显示直接落在当前标签，不续播旧速度。
            self.navigation.tab_motion = None;
            self.navigation.tab_content_started = None;
        }
        if changed || self.home_visible || self.mini.active() || !self.explorer.sidebar_visible(&settings) {
            self.finish_navigation_resize(cx);
            self.navigation.split_shadow.reset();
            if self.navigation.divider_focus.contains_focused(window, cx) && !window.has_active_prompt() {
                self.focus_document(window, cx);
            }
        }
        if changed {
            self.navigation.layout = Some(settings.document_navigation_layout);
            self.outline_ui.suspend_for_navigation();
        }
        let tree_visible = self.tree_panel_visible(&settings);
        if tree_visible != self.navigation.tree_visible {
            self.navigation.tree_visible = tree_visible;
            if tree_visible { self.resume_tree(cx); }
            else {
                let focused = self.explorer.search_field.as_ref().is_some_and(|field| field.focus_handle(cx).contains_focused(window, cx))
                    || self.explorer.edit.as_ref().is_some_and(|edit| edit.field.focus_handle(cx).contains_focused(window, cx));
                if focused && !window.has_active_prompt() { self.focus_document(window, cx); }
                self.suspend_tree(cx);
            }
        }
        let visible = self.outline_visible(cx);
        if !visible {
            if self.outline_ui.contains_focus(window, cx) && !window.has_active_prompt() { self.focus_document(window, cx); }
            if settings.document_navigation_layout != DocumentNavigationLayout::RightOutline || self.home_visible {
                self.outline_ui.suspend_for_navigation();
            }
        }
        // 右侧浮层在自己的渲染入口截取退场快照后再停用，避免淡出时丢失标题页。
        if !self.mini.active() && settings.document_navigation_layout != DocumentNavigationLayout::RightOutline
            && let Some(document) = self.document() {
            document.update(cx, |host, cx| host.set_outline_enabled(visible, settings.outline_level, cx));
        }
    }

    fn focus_document(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = self.document() { window.focus(&document.focus_handle(cx), cx); }
        else { window.focus(&self.focus, cx); }
    }

    pub(super) fn select_navigation_tab(&mut self, outline: bool, window: &mut Window, cx: &mut Context<Self>) {
        if outline { Settings::set(cx, "outlineVisible", serde_json::json!(true)); }
        Settings::set(cx, "documentNavigationTab", serde_json::json!(if outline { "outline" } else { "files" }));
        // 点击入口不改变正文选区；键盘焦点落到新面板的现有导航控件。
        if outline { self.outline_ui.focus(window, cx); }
        else if self.tree.is_some() && self.explorer.search_field.is_some() { self.focus_tree_search(None, window, cx); }
        else { self.focus_document(window, cx); }
        cx.notify();
    }

    pub(super) fn render_navigation_sidebar(&mut self, width: f32, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let settings = cx.global::<Settings>().clone();
        let layout = settings.document_navigation_layout;
        let outline_tab = settings.string("documentNavigationTab") == "outline";
        let tab_progress = (layout == DocumentNavigationLayout::LeftTabs)
            .then(|| self.navigation.tab_progress(outline_tab, self.explorer.reduce_motion, window));
        let header = tab_progress.map(|(progress, _)| {
            let locale = ToolbarLocale::current(cx);
            let segment_width = ((width - TAB_MARGIN * 2.0 - TAB_INSET * 2.0) / 2.0).max(0.0);
            let track = if theme.dark { theme.app_bg } else { theme.rail_bg.blend(theme.fg.opacity(0.065)) };
            let surface = if theme.dark { theme.surface.blend(theme.fg.opacity(0.13)) } else { theme.surface };
            let indicator = div().absolute().top(px(TAB_INSET)).left(px(TAB_INSET + segment_width * progress))
                .w(px(segment_width)).h(px(TAB_HEIGHT - TAB_INSET * 2.0)).rounded(px(7.0))
                .bg(surface).border_1().border_color(theme.fg.opacity(if theme.dark { 0.05 } else { 0.025 }))
                .shadow(vec![gpui::BoxShadow { inset: false,
                    color: gpui::rgba(if theme.dark { 0x0000002b } else { 0x16202c19 }).into(),
                    offset: gpui::point(px(0.0), px(1.0)), blur_radius: px(3.0), spread_radius: px(0.0),
                }]);
            let mut navigation = div().relative().w_full().h(px(TAB_HEIGHT)).flex_none().p(px(TAB_INSET))
                .flex().items_center().rounded(px(10.0)).bg(track).child(indicator);
            for (outline, key, glyph) in [(false, "navigation.files", IconName::Folder), (true, "navigation.outline", IconName::ListTree)] {
                let selected = outline == outline_tab;
                let active = if outline { progress } else { 1.0 - progress };
                navigation = navigation.child(div().id(if outline { "navigation-outline-tab" } else { "navigation-files-tab" })
                    .relative().h_full().px(px(8.0)).flex_1().min_w_0().flex().items_center().justify_center().gap(px(7.0))
                    .rounded(px(7.0)).cursor_pointer().tab_index(0)
                    .tooltip(tooltip(locale.app_text(key)))
                    .text_size(px(12.0)).text_color(motion::color(theme.muted, theme.fg, active))
                    .when(selected, |el| el.font_weight(FontWeight::MEDIUM))
                    .when(!selected, |el| el.hover(|style| style.text_color(theme.fg)))
                    .child(icon(glyph, 14.0, motion::color(theme.muted, theme.accent, active)))
                    .child(div().min_w_0().truncate().child(locale.app_text(key)))
                    .on_click(cx.listener(move |workspace, _, window, cx| workspace.select_navigation_tab(outline, window, cx))));
            }
            div().w_full().flex_none().px(px(TAB_MARGIN)).pt(px(6.0)).pb(px(6.0))
                .child(navigation)
        });
        let content = match layout {
            DocumentNavigationLayout::RightOutline => self.render_explorer_tree(width, theme, cx),
            DocumentNavigationLayout::LeftTabs if !outline_tab => self.render_explorer_tree(width, theme, cx),
            DocumentNavigationLayout::LeftTabs => self.render_navigation_outline(theme, window, cx),
            DocumentNavigationLayout::LeftSplit => self.render_navigation_split(width, theme, window, cx),
        };
        let content = if let Some((_, progress)) = tab_progress {
            let panel = div().relative().size_full()
                .when(progress < 1.0, |el| el.left(px(-4.0 * (1.0 - progress))).opacity(0.78 + 0.22 * progress))
                .child(content);
            self.outline_region(panel.into_any_element(), cx)
        } else { content };
        let bounds = self.navigation.bounds.clone();
        let resize = div().id("navigation-width-resizer").absolute().right_0().top_0().bottom_0().w(px(5.0))
            .cursor_col_resize().on_mouse_down(MouseButton::Left, cx.listener(|workspace, _, _, cx| {
                workspace.explorer.resizing = true; cx.stop_propagation(); cx.notify();
            }));
        // 普通 Div 的移动监听会被大纲卡片遮挡。可见侧栏预先注册捕获监听，
        // 按下后尚未重绘时也能接住移动；否则快速拖动会在下一帧注册前丢失。
        // 回调仅存在于当前帧且只持有弱引用，未拖动时立即返回，隐藏后随元素释放。
        let tracking = {
            let workspace = cx.entity().downgrade();
            let vertical = self.navigation.split_grab_offset.is_some();
            let horizontal = self.explorer.resizing;
            gpui::canvas(|_, _, _| (), move |_, _, window, _| {
                if vertical || horizontal {
                    window.set_window_cursor_style(if vertical { gpui::CursorStyle::ResizeUpDown } else { gpui::CursorStyle::ResizeLeftRight });
                }
                let moving = workspace.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Capture { return; }
                    let _ = moving.update(cx, |workspace, cx| {
                        if workspace.explorer.resizing || workspace.navigation.split_grab_offset.is_some() {
                            workspace.navigation_mouse_move(event, window, cx);
                            cx.stop_propagation();
                        }
                    });
                });
                let released = workspace.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Capture || event.button != MouseButton::Left { return; }
                    let _ = released.update(cx, |workspace, cx| {
                        if workspace.explorer.resizing || workspace.navigation.split_grab_offset.is_some() {
                            workspace.finish_navigation_resize(cx);
                            cx.notify();
                        }
                    });
                });
            }).absolute().size_full()
        };
        let home = crate::ui::home::nav_home_row(false, theme, cx)
            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.show_home(window, cx)));
        div().relative().w(px(width)).h_full().min_h_0().flex_none().flex().flex_col().bg(theme.rail_bg)
            .child(gpui::canvas(move |area, _, _| bounds.set(Some(area)), |_, _, _, _| {}).absolute().size_full())
            .child(div().w_full().h(px(TAB_RAIL_HEIGHT)).flex_none().px(px(8.0)).flex().items_center().child(home))
            .children(header).child(div().w_full().flex_1().min_h_0().overflow_hidden().child(content)).child(resize).child(tracking)
    }

    fn render_navigation_outline(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.document().is_none() {
            return div().p(px(14.0)).text_size(px(13.0)).text_color(theme.muted)
                .child(ToolbarLocale::current(cx).app_text("navigation.noDocument")).into_any_element();
        }
        if !cx.global::<Settings>().show_outline {
            return div().p(px(10.0)).child(button("navigation-show-outline", ToolbarLocale::current(cx).app_text("settings.outlineVisible.label"),
                Some(IconName::ListTree), ButtonKind::Ghost, theme).on_click(cx.listener(|workspace, _, _, cx| workspace.toggle_outline(cx)))).into_any_element();
        }
        let mode = self.tab().map_or(EditorMode::Semantic, |tab| tab.mode);
        div().size_full().children(self.render_docked_outline(theme, mode, window, cx)).into_any_element()
    }

    fn outline_region(&self, content: AnyElement, cx: &Context<Self>) -> AnyElement {
        let bounds = self.outline_ui.sidebar_bounds.clone();
        let weak = cx.entity().downgrade();
        // 标签模式始终测量同一个视口，切到大纲时已有尺寸，入场位移也不触发重新测量。
        div().id("navigation-outline-region").relative().size_full().overflow_hidden()
            .child(gpui::canvas(move |area, window, _| {
                if bounds.replace(Some(area)) != Some(area) {
                    let weak = weak.clone();
                    window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |_, cx| cx.notify()); });
                }
            }, |_, _, _, _| {}).absolute().size_full()).child(content).into_any_element()
    }

    fn render_navigation_split(&mut self, width: f32, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>();
        let outline_collapsed = settings.bool("documentNavigationOutlineCollapsed") || !settings.show_outline;
        // 分组标题移除后文件树始终可见；大纲仍可从显示菜单或快捷键切换，旧的文件区折叠值不再隐藏入口。
        if outline_collapsed {
            self.navigation.split_shadow.reset();
            return self.render_explorer_tree(width, theme, cx);
        }
        let ratio = self.navigation.split_ratio.unwrap_or_else(|| split_ratio(settings));
        let height = self.navigation.split_bounds.get().map_or(400.0, |bounds| f32::from(bounds.size.height));
        let (top_height, _) = split_heights(height, ratio);
        let tree = self.render_explorer_tree(width, theme, cx);
        let outline = self.render_navigation_outline(theme, window, cx);
        let top = div().w_full().h(px(top_height)).flex_none().min_h_0().overflow_hidden().child(tree);
        let bottom = div().w_full().flex_1().min_h_0().overflow_hidden().child(self.outline_region(outline, cx));
        // 分界位置直接跟随指针，按下与松开只过渡阴影浓度。
        let dragging = self.navigation.split_grab_offset.is_some();
        let shadow = &mut self.navigation.split_shadow;
        if self.explorer.reduce_motion {
            *shadow = motion::ScalarMotion::settled(dragging);
        } else {
            shadow.set(dragging, false, SPLIT_SHADOW_DURATION);
            if shadow.running() { window.request_animation_frame(); }
        }
        let shadow_color: gpui::Hsla = gpui::rgb(if theme.dark { 0x000000 } else { 0x19262d }).into();
        let strength = if theme.dark { 0.42 + 0.09 * shadow.value() } else { 0.13 + 0.06 * shadow.value() };
        let divider = div().id("navigation-split-resizer").h(px(SPLIT_DIVIDER)).w_full().flex_none()
            .cursor_row_resize().bg(gpui::linear_gradient(180.0,
                gpui::linear_color_stop(shadow_color.opacity(strength), 0.0),
                gpui::linear_color_stop(shadow_color.opacity(0.0), 1.0)))
            .track_focus(&self.navigation.divider_focus).tab_index(0)
            .tooltip(tooltip(ToolbarLocale::current(cx).app_text("navigation.resize")))
            .on_mouse_down(MouseButton::Left, cx.listener(move |workspace, event: &gpui::MouseDownEvent, window, cx| {
                let Some(bounds) = workspace.navigation.split_bounds.get() else { return; };
                let offset = f32::from(event.position.y - bounds.top()) - top_height;
                workspace.navigation.split_grab_offset = Some(offset.clamp(0.0, SPLIT_DIVIDER));
                window.focus(&workspace.navigation.divider_focus, cx); cx.stop_propagation(); cx.notify();
            }))
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                let ratio = split_ratio(cx.global::<Settings>());
                let step = if event.keystroke.modifiers.shift { 0.1 } else { 0.02 };
                let value = match event.keystroke.key.as_str() { "up" => ratio - step, "down" => ratio + step, "home" => 0.1, "end" => 0.9, _ => return };
                Settings::set(cx, "documentNavigationSplitRatio", serde_json::json!(value.clamp(0.1, 0.9)));
                cx.stop_propagation(); cx.notify();
            }));
        let bounds = self.navigation.split_bounds.clone();
        let weak = cx.entity().downgrade();
        div().relative().size_full().flex().flex_col()
            .child(gpui::canvas(move |area, window, _| {
                if bounds.replace(Some(area)) != Some(area) {
                    let weak = weak.clone(); window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |_, cx| cx.notify()); });
                }
            }, |_, _, _, _| {}).absolute().size_full())
            .child(top).child(divider).child(bottom).into_any_element()
    }

    pub(super) fn navigation_mouse_move(&mut self, event: &gpui::MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.explorer.resizing && self.navigation.split_grab_offset.is_none() { return; }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_navigation_resize(cx);
            cx.notify();
            return;
        }
        let mut changed = self.explorer.resizing;
        if self.explorer.resizing && let Some(bounds) = self.navigation.bounds.get() {
            let max = (f32::from(window.viewport_size().width) - 420.0).clamp(180.0, 500.0);
            self.explorer.sidebar_width = Some(f32::from(event.position.x - bounds.left()).clamp(180.0, max));
        }
        if let Some(offset) = self.navigation.split_grab_offset
            && let Some(bounds) = self.navigation.split_bounds.get() {
            let height = f32::from(bounds.size.height);
            let available = (height - SPLIT_DIVIDER).max(1.0);
            let raw = (f32::from(event.position.y - bounds.top()) - offset) / available;
            let next = (split_heights(height, raw).0 / available).clamp(0.1, 0.9);
            let current = self.navigation.split_ratio.unwrap_or_else(|| split_ratio(cx.global::<Settings>()));
            if (split_heights(height, next).0 - split_heights(height, current).0).abs() >= 0.01 {
                self.navigation.split_ratio = Some(next);
                changed = true;
            }
        }
        if changed { cx.notify(); }
    }

    pub(super) fn finish_navigation_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.explorer.resizing) && let Some(width) = self.explorer.sidebar_width {
            Settings::set(cx, "documentSidebarWidth", serde_json::json!(width));
        }
        if self.navigation.split_grab_offset.take().is_some() && let Some(ratio) = self.navigation.split_ratio.take() {
            Settings::set(cx, "documentNavigationSplitRatio", serde_json::json!(ratio));
        }
    }
}

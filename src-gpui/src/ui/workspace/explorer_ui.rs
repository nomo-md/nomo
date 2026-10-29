//! 虚拟目录列表及其滚动条；根目录与普通行共享同一滚动范围。
use super::*;
#[path = "explorer_label.rs"]
mod label;
use label::TreeLabel;

impl Workspace {
    pub(super) fn render_sidebar_animated(&mut self, visible: bool, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.explorer.reduce_motion {
            self.explorer.sidebar_target = Some(visible);
            self.explorer.sidebar_motion = None;
            self.explorer.sidebar_motion_task = None;
            if !visible { return None; }
            let width = self.explorer.sidebar_width.unwrap_or(250.0)
                .min((f32::from(window.viewport_size().width) - 420.0).max(180.0));
            return Some(self.render_explorer_sidebar(width, theme, cx));
        }
        let target = *self.explorer.sidebar_target.get_or_insert(visible);
        if target != visible {
            let from = self.explorer.sidebar_motion.map(|(started, _, from, to, duration)| {
                let progress = (started.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
                from + (to - from) * ease_in_out(progress)
            }).unwrap_or(if target { 1.0 } else { 0.0 });
            let to: f32 = if visible { 1.0 } else { 0.0 };
            let duration = Duration::from_secs_f32(((if visible { 0.28 } else { 0.24 }) * (to - from).abs()).max(0.001));
            self.explorer.sidebar_target = Some(visible);
            self.explorer.sidebar_motion_id = self.explorer.sidebar_motion_id.wrapping_add(1);
            let id = self.explorer.sidebar_motion_id;
            self.explorer.sidebar_motion = Some((Instant::now(), id, from, to, duration));
            self.explorer.sidebar_motion_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(duration).await;
                let _ = this.update(cx, |workspace, cx| {
                    if workspace.explorer.sidebar_motion.is_some_and(|(_, current, _, _, _)| current == id) {
                        workspace.explorer.sidebar_motion = None;
                        cx.notify();
                    }
                });
            }));
        }
        if !visible && self.explorer.sidebar_motion.is_none() { return None; }
        let width = self.explorer.sidebar_width.unwrap_or(250.0)
            .min((f32::from(window.viewport_size().width) - 420.0).max(180.0));
        let sidebar = self.render_explorer_sidebar(width, theme, cx);
        if let Some((_, id, from, to, duration)) = self.explorer.sidebar_motion {
            Some(div().h_full().flex_none().overflow_hidden().child(sidebar)
                .with_animation(("sidebar-motion", id), Animation::new(duration).with_easing(ease_in_out),
                    move |element, progress| element.w(px(width * (from + (to - from) * progress))))
                .into_any_element())
        } else { Some(sidebar) }
    }

    /// 文件栏 = 与主页相同的“主页”导航头 + 文件树视口；导航头不进入虚拟列表，滚动条和拖宽仍以列表边界计算。
    fn render_explorer_sidebar(&mut self, width: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tree = self.render_explorer_tree(width, theme, cx);
        let home = crate::ui::home::nav_home_row(false, theme)
            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.show_home(window, cx)));
        div().w(px(width)).h_full().min_h(px(0.0)).flex_none().flex().flex_col().bg(theme.rail_bg)
            .child(div().w_full().flex_none().px(px(8.0)).pt(px(10.0)).pb(px(6.0)).child(home))
            .child(div().w_full().flex_1().min_h(px(0.0)).child(tree))
            .into_any_element()
    }

    fn render_explorer_tree(&mut self, width: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (root_name, root_path, root_expanded, root_loading, rows) = match &self.tree {
            Some(tree) => (tree.root_name(), Some(tree.root.clone()), tree.root_expanded(), tree.loading(&tree.root), tree.rows()),
            None => return div().id("tree-viewport").relative().w(px(width)).h_full().min_h(px(0.0)).flex_none()
                .bg(theme.rail_bg).into_any_element(),
        };
        let active_path = self.tab().and_then(|tab| tab.document.read(cx).path());
        let preview_path = self.tabs.iter().find(|tab| tab.preview).and_then(|tab| tab.document.read(cx).path());
        if self.tree_animation.as_ref().is_some_and(|animation| animation.started.elapsed() >= TREE_CHEVRON_DURATION) {
            self.tree_animation = None;
        }
        let tree_animation = self.tree_animation.clone();
        let reduce_motion = self.explorer.reduce_motion;
        let selection_animation = self.tree_selection_animation.clone();
        let hovered = self.explorer.hovered.clone();
        let edit = self.explorer.edit.as_ref().map(|edit| (edit.kind, edit.path.clone(), edit.field.clone()));
        let insert_index = self.tree_create_insertion(&rows);
        let search_path = self.explorer.search_matches.get(self.explorer.search_index)
            .and_then(|index| rows.get(*index)).map(|row| row.entry.path.clone())
            .or_else(|| self.explorer.search_keyboard_nav.then(|| self.explorer.search_focused.clone()).flatten());
        let search_query = self.explorer.search_query.clone();
        let count = rows.len() + 1 + usize::from(insert_index.is_some());
        let theme = theme.clone();
        let list = uniform_list("file-tree", count, {
            let theme = theme.clone();
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range.map(|index| {
                    if index == 0 {
                        let path = root_path.clone();
                        let hover_path = path.clone();
                        let root_tip = path.as_ref().map(|path| super::explorer_actions::display_tree_path(path)).unwrap_or_else(|| "打开文件夹".into());
                        let root_hover = path.as_ref().is_some_and(|path| hovered.as_ref() == Some(path) || search_path.as_ref() == Some(path));
                        let chevron = if root_loading {
                            tree_spinner("tree-root-spinner", theme.accent, reduce_motion)
                        } else {
                            tree_chevron(root_expanded, path.as_deref(), tree_animation.as_ref(), theme.accent)
                        };
                        let controls = path.as_ref().filter(|_| root_hover).map(|path| tree_controls(path, true, &theme, cx));
                        let mut root = div().id("tree-root").relative().top(px(-4.0)).mx(px(8.0)).w_full().h(px(28.0))
                            .pl(px(12.0)).pr(px(4.0)).flex().items_center().gap(px(6.0))
                            .rounded(px(4.0)).text_size(px(15.0)).text_color(theme.fg)
                            .font_weight(FontWeight::SEMIBOLD).hover(|style| style.bg(theme.hover))
                            .when(search_path.as_ref() == path.as_ref() && search_path.is_some(), |el| el.bg(theme.sidebar_active))
                            .tooltip(tooltip(root_tip))
                            .child(div().w(px(16.0)).flex_none().child(chevron)).child(icon(IconName::FolderOpen, 14.0, theme.accent))
                            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(TreeLabel::new(root_name.clone(), "", theme.search_match)))
                            .children(controls)
                            .on_hover(cx.listener(move |workspace, entered, _, cx| {
                                if *entered { workspace.explorer.hovered = hover_path.clone(); }
                                else if workspace.explorer.hovered == hover_path { workspace.explorer.hovered = None; }
                                cx.notify();
                            }))
                            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                let focused = workspace.tree.as_ref().map(|tree| tree.root.clone());
                                workspace.focus_tree_search(focused, window, cx);
                                if let Some(tree) = &mut workspace.tree {
                                    let path = tree.root.clone();
                                    match tree.toggle_root() {
                                        Ok(expanded) => {
                                            workspace.animate_tree_chevron(path.clone(), expanded);
                                            workspace.sync_tree_selection(false, cx);
                                            if expanded { workspace.queue_tree_scan(path, false, cx); }
                                            else { workspace.cancel_hidden_tree_scan(); }
                                        }
                                        Err(error) => workspace.notice = Some(error.into()),
                                    }
                                    cx.notify();
                                } else { workspace.prompt_open(true, window, cx); }
                            }));
                        if let Some(path) = path {
                            root = root.on_mouse_down(MouseButton::Right, cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| {
                                workspace.open_tree_menu(path.clone(), true, true, event.position, cx);
                            }));
                        }
                        return div().relative().w_full().h(px(TREE_ROW_HEIGHT)).flex().items_center().child(root).into_any_element();
                    }
                    if insert_index == Some(index) {
                        let (kind, parent, field) = edit.as_ref().expect("creation row has an editor");
                        let depth = rows.iter().find(|row| &row.entry.path == parent).map_or(0, |row| row.depth);
                        let icon_name = if matches!(kind, super::explorer_actions::EditKind::NewFolder) { IconName::FolderOpen } else { IconName::FileText };
                        let field = div().id(("tree-create-field", index)).relative().w_full().h(px(TREE_ROW_HEIGHT)).flex().items_center().gap(px(6.0))
                            .pl(px(42.0 + depth as f32 * 12.0)).pr(px(12.0))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(icon(icon_name, 13.0, theme.muted)).child(field.clone());
                        return if reduce_motion { field.into_any_element() } else {
                            field.with_animation(("tree-create-motion", index), Animation::new(TREE_SELECTION_DURATION).with_easing(tree_selection_ease),
                                |element, progress| element.top(px(-4.0 * (1.0 - progress))).opacity(progress))
                            .into_any_element()
                        };
                    }
                    let row_index = index - 1 - usize::from(insert_index.is_some_and(|insert| insert < index));
                    let row = rows[row_index].clone();
                    let matches_path = |path: Option<&Path>| path == Some(row.entry.path.as_path()) || path == row.entry.resolved_path.as_deref();
                    let active = active_path.is_some() && matches_path(active_path.as_deref());
                    let preview = preview_path.is_some() && matches_path(preview_path.as_deref());
                    let path = row.entry.path.clone();
                    let path_tip = super::explorer_actions::display_tree_path(&path);
                    let hover_path = path.clone();
                    let menu_path = path.clone();
                    let is_dir = row.entry.is_dir;
                    let expanded = row.expanded;
                    let can_expand = row.entry.has_children || expanded;
                    let chevron = is_dir.then(|| {
                        let indicator = if row.entry.has_children || expanded { Some(tree_chevron(expanded, Some(&path), tree_animation.as_ref(), theme.muted)) }
                            else { None };
                        div().w(px(16.0)).flex_none().children(indicator).into_any_element()
                    });
                    let kind = if is_dir { IconName::FolderOpen } else { IconName::FileText };
                    let file_icon = if row.loading { tree_spinner(("tree-spinner", index), theme.muted, reduce_motion) }
                        else { icon(kind, 13.0, if active { theme.accent } else { theme.muted }).into_any_element() };
                    let marker = active.then(|| tree_selection_marker(index, row.depth, selection_animation.as_ref(), theme.accent));
                    let controls = (is_dir && (hovered.as_ref() == Some(&path) || search_path.as_ref() == Some(&path)))
                        .then(|| tree_controls(&path, false, &theme, cx));
                    let editing = edit.as_ref().filter(|(kind, edit_path, _)| matches!(kind, super::explorer_actions::EditKind::Rename) && edit_path == &path)
                        .map(|(_, _, field)| field.clone());
                    let label = if let Some(field) = editing {
                        div().id(("tree-rename-field", index)).flex_1().min_w(px(0.0))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(|_, _, cx| cx.stop_propagation()).child(field).into_any_element()
                    } else {
                        div().flex_1().min_w(px(0.0)).overflow_hidden().when(preview, |el| el.italic().opacity(0.85))
                            .child(TreeLabel::new(row.entry.name.clone(), &search_query, theme.search_match)).into_any_element()
                    };
                    let button = div().id(("tree", index)).w_full().h(px(28.0)).flex().items_center().gap(px(6.0))
                        .pl(px((if is_dir { 12.0 } else { 34.0 }) + row.depth as f32 * 12.0)).pr(px(4.0))
                        .rounded(px(4.0)).cursor_pointer().text_color(if active { theme.accent } else { theme.fg })
                        .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .when(active || search_path.as_ref() == Some(&path), |el| el.bg(theme.sidebar_active))
                        .tooltip(tooltip(path_tip))
                        .hover(|style| style.bg(gpui::rgba(0x80808014)))
                        .children(chevron).child(file_icon)
                        .child(label)
                        .children(controls)
                        .on_hover(cx.listener(move |workspace, entered, _, cx| {
                            if *entered { workspace.explorer.hovered = Some(hover_path.clone()); }
                            else if workspace.explorer.hovered.as_ref() == Some(&hover_path) { workspace.explorer.hovered = None; }
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| {
                            workspace.open_tree_menu(menu_path.clone(), is_dir, false, event.position, cx);
                        }))
                        .on_click(cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            workspace.focus_tree_search(Some(path.clone()), window, cx);
                            if is_dir && event.click_count() >= 2 {
                                workspace.run_tree_action(super::explorer_actions::TreeAction::Rename, path.clone(), window, cx);
                            } else if is_dir && can_expand { workspace.toggle_tree_folder(&path, cx); }
                            else if is_dir { }
                            else { workspace.open_tree_file(path.clone(), event.click_count(), window, cx); }
                        }));
                    div().relative().w_full().h(px(TREE_ROW_HEIGHT)).px(px(8.0)).flex().items_center().child(button).children(marker).into_any_element()
                }).collect()
            })
        // GPUI 的 uniform_list 在内容尺寸和子项起点各计入一次 padding。
        }).track_scroll(&self.tree_scroll).pt(px(4.0)).pb(px(9.0)).min_h(px(0.0)).size_full();

        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let height = f32::from(handle.bounds().size.height).max(0.0);
        let content = count as f32 * TREE_ROW_HEIGHT + 26.0;
        let max_scroll = (content - height).max(0.0);
        let thumb_height = if content > 0.0 { (height * height / content).clamp(20.0_f32.min(height), height) } else { height };
        let thumb_top = if max_scroll > 0.0 { (-f32::from(handle.offset().y) / max_scroll).clamp(0.0, 1.0) * (height - thumb_height) } else { 0.0 };
        let scroll_animation = self.explorer.scroll_animation.filter(|(started, _, _, _)| started.elapsed() < Duration::from_millis(120));
        let scrollbar = (max_scroll > 0.0 && (self.explorer.scroll_visible || scroll_animation.is_some())).then(|| {
            let thumb = div().absolute().top(px(thumb_top)).left(px(2.0)).w(px(4.0)).h(px(thumb_height))
                .rounded_full().bg(theme.scrollbar);
            let thumb = if let Some((_, id, from, to)) = scroll_animation {
                thumb.with_animation(("tree-scrollbar-fade", id), Animation::new(Duration::from_millis(120)).with_easing(ease_in_out),
                    move |thumb, progress| thumb.opacity(from + (to - from) * progress)).into_any_element()
            } else { thumb.into_any_element() };
            div().id("tree-scrollbar").absolute().right(px(0.0)).top(px(0.0)).bottom(px(0.0)).w(px(8.0))
                .on_mouse_down(MouseButton::Left, cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let y = f32::from(event.position.y - workspace.tree_scroll.0.borrow().base_handle.bounds().top());
                    workspace.explorer.scroll_dragging = true;
                    workspace.explorer.scroll_drag_anchor = if y >= thumb_top && y <= thumb_top + thumb_height { y - thumb_top } else { thumb_height / 2.0 };
                    workspace.move_tree_scrollbar(y, cx);
                }))
                .child(thumb)
        });
        let menu = self.render_tree_menu(&theme, cx);
        let search_field = self.explorer.search_field.clone();
        let search_visible = self.explorer.search_visible || !self.explorer.search_query.is_empty();
        let search_count = if self.explorer.search_query.is_empty() { String::new() } else {
            format!("{}/{}", if self.explorer.search_matches.is_empty() { 0 } else { self.explorer.search_index + 1 }, self.explorer.search_matches.len())
        };
        let search = search_field.map(|field| div().id("tree-speed-search").absolute()
            .top(px(if search_visible { 8.0 } else { 14.0 })).left(px(if search_visible { 12.0 } else { 20.0 }))
            .w(px(if search_visible { 180.0 } else { 1.0 })).h(px(26.0)).flex().items_center()
            .when(!search_visible, |el| el.opacity(0.0))
            .when(search_visible, |el| el.rounded(px(6.0)).bg(theme.surface).border_1().border_color(theme.border))
            .when(search_visible, |el| el.child(icon(IconName::Search, 13.0, theme.muted)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(field).when(search_visible, |el| el.child(div().px(px(5.0)).text_size(px(11.0)).text_color(theme.muted).child(search_count))));
        let resizer = div().id("tree-resizer").absolute().right(px(-3.0)).top(px(0.0)).bottom(px(0.0)).w(px(6.0))
            .flex().justify_center().cursor_col_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|workspace, _, _, cx| {
                cx.stop_propagation();
                workspace.explorer.resizing = true;
                cx.notify();
            }));
        let tracking = (self.explorer.resizing || self.explorer.scroll_dragging).then(|| {
            let workspace = cx.entity().downgrade();
            div().absolute().size_full().child(gpui::canvas(|_, _, _| (), move |_, _, window, _| {
                let moving = workspace.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble { return; }
                    let _ = moving.update(cx, |workspace, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            workspace.explorer.resizing = false;
                            workspace.end_tree_scroll_drag(cx);
                            cx.notify();
                            return;
                        }
                        let bounds = workspace.tree_scroll.0.borrow().base_handle.bounds();
                        if workspace.explorer.resizing {
                            let max = (f32::from(window.viewport_size().width) - 420.0).clamp(180.0, 500.0);
                            workspace.explorer.sidebar_width = Some(f32::from(event.position.x - bounds.left()).clamp(180.0, max));
                            cx.notify();
                        }
                        if workspace.explorer.scroll_dragging {
                            workspace.move_tree_scrollbar(f32::from(event.position.y - bounds.top()), cx);
                        }
                    });
                });
                let released = workspace.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble && event.button == MouseButton::Left {
                        let _ = released.update(cx, |workspace, cx| {
                            workspace.explorer.resizing = false;
                            workspace.end_tree_scroll_drag(cx);
                            cx.notify();
                        });
                    }
                });
            }).size_full())
        });
        div().id("tree-viewport").relative().w(px(width)).h_full().min_h(px(0.0)).flex_none()
            .bg(theme.rail_bg).text_size(px(14.0))
            .on_key_down(cx.listener(|workspace, event: &gpui::KeyDownEvent, window, cx| {
                if workspace.explorer.search_field.as_ref().is_some_and(|field| field.focus_handle(cx).is_focused(window)) {
                    workspace.tree_search_key(event, window, cx);
                }
            }))
            .on_click(cx.listener(|workspace, _, window, cx| workspace.focus_tree_search(None, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|workspace, event: &gpui::MouseDownEvent, _, cx| {
                if let Some(root) = workspace.tree.as_ref().map(|tree| tree.root.clone()) {
                    workspace.open_tree_menu(root, true, true, event.position, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(|workspace, _, _, cx| workspace.show_tree_scrollbar(Duration::from_millis(900), cx)))
            .on_mouse_move(cx.listener(|workspace, event: &gpui::MouseMoveEvent, _, cx| {
                let bounds = workspace.tree_scroll.0.borrow().base_handle.bounds();
                if workspace.explorer.scroll_dragging {
                    workspace.move_tree_scrollbar(f32::from(event.position.y - bounds.top()), cx);
                } else {
                    let near = event.position.x >= bounds.right() - px(20.0);
                    if near != workspace.explorer.scroll_near {
                        workspace.explorer.scroll_near = near;
                        workspace.show_tree_scrollbar(if near { Duration::from_millis(900) } else { Duration::from_millis(240) }, cx);
                    }
                }
            }))
            .on_hover(cx.listener(|workspace, hovered: &bool, _, cx| {
                if !*hovered {
                    workspace.explorer.scroll_near = false;
                    if workspace.explorer.scroll_visible { workspace.show_tree_scrollbar(Duration::from_millis(240), cx); }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|workspace, _, _, cx| workspace.end_tree_scroll_drag(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|workspace, _, _, cx| workspace.end_tree_scroll_drag(cx)))
            .child(list).children(scrollbar).children(search).child(resizer).children(tracking).children(menu).into_any_element()
    }

    fn move_tree_scrollbar(&mut self, y: f32, cx: &mut Context<Self>) {
        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let height = f32::from(handle.bounds().size.height).max(0.0);
        let count = self.tree.as_ref().map_or(1, |tree| {
            let rows = tree.rows();
            rows.len() + 1 + usize::from(self.tree_create_insertion(&rows).is_some())
        });
        let content = count as f32 * TREE_ROW_HEIGHT + 26.0;
        let max_scroll = (content - height).max(0.0);
        if height <= 0.0 || max_scroll <= 0.0 { return; }
        let thumb = (height * height / content).clamp(20.0_f32.min(height), height);
        let track = (height - thumb).max(1.0);
        let fraction = ((y - self.explorer.scroll_drag_anchor) / track).clamp(0.0, 1.0);
        handle.set_offset(gpui::point(px(0.0), px(-fraction * max_scroll)));
        cx.notify();
    }

    fn end_tree_scroll_drag(&mut self, cx: &mut Context<Self>) {
        if self.explorer.scroll_dragging {
            self.explorer.scroll_dragging = false;
            self.show_tree_scrollbar(Duration::from_millis(900), cx);
        }
    }

    pub(super) fn toggle_tree_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(tree) = &mut self.tree {
            match tree.toggle(path) {
                Ok(expanded) => {
                    self.animate_tree_chevron(path.to_path_buf(), expanded);
                    self.sync_tree_selection(false, cx);
                    if expanded { self.queue_tree_scan(path.to_path_buf(), false, cx); }
                    else { self.cancel_hidden_tree_scan(); }
                }
                Err(error) => self.notice = Some(error.into()),
            }
            self.recompute_tree_search(cx);
            cx.notify();
        }
    }
}

fn tree_spinner(id: impl Into<gpui::ElementId>, color: gpui::Hsla, reduce_motion: bool) -> AnyElement {
    let spinner = div().id(id).w(px(16.0)).flex_none();
    if reduce_motion { return spinner.child(icon(IconName::RotateCcw, 12.0, color)).into_any_element(); }
    spinner.child(icon(IconName::RotateCcw, 12.0, color)
        .with_animation("tree-loading", Animation::new(Duration::from_millis(900)).repeat(), |element, progress| {
            element.with_transformation(Transformation::rotate(radians(progress * std::f32::consts::TAU)))
        })).into_any_element()
}

fn tree_controls(path: &Path, root: bool, theme: &Theme, cx: &mut Context<Workspace>) -> AnyElement {
    let actions: &[(IconName, &'static str, super::explorer_actions::TreeAction)] = if root {
        &[(IconName::FilePlus, "新建文件", super::explorer_actions::TreeAction::NewFile),
          (IconName::FolderPlus, "新建文件夹", super::explorer_actions::TreeAction::NewFolder),
          (IconName::RefreshCw, "刷新", super::explorer_actions::TreeAction::Refresh),
          (IconName::ChevronsUp, "全部折叠", super::explorer_actions::TreeAction::Collapse)]
    } else {
        &[(IconName::FilePlus, "新建文件", super::explorer_actions::TreeAction::NewFile),
          (IconName::FolderPlus, "新建文件夹", super::explorer_actions::TreeAction::NewFolder)]
    };
    let mut controls = div().flex().items_center().gap(px(1.0)).flex_none();
    for (index, (name, tip, action)) in actions.iter().enumerate() {
        let path = path.to_path_buf();
        let action = *action;
        controls = controls.child(div().id(("tree-control", index)).size(px(20.0)).flex_none().flex().items_center().justify_center()
            .rounded(px(5.0)).cursor_pointer().hover(|style| style.bg(theme.hover))
            .child(icon(*name, 12.0, theme.muted)).tooltip(tooltip(*tip))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |workspace, _, window, cx| {
                cx.stop_propagation();
                workspace.run_tree_action(action, path.clone(), window, cx);
            })));
    }
    controls.into_any_element()
}

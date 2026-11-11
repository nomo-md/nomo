//! 虚拟目录列表及其滚动条；根目录与普通行共享同一滚动范围。
use super::*;
use crate::ui::toolbar_i18n::ToolbarLocale;
#[path = "explorer_label.rs"]
mod label;
use label::TreeLabel;
use crate::ui::path_drag::{PathDrag, PathDragPreview};
use crate::ui::tree_drag;

impl Workspace {
    fn explorer_drag_rows(&self) -> Vec<tree_drag::Row> {
        let Some(tree) = &self.tree else { return Vec::new(); };
        let root: std::sync::Arc<Path> = tree.root.as_path().into();
        std::iter::once(tree_drag::Row { path: root.clone(), root: root.clone(), depth: 0, folder: true })
            .chain(tree.rows().iter().map(|row| tree_drag::Row { path: row.entry.path.as_path().into(), root: root.clone(), depth: row.depth, folder: row.entry.is_dir })).collect()
    }

    fn begin_explorer_drag(&mut self, item: &PathDrag) {
        if self.explorer.drag.active() { return; }
        if let Some(tree) = &self.tree {
            self.explorer.drag_rows = Some(tree.rows());
            let revision = std::sync::Arc::as_ptr(&tree.rows()) as *const () as usize;
            self.explorer.drag.begin(item, self.explorer_drag_rows(), revision, self.explorer.reduce_motion);
        }
    }

    pub(super) fn render_workspace_sidebar(
        &mut self,
        frame: motion::SidebarFrame,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(mini_frame) = self.mini.shell_frame() {
            if mini_frame.sidebar_width <= px(0.0) { return None; }
            let sidebar = self.render_explorer_sidebar(f32::from(mini_frame.sidebar_width), theme, cx);
            return Some(div().relative().h_full().w(mini_frame.sidebar_width * mini_frame.visibility).flex_none().overflow_hidden()
                .child(div().relative().h_full().w(mini_frame.sidebar_width).left(px(-12.0 * (1.0 - mini_frame.visibility)))
                    .opacity(mini_frame.visibility).child(sidebar))
                .child(div().absolute().inset_0().occlude()).into_any_element());
        }
        if frame.occupied_width() <= 0.0 { return None; }
        if self.explorer.drag.running() { window.request_animation_frame(); }
        let sidebar = self.render_explorer_sidebar(frame.full_width, theme, cx);
        Some(frame.clip(sidebar).into_any_element())
    }

    /// 文件栏 = 与主页相同的“主页”导航头 + 文件树视口；导航头不进入虚拟列表，滚动条和拖宽仍以列表边界计算。
    fn render_explorer_sidebar(&mut self, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        if let Some(edit) = &self.explorer.edit {
            let label = ToolbarLocale::current(cx).app_text("tree.name");
            edit.field.update(cx, |field, cx| field.set_placeholder(label, cx));
        }
        let tree = self.render_explorer_tree(width, theme, cx);
        let home = crate::ui::home::nav_home_row(false, theme, cx)
            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.show_home(window, cx)));
        div().w(px(width)).h_full().min_h(px(0.0)).flex_none().flex().flex_col().bg(theme.rail_bg)
            .child(div().w_full().flex_none().px(px(8.0)).pt(px(10.0)).pb(px(6.0)).child(home))
            .child(div().w_full().flex_1().min_h(px(0.0)).child(tree))
    }

    fn render_explorer_tree(&mut self, width: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.explorer.drag.active() && !self.explorer.drag.committing && !cx.has_active_drag() { self.explorer.drag.release(self.explorer.reduce_motion); }
        let (root_name, root_path, root_expanded, root_loading, rows) = match &self.tree {
            Some(tree) => (tree.root_name(), Some(tree.root.clone()), tree.root_expanded(), tree.loading(&tree.root), tree.rows()),
            None => return div().id("tree-viewport").relative().w(px(width)).h_full().min_h(px(0.0)).flex_none()
                .bg(theme.rail_bg).into_any_element(),
        };
        let active_path = self.tab().and_then(|tab| tab.document.read(cx).path());
        let revision = std::sync::Arc::as_ptr(&rows) as *const () as usize;
        if self.explorer.drag.needs_commit_check(revision) {
            let model = self.explorer_drag_rows();
            self.explorer.drag.check_commit(revision, || model, self.explorer.reduce_motion);
        }
        if !self.explorer.drag.active() { self.explorer.drag_rows = None; }
        let rows = self.explorer.drag_rows.clone().unwrap_or(rows);
        self.explorer.drag.begin_frame(self.explorer.reduce_motion);
        let drag_layout = self.explorer.drag.layout();
        let landing = self.explorer.drag.landing_index();
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
        let count = drag_layout.len(rows.len() + 1) + usize::from(insert_index.is_some());
        let row_hover = gpui::rgba(0x80808014);
        let theme = theme.clone();
        let list = uniform_list("file-tree", count, {
            let theme = theme.clone();
            cx.processor(move |workspace, range: std::ops::Range<usize>, window, cx| {
                let items = range.map(|display_index| {
                    let Some(index) = drag_layout.original(display_index).or(landing) else { return div().w_full().h(px(TREE_ROW_HEIGHT)).px(px(8.0)).child(workspace.explorer.drag.gap_row(&theme)).into_any_element(); };
                    if index == 0 {
                        let path = root_path.clone();
                        let hover_path = path.clone();
                        let root_hover = path.as_ref().is_some_and(|path| hovered.as_ref() == Some(path) || search_path.as_ref() == Some(path));
                        let chevron = if root_loading {
                            tree_spinner("tree-root-spinner", theme.accent, reduce_motion)
                        } else {
                            tree_chevron(root_expanded, path.as_deref(), tree_animation.as_ref(), theme.accent)
                        };
                        let controls = path.as_ref().filter(|_| root_hover).map(|path| tree_controls(path, true, &theme, cx));
                        let target = path.as_deref().is_some_and(|path| workspace.explorer.drag.is_target(path));
                        let mut root = div().id("tree-root").relative().top(px(-4.0)).mx(px(8.0)).w_full().h(px(28.0))
                            .pl(px(12.0)).pr(px(4.0)).flex().items_center().gap(px(6.0))
                            .rounded(px(4.0)).cursor_pointer().text_size(px(15.0)).text_color(theme.fg)
                            .font_weight(FontWeight::SEMIBOLD).when(!target, |el| el.hover(|style| style.bg(row_hover)))
                            .when(search_path.as_ref() == path.as_ref() && search_path.is_some(), |el| el.bg(theme.sidebar_active))
                            .when(target, |el| el.bg(theme.sidebar_active).rounded(px(9.0)))
                            .child(div().w(px(16.0)).flex_none().child(chevron))
                            .child(if target { tree_drag::receiving_icon(&theme) } else { icon(IconName::FolderOpen, 14.0, theme.accent).into_any_element() })
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
                                    match tree.toggle_root_message() {
                                        Ok(expanded) => {
                                            workspace.animate_tree_chevron(path.clone(), expanded);
                                            workspace.sync_tree_selection(false, cx);
                                            if expanded { workspace.queue_tree_scan(path, false, cx); }
                                            else { workspace.cancel_hidden_tree_scan(); }
                                        }
                                        Err(error) => workspace.notice = Some(error.render(ToolbarLocale::current(cx)).into()),
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
                    let hover_path = path.clone();
                    let menu_path = path.clone();
                    let is_dir = row.entry.is_dir;
                    let expanded = row.expanded;
                    let can_expand = row.entry.has_children || expanded;
                    let chevron = is_dir.then(|| {
                        let folder = path.clone();
                        // 保留固定的点击区域，覆盖收起后箭头消失的情况。
                        div().id("tree-chevron").w(px(16.0)).h_full().flex_none().flex().items_center()
                            .when(can_expand, |el| el.child(tree_chevron(expanded, Some(&path), tree_animation.as_ref(), theme.muted)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |workspace, _, window, cx| {
                                cx.stop_propagation();
                                workspace.focus_tree_search(Some(folder.clone()), window, cx);
                                if can_expand { workspace.toggle_tree_folder(&folder, cx); }
                            }))
                            .into_any_element()
                    });
                    let kind = if is_dir { IconName::FolderOpen } else { IconName::FileText };
                    let target = workspace.explorer.drag.is_target(&path);
                    let file_icon = if target { tree_drag::receiving_icon(&theme) } else if row.loading { tree_spinner(("tree-spinner", index), theme.muted, reduce_motion) }
                        else { icon(kind, 13.0, if active { theme.accent } else { theme.muted }).into_any_element() };
                    let marker = active.then(|| tree_selection_marker(display_index, row.depth, selection_animation.as_ref(), theme.accent));
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
                    let drag = PathDrag { path: path.clone(), root: root_path.clone().unwrap_or_default(), directory: is_dir, title: row.entry.name.clone().into(), visual: Default::default() };
                    let weak = cx.entity().downgrade();
                    let indent = 34.0 + row.depth as f32 * 12.0;
                    let animated_path = path.clone();
                    let button = div().id(("tree", index)).relative().w_full().h(px(28.0)).flex().items_center().gap(px(6.0))
                        .pl(px((if is_dir { 12.0 } else { 34.0 }) + row.depth as f32 * 12.0)).pr(px(4.0))
                        .rounded(px(4.0)).cursor_pointer().text_color(if active { theme.accent } else { theme.fg })
                        .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .when((active || search_path.as_ref() == Some(&path)) && !target, |el| el.bg(theme.sidebar_active))
                        .when(target, |el| el.child(div().absolute().left(px(row.depth as f32 * 12.0 + 3.0)).right(px(3.0)).top_0().bottom_0().rounded(px(9.0)).bg(theme.sidebar_active)))
                        .when(!target, |el| el.hover(|style| style.bg(row_hover)))
                        .children(chevron).child(file_icon)
                        .child(label)
                        .children(controls)
                        .when(edit.is_none() && workspace.explorer.mutating.is_empty() && !workspace.explorer.drag.committing, |el| el.on_drag(drag, move |item, offset, window, cx| {
                            *item.visual.lock().expect("drag geometry lock") = Some(crate::ui::path_drag::DragPose {
                                bounds: gpui::Bounds::new(window.mouse_position() - offset, gpui::size(px(width - 16.0), px(28.0))), lift: 0.0, row: true, settled: false });
                            let _ = weak.update(cx, |workspace, cx| { workspace.begin_explorer_drag(item); cx.notify(); });
                            let released = weak.clone();
                            cx.new(|cx| {
                                cx.on_release(move |_: &mut PathDragPreview, cx| {
                                    let _ = released.update(cx, |workspace, cx| {
                                        if !workspace.explorer.drag.committing { workspace.explorer.drag.release(workspace.explorer.reduce_motion); }
                                        cx.notify();
                                    });
                                }).detach();
                                PathDragPreview { item: item.clone(), document: None, document_motion: Default::default(), folder: None, folder_fx: None, size: None, row: Some((width - 16.0, indent)), row_icon: kind,
                                    lift: crate::ui::motion::Spring::new([0.0], 520.0, 24.0), last: Instant::now() }
                            })
                        }))
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
                    let row = div().relative().size_full().child(button)
                        .children(marker.map(|marker| div().absolute().left(px(-8.0)).top(px(-1.0)).size_full().child(marker))).into_any_element();
                    let row = workspace.explorer.drag.animate_row(&animated_path, display_index, row, reduce_motion, &theme);
                    div().w_full().h(px(TREE_ROW_HEIGHT)).px(px(8.0)).flex().items_center()
                        .child(div().w_full().h(px(28.0)).child(row)).into_any_element()
                }).collect();
                if workspace.explorer.drag.running() { window.request_animation_frame(); }
                items
            })
        // GPUI 的 uniform_list 在内容尺寸和子项起点各计入一次 padding。
        }).track_scroll(&self.tree_scroll).pt(px(4.0)).pb(px(9.0)).min_h(px(0.0)).size_full();

        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let backdrop = self.explorer.drag.backdrop(f32::from(handle.offset().y) + 8.0, 8.0, &theme);
        let feedback = self.explorer.drag.feedback(f32::from(handle.offset().y) + 8.0, 8.0, &theme);
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
                .cursor(gpui::CursorStyle::OpenHand)
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
            let dragging = self.explorer.scroll_dragging;
            div().absolute().size_full().child(gpui::canvas(|_, _, _| (), move |_, _, window, _| {
                if dragging { window.set_window_cursor_style(gpui::CursorStyle::ClosedHand); }
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
            .on_drag_move(cx.listener(|workspace, event: &gpui::DragMoveEvent<PathDrag>, _, cx| {
                if workspace.explorer.edit.is_some() || !workspace.explorer.mutating.is_empty() || workspace.explorer.drag.committing { return; }
                let item = event.drag(cx).clone();
                if event.bounds.contains(&event.event.position) {
                    workspace.begin_explorer_drag(&item);
                    let scroll = workspace.tree_scroll.0.borrow().base_handle.offset().y;
                    let y = f32::from(event.event.position.y - event.bounds.top() - scroll) - 8.0;
                    let x = f32::from(event.event.position.x - event.bounds.left()) - 8.0;
                    if workspace.explorer.drag.hover(x, y, workspace.explorer.reduce_motion) { cx.notify(); }
                } else if workspace.explorer.drag.leave(workspace.explorer.reduce_motion) { cx.notify(); }
            }))
            .on_drop(cx.listener(|workspace, item: &PathDrag, _, cx| {
                if workspace.explorer.edit.is_some() || !workspace.explorer.mutating.is_empty() || workspace.explorer.drag.committing { return; }
                if let Some(placement) = workspace.explorer.drag.placement() {
                    workspace.explorer.drag.commit(workspace.explorer.reduce_motion);
                    workspace.place_tree_entry(item.clone(), placement, cx); cx.notify();
                }
            }))
            .on_key_down(cx.listener(|workspace, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && cx.has_active_drag() {
                    workspace.explorer.drag.release(workspace.explorer.reduce_motion); cx.stop_active_drag(window); cx.notify(); return;
                }
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
            .child(backdrop).child(list).child(feedback).children(scrollbar).children(search).child(resizer).children(tracking).children(menu).into_any_element()
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
            match tree.toggle_message(path) {
                Ok(expanded) => {
                    self.animate_tree_chevron(path.to_path_buf(), expanded);
                    self.sync_tree_selection(false, cx);
                    if expanded { self.queue_tree_scan(path.to_path_buf(), false, cx); }
                    else { self.cancel_hidden_tree_scan(); }
                }
                Err(error) => self.notice = Some(error.render(ToolbarLocale::current(cx)).into()),
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
        &[(IconName::FilePlus, "tree.newFile", super::explorer_actions::TreeAction::NewFile),
          (IconName::FolderPlus, "tree.newFolder", super::explorer_actions::TreeAction::NewFolder),
          (IconName::RefreshCw, "tree.refresh", super::explorer_actions::TreeAction::Refresh),
          (IconName::ChevronsUp, "tree.collapseAll", super::explorer_actions::TreeAction::Collapse)]
    } else {
        &[(IconName::FilePlus, "tree.newFile", super::explorer_actions::TreeAction::NewFile),
          (IconName::FolderPlus, "tree.newFolder", super::explorer_actions::TreeAction::NewFolder)]
    };
    let mut controls = div().flex().items_center().gap(px(1.0)).flex_none();
    for (index, (name, tip, action)) in actions.iter().enumerate() {
        let path = path.to_path_buf();
        let action = *action;
        controls = controls.child(div().id(("tree-control", index)).size(px(20.0)).flex_none().flex().items_center().justify_center()
            .rounded(px(5.0)).cursor_pointer().hover(|style| style.bg(theme.hover))
            .child(icon(*name, 12.0, theme.muted)).tooltip(tooltip(ToolbarLocale::current(cx).app_text(tip)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |workspace, _, window, cx| {
                cx.stop_propagation();
                workspace.run_tree_action(action, path.clone(), window, cx);
            })));
    }
    controls.into_any_element()
}

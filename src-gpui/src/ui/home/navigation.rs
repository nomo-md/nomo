//! 主页分组入口、文件夹卡片和拖放反馈；文件夹点击直接打开工作区。
use super::*;

const REFLOW_ARRIVAL_DURATION: Duration = Duration::from_millis(150);

impl Home {
    pub(super) fn draggable(&self, element: gpui::Stateful<gpui::Div>, node: &Node, size: Option<(f32, f32)>, document: Option<document_card::DocumentCover>, row_width: f32, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let weak = cx.entity().downgrade();
        let row = size.is_none().then_some((row_width, 12.0));
        let folder = (size.is_some() && node.directory).then(|| self.folder_cover(node, cx));
        let folder_fx = self.card_motion.get(node.entry.path.as_path()).and_then(|state| state.folder.clone());
        let mut item = node.drag();
        if size.is_some() { if let Some(CachedPreview { preview: Preview::Ready { title, .. }, .. }) = self.previews.get(&node.entry.path) { item.title = title.clone(); } }
        element.when(!self.tree_drag.committing && !self.grid_drag.committing, |el| el.on_drag(item, move |item, offset, window, cx| {
            let dimensions = size.map(|(w, h)| (w, h + CARD_LIFT)).unwrap_or((row_width, 28.0));
            *item.visual.lock().expect("drag geometry lock") = Some(crate::ui::path_drag::DragPose {
                bounds: gpui::Bounds::new(window.mouse_position() - offset, gpui::size(px(dimensions.0), px(dimensions.1))), lift: 0.0, row: row.is_some(), settled: false });
            let document_motion = weak.update(cx, |home, cx| {
                home.dragging = Some(item.path.clone());
                if row.is_some() { home.begin_tree_drag(item, cx); } else { home.begin_grid_drag(item, cx); }
                home.set_card_pressed(item.path.as_path().into(), false, cx);
                home.set_card_hover(item.path.as_path().into(), false, cx);
                cx.notify();
                home.card_motion.get(item.path.as_path()).and_then(|state| state.document.clone())
            }).unwrap_or_default();
            let released = weak.clone();
            cx.new(|cx| {
                cx.on_release(move |preview: &mut PathDragPreview, cx| {
                    let _ = released.update(cx, |home, cx| {
                        if !home.tree_drag.committing && !home.grid_drag.committing {
                            home.land_grid(&preview.item, false);
                            home.release_drag(cx);
                        }
                        cx.notify();
                    });
                }).detach();
                PathDragPreview { item: item.clone(), document: document.clone(), document_motion, folder: folder.clone(), folder_fx: folder_fx.clone(), size, row,
                    row_icon: if item.directory { IconName::Folder } else { IconName::FileText },
                    lift: motion::Spring::new([0.0], 520.0, 24.0), last: Instant::now() }
            })
        }))
    }

    pub(crate) fn move_finished(&mut self, source: &Path, destination: &Path, error: Option<UiMessage>, cx: &mut Context<Self>) {
        if error.is_some() { self.tree_drag.end(reduced_motion(cx)); }
        self.browser.error = error;
        if let Some(parent) = source.parent() { self.queue_directory(parent.to_path_buf(), true, cx); }
        self.queue_directory(destination.to_path_buf(), true, cx);
        cx.notify();
    }

    fn tree_row(&self, node: &Node, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let path = node.entry.path.clone();
        let open = path.clone();
        let directory = node.directory;
        let title = if directory { entry_name(&path) } else { document_name(&path) };
        let target = self.tree_drag.is_target(&path);
        let remove_path = path.clone();
        let row = div().id((gpui::ElementId::Path(path.as_path().into()), "home-tree-row")).group("home-tree-row")
            .relative().w_full().h(px(28.0)).flex().items_center().gap(px(6.0))
            .px(px(12.0)).rounded(px(if target { 9.0 } else { 4.0 })).cursor_pointer()
            .text_size(px(14.0)).text_color(theme.fg).when(!target, |el| el.hover(|el| el.bg(theme.hover)))
            .when(target, |el| el.child(div().absolute().left(px(3.0)).right(px(3.0)).top(px(1.0)).bottom(px(1.0)).rounded(px(9.0)).bg(theme.sidebar_active)))
            .tooltip(tooltip(title.clone()))
            .child(if target { crate::ui::tree_drag::receiving_icon(theme) } else { icon(if directory { IconName::Folder } else { IconName::FileText }, 13.0, theme.muted).into_any_element() })
            .child(div().flex_1().min_w(px(0.0)).truncate().child(title))
            .child(small_icon_button((gpui::ElementId::Path(remove_path.as_path().into()), "home-remove"), IconName::Close, false, theme)
                .absolute().right(px(4.0)).top(px(3.0)).bg(theme.hover)
                .invisible().group_hover("home-tree-row", |el| el.visible())
                .tooltip(tooltip(ToolbarLocale::current(cx).app_text(if directory { "home.removeFolder" } else { "home.removeRecent" })))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |home, _, _, cx| {
                    cx.stop_propagation(); home.library.update(cx, |library, cx| library.change(Change::Remove { path: remove_path.clone(), folder: directory }, cx));
                })))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(open.clone()))));
        let row = self.draggable(row, node, None, None, width, cx);
        row.into_any_element()
    }

    fn folder_cover(&self, node: &Node, cx: &App) -> folder_card::FolderCover {
        let path = &node.entry.path;
        let locale = ToolbarLocale::current(cx);
        let summary = self.browser.entries(path).map(|entries| {
            if entries.is_empty() { locale.app_text("home.emptyFolder").to_owned() }
            else { locale.app_format("home.folderItems", &[("count", &entries.len().to_string())]) }
        }).unwrap_or_else(|| "…".to_owned());
        let document = self.browser.cover_document(path);
        let preview = document.and_then(|entry| self.previews.get(&entry.path)).map(|entry| &entry.preview);
        let document_title = match preview {
            Some(Preview::Ready { title, .. }) => title.clone(),
            _ => document.map(|entry| document_name(&entry.path)).unwrap_or_default(),
        };
        folder_card::FolderCover {
            title: entry_name(path), summary: summary.into(), document_title,
            image: match preview { Some(Preview::Ready { image, .. }) => Some(image.clone()), _ => None },
            error: match preview { Some(Preview::Failed(error)) => Some(error.render(locale).into()), _ => None },
            papers: self.browser.cover_papers(path), font_family: self.preview_theme.font_family.clone().into(),
        }
    }

    pub(super) fn folder_card(&mut self, node: &Node, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let path = node.entry.path.clone();
        let hover_path: Arc<Path> = path.as_path().into();
        let press_path = hover_path.clone();
        let release_path = hover_path.clone();
        let outside_path = hover_path.clone();
        let open = path.clone();
        let reduced = reduced_motion(cx);
        let state = self.card_motion.entry(hover_path.clone()).or_default();
        let fx = state.folder.get_or_insert_with(Default::default);
        let mut fx = fx.borrow_mut();
        fx.hover(state.hovered && self.dragging.is_none() && !reduced);
        let pose = fx.pose(self.grid_time, reduced);
        drop(fx);
        let height = card_height(width);
        let cover = self.folder_cover(node, cx);
        let placeholder = self.dragging.as_ref() == Some(&path) && !self.grid_drag.committing && !self.tree_drag.committing;
        let paper = div().absolute().left_0().top(px(CARD_LIFT))
            .child(cover.render(width, height, pose, theme)).when(placeholder, |el| el.opacity(0.0));
        let card = div().id((gpui::ElementId::Path(path.as_path().into()), "home-folder-card"))
            .relative().w(px(width)).h(px(height + CARD_LIFT)).flex_none().cursor_pointer()
            .tooltip(tooltip(cover.title)).when(placeholder, |el| el.bg(theme.hover).rounded(px(14.0))).child(paper)
            .on_hover(cx.listener(move |home, hovered, _, cx| home.set_card_hover(hover_path.clone(), *hovered, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(press_path.clone(), true, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(release_path.clone(), false, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(move |home, _, _, cx| home.set_card_pressed(outside_path.clone(), false, cx)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(open.clone()))));
        self.draggable(card, node, Some((width, height)), None, 0.0, cx).into_any_element()
    }

    pub(super) fn drag_rows(rows: &[Node]) -> Vec<crate::ui::tree_drag::Row> {
        rows.iter().map(|node| crate::ui::tree_drag::Row { path: node.entry.path.as_path().into(), root: node.root.clone(), depth: 0, folder: node.directory }).collect()
    }

    fn begin_tree_drag(&mut self, item: &PathDrag, cx: &App) {
        if !self.tree_drag.active() && self.browser.rows.iter().any(|node| node.entry.path == item.path) {
            self.dragging = Some(item.path.clone());
            self.tree_drag_rows = Some(self.browser.rows.clone());
            self.tree_drag.set_sections(self.browser.rows.iter().take_while(|node| node.directory).count());
            self.tree_drag.begin(item, Self::drag_rows(&self.browser.rows), self.browser.tree_revision as usize, reduced_motion(cx));
        }
    }

    pub(super) fn begin_grid_drag(&mut self, item: &PathDrag, cx: &App) {
        if !self.grid_drag.active() && self.browser.grid.iter().any(|node| node.entry.path == item.path) {
            self.dragging = Some(item.path.clone());
            self.grid_drag_rows = Some(self.browser.grid.clone());
            self.grid_drag.begin(item, Self::drag_rows(&self.browser.grid), self.browser.revision as usize, reduced_motion(cx));
        }
    }

    pub(super) fn animate_card(
        &mut self,
        path: &Path,
        index: usize,
        layout: GridLayout,
        row_height: f32,
        reflow: bool,
        child: gpui::AnyElement,
        reduced: bool,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let pitch = layout.card_width + GRID_GAP;
        let target = [(index % layout.columns) as f32 * pitch, (index / layout.columns) as f32 * row_height];
        let position = self.grid_positions.entry(path.to_path_buf()).or_default();
        position.frame = self.grid_frame;
        let can_animate_reflow = !reduced && self.dragging.is_none();
        position.reflow_started = position.reflow_started.filter(|started| {
            can_animate_reflow && self.grid_time.saturating_duration_since(*started) < REFLOW_ARRIVAL_DURATION
        });
        if can_animate_reflow && reflow && position.motion.borrow_mut().snap_distant_reflow(target, [pitch + 0.5, GRID_GAP]) {
            // 跨行或跨多列时在目标格入场，避免卡片沿对角线扫过网格。
            position.reflow_started.get_or_insert(self.grid_time);
        }
        let child = if let Some(started) = position.reflow_started {
            let elapsed = self.grid_time.saturating_duration_since(started);
            let progress = motion::ease(elapsed.as_secs_f32() / REFLOW_ARRIVAL_DURATION.as_secs_f32());
            div().relative().size_full().top(px(8.0 * (1.0 - progress)))
                .opacity(0.65 + 0.35 * progress).child(child).into_any_element()
        } else {
            child
        };
        ItemMotion::draw(position.motion.clone(), target, child, false, self.grid_time, reduced, theme)
    }

    pub(super) fn land_grid(&mut self, item: &PathDrag, landed: bool) {
        let mut visual = item.visual.lock().expect("drag geometry lock");
        if let Some(pose) = visual.as_mut().filter(|pose| !pose.row && !pose.settled) {
            pose.settled = true;
            let pose = *pose;
            drop(visual);
            let position = self.grid_positions.entry(item.path.clone()).or_default();
            position.frame = self.grid_frame;
            position.reflow_started = None;
            position.motion.borrow_mut().seed(pose, landed);
        }
    }

    pub(super) fn release_drag(&mut self, cx: &App) {
        if self.tree_drag.committing || self.grid_drag.committing { return; }
        if let Some(item) = self.grid_drag.item().cloned() { self.land_grid(&item, false); }
        self.tree_drag.release(reduced_motion(cx));
        self.grid_drag.end(reduced_motion(cx));
        self.dragging = None;
    }

    pub(super) fn sidebar(&mut self, width: f32, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let rows = self.tree_drag_rows.as_ref().unwrap_or(&self.browser.rows).clone();
        self.tree_drag.set_sections(rows.iter().take_while(|node| node.directory).count());
        self.tree_drag.begin_frame(reduced_motion(cx));
        let layout = self.tree_drag.layout();
        let landing = self.tree_drag.landing_index();
        let folders = self.tree_drag.folder_section_end().unwrap_or(0);
        let count = layout.len(rows.len()) + 2;
        let scroll = f32::from(self.sidebar_scroll.0.borrow().base_handle.offset().y);
        let backdrop = self.tree_drag.backdrop(scroll, 0.0, theme);
        let feedback = self.tree_drag.feedback(scroll, 0.0, theme);
        let theme = theme.clone();
        let list = uniform_list("home-sidebar-tree", count, cx.processor(move |home, range: std::ops::Range<usize>, window, cx| {
            let items = range.map(|index| {
                if index == 0 || index == folders + 1 {
                    let folder = index == 0;
                    let offset = if folder { 0.0 } else { home.tree_drag.section_offset(index, reduced_motion(cx)) };
                    return div().w_full().h(px(30.0)).child(div().relative().top(px(offset)).size_full().px(px(12.0)).flex().items_center().justify_between()
                        .text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(theme.muted)
                        .child(ToolbarLocale::current(cx).app_text(if folder { "home.folders" } else { "home.recentDocuments" })))
                        .into_any_element();
                }
                let logical = index - 1 - usize::from(index > folders + 1);
                let Some(original) = layout.original(logical).or(landing) else { return home.tree_drag.gap_row(&theme); };
                let node = &rows[original];
                let row = home.tree_row(node, width, &theme, cx);
                let row = home.tree_drag.animate_row(&node.entry.path, index, row, reduced_motion(cx), &theme);
                div().w_full().h(px(30.0)).flex().items_center().child(div().w_full().h(px(28.0)).child(row)).into_any_element()
            }).collect::<Vec<_>>();
            if home.tree_drag.running() { window.request_animation_frame(); }
            items
        })).w_full().h_full().track_scroll(&self.sidebar_scroll);
        div().id("home-tree-drop-area").relative().w(px(width)).h_full().overflow_hidden().child(backdrop).child(list).child(feedback)
            .on_drag_move(cx.listener(|home, event: &gpui::DragMoveEvent<PathDrag>, _, cx| {
                if home.tree_drag.committing || home.grid_drag.committing { return; }
                let item = event.drag(cx).clone();
                if event.bounds.contains(&event.event.position) {
                    let started = !home.tree_drag.active();
                    home.begin_tree_drag(&item, cx);
                    let scroll = home.sidebar_scroll.0.borrow().base_handle.offset().y;
                    let y = f32::from(event.event.position.y - event.bounds.top() - scroll);
                    let x = f32::from(event.event.position.x - event.bounds.left());
                    let changed = home.tree_drag.hover(x, y, reduced_motion(cx));
                    if changed || (started && home.tree_drag.active()) { cx.notify(); }
                } else if home.tree_drag.leave(reduced_motion(cx)) { cx.notify(); }
            }))
            .on_drop(cx.listener(|home, item: &PathDrag, _, cx| {
                if home.tree_drag.committing || home.grid_drag.committing { return; }
                if let Some(placement) = home.tree_drag.placement() {
                    home.grid_drag.end(reduced_motion(cx));
                    home.tree_drag.commit(reduced_motion(cx));
                    cx.emit(HomeEvent::Place { item: item.clone(), placement }); cx.notify();
                }
            })).into_any_element()
    }

    pub(crate) fn order_finished(&mut self, cx: &mut Context<Self>) {
        self.tree_drag.end(reduced_motion(cx)); self.grid_drag.end(reduced_motion(cx)); self.dragging = None; cx.notify();
    }

}

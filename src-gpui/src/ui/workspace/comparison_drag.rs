use super::*;
use gpui::{Pixels, Point, WindowHandle};

#[derive(Clone, Copy, PartialEq)]
pub(in crate::ui::workspace) enum Placement {
    Before(u64),
    After(u64),
    Combine(u64),
    End,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct DragPoint {
    screen: (i32, i32),
    root: usize,
}

// 鼠标捕获可能把松手事件交给源窗，也可能在切换前景后交给目标窗。
// 两者只记录同一次手势的落点，不持有窗口或文档。
#[cfg(windows)]
struct DragRelease(std::rc::Weak<Cell<Option<DragPoint>>>);
#[cfg(windows)]
impl gpui::Global for DragRelease {}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq)]
enum DragWindow {
    Source,
    Destination(WindowHandle<Workspace>),
    Blocked,
    Outside,
}

fn insert_item(
    items: &mut Vec<TabItem>,
    item: TabItem,
    placement: Placement,
    locale: ToolbarLocale,
) -> Result<TabItem, String> {
    let pos = match placement {
        Placement::End => items.len(),
        Placement::Before(k) | Placement::After(k) | Placement::Combine(k) => items
            .iter()
            .position(|i| i.contains(k))
            .ok_or(locale.app_text("drag.targetChanged"))?,
    };
    if let Placement::Combine(target) = placement {
        if items[pos] != TabItem::Single(target) || item.pair().is_some() || item.contains(target) {
            return Err(locale.app_text("drag.cannotCombine").into());
        }
        let group = TabItem::Pair(Pair {
            left: target,
            right: item.first(),
            focused: item.first(),
            ratio: 0.5,
        });
        items[pos] = group;
        return Ok(group);
    }
    items.insert(
        pos + usize::from(matches!(placement, Placement::After(_))),
        item,
    );
    Ok(item)
}

fn extract_item(items: &mut Vec<TabItem>, item: TabItem, member: Option<u64>) -> Option<TabItem> {
    let pos = items.iter().position(|i| *i == item)?;
    if let Some(key) = member {
        let pair = item.pair()?;
        if !item.contains(key) {
            return None;
        }
        items[pos] = TabItem::Single(if key == pair.left {
            pair.right
        } else {
            pair.left
        });
        Some(TabItem::Single(key))
    } else {
        Some(items.remove(pos))
    }
}

impl Workspace {
    pub(in crate::ui::workspace) fn drag_suppresses_click(&self) -> bool {
        self.comparison
            .suppress_click
            .is_some_and(|until| Instant::now() < until)
    }
    pub(in crate::ui::workspace) fn drop_marker(
        highlight: Option<Placement>,
        item: TabItem,
        index: usize,
        count: usize,
        theme: &Theme,
    ) -> Option<AnyElement> {
        let marker = div().absolute().top_0().bottom_0();
        match highlight? {
            Placement::Combine(key) if item.contains(key) => Some(
                marker
                    .left_0()
                    .right_0()
                    .rounded_t_md()
                    .border_2()
                    .border_color(theme.accent)
                    .bg(theme.accent.opacity(0.10))
                    .into_any_element(),
            ),
            Placement::Before(key) if item.contains(key) => Some(
                marker
                    .left_0()
                    .w(px(2.))
                    .bg(theme.accent)
                    .into_any_element(),
            ),
            Placement::After(key) if item.contains(key) => Some(
                marker
                    .right_0()
                    .w(px(2.))
                    .bg(theme.accent)
                    .into_any_element(),
            ),
            Placement::End if index + 1 == count => Some(
                marker
                    .right_0()
                    .w(px(2.))
                    .bg(theme.accent)
                    .into_any_element(),
            ),
            _ => None,
        }
    }
    pub(in crate::ui::workspace) fn drag_ghost(
        &self,
        theme: &Theme,
        cx: &App,
    ) -> Option<AnyElement> {
        if self.home_visible && self.comparison.highlight == Some(Placement::End) {
            return Some(
                gpui::deferred(
                    div()
                        .absolute()
                        .top(px(36.))
                        .left_0()
                        .right_0()
                        .h(px(40.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.accent)
                        .child(
                            crate::ui::toolbar_i18n::ToolbarLocale::current(cx)
                                .text("tab_drop_here"),
                        ),
                )
                .with_priority(7)
                .into_any_element(),
            );
        }
        let (position, title) = self.comparison.ghost.as_ref()?;
        Some(
            gpui::deferred(
                div()
                    .absolute()
                    .left(position.x + px(12.))
                    .top(position.y + px(12.))
                    .max_w(px(260.))
                    .truncate()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.accent)
                    .text_size(px(12.))
                    .child(title.clone()),
            )
            .with_priority(7)
            .into_any_element(),
        )
    }
    fn tab_drop_at(&self, point: Point<Pixels>, combine: bool, cx: &App) -> Option<Placement> {
        if self.closing_window || self.application_dialogs.busy || self.mini.active() {
            return None;
        }
        if self.home_visible {
            return (self.comparison.items.is_empty()
                && (36.0..76.0).contains(&f32::from(point.y)))
            .then_some(Placement::End);
        }
        if !self
            .comparison
            .rail
            .get()
            .is_some_and(|rail| rail.contains(&point))
        {
            return None;
        }
        let viewport = self.tab_scroll.bounds();
        let offset = self.tab_scroll.offset();
        let mut last_right = None;
        for (index, item) in self.comparison.items.iter().enumerate() {
            let mut rect = self.tab_scroll.bounds_for_item(index)?;
            rect.origin += offset;
            last_right = Some(rect.right().min(viewport.right()));
            if !viewport.contains(&point) || !rect.contains(&point) {
                continue;
            }
            let fraction = f32::from(point.x - rect.left()) / f32::from(rect.size.width).max(1.);
            let key = item.first();
            let eligible = item.pair().is_none() && self.markdown_tab(key, cx);
            return if eligible && (0.25..0.75).contains(&fraction) {
                combine.then_some(Placement::Combine(key))
            } else if fraction < 0.5 {
                Some(Placement::Before(key))
            } else {
                Some(Placement::After(key))
            };
        }
        // 只有最后一个可见标签右边的空白可以追加；测量缺失或没命中不能变成独立标签。
        last_right
            .filter(|right| point.x >= *right)
            .map(|_| Placement::End)
    }
    fn local_drop(
        &mut self,
        item: TabItem,
        member: Option<u64>,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_interaction_busy(window) || self.comparison.item(item.first()) != Some(item) {
            return;
        }
        if self.load_before(item, window, cx, move |w, win, cx| {
            w.local_drop(item, member, placement, win, cx)
        }) {
            return;
        }
        if let Placement::Combine(target) = placement {
            if !item.contains(target) {
                self.group_tabs(member.unwrap_or(item.first()), target, window, cx);
            }
            return;
        }
        if member.is_none()
            && matches!(placement,Placement::Before(k)|Placement::After(k) if item.contains(k))
        {
            return;
        }
        if !self.prepare_members(&item.keys(), false, window, cx, move |w, win, cx| {
            w.local_drop(item, member, placement, win, cx)
        }) {
            return;
        }
        let mut items = self.comparison.items.clone();
        let Some(moved) = extract_item(&mut items, item, member) else {
            return;
        };
        let placement = match (member, placement) {
            (Some(k), Placement::Before(t)) if k == t => {
                Placement::Before(item.keys().into_iter().find(|id| *id != k).unwrap())
            }
            (Some(k), Placement::After(t)) if k == t => {
                Placement::After(item.keys().into_iter().find(|id| *id != k).unwrap())
            }
            _ => placement,
        };
        if insert_item(&mut items, moved, placement, ToolbarLocale::current(cx)).is_err() {
            return;
        }
        self.comparison.items = items;
        if let Some(tab) = self.tab() {
            self.activate_key(tab.key, window, cx);
        }
        self.record_layout(cx);
        cx.notify();
    }
    #[cfg(not(windows))]
    pub(in crate::ui::workspace) fn arm_tab_drag(
        &mut self,
        _: TabItem,
        _: Option<u64>,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
    #[cfg(windows)]
    pub(in crate::ui::workspace) fn arm_tab_drag(
        &mut self,
        item: TabItem,
        member: Option<u64>,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .comparison
            .item(item.first())
            .filter(|live| live.same_members(item))
        else {
            return;
        };
        if self.tab_interaction_busy(window)
            || !item.keys().iter().all(|key| self.markdown_tab(*key, cx))
        {
            return;
        }
        let title = member
            .map(|k| vec![k])
            .unwrap_or_else(|| item.keys())
            .iter()
            .filter_map(|key| self.host_for(*key).map(|doc| doc.read(cx).display_title(cx)))
            .collect::<Vec<_>>()
            .join("｜");
        let release = Rc::new(Cell::new(None));
        cx.set_global(DragRelease(Rc::downgrade(&release)));
        self.comparison.drag = Some(cx.spawn_in(window, async move |this, cx| {
            let mut started = false;
            let mut previous = DragWindow::Source;
            let mut pending_up = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let released = release.get();
                let pressed = released.is_none() && pending_up.is_none() && native::pressed();
                let sample = released.or(pending_up).or_else(native::sample);
                let cancelled = native::escape() || sample.is_none();
                if !pressed && !cancelled && released.is_none() && pending_up.is_none() {
                    // 按键状态可先于 WM_LBUTTONUP 到达。留一帧收取真实释放点；
                    // 捕获丢失时用第一次观察到松手的位置，不追逐其后的鼠标移动。
                    pending_up = sample;
                    continue;
                }

                let mut raise = None;
                let done = this.update_in(cx, |workspace, win, cx| {
                    let cancelled = cancelled || workspace.tab_interaction_busy(win);
                    let here = sample.and_then(|p| native::client_point(win, p.screen));
                    let crossed_threshold = here.is_some_and(|p| {
                        f32::from(p.x - origin.x).hypot(f32::from(p.y - origin.y)) >= 6.
                    });
                    started |= !cancelled && (pressed || released.is_some()) && crossed_threshold;
                    let (surface, point, mut placement) = if started && !cancelled {
                        workspace.tab_drag_target(sample.unwrap(), here, member.is_some() || item.pair().is_none(), win, cx)
                    } else {
                        (DragWindow::Blocked, None, None)
                    };
                    if surface == DragWindow::Source
                        && matches!(placement, Some(Placement::Combine(key)) if item.contains(key))
                    {
                        placement = None;
                    }

                    if (previous != surface || !pressed || cancelled)
                        && let DragWindow::Destination(handle) = previous
                    {
                        let _ = handle.update(cx, |w, _, cx| w.set_drag_preview(None, None, &title, cx));
                    }
                    let preview = started && pressed && !cancelled;
                    if started {
                        workspace.comparison.suppress_click = Some(Instant::now() + Duration::from_millis(250));
                    }
                    if preview && surface == DragWindow::Source {
                        workspace.set_drag_preview(placement, point, &title, cx);
                    } else {
                        workspace.set_drag_preview(None, None, &title, cx);
                    }
                    if preview && let DragWindow::Destination(handle) = surface {
                        let _ = handle.update(cx, |w, _, cx| w.set_drag_preview(placement, point, &title, cx));
                    }
                    if preview && previous != surface && matches!(surface, DragWindow::Source | DragWindow::Destination(_)) {
                        raise = sample.map(|p| p.root);
                    }
                    previous = surface;

                    if !pressed || cancelled {
                        if started && !cancelled
                            && let Some(live) = workspace.comparison.item(item.first()).filter(|live| live.same_members(item))
                        {
                            match (surface, placement) {
                                (DragWindow::Source, Some(place)) => workspace.local_drop(live, member, place, win, cx),
                                (DragWindow::Destination(target), Some(place)) => workspace.transfer_item(live, member, Some(target), place, win, cx),
                                (DragWindow::Outside, None) => workspace.transfer_item(live, member, None, Placement::End, win, cx),
                                _ => {}
                            }
                        }
                        return true;
                    }
                    false
                }).unwrap_or(true);
                if done {
                    break;
                }
                if let Some(hwnd) = raise {
                    // 激活会同步派发窗口消息，放在 GPUI 释放实体和窗口借用之后。
                    native::raise_during_drag(hwnd);
                }
            }
        }));
    }

    #[cfg(windows)]
    pub(in crate::ui::workspace) fn remember_tab_release(
        event: &gpui::MouseUpEvent,
        win: &mut Window,
        cx: &mut App,
    ) {
        if event.button == MouseButton::Left
            && let Some(release) = cx
                .try_global::<DragRelease>()
                .and_then(|drag| drag.0.upgrade())
            && release.get().is_none()
        {
            release.set(native::release_point(win, event.position));
        }
    }

    #[cfg(windows)]
    fn set_drag_preview(
        &mut self,
        placement: Option<Placement>,
        point: Option<Point<Pixels>>,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let previous = self
            .comparison
            .ghost
            .as_ref()
            .map(|(p, name)| (*p, name.as_str()));
        if self.comparison.highlight != placement || previous != point.map(|p| (p, title)) {
            self.comparison.highlight = placement;
            self.comparison.ghost = point.map(|p| (p, title.to_owned()));
            cx.notify();
        }
    }

    #[cfg(windows)]
    fn tab_drag_target(
        &self,
        at: DragPoint,
        here: Option<Point<Pixels>>,
        combine: bool,
        win: &Window,
        cx: &mut Context<Self>,
    ) -> (DragWindow, Option<Point<Pixels>>, Option<Placement>) {
        if native::hwnd(win) == Some(at.root) {
            return (
                DragWindow::Source,
                here,
                here.and_then(|p| self.tab_drop_at(p, combine, cx)),
            );
        }
        for any in cx
            .windows()
            .into_iter()
            .filter(|w| *w != win.window_handle())
        {
            if let Some(handle) = any.downcast::<Workspace>() {
                let hit = handle.update(cx, |w, win, cx| {
                    if native::hwnd(win) != Some(at.root) {
                        return None;
                    }
                    if w.tab_interaction_busy(win)
                        || !w.opening.is_empty()
                        || w.comparison.layout.loading.is_some()
                    {
                        return Some((DragWindow::Blocked, None, None));
                    }
                    let point = native::client_point(win, at.screen);
                    Some((
                        DragWindow::Destination(handle),
                        point,
                        point.and_then(|p| w.tab_drop_at(p, combine, cx)),
                    ))
                });
                if let Ok(Some(hit)) = hit {
                    return hit;
                }
            } else if any
                .update(cx, |_, win, _| native::hwnd(win) == Some(at.root))
                .unwrap_or(false)
            {
                return (DragWindow::Blocked, None, None);
            }
        }
        (DragWindow::Outside, None, None)
    }

    pub(super) fn transfer_item(
        &mut self,
        item: TabItem,
        member: Option<u64>,
        target: Option<WindowHandle<Workspace>>,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cfg!(windows)
            || self.tab_interaction_busy(window)
            || self.comparison.item(item.first()) != Some(item)
            || !self.opening.is_empty()
            || self.comparison.layout.loading.is_some()
        {
            return;
        }
        if self.load_before(item, window, cx, move |w, win, cx| {
            w.transfer_item(item, member, target, placement, win, cx)
        }) {
            return;
        }
        let mut source_layout = self.layout_snapshot(cx);
        let source_pos = self.comparison.position(item.first()).unwrap();
        let Some(moving) = extract_item(&mut source_layout.items, item, member) else {
            return;
        };
        let keys = moving.keys();
        if !self.prepare_members(&keys, false, window, cx, move |w, win, cx| {
            w.transfer_item(item, member, target, placement, win, cx)
        }) {
            return;
        }
        if self.pending_saves.iter().any(|save| {
            keys.iter()
                .any(|key| self.host_for(*key).as_ref() == Some(&save.document))
        }) {
            self.failure_notice = Some(ToolbarLocale::current(cx).app_text("drag.waitSave").into());
            cx.notify();
            return;
        }
        let mut prepared = Vec::new();
        for key in &keys {
            let Some(doc) = self.host_for(*key) else {
                return;
            };
            let Some(saved) = source_layout.documents.remove(key) else {
                return;
            };
            let host = doc.read(cx);
            match host.transfer_adapter(saved.byte, cx) {
                Ok(adapter) => {
                    prepared.push((*key, host.port(), host.status().clone(), adapter, saved))
                }
                Err(error) => {
                    self.failure_notice = Some(error.to_string().into());
                    cx.notify();
                    return;
                }
            }
        }
        if source_layout.active.is_some_and(|key| moving.contains(key)) {
            source_layout.active = source_layout
                .items
                .get(source_pos.min(source_layout.items.len().saturating_sub(1)))
                .map(|i| i.focused());
        }
        let fresh = target.is_none();
        let target = match target
            .map(Ok)
            .unwrap_or_else(|| crate::try_open_workspace(Vec::new(), cx))
        {
            Ok(handle) => handle,
            Err(error) => {
                self.failure_notice = Some(error.into());
                cx.notify();
                return;
            }
        };
        #[cfg(windows)]
        if fresh && self.drag_suppresses_click() {
            let _ = target.update(cx, |_, win, _| native::position_new(win));
        }
        let manager = self.services.as_ref().unwrap().clone();
        let source_items = source_layout.items.clone();
        let source_active = source_layout.active;
        let source_handle = window.window_handle().downcast::<Workspace>().unwrap();
        let result = target
            .update(cx, |dest, win, cx| -> Result<bool, String> {
                if dest.tab_interaction_busy(win)
                    || !dest.opening.is_empty()
                    || dest.comparison.layout.loading.is_some()
                {
                    return Err(ToolbarLocale::current(cx).app_text("drag.busy").into());
                }
                let mut dest_layout = dest.layout_snapshot(cx);
                if prepared.iter().any(|(_, _, _, _, saved)| {
                    saved.path.as_ref().is_some_and(|p| {
                        dest_layout
                            .documents
                            .values()
                            .any(|other| other.path.as_ref() == Some(p))
                    })
                }) {
                    return Err(ToolbarLocale::current(cx).app_text("drag.duplicate").into());
                }
                let mut targets: Vec<_> = dest
                    .tabs
                    .iter()
                    .filter(|tab| tab.document.read(cx).is_visible())
                    .map(|tab| tab.key)
                    .collect();
                let inherited_folder = if fresh {
                    self.tree
                        .as_ref()
                        .map(|tree| tree.root.clone())
                        .or_else(|| {
                            prepared.iter().find_map(|(_, _, _, _, doc)| {
                                doc.path
                                    .as_deref()
                                    .and_then(Path::parent)
                                    .map(Path::to_path_buf)
                            })
                        })
                } else {
                    None
                };
                if let Some(folder) = &inherited_folder {
                    dest_layout.folder = Some(folder.clone());
                }
                if let Placement::Combine(key) = placement {
                    if dest.comparison.item(key) != Some(TabItem::Single(key))
                        || !dest.markdown_tab(key, cx)
                    {
                        return Err(ToolbarLocale::current(cx).app_text("drag.targetCombine").into());
                    }
                    if dest.load_before(TabItem::Single(key), win, cx, move |_, win, _| {
                        win.on_next_frame(move |_, cx| {
                            let _ = source_handle.update(cx, |w, win, cx| {
                                w.transfer_item(item, member, Some(target), placement, win, cx)
                            });
                        });
                    }) {
                        return Ok(false);
                    }
                    if !targets.contains(&key) {
                        targets.push(key);
                    }
                }
                if !dest.prepare_members(&targets, false, win, cx, move |_, win, _| {
                    win.on_next_frame(move |_, cx| {
                        let _ = source_handle.update(cx, |w, win, cx| {
                            w.transfer_item(item, member, Some(target), placement, win, cx)
                        });
                    });
                }) {
                    return Ok(false);
                }
                let dest_item = insert_item(&mut dest_layout.items, moving, placement, ToolbarLocale::current(cx))?;
                let mode = if let Placement::Combine(key) = placement {
                    Some(
                        if dest.tabs[dest.tab_index(key).unwrap()].mode == EditorMode::Source {
                            EditorMode::Source
                        } else {
                            EditorMode::Semantic
                        },
                    )
                } else {
                    None
                };
                if let Placement::Combine(key) = placement {
                    if let Some(doc) = dest_layout.documents.get_mut(&key) {
                        doc.mode = mode.unwrap();
                    }
                }
                let mut owners = Vec::new();
                let mut new_owners = Vec::new();
                for (key, port, _, _, saved) in &prepared {
                    let mut saved = saved.clone();
                    if let Some(mode) = mode {
                        saved.mode = mode;
                    }
                    let new_owner = format!("{}:handoff-{}", dest.owner, next_tab_key());
                    owners.push((
                        crate::text_document::markdown_session::SessionMember {
                            document_id: port.document_id.clone(),
                            owner: port.owner.clone(),
                            revision: Some(saved.revision),
                        },
                        new_owner.clone(),
                    ));
                    new_owners.push(new_owner);
                    dest_layout.documents.insert(*key, saved);
                }
                dest_layout.active = Some(dest_item.focused());
                let dest_items = dest_layout.items.clone();
                let previous_mode = if let Placement::Combine(key) = placement {
                    let before = dest.tabs[dest.tab_index(key).unwrap()].mode;
                    if !dest.set_member_modes(&[key], mode.unwrap(), cx) {
                        return Err(ToolbarLocale::current(cx).app_text("drag.mode").into());
                    }
                    Some((key, before))
                } else {
                    None
                };
                let result = manager.transfer_documents_with(owners, || {
                    Workspace::publish_layouts(vec![source_layout, dest_layout], cx)
                });
                if let Err(error) = result {
                    if let Some((key, old)) = previous_mode {
                        dest.set_member_modes(&[key], old, cx);
                    }
                    return Err(error);
                }
                if let Some(folder) = inherited_folder {
                    dest.replace_tree(folder, cx);
                }
                dest.comparison.items = dest_items;
                dest.comparison.layout.tracked = true;
                dest.claim_layout(cx);
                for ((key, mut port, status, adapter, saved), new_owner) in
                    prepared.into_iter().zip(new_owners)
                {
                    port.owner = new_owner;
                    let mode = mode.unwrap_or(saved.mode);
                    let id = status.document_id.clone();
                    if let Some(change) = self.application_dialogs.ignored.remove(&id) {
                        dest.application_dialogs.ignored.insert(id.clone(), change);
                    }
                    self.application_dialogs.mini_deferred.remove(&id);
                    let document = cx.new(|cx| {
                        DocumentHost::workspace(
                            port,
                            adapter,
                            status,
                            if mode == EditorMode::Source {
                                ViewMode::Source
                            } else {
                                ViewMode::Semantic
                            },
                            None,
                            cx,
                        )
                    });
                    let mut actual = mode;
                    if mode == EditorMode::Split
                        && let Err(error) =
                            document.update(cx, |host, cx| host.change_mode(mode, cx))
                    {
                        actual = EditorMode::Semantic;
                        dest.failure_notice = Some(error.to_string().into());
                    }
                    document.update(cx, |host, cx| {
                        host.restore_workspace_position(
                            saved.byte,
                            saved.fraction,
                            saved.selection,
                            cx,
                        )
                    });
                    let subscriptions = dest.subscribe_document(&document, win, cx);
                    dest.tabs.push(Tab {
                        key,
                        document: document.clone(),
                        mode: actual,
                        preview: false,
                        opened_at: Instant::now(),
                        dirty: saved.dirty,
                        dirty_animation_started: None,
                        hover: Default::default(),
                        close_hover: Default::default(),
                        active_color: Default::default(),
                        hidden_hover: Default::default(),
                        autosave: None,
                        _subscriptions: subscriptions,
                    });
                    if saved.dirty {
                        dest.schedule_autosave(document, win, cx);
                    }
                }
                if let Placement::Combine(key) = placement
                    && let Some(index) = dest.tab_index(key)
                {
                    dest.tabs[index].preview = false;
                }
                dest.activate_key(dest_item.focused(), win, cx);
                if !win.is_window_active() {
                    dest.reveal_window(win, cx);
                }
                cx.notify();
                Ok(true)
            })
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match result {
            Ok(true) => {
                self.comparison.items = source_items;
                self.comparison.layout.tracked = true;
                for key in keys {
                    if let Some(idx) = self.tab_index(key) {
                        let tab = self.tabs.remove(idx);
                        self.forget_document_transition(&tab.document);
                        if let Err(error) = tab
                            .document
                            .update(cx, |host, cx| host.release_closed_view(false, cx))
                        {
                            self.failure_notice = Some(error.to_string().into());
                        }
                    }
                }
                self.popover = None;
                self.search = None;
                self.close_status(cx);
                self.active = source_active.and_then(|k| self.tab_index(k)).unwrap_or(0);
                if self.tabs.is_empty() && self.comparison.items.is_empty() {
                    let queued = std::mem::take(&mut self.open_requests.pending);
                    let _ = target.update(cx, |w, win, cx| {
                        for r in queued {
                            w.enqueue_open(r.path, r.preview, r.anchor, r.choose_window, cx);
                        }
                        w.drain_open_requests(win, cx);
                    });
                    window.remove_window();
                } else if let Some(next) = source_active.and_then(|k| self.comparison.item(k)) {
                    self.activate_item(next, window, cx);
                }
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                if fresh {
                    let _ = target.update(cx, |w, win, _| {
                        if w.tabs.is_empty() {
                            win.remove_window();
                        }
                    });
                }
                self.failure_notice = Some(ToolbarLocale::current(cx).app_format("drag.failed", &[("error", &error)]).into());
                cx.notify();
            }
        }
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{ClientToScreen, ScreenToClient},
        UI::WindowsAndMessaging::*,
    };
    pub(super) fn hwnd(win: &Window) -> Option<usize> {
        match HasWindowHandle::window_handle(win).ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as usize),
            _ => None,
        }
    }
    fn hit(p: POINT) -> DragPoint {
        DragPoint {
            screen: (p.x, p.y),
            root: unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) } as usize,
        }
    }
    pub(super) fn sample() -> Option<DragPoint> {
        let mut p = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut p) } != 0).then(|| hit(p))
    }
    pub(super) fn release_point(win: &Window, point: Point<Pixels>) -> Option<DragPoint> {
        let mut p = POINT {
            x: (f32::from(point.x) * win.scale_factor()).round() as i32,
            y: (f32::from(point.y) * win.scale_factor()).round() as i32,
        };
        (unsafe { ClientToScreen(hwnd(win)? as _, &mut p) } != 0).then(|| hit(p))
    }
    pub(super) fn raise_during_drag(target: usize) {
        // 只在本次主动拖动、指针仍在该目标且 Nomo 已处于前台时切换，不合成输入。
        if !pressed() || escape() || !sample().is_some_and(|p| p.root == target) {
            return;
        }
        unsafe {
            let foreground = GetForegroundWindow();
            let mut process = 0;
            if !foreground.is_null() && foreground as usize != target {
                GetWindowThreadProcessId(foreground, &mut process);
                if process == std::process::id() {
                    SetForegroundWindow(target as _);
                }
            }
        }
    }
    pub(super) fn client_point(win: &Window, at: (i32, i32)) -> Option<Point<Pixels>> {
        let mut p = POINT { x: at.0, y: at.1 };
        if unsafe { ScreenToClient(hwnd(win)? as _, &mut p) } == 0 {
            return None;
        }
        Some(gpui::point(
            px(p.x as f32 / win.scale_factor()),
            px(p.y as f32 / win.scale_factor()),
        ))
    }
    pub(super) fn pressed() -> bool {
        unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
    }
    pub(super) fn escape() -> bool {
        unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0 }
    }
    pub(super) fn position_new(win: &Window) {
        if let (Some(hwnd), Some(DragPoint { screen: (x, y), .. })) = (hwnd(win), sample()) {
            unsafe {
                SetWindowPos(
                    hwnd as _,
                    std::ptr::null_mut(),
                    x - 70,
                    y - 15,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

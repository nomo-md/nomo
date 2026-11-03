//! 标签顺序独立于文档宿主。组合只引用两篇已有文档，命令仍由活动成员处理。
use super::*;
use serde::{Deserialize, Serialize};

#[path = "comparison_close.rs"]
mod close;
#[path = "comparison_drag.rs"]
mod drag;
#[path = "comparison_layout.rs"]
mod layout;
#[path = "comparison_ui.rs"]
mod ui;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Pair {
    pub left: u64,
    pub right: u64,
    pub focused: u64,
    pub ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum TabItem {
    Single(u64),
    Pair(Pair),
}

pub(super) fn next_tab_key() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl TabItem {
    fn same_members(self, other: Self) -> bool {
        match (self, other) {
            (Self::Single(a), Self::Single(b)) => a == b,
            (Self::Pair(a), Self::Pair(b)) => a.left == b.left && a.right == b.right,
            _ => false,
        }
    }
    pub(super) fn pair(self) -> Option<Pair> {
        match self {
            Self::Pair(pair) => Some(pair),
            _ => None,
        }
    }
    pub(super) fn focused(self) -> u64 {
        match self {
            Self::Single(key) => key,
            Self::Pair(pair) => pair.focused,
        }
    }
    pub(super) fn first(self) -> u64 {
        match self {
            Self::Single(key) => key,
            Self::Pair(pair) => pair.left,
        }
    }
    pub(super) fn contains(self, key: u64) -> bool {
        match self {
            Self::Single(id) => id == key,
            Self::Pair(pair) => pair.left == key || pair.right == key,
        }
    }
    pub(super) fn keys(self) -> Vec<u64> {
        match self {
            Self::Single(id) => vec![id],
            Self::Pair(pair) => vec![pair.left, pair.right],
        }
    }
    pub(super) fn focus(&mut self, key: u64) {
        if let Self::Pair(pair) = self {
            pair.focused = key;
        }
    }
}

pub(super) struct ComparisonState {
    pub(super) items: Vec<TabItem>,
    menu: Option<ui::TabMenu>,
    divider_focus: FocusHandle,
    pub(super) pane_focus: Option<ui::PaneFocus>,
    pub(super) resizing: bool,
    bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub(super) rail: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    drag: Option<Task<()>>,
    pub(super) highlight: Option<drag::Placement>,
    ghost: Option<(gpui::Point<gpui::Pixels>, String)>,
    suppress_click: Option<Instant>,
    pub(super) closing: Option<close::PairClose>,
    pub(super) batch: std::collections::VecDeque<u64>,
    layout: layout::LayoutState,
}

impl ComparisonState {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            items: Vec::new(),
            menu: None,
            divider_focus: cx.focus_handle(),
            pane_focus: None,
            resizing: false,
            bounds: Default::default(),
            rail: Default::default(),
            drag: None,
            highlight: None,
            ghost: None,
            suppress_click: None,
            closing: None,
            batch: Default::default(),
            layout: Default::default(),
        }
    }
    pub(super) fn position(&self, key: u64) -> Option<usize> {
        self.items.iter().position(|item| item.contains(key))
    }
    pub(super) fn item(&self, key: u64) -> Option<TabItem> {
        self.position(key).map(|pos| self.items[pos])
    }
    pub(super) fn item_mut(&mut self, key: u64) -> Option<&mut TabItem> {
        self.items.iter_mut().find(|item| item.contains(key))
    }
    pub(super) fn remove_member(&mut self, key: u64) {
        if let Some(pos) = self.position(key) {
            match self.items[pos] {
                TabItem::Single(_) => {
                    self.items.remove(pos);
                }
                TabItem::Pair(pair) => {
                    self.items[pos] = TabItem::Single(if pair.left == key {
                        pair.right
                    } else {
                        pair.left
                    })
                }
            }
        }
    }
}

impl Workspace {
    pub(super) fn active_pair(&self) -> Option<Pair> {
        self.tab()
            .and_then(|tab| self.comparison.item(tab.key))?
            .pair()
    }
    pub(super) fn tab_index(&self, key: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.key == key)
    }
    fn host_for(&self, key: u64) -> Option<Entity<DocumentHost>> {
        self.tab_index(key)
            .map(|index| self.tabs[index].document.clone())
    }
    fn markdown_tab(&self, key: u64, cx: &App) -> bool {
        self.host_for(key)
            .map(|doc| doc.read(cx).path().is_none_or(|path| markdown_path(&path)))
            .or_else(|| {
                self.comparison
                    .layout
                    .pending
                    .get(&key)
                    .map(|doc| doc.path.as_deref().is_none_or(markdown_path))
            })
            .unwrap_or(false)
    }
    fn can_combine(&self, source: u64, target: u64, cx: &App) -> bool {
        if source == target
            || self.comparison.item(target) != Some(TabItem::Single(target))
            || !self.markdown_tab(source, cx)
            || !self.markdown_tab(target, cx)
        {
            return false;
        }
        let source_path = self.host_for(source).and_then(|host| host.read(cx).path());
        source_path.is_none()
            || source_path != self.host_for(target).and_then(|host| host.read(cx).path())
    }
    fn tab_interaction_busy(&self, window: &Window) -> bool {
        window.has_active_prompt()
            || self.close_prompt
            || self.comparison.closing.is_some()
            || self.closing_window
            || self.application_dialogs.busy
            || self.mini.active()
    }
    fn prepare_members<F>(
        &mut self,
        keys: &[u64],
        closing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        resume: F,
    ) -> bool
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + Clone + 'static,
    {
        for key in keys {
            let Some(host) = self.host_for(*key) else {
                return false;
            };
            if !self.prepare_document_transition(&host, closing, window, cx, resume.clone()) {
                return false;
            }
        }
        true
    }
    pub(super) fn show_members(&mut self, keys: &[u64], cx: &mut Context<Self>) -> bool {
        let changes: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let show = keys.contains(&tab.key);
                (tab.document.read(cx).is_visible() != show).then(|| (tab.document.clone(), show))
            })
            .collect();
        for (doc, _) in &changes {
            if let Err(error) = doc.read(cx).ready(cx) {
                self.failure_notice = Some(error.to_string().into());
                cx.notify();
                return false;
            }
        }
        for (doc, show) in changes {
            if let Err(error) = doc.update(cx, |host, cx| host.set_active(show, cx)) {
                self.failure_notice = Some(error.to_string().into());
                cx.notify();
                return false;
            }
        }
        true
    }
    fn set_member_modes(&mut self, keys: &[u64], mode: EditorMode, cx: &mut Context<Self>) -> bool {
        let mut changed = Vec::new();
        for key in keys {
            let Some(index) = self.tab_index(*key) else {
                return false;
            };
            let old = self.tabs[index].mode;
            if old == mode {
                continue;
            }
            if let Err(error) = self.tabs[index]
                .document
                .update(cx, |host, cx| host.change_mode(mode, cx))
            {
                for (index, old) in changed.into_iter().rev() {
                    let tab: &mut Tab = &mut self.tabs[index];
                    if tab
                        .document
                        .update(cx, |host, cx| host.change_mode(old, cx))
                        .is_ok()
                    {
                        tab.mode = old;
                    }
                }
                self.failure_notice = Some(error.to_string().into());
                cx.notify();
                return false;
            }
            self.tabs[index].mode = mode;
            changed.push((index, old));
        }
        true
    }
    fn group_tabs(
        &mut self,
        source: u64,
        target: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cfg!(windows)
            || self.tab_interaction_busy(window)
            || !self.can_combine(source, target, cx)
        {
            return;
        }
        for key in [source, target] {
            let Some(item) = self.comparison.item(key) else {
                return;
            };
            if self.load_before(item, window, cx, move |w, win, cx| {
                w.group_tabs(source, target, win, cx)
            }) {
                return;
            }
        }
        let mut keys = vec![source, target];
        if let Some(tab) = self.tab() {
            if !keys.contains(&tab.key) {
                keys.push(tab.key);
            }
        }
        if !self.prepare_members(&keys, false, window, cx, move |w, win, cx| {
            w.group_tabs(source, target, win, cx)
        }) {
            return;
        }
        let mode = if self.tab().is_some_and(|tab| tab.mode == EditorMode::Source) {
            EditorMode::Source
        } else {
            EditorMode::Semantic
        };
        if !self.set_member_modes(&[source, target], mode, cx)
            || !self.show_members(&[source, target], cx)
        {
            return;
        }
        self.comparison.remove_member(source);
        let pos = self.comparison.position(target).expect("validated target");
        self.comparison.items[pos] = TabItem::Pair(Pair {
            left: target,
            right: source,
            focused: source,
            ratio: 0.5,
        });
        for tab in &mut self.tabs {
            if tab.key == source || tab.key == target {
                tab.preview = false;
            }
        }
        self.comparison.layout.tracked = true;
        self.activate_key(source, window, cx);
    }
    pub(super) fn change_pair_mode(
        &mut self,
        pair: Pair,
        mode: EditorMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys = [pair.left, pair.right];
        if !self.prepare_members(&keys, false, window, cx, move |w, win, cx| {
            if w.active_pair()
                .is_some_and(|p| p.left == pair.left && p.right == pair.right)
            {
                w.set_mode(mode, win, cx);
            }
        }) {
            return;
        }
        if self.set_member_modes(&keys, mode, cx) {
            Settings::set(
                cx,
                "editorMode",
                serde_json::json!(if mode == EditorMode::Source {
                    "source"
                } else {
                    "semantic"
                }),
            );
            self.popover = None;
            self.toolbar_popup = None;
            self.close_status(cx);
            self.record_layout(cx);
            if let Some(doc) = self.document() {
                window.focus(&doc.focus_handle(cx), cx);
            }
            cx.notify();
        }
    }
    fn split_or_swap(&mut self, key: u64, swap: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_interaction_busy(window) {
            return;
        }
        let Some(pos) = self.comparison.position(key) else {
            return;
        };
        let Some(mut pair) = self.comparison.items[pos].pair() else {
            return;
        };
        let loaded: Vec<_> = [pair.left, pair.right]
            .into_iter()
            .filter(|k| self.tab_index(*k).is_some())
            .collect();
        if !self.prepare_members(&loaded, false, window, cx, move |w, win, cx| {
            w.split_or_swap(key, swap, win, cx)
        }) {
            return;
        }
        if swap {
            std::mem::swap(&mut pair.left, &mut pair.right);
            self.comparison.items[pos] = TabItem::Pair(pair);
        } else {
            if self.active_pair().is_some_and(|p| p.contains_key(key))
                && !self.show_members(&[pair.focused], cx)
            {
                return;
            }
            self.comparison.items.splice(
                pos..=pos,
                [TabItem::Single(pair.left), TabItem::Single(pair.right)],
            );
        }
        self.record_layout(cx);
        cx.notify();
    }
    fn activate_key(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_index(key) {
            self.activate(index, window, cx);
        }
    }
    pub(super) fn cycle_item(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(pos) = self.comparison.position(tab.key) else {
            return;
        };
        let count = self.comparison.items.len();
        let item = self.comparison.items[(pos + if forward { 1 } else { count - 1 }) % count];
        self.activate_item(item, window, cx);
    }
    pub(super) fn activate_item(
        &mut self,
        item: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_interaction_busy(window) {
            return;
        }
        if self.load_saved_item(item, window, cx) {
            return;
        }
        self.activate_key(item.focused(), window, cx);
    }
}
fn markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

impl Pair {
    fn contains_key(self, key: u64) -> bool {
        self.left == key || self.right == key
    }
}

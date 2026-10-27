//! 目录树键盘快速搜索；复用支持输入法组字的单行输入控件。
use super::*;
use unicode_normalization::UnicodeNormalization;

impl Workspace {
    pub(super) fn init_tree_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = cx.new(|cx| { let mut field = TextField::new("", cx); field.set_tab_stop(); field });
        let focus = field.focus_handle(cx);
        let subscription = cx.subscribe_in(&field, window, |workspace, field, event, window, cx| {
            if field.read(cx).is_composing() { return; }
            match event {
                FieldEvent::Changed => workspace.recompute_tree_search(cx),
                FieldEvent::Submit { .. } => workspace.activate_tree_search(window, cx),
                FieldEvent::Cancel => workspace.clear_tree_search(cx),
            }
        });
        let blur = cx.on_blur(&focus, window, |workspace, _, cx| workspace.clear_tree_search(cx));
        let focus_subscription = cx.on_focus(&focus, window, |workspace, _, cx| {
            if workspace.explorer.search_focused.is_none() {
                workspace.explorer.search_focused = workspace.tree.as_ref().map(|tree| tree.root.clone());
            }
            workspace.explorer.search_keyboard_nav = true;
            cx.notify();
        });
        self.explorer.search_field = Some(field);
        self.explorer.search_subscription = Some(subscription);
        self.explorer.search_blur = Some(blur);
        self.explorer.search_focus = Some(focus_subscription);
    }

    pub(super) fn focus_tree_search(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_tree_search(cx);
        self.explorer.search_focused = path;
        if let Some(field) = &self.explorer.search_field { window.focus(&field.focus_handle(cx), cx); }
    }

    pub(super) fn clear_tree_search(&mut self, cx: &mut Context<Self>) {
        self.explorer.search_revision = self.explorer.search_revision.wrapping_add(1);
        self.explorer.search_task = None;
        self.explorer.search_query.clear();
        self.explorer.search_matches.clear();
        self.explorer.search_index = 0;
        self.explorer.search_visible = false;
        self.explorer.search_keyboard_nav = false;
        if let Some(field) = &self.explorer.search_field {
            if !field.read(cx).content.is_empty() { field.update(cx, |field, cx| field.set_text("", cx)); }
        }
        cx.notify();
    }

    pub(super) fn recompute_tree_search(&mut self, cx: &mut Context<Self>) {
        let Some(field) = &self.explorer.search_field else { return; };
        if field.read(cx).is_composing() { return; }
        let query: String = field.read(cx).content.to_lowercase().nfc().collect();
        self.explorer.search_query = query.clone();
        self.explorer.search_revision = self.explorer.search_revision.wrapping_add(1);
        let revision = self.explorer.search_revision;
        self.explorer.search_task = None;
        self.explorer.search_matches.clear();
        self.explorer.search_index = 0;
        if query.is_empty() { cx.notify(); return; }
        self.pending_tree_click.take();
        let Some(rows) = self.tree.as_ref().map(FileTree::rows) else { return; };
        let snapshot = rows.clone();
        let task = cx.background_executor().spawn(async move {
            snapshot.iter().enumerate().filter_map(|(index, row)| row.entry.search_name.contains(&query).then_some(index)).collect::<Vec<_>>()
        });
        self.explorer.search_task = Some(cx.spawn(async move |this, cx| {
            let matches = task.await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.explorer.search_revision != revision || !workspace.tree.as_ref().is_some_and(|tree| std::sync::Arc::ptr_eq(&tree.rows(), &rows)) { return; }
                workspace.explorer.search_matches = matches;
                workspace.reveal_tree_search_match();
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reveal_tree_search_match(&mut self) {
        let Some(row) = self.explorer.search_matches.get(self.explorer.search_index).copied() else { return; };
        let Some(tree) = &self.tree else { return; };
        let rows = tree.rows();
        let index = row + 1 + usize::from(self.tree_create_insertion(&rows).is_some_and(|insert| insert <= row + 1));
        self.tree_scroll.scroll_to_item(index, ScrollStrategy::Top);
        self.explorer.search_focused = rows.get(row).map(|row| row.entry.path.clone());
    }

    pub(super) fn tree_search_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &self.explorer.search_field else { return; };
        if field.read(cx).is_composing() { return; }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform { return; }
        match event.keystroke.key.as_str() {
            "tab" => {
                self.clear_tree_search(cx);
                if modifiers.shift { window.focus_prev(cx); } else { window.focus_next(cx); }
                cx.stop_propagation();
            }
            "f2" | "delete" if self.explorer.search_query.is_empty() => {
                if let Some(path) = self.explorer.search_focused.clone().filter(|path| self.tree.as_ref().is_some_and(|tree| tree.root != *path)) {
                    let action = if event.keystroke.key == "f2" { super::explorer_actions::TreeAction::Rename } else { super::explorer_actions::TreeAction::Delete };
                    self.run_tree_action(action, path, window, cx);
                    cx.stop_propagation();
                }
            }
            "left" | "right" if self.explorer.search_query.is_empty() => {
                if let Some(path) = self.explorer.search_focused.clone() {
                    let state = self.tree.as_ref().and_then(|tree| {
                        if tree.root == path { Some((true, tree.root_expanded())) }
                        else { tree.rows().iter().find(|row| row.entry.path == path && row.entry.is_dir)
                            .map(|row| (row.entry.has_children || row.expanded, row.expanded)) }
                    });
                    if let Some((true, expanded)) = state {
                        if (event.keystroke.key == "right" && !expanded) || (event.keystroke.key == "left" && expanded) {
                            self.toggle_tree_folder(&path, cx);
                        }
                        cx.stop_propagation();
                    }
                }
            }
            "up" | "down" if !self.explorer.search_query.is_empty() => {
                let count = self.explorer.search_matches.len();
                if count > 0 {
                    self.explorer.search_index = if event.keystroke.key == "down" { (self.explorer.search_index + 1) % count }
                        else { (self.explorer.search_index + count - 1) % count };
                    self.reveal_tree_search_match();
                }
                cx.stop_propagation();
                cx.notify();
            }
            "up" | "down" if self.explorer.search_query.is_empty() => {
                let Some(tree) = &self.tree else { return; };
                let rows = tree.rows();
                let current = self.explorer.search_focused.as_ref().and_then(|path| {
                    if *path == tree.root { Some(0) } else { rows.iter().position(|row| row.entry.path == *path).map(|index| index + 1) }
                }).unwrap_or(0);
                let index = if event.keystroke.key == "down" { (current + 1).min(rows.len()) } else { current.saturating_sub(1) };
                self.explorer.search_focused = if index == 0 { Some(tree.root.clone()) } else { Some(rows[index - 1].entry.path.clone()) };
                self.explorer.search_keyboard_nav = true;
                let index = index + usize::from(self.tree_create_insertion(&rows).is_some_and(|insert| insert <= index));
                self.tree_scroll.scroll_to_item(index, ScrollStrategy::Top);
                cx.stop_propagation();
                cx.notify();
            }
            key if key.chars().count() == 1 || key == "backspace" => {
                if !self.explorer.search_visible { self.explorer.search_visible = true; cx.notify(); }
            }
            _ => {},
        }
    }

    fn activate_tree_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = if self.explorer.search_query.is_empty() { self.explorer.search_focused.clone() }
            else { self.explorer.search_matches.get(self.explorer.search_index).and_then(|index| self.tree.as_ref()?.rows().get(*index).map(|row| row.entry.path.clone())) };
        let Some(path) = path else { return; };
        let directory = self.tree.as_ref().is_some_and(|tree| tree.root == path || tree.rows().iter().any(|row| row.entry.path == path && row.entry.is_dir));
        self.clear_tree_search(cx);
        if directory { self.toggle_tree_folder(&path, cx); }
        else { self.open_path_with_preview(&path, true, window, cx); }
    }
}

use super::*;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};

pub(in crate::ui::workspace) struct GroupClose {
    pair: Option<Pair>,
    keys: Vec<u64>,
    // 全部决定绑定对应修订。后续取消或保存失败时，已选择保留的草稿仍然打开。
    decisions: Vec<(u64, u64, CloseDisposition)>,
    saving: Option<u64>,
}

impl Workspace {
    pub(in crate::ui::workspace) fn close_item(
        &mut self,
        item: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comparison.item(item.first()) != Some(item) {
            return;
        }
        if self.load_to_close(item, window, cx) {
            return;
        }
        if let Some(index) = self.tab_index(item.focused()) {
            self.close_tab(index, window, cx);
        }
    }
    pub(in crate::ui::workspace) fn next_batch_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while let Some(key) = self.comparison.batch.pop_front() {
            if let Some(item) = self.comparison.item(key) {
                self.close_item(item, window, cx);
                return;
            }
        }
        self.finish_folder_open(window, cx);
    }
    pub(in crate::ui::workspace) fn close_pair(
        &mut self,
        pair: Pair,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comparison.closing.is_some() || window.has_active_prompt() {
            return;
        }
        if !self.prepare_members(
            &[pair.left, pair.right],
            true,
            window,
            cx,
            move |w, win, cx| {
                if w.comparison.item(pair.left) == Some(TabItem::Pair(pair)) {
                    w.close_pair(pair, win, cx);
                }
            },
        ) {
            return;
        }
        self.comparison.closing = Some(GroupClose {
            pair: Some(pair),
            keys: vec![pair.left, pair.right],
            decisions: Vec::new(),
            saving: None,
        });
        self.advance_group_close(window, cx);
    }

    pub(in crate::ui::workspace) fn begin_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_drafts = self.tabs.iter().any(|tab| {
            let doc = tab.document.read(cx);
            doc.path().is_none() && doc.is_dirty(cx) && !doc.has_failed_input(cx)
        });
        if !has_drafts || crate::quit_clears_drafts(cx) {
            self.close_window_normally(window, cx);
            return;
        }
        if cx.global::<Settings>().bool("keepUntitledDraftsOnClose") {
            self.close_window_keeping_drafts(window, cx);
            return;
        }
        use dialogs::{SecondaryAction as A, SecondaryDialog, SecondaryKind, Tone};
        self.close_prompt = true;
        let response = dialogs::choose(SecondaryDialog {
            kind: SecondaryKind::Recovery,
            title: UiMessage::new("draftClose.title"),
            message: UiMessage::new("draftClose.message"),
            actions: vec![
                A::new(UiMessage::new("draftClose.keep"), IconName::FileText, Tone::Primary),
                A::new(UiMessage::new("draftClose.review"), IconName::Save, Tone::Normal),
                A::new(UiMessage::toolbar("cancel"), IconName::Close, Tone::Quiet),
            ],
            cancel: 2, detail: None, notes: None, remember: None, leading: Vec::new(),
        }, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let choice = response.await;
            let _ = this.update_in(cx, |workspace, window, cx| {
                workspace.close_prompt = false;
                match choice.index {
                    0 => workspace.close_window_keeping_drafts(window, cx),
                    1 => workspace.close_window_normally(window, cx),
                    _ => workspace.cancel_close(cx),
                }
            });
        }).detach();
    }

    fn close_window_normally(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_layout(cx);
        self.closing_window = true;
        self.close_tab(self.active, window, cx);
    }

    fn close_window_keeping_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<_> = self.tabs.iter().map(|tab| tab.key).collect();
        if !self.prepare_members(&keys, true, window, cx, Self::close_window_keeping_drafts) { return; }
        self.closing_window = true;
        self.comparison.closing = Some(GroupClose { pair: None, keys, decisions: Vec::new(), saving: None });
        self.advance_group_close(window, cx);
    }

    fn advance_group_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(close) = &self.comparison.closing else {
            return;
        };
        let pair = close.pair;
        if pair.is_some_and(|pair| self.comparison.item(pair.left).and_then(TabItem::pair)
            .is_none_or(|p| p.left != pair.left || p.right != pair.right)) {
            self.cancel_close(cx);
            return;
        }
        while let Some(&key) = self.comparison.closing.as_ref().and_then(|c| c.keys.get(c.decisions.len())) {
            let Some(doc) = self.host_for(key) else {
                self.cancel_close(cx);
                return;
            };
            let revision = doc.read(cx).status().revision;
            let host = doc.read(cx);
            let automatic = if !host.is_dirty(cx) { Some(CloseDisposition::Clean) }
                else if pair.is_none() && host.path().is_none() && !host.has_failed_input(cx) { Some(CloseDisposition::KeepDraft) }
                else { None };
            if let Some(decision) = automatic {
                self.comparison.closing.as_mut().unwrap().decisions.push((key, revision, decision));
                continue;
            }
            self.close_prompt = true;
            let response = dialogs::confirm_document_close(doc.read(cx).title_message(), doc.read(cx).has_failed_input(cx), window, cx);
            cx.spawn_in(window, async move |this, cx| {
                let choice = response.await;
                let _ = this.update_in(cx, |w, win, cx| {
                    w.close_prompt = false;
                    if w.comparison
                        .closing
                        .as_ref()
                        .is_none_or(|close| close.pair != pair)
                    {
                        return;
                    }
                    match choice {
                        SaveChoice::Cancel => w.cancel_close(cx),
                        SaveChoice::Discard => {
                            w.comparison
                                .closing
                                .as_mut()
                                .unwrap()
                                .decisions
                                .push((key, revision, CloseDisposition::Discard));
                            w.advance_group_close(win, cx);
                        }
                        SaveChoice::Save => {
                            w.comparison.closing.as_mut().unwrap().saving = Some(key);
                            w.save_document(&doc, false, true, win, cx);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let close = self.comparison.closing.as_ref().unwrap();
        if pair.is_none() && (!self.open_requests.pending.is_empty() || !self.opening.is_empty()) {
            self.cancel_close(cx);
            self.notice = Some(ToolbarLocale::current(cx).app_text("workspace.openingClose").into());
            self.drain_open_requests(window, cx);
            cx.notify();
            return;
        }
        if pair.is_none() && close.keys.len() != self.tabs.len() {
            self.cancel_close(cx);
            self.failure_notice = Some(ToolbarLocale::current(cx).app_text("draftClose.changed").into());
            cx.notify();
            return;
        }
        let mut documents = Vec::new();
        for &(key, revision, decision) in &close.decisions {
            let Some(doc) = self.host_for(key) else {
                self.cancel_close(cx);
                return;
            };
            let host = doc.read(cx);
            if host.status().revision != revision
                || host.ready_for_close(decision == CloseDisposition::Discard, cx).is_err()
                || (decision == CloseDisposition::Clean && host.is_dirty(cx))
            {
                let message = if pair.is_some() { "comparison.changed" } else { "draftClose.changed" };
                self.failure_notice = Some(ToolbarLocale::current(cx).app_text(message).into());
                self.cancel_close(cx);
                cx.notify();
                return;
            }
            let port = host.port();
            documents.push((
                crate::text_document::markdown_session::SessionMember {
                    document_id: port.document_id,
                    owner: port.owner,
                    revision: Some(revision),
                },
                decision,
            ));
        }
        let Some(manager) = &self.services else {
            self.cancel_close(cx);
            return;
        };
        let layout = pair.is_none().then(|| self.layout_for_close(&close.decisions, manager.process_directory(), cx));
        let kept = match manager.close_documents(documents, || {
            if let Some(layout) = layout { Self::publish_layouts(vec![layout], cx)?; }
            Ok(())
        }) {
            Ok(kept) => kept,
            Err(error) => {
                self.failure_notice = Some(error.to_string().into());
                self.cancel_close(cx);
                cx.notify();
                return;
            }
        };
        Self::register_kept_drafts(kept, cx);
        let close = self.comparison.closing.take().unwrap();
        let pos = pair.and_then(|p| self.comparison.position(p.left));
        let previous = self.tabs.get(self.active).map(|tab| tab.key);
        for (key, _, decision) in close.decisions {
            if let Some(index) = self.tab_index(key) {
                let tab = self.tabs.remove(index);
                let discard = decision == CloseDisposition::Discard;
                if pair.is_some() { self.note_closed_layout(key, &tab.document, discard, cx); }
                self.cancel_export_for(&tab.document);
                if let Err(error) = tab
                    .document
                    .update(cx, |host, cx| host.release_closed_view(discard, cx))
                {
                    self.failure_notice = Some(error.to_string().into());
                }
                application_dialogs::clear_external_after_save(self, &tab.document, cx);
                self.forget_document_transition(&tab.document);
                self.pending_saves
                    .retain(|pending| pending.document != tab.document);
            }
        }
        let Some(pos) = pos else {
            self.comparison.items.clear();
            self.active = 0;
            self.after_tabs_removed(window, cx);
            return;
        };
        self.comparison.items.remove(pos);
        let next = previous
            .filter(|k| self.tab_index(*k).is_some())
            .or_else(|| {
                self.comparison
                    .items
                    .get(pos.min(self.comparison.items.len().saturating_sub(1)))
                    .map(|item| item.focused())
            });
        self.active = next.and_then(|key| self.tab_index(key)).unwrap_or(0);
        self.popover = None;
        self.record_layout(cx);
        if !self.closing_window
            && !self.folder_open_pending()
            && self.comparison.batch.is_empty()
            && let Some(item) = next.and_then(|key| self.comparison.item(key))
            && self.load_saved_item(item, window, cx)
        {
            return;
        }
        self.after_tabs_removed(window, cx);
    }
    pub(in crate::ui::workspace) fn continue_group_save(
        &mut self,
        doc: &Entity<DocumentHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .tabs
            .iter()
            .find(|tab| tab.document == *doc)
            .map(|tab| tab.key)
        else {
            return;
        };
        let Some(close) = self.comparison.closing.as_mut() else {
            return;
        };
        if close.saving != Some(key) {
            return;
        }
        if doc.read(cx).is_dirty(cx) {
            let message = if close.pair.is_some() { "comparison.saveChanged" } else { "workspace.saveChanged" };
            self.cancel_close(cx);
            self.failure_notice = Some(ToolbarLocale::current(cx).app_text(message).into());
            cx.notify();
            return;
        }
        close.saving = None;
        close
            .decisions
            .push((key, doc.read(cx).status().revision, CloseDisposition::Clean));
        self.advance_group_close(window, cx);
    }
}

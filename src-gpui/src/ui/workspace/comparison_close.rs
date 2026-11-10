use super::*;
use crate::ui::toolbar_i18n::ToolbarLocale;

pub(in crate::ui::workspace) struct PairClose {
    pair: Pair,
    // 每次用户确认都绑定对应修订，另一栏等待期间的输入不能被旧决定丢弃。
    decisions: Vec<(u64, u64, bool)>,
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
        self.comparison.closing = Some(PairClose {
            pair,
            decisions: Vec::new(),
            saving: None,
        });
        self.advance_pair_close(window, cx);
    }
    fn advance_pair_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(close) = &self.comparison.closing else {
            return;
        };
        let pair = close.pair;
        if self
            .comparison
            .item(pair.left)
            .and_then(TabItem::pair)
            .is_none_or(|p| p.left != pair.left || p.right != pair.right)
        {
            self.cancel_close(cx);
            return;
        }
        if let Some(&key) = [pair.left, pair.right].get(close.decisions.len()) {
            let Some(doc) = self.host_for(key) else {
                self.cancel_close(cx);
                return;
            };
            let revision = doc.read(cx).status().revision;
            if !doc.read(cx).is_dirty(cx) {
                self.comparison
                    .closing
                    .as_mut()
                    .unwrap()
                    .decisions
                    .push((key, revision, false));
                self.advance_pair_close(window, cx);
                return;
            }
            self.close_prompt = true;
            let response = dialogs::confirm_save(doc.read(cx).title_message(), window, cx);
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
                                .push((key, revision, true));
                            w.advance_pair_close(win, cx);
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
        let mut documents = Vec::new();
        for &(key, revision, discard) in &close.decisions {
            let Some(doc) = self.host_for(key) else {
                self.cancel_close(cx);
                return;
            };
            let host = doc.read(cx);
            if host.status().revision != revision
                || host.ready(cx).is_err()
                || (!discard && host.is_dirty(cx))
            {
                self.failure_notice =
                    Some(ToolbarLocale::current(cx).app_text("comparison.changed").into());
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
                discard,
            ));
        }
        let Some(manager) = &self.services else {
            self.cancel_close(cx);
            return;
        };
        if let Err(error) = manager.close_documents(documents) {
            self.failure_notice = Some(error.to_string().into());
            self.cancel_close(cx);
            cx.notify();
            return;
        }
        let close = self.comparison.closing.take().unwrap();
        let pos = self.comparison.position(pair.left).unwrap();
        let previous = self.tabs.get(self.active).map(|tab| tab.key);
        for (key, _, discard) in close.decisions {
            if let Some(index) = self.tab_index(key) {
                let tab = self.tabs.remove(index);
                self.note_closed_layout(key, &tab.document, discard, cx);
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
    pub(in crate::ui::workspace) fn continue_pair_save(
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
            self.cancel_close(cx);
            self.failure_notice = Some(ToolbarLocale::current(cx).app_text("comparison.saveChanged").into());
            cx.notify();
            return;
        }
        close.saving = None;
        close
            .decisions
            .push((key, doc.read(cx).status().revision, false));
        self.advance_pair_close(window, cx);
    }
}

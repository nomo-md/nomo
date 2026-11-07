//! 宿主只保存有界诊断和任务句柄；源码读取、解析均在后台固定修订中完成。
use super::*;
use crate::text_document::markdown_lint::{Issue, Report, Request, RuleSet};
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub(crate) struct LintState {
    pub(crate) enabled: bool,
    pub(crate) rule_set: RuleSet,
    pub(crate) pending: bool,
    pub(crate) report: Option<Arc<Report>>,
    pub(crate) error: Option<String>,
    request: Request,
    task: Option<gpui::Task<()>>,
    signature: Option<(u64, RuleSet)>,
    serial: u64,
}
impl LintState {
    pub(super) fn cancel(&mut self) {
        self.request.cancel();
        self.task = None;
        self.pending = false;
        self.signature = None;
        self.report = None;
        self.error = None;
        self.serial = self.serial.wrapping_add(1);
    }
}
impl Drop for LintState {
    fn drop(&mut self) {
        self.request.cancel();
    }
}

impl DocumentHost {
    pub(crate) fn lint(&self) -> &LintState {
        &self.queries.lint
    }
    pub(crate) fn sync_lint(&mut self, cx: &mut Context<Self>) {
        self.schedule_lint(false, cx);
    }
    pub(crate) fn retry_lint(&mut self, cx: &mut Context<Self>) {
        self.queries.lint.signature = None;
        self.schedule_lint(true, cx);
    }
    fn schedule_lint(&mut self, force: bool, cx: &mut Context<Self>) {
        let settings = cx.global::<crate::settings::Settings>();
        let enabled = settings.bool("markdownLintEnabled");
        let rules = RuleSet::from_setting(&settings.string("markdownLintRuleSet"));
        let revision = self.status().revision;
        if !enabled || self.hidden || self.closed {
            let state = &mut self.queries.lint;
            let changed = state.enabled != enabled || state.signature.is_some();
            state.cancel();
            state.enabled = enabled;
            state.rule_set = rules;
            if changed {
                cx.emit(DocumentEvent::Status);
                cx.notify();
            }
            return;
        }
        let state = &mut self.queries.lint;
        if !force && state.signature == Some((revision, rules)) {
            return;
        }
        let immediate =
            force || !state.enabled || state.rule_set != rules || state.signature.is_none();
        state.cancel();
        state.enabled = true;
        state.rule_set = rules;
        state.pending = true;
        state.signature = Some((revision, rules));
        state.request = Request::default();
        let request = state.request.clone();
        let serial = state.serial;
        let port = self.port.clone();
        state.task = Some(cx.spawn(async move |this, cx| {
            if !immediate {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
            if request.check().is_err() {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    port.manager
                        .markdown_lint(&port.owner, &port.document_id, revision, rules, &request)
                        .await
                })
                .await;
            let _ = this.update(cx, |host, cx| {
                if host.closed
                    || host.hidden
                    || host.queries.lint.serial != serial
                    || host.status().revision != revision
                {
                    return;
                }
                let settings = cx.global::<crate::settings::Settings>();
                if !settings.bool("markdownLintEnabled")
                    || rules != RuleSet::from_setting(&settings.string("markdownLintRuleSet"))
                {
                    host.sync_lint(cx);
                    return;
                }
                let state = &mut host.queries.lint;
                state.pending = false;
                match result {
                    Ok(report) => state.report = Some(Arc::new(report)),
                    Err(error)
                        if matches!(
                            error.code,
                            "lint-cancelled"
                                | "revision-conflict"
                                | "document-closed"
                            | "document-owner-conflict"
                        ) =>
                    {
                        state.signature = None;
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        }));
        cx.emit(DocumentEvent::Status);
        cx.notify();
    }
    pub(crate) fn reveal_lint_issue(
        &mut self,
        report: &Arc<Report>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let settings = cx.global::<crate::settings::Settings>();
        if !settings.bool("markdownLintEnabled") || report.rule_set != RuleSet::from_setting(&settings.string("markdownLintRuleSet")) { return false; }
        if self.hidden
            || self.closed
            || report.document_id != self.port.document_id
            || report.revision != self.status().revision
            || !self
                .queries
                .lint
                .report
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, report))
            || self.ready(cx).is_err()
        {
            return false;
        }
        let Some(Issue { range, .. }) = report.issues.get(index) else {
            return false;
        };
        let selection = DocumentSelection {
            anchor_byte: range.start,
            head_byte: range.end,
        };
        if !self.ready_for_session_action(cx) {
            return false;
        }
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let revision = report.revision;
        let byte = range.start;
        let task = cx
            .background_executor()
            .spawn(async move { port.load_group(revision, byte, None, &request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if host.closed || host.hidden || host.status().revision != revision {
                    return;
                }
                match result.and_then(|loaded| host.install(loaded, Some(selection), cx)) {
                    Ok(()) => {
                        host.focus_pending = true;
                        // 原文标记在语义模式可能隐藏；既有光标映射恢复到可见位置。
                        host.control
                            .update(cx, |view, cx| view.defer_navigation_caret(cx));
                    }
                    Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        true
    }
}

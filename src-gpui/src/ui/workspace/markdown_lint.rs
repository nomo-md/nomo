//! 状态栏的只读诊断浮层；只消费宿主的有界结果，不持有正文。
use super::*;
use crate::text_document::markdown_lint::{RULES, Report, RuleSet, SkipReason};
use std::sync::{Arc, OnceLock};

pub(super) fn text(locale: ToolbarLocale, key: &str) -> &'static str {
    static VALUES: OnceLock<serde_json::Value> = OnceLock::new();
    let values = VALUES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../resources/markdown-lint-locales.json"
        ))
        .expect("bundled Markdown lint translations")
    });
    values
        .get(locale.code())
        .and_then(|language| language.get(key))
        .and_then(|value| value.as_str())
        .or_else(|| {
            values
                .get("en-US")
                .and_then(|language| language.get(key))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("Markdown")
}

impl Workspace {
    pub(super) fn lint_label(&self, cx: &App) -> String {
        let locale = ToolbarLocale::current(cx);
        let Some(document) = self.document() else {
            return text(locale, "title").into();
        };
        let state = document.read(cx).lint();
        if state.pending {
            text(locale, "checking").into()
        } else if state.error.is_some() {
            text(locale, "failed").into()
        } else if let Some(report) = &state.report {
            if report.skipped.is_some() {
                text(locale, "skipped").into()
            } else if report.total > 0 {
                text(locale, "count").replace("{count}", &report.total.to_string())
            } else {
                text(locale, "partial").into()
            }
        } else {
            text(locale, "pending").into()
        }
    }
    fn select_lint_issue(&mut self, report: Arc<Report>, index: usize, cx: &mut Context<Self>) {
        let selected = self.document().is_some_and(|document| {
            document.update(cx, |host, cx| host.reveal_lint_issue(&report, index, cx))
        });
        if selected {
            self.close_status(cx);
        } else {
            self.status_ui.lint_hint = true;
        }
        cx.notify();
    }
    pub(super) fn render_lint_popup(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let locale = ToolbarLocale::current(cx);
        let document = self.document()?;
        let state = document.read(cx).lint();
        let (pending, error, report, rules) = (
            state.pending,
            state.error.clone(),
            state.report.clone(),
            state.rule_set,
        );
        let keyboard_report = report.clone();
        let mut panel = div()
            .id("markdown-lint-popup")
            .track_focus(&self.status_ui.focus)
            .role(gpui::Role::Dialog)
            .aria_label(text(locale, "title"))
            .w(px(420.0))
            .max_h(px(520.0))
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .text_color(theme.fg)
            .text_size(px(13.0))
            .shadow(vec![shadow(theme, 0.0, true)])
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(
                cx.listener(|workspace, event: &gpui::MouseDownEvent, _, cx| {
                    if !workspace.status_ui.anchors[2]
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        workspace.close_status(cx);
                        cx.notify();
                    }
                }),
            )
            .on_key_down(
                cx.listener(move |workspace, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            workspace.close_status(cx);
                            window.focus(&workspace.status_ui.triggers[2], cx);
                        }
                        "up" | "down" => {
                            if let Some(report) = &keyboard_report {
                                let count = report.issues.len();
                                if count > 0 {
                                    workspace.status_ui.selected =
                                        (workspace.status_ui.selected.min(count - 1)
                                            + if event.keystroke.key == "up" {
                                                count - 1
                                            } else {
                                                1
                                            })
                                            % count;
                                    workspace.status_ui.keyboard_selection = true;
                                }
                            }
                        }
                        "enter" if workspace.status_ui.focus.is_focused(window) => {
                            if let Some(report) = &keyboard_report {
                                workspace.select_lint_issue(
                                    report.clone(),
                                    workspace.status_ui.selected,
                                    cx,
                                );
                            }
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text(locale, "title")),
                    )
                    .child(
                        div()
                            .id("markdown-lint-retry")
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .cursor_pointer()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_color(theme.accent)
                            .child(text(locale, "retry"))
                            .focus_visible(|style| style.bg(theme.accent.opacity(0.1)))
                            .on_click(cx.listener(|workspace, _, _, cx| {
                                workspace.status_ui.lint_hint = false;
                                if let Some(document) = workspace.document() {
                                    document.update(cx, |host, cx| host.retry_lint(cx));
                                }
                                cx.notify();
                            }))
                            .on_key_down(cx.listener(|workspace, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    workspace.status_ui.lint_hint = false;
                                    if let Some(document) = workspace.document() {
                                        document.update(cx, |host, cx| host.retry_lint(cx));
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            })),
                    ),
            );
        let mode = text(
            locale,
            if rules == RuleSet::Relaxed {
                "relaxed"
            } else {
                "default"
            },
        );
        let coverage = text(locale, "coverage")
            .replace("{supported}", &rules.supported_count().to_string())
            .replace(
                "{total}",
                &RULES
                    .iter()
                    .filter(|rule| rules.enabled(**rule))
                    .count()
                    .to_string(),
            );
        panel = panel.child(
            div()
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .flex_col()
                .gap(px(5.0))
                .text_color(theme.muted)
                .child(mode)
                .child(coverage)
                .child(text(locale, "boundary")),
        );
        if self.status_ui.lint_hint {
            panel = panel.child(
                div()
                    .px(px(12.0))
                    .pb(px(8.0))
                    .text_color(theme.accent)
                    .child(text(locale, "stale")),
            );
        }
        if pending {
            panel = panel.child(div().p(px(12.0)).child(text(locale, "checking")));
        } else if let Some(error) = error {
            panel = panel.child(
                div()
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(text(locale, "failed"))
                    .child(error),
            );
        } else if let Some(report) = report {
            let seconds = report.checked_at.elapsed().unwrap_or_default().as_secs();
            panel = panel.child(
                div()
                    .px(px(12.0))
                    .pb(px(8.0))
                    .text_color(theme.muted)
                    .child(text(locale, "checked").replace("{seconds}", &seconds.to_string())),
            );
            if let Some(reason) = report.skipped {
                let key = match reason {
                    SkipReason::SourceBudget => "source_budget",
                    SkipReason::LineBudget => "line_budget",
                    SkipReason::SyntaxBudget => "syntax_budget",
                    SkipReason::InlineConfiguration => "inline_configuration",
                };
                panel = panel.child(div().p(px(12.0)).child(text(locale, key)));
            } else {
                if report.total == 0 {
                    panel =
                        panel.child(div().p(px(12.0)).child(text(locale, "no_supported_issues")));
                }
                for (index, issue) in report.issues.iter().enumerate() {
                    let code = format!("MD{:03}", issue.rule);
                    let position = text(locale, "position")
                        .replace("{line}", &issue.line.to_string())
                        .replace("{column}", &issue.column.to_string());
                    let title = format!("{code} · {}", text(locale, &code));
                    let selected =
                        self.status_ui.keyboard_selection && self.status_ui.selected == index;
                    let click_report = report.clone();
                    let key_report = report.clone();
                    panel = panel.child(
                        div()
                            .id(("markdown-lint-issue", index))
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .aria_label(format!("{title}, {position}"))
                            .cursor_pointer()
                            .mx(px(6.0))
                            .mb(px(4.0))
                            .px(px(8.0))
                            .py(px(7.0))
                            .rounded(px(4.0))
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .border_1()
                            .border_color(theme.border.opacity(0.6))
                            .when(selected, |element| element.bg(theme.accent.opacity(0.1)))
                            .hover(|style| style.bg(theme.accent.opacity(0.07)))
                            .focus_visible(|style| style.bg(theme.accent.opacity(0.1)))
                            .child(div().font_weight(FontWeight::MEDIUM).child(title))
                            .child(div().text_color(theme.muted).child(position))
                            .when(!issue.detail.is_empty(), |element| {
                                element.child(
                                    div().text_color(theme.muted).child(issue.detail.clone()),
                                )
                            })
                            .on_click(cx.listener(move |workspace, _, _, cx| {
                                workspace.select_lint_issue(click_report.clone(), index, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |workspace, event: &KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        workspace.select_lint_issue(key_report.clone(), index, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            )),
                    );
                }
                panel = panel.child(
                    div().p(px(12.0)).text_color(theme.muted).child(
                        text(locale, "shown")
                            .replace("{shown}", &report.issues.len().to_string())
                            .replace("{total}", &report.total.to_string()),
                    ),
                );
            }
        }
        let anchor =
            PopupAnchor::Tracked(self.status_ui.anchors[2].clone(), point(px(16.0), px(14.0)));
        Some(
            gpui::deferred(status_popup(
                ("markdown-lint-open", self.status_ui.generation),
                anchor,
                panel,
                self.explorer.reduce_motion,
            ))
            .with_priority(6)
            .into_any_element(),
        )
    }
}

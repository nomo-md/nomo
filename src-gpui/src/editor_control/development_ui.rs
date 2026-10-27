//! 独立开发入口的调试外壳；正式 Workspace 不呈现这些操作。
use super::*;
impl DocumentHost {
    pub(super) fn render_development(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = cx.entity().downgrade();
        if self.focus_pending { self.focus_pending = false; window.focus(&self.control.focus_handle(cx)); }
        if self.production {
            let title = self.port.status().ok().and_then(|s| s.path).and_then(|p| std::path::PathBuf::from(p).file_name().map(|n|n.to_string_lossy().into_owned())).unwrap_or_else(|| "未命名".into());
            window.set_window_title(&format!("{title} — Nomo 原生编辑"));
        }
        let theme = cx.global::<Theme>().clone();
        let control = self.control.clone();
        let mut toolbar = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme.border);
        if self.production {
            for (index, (label, forward)) in [("上一窗口", false), ("下一窗口", true)]
                .into_iter()
                .enumerate()
            {
                toolbar = toolbar.child(
                    div()
                        .id(("adjacent", index))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |v, _, _, cx| v.adjacent(forward, false, cx))),
                );
            }
            toolbar = toolbar.child(
                div()
                    .id("open-real-file")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("打开")
                    .on_click(cx.listener(|v, _, _, cx| v.choose_file(cx))),
            );
            toolbar = toolbar.child(
                div()
                    .id("reload-window")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("重新定位")
                    .on_click(cx.listener(|v, _, _, cx| {
                        let byte = v.control.read(cx).host.selection.head_byte;
                        v.load_at(byte, None, cx);
                    })),
            );
        }
        for (index, (name, format)) in [
            ("粗体", InlineFormat::Strong),
            ("斜体", InlineFormat::Emphasis),
            ("代码", InlineFormat::Code),
            ("删除线", InlineFormat::Strike),
            ("下划线", InlineFormat::Underline),
            ("高亮", InlineFormat::Highlight),
            ("上标", InlineFormat::Superscript),
            ("下标", InlineFormat::Subscript),
        ]
        .into_iter()
        .enumerate()
        {
            let control = control.clone();
            toolbar = toolbar.child(
                div()
                    .id(("format", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .hover(|s| s.bg(theme.hover))
                    .cursor_pointer()
                    .child(name)
                    .on_click(move |_, window, cx| {
                        control.update(cx, |view, cx| {
                            view.command(EditIntent::ToggleInline(format), cx)
                        });
                        window.focus(&control.focus_handle(cx));
                    }),
            );
        }
        for (index, (name, mode)) in [("语义", ViewMode::Semantic), ("源码", ViewMode::Source)]
            .into_iter()
            .enumerate()
        {
            toolbar = toolbar.child(
                div()
                    .id(("mode", index))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child(name)
                    .on_click(cx.listener(move |host, _, window, cx| {
                        if let Err(error)=host.control.update(cx, |view,cx|view.set_mode(mode,cx)) { host.message=error.to_string(); return; }
                        for mount in &host.panes[host.active_pane].viewport.mounts {
                            if mount.view != host.control { let _=mount.view.update(cx, |view,cx|view.set_mode(mode,cx)); }
                        }
                        window.focus(&host.control.focus_handle(cx));
                    })),
            );
        }
        toolbar = toolbar
            .child(
                div()
                    .id("hide")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child(if self.hidden {
                        "恢复窗口"
                    } else {
                        "隐藏窗口"
                    })
                    .on_click(cx.listener(|v, _, _, cx| v.toggle_hidden(cx))),
            )
            .child(
                div()
                    .id("save")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("保存")
                    .on_click(cx.listener(|v, _, window, cx| v.save(false, window, cx))),
            )
            .child(
                div()
                    .id("insert")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("插入 / 格式")
                    .on_click(cx.listener(|v, _, window, cx| {
                        v.control.update(cx, |v, cx| v.open_menu(cx));
                        window.focus(&v.control.focus_handle(cx));
                    })),
            )
            .child(
                div()
                    .id("undo")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("撤销")
                    .on_click(cx.listener(|v, _, _, cx| v.history(false, cx))),
            )
            .child(
                div()
                    .id("redo")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("重做")
                    .on_click(cx.listener(|v, _, _, cx| v.history(true, cx))),
            );
        if let Some((row, column)) = control.read(cx).active_cell() {
            use crate::editor_control::table::{Alignment, TableIntent};
            for (index, (label, intent)) in [
                ("加行", TableIntent::InsertRow { after: row }),
                ("删行", TableIntent::DeleteRow { row }),
                ("加列", TableIntent::InsertColumn { at: column + 1 }),
                ("删列", TableIntent::DeleteColumn { column }),
                (
                    "居中",
                    TableIntent::Align {
                        column,
                        alignment: Alignment::Center,
                    },
                ),
                (
                    "左对齐",
                    TableIntent::Align {
                        column,
                        alignment: Alignment::Left,
                    },
                ),
                (
                    "右对齐",
                    TableIntent::Align {
                        column,
                        alignment: Alignment::Right,
                    },
                ),
                ("显示表头", TableIntent::Header { enabled: true }),
                ("取消表头", TableIntent::Header { enabled: false }),
            ]
            .into_iter()
            .enumerate()
            {
                let control = control.clone();
                toolbar = toolbar.child(
                    div()
                        .id(("table", index))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |_, window, cx| {
                            control.update(cx, |v, cx| v.table_command(intent, cx));
                            window.focus(&control.focus_handle(cx));
                        }),
                );
            }
            toolbar = toolbar.child(
                div()
                    .id("table-size")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("行列数量")
                    .on_click(cx.listener(|host, _, window, cx| {
                        host.control
                            .update(cx, |view, cx| view.edit_table_size(window, cx))
                    })),
            );
        }
        if control.read(cx).host.failure().is_some() {
            toolbar = toolbar.child(
                div()
                    .id("retry")
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child("重试提交")
                    .on_click(cx.listener(|host, _, _, cx| {
                        if let Err(error) = host.control.update(cx, |view, cx| view.retry(cx)) {
                            host.message = error.to_string();
                        }
                        cx.notify();
                    })),
            );
        }
        let bytes = control.read(cx).estimated_bytes();
        let revision = control.read(cx).host.parsed.identity.revision;
        let result = div()
            .key_context("Workspace")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .text_color(theme.fg)
            .text_size(px(14.0))
            .child(toolbar)
            .on_action(cx.listener(|host, _: &crate::ui::workspace::ModeSemantic, _, cx| {
                if let Err(error)=host.change_mode(crate::settings::EditorMode::Semantic,cx){host.report(error,cx);}
            }))
            .on_action(cx.listener(|host, _: &crate::ui::workspace::ModeSource, _, cx| {
                if let Err(error)=host.change_mode(crate::settings::EditorMode::Source,cx){host.report(error,cx);}
            }))
            .on_action(
                cx.listener(|host, _: &crate::ui::workspace::Save, window, cx| {
                    host.save(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|host, _: &crate::ui::workspace::OpenFile, _, cx| {
                    if host.production {
                        host.choose_file(cx);
                    }
                }),
            )
            .on_action(
                cx.listener(|host, _: &crate::ui::workspace::Quit, window, cx| {
                    if host.request_close(window, cx) {
                        window.remove_window();
                    }
                }),
            )
            .child(
                div()
                    .on_children_prepainted(move |_, _, cx| {
                        let _ = host.update(cx, |host, cx| host.reveal_pane(host.active_pane,cx));
                    })
                    .id("control-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.panes[self.active_pane].scroll)
                    .p_8()
                    .on_scroll_wheel(cx.listener(|host, _, _, cx| {
                        host.panes[host.active_pane].reveal_selection = false;
                        host.schedule_viewport(16,cx);
                    }))
                    .children(self.pane_children(self.active_pane,cx)),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .p_2()
                    .text_color(theme.muted)
                    .child(format!(
                        "{} · revision {revision} · 已挂载 {} 窗口 · 控件估算 {} KiB",
                        self.message,
                        self.panes[self.active_pane].viewport.mounts.len(),
                        bytes / 1024
                    )),
            );
        result
    }
}

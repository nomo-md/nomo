//! GPUI 会话宿主；开发样本和真实文件入口共用事务、保存及窗口生命周期连接。
use super::{
    commands::EditIntent, host::WindowHostAdapter, native_port::NativeWindowPort,
    parse::InlineFormat, protocol::*, resources::ResourceRequest, view::EditorControl,
};
use crate::{editor::ViewMode, theme::Theme};
use gpui::{
    AppContext, Context, Entity, Focusable, IntoElement, PromptLevel, Render, Subscription, Window,
    div, prelude::*, px,
};
#[path = "session_actions.rs"]
mod session_actions;
#[path = "viewport_ui.rs"]
mod viewport_ui;

pub const SAMPLE: &str = "# Nomo 原生窗口控件\n\n在 **粗体**、*斜体*、~~删除线~~、<u>下划线</u>、<mark>高亮</mark>、H~2~O 和 x^2^ 中移动光标。文字保留排版，相关语法局部显示。\n\n中文与 Emoji：你好 👨‍👩‍👧‍👦 😀\n\n- [ ] 输入、选择、撤销\n- [x] 原版会话确认\n\n> [!NOTE]\n> 此入口直接复用原版 Rust 会话，加载调度仍属于宿主。\n\n| 名称 | 内容 |\n| --- | ---: |\n| 单元格 | **文字** |\n| 转义 | a\\|b |\n\n```rust\nfn main() { println!(\"Nomo\"); }\n```\n\n行内公式 $x^2+y^2=z^2$。\n";

pub struct ControlDemo {
    port: NativeWindowPort,
    control: Entity<EditorControl>,
    message: String,
    hidden: bool,
    close_prompt: bool,
    saving: bool,
    production: bool,
    navigating: bool,
    request: super::session_window::WindowRequest,
    total_bytes: u64,
    scroll: gpui::ScrollHandle,
    focus_pending: bool,
    viewport: viewport_ui::Viewport,
}
impl ControlDemo {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_content(SAMPLE, false, window, cx)
    }
    pub fn file(
        path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut host = Self::with_content("", true, window, cx);
        if let Some(path) = path {
            host.open_path(path, cx);
        }
        host
    }
    fn with_content(
        text: &str,
        production: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let root = std::env::temp_dir().join("nomo-gpui-control-sessions");
        let port =
            NativeWindowPort::open(root, "gpui-control-demo".into(), None, Some(text.into()))
                .expect("create original session");
        let status = port.status().expect("session status");
        let input = port
            .read_window(status.revision, 0..status.byte_length)
            .expect("read supplied sample");
        let host = WindowHostAdapter::new(input, "demo-window", 0, WindowContext::Complete)
            .expect("valid sample");
        let control = cx.new(|cx| EditorControl::new(host, cx));
        let subscription = cx.subscribe(&control, Self::event);
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |view, cx| view.request_close(window, cx))
                .unwrap_or(true)
        });
        window.focus(&control.focus_handle(cx));
        Self {
            port,
            control: control.clone(),
            message: "原版会话已连接 · 固定片段开发入口".into(),
            hidden: false,
            close_prompt: false,
            saving: false,
            production,
            navigating: false,
            request: Default::default(),
            total_bytes: status.byte_length,
            scroll: gpui::ScrollHandle::new(),
            focus_pending: false,
            viewport: viewport_ui::Viewport::new(control.clone(), subscription),
        }
    }
    fn event(&mut self, source: Entity<EditorControl>, event: &ControlEvent, cx: &mut Context<Self>) {
        if let ControlEvent::LayoutReady { stamp, height } | ControlEvent::HeightChanged { stamp, height } = event {
            self.viewport_height(&source, stamp, *height, cx);
            return;
        }
        if matches!(event, ControlEvent::SelectionChanged { .. }) && source != self.control {
            if !self.promote(&source, cx) { return; }
        }
        if source != self.control && !matches!(event, ControlEvent::ResourceDemand { .. } | ControlEvent::ResourceUsage { .. } | ControlEvent::Geometry { .. } | ControlEvent::Released(_)) { return; }
        match event {
            ControlEvent::ResourceUsage { .. } => { self.update_resident_cost(cx); self.schedule_viewport(16,cx); }
            ControlEvent::Submit(tx) => {
                let port = self.port.clone();
                let tx = tx.clone();
                let edits = tx.edits.clone();
                let operation = tx.operation_id.clone();
                let task = cx
                    .background_executor()
                    .spawn(async move { port.submit(tx) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |host, cx| {
                        match result {
                            Ok(revision) => {
                                if let Ok(status) = host.port.status() {
                                    host.total_bytes = status.byte_length;
                                }
                                host.viewport_edited(&edits, cx);
                                if let Err(error) = host
                                    .control
                                    .update(cx, |v, cx| v.acknowledge(&operation, revision, cx))
                                {
                                    host.message = error.to_string();
                                }
                                if host.control.read(cx).host.pending_count() == 0 {
                                    let identity = &host.control.read(cx).host.parsed.identity;
                                    match host
                                        .port
                                        .read_window(revision, identity.from_byte..identity.to_byte)
                                    {
                                        Ok(window) => {
                                            host.message = match host.control.update(cx, |v, _| {
                                                v.host.verify_authoritative(&window)
                                            }) {
                                                Ok(()) => {
                                                    format!("原版会话已确认 · revision {revision}")
                                                }
                                                Err(error) => error.to_string(),
                                            };
                                        }
                                        Err(error) => host
                                            .control
                                            .update(cx, |v, cx| v.fail(error.to_string(), cx)),
                                    }
                                }
                            }
                            Err(error) => host
                                .control
                                .update(cx, |v, cx| v.fail(error.to_string(), cx)),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            ControlEvent::Undo | ControlEvent::Redo => {
                self.history(matches!(event, ControlEvent::Redo), cx)
            }
            ControlEvent::Navigate {
                identity,
                source_byte,
                ..
            } if self.production => {
                if self.control.read(cx).host.parsed.identity == *identity {
                    self.load_at(
                        *source_byte,
                        Some(DocumentSelection::caret(*source_byte)),
                        cx,
                    );
                }
            }
            ControlEvent::EditOutsideWindow {
                identity,
                selection,
                intent,
            } if self.production => {
                if self.control.read(cx).host.parsed.identity == *identity {
                    self.global_edit(*selection, intent.clone(), cx);
                }
            }
            ControlEvent::ClipboardRequest {
                identity,
                selection,
                cut,
            } if self.production => {
                if self.control.read(cx).host.parsed.identity == *identity {
                    self.clipboard(*selection, *cut, cx);
                }
            }
            ControlEvent::SelectAll => {
                if let Ok(status) = self.port.status() {
                    self.control.update(cx, |view, cx| {
                        view.select(
                            DocumentSelection {
                                anchor_byte: 0,
                                head_byte: status.byte_length,
                            },
                            cx,
                        )
                    });
                }
            }
            ControlEvent::EmojiCatalogRequested { identity } => {
                if self.control.read(cx).host.parsed.identity.document_id != identity.document_id {
                    return;
                }
                let identity = identity.clone();
                self.message = "正在获取 Emoji 词库".into();
                let task = cx.background_executor().spawn(async move {
                    // 复用原版经过哈希校验、有大小与超时限制的加载器。
                    // 运行时仅属于本次宿主资源任务，不成为每个控件的常驻线程。
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    runtime.block_on(shared_emoji_catalog())
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |host, cx| {
                        let current = &host.control.read(cx).host.parsed.identity;
                        if current.document_id != identity.document_id
                            || current.window_id != identity.window_id
                        {
                            return;
                        }
                        match result {
                            Ok(catalog) => {
                                host.message =
                                    format!("Emoji 词库已就绪 · {} 项", catalog.entries.len());
                                let _ = host
                                    .control
                                    .update(cx, |view, cx| view.set_emoji_catalog(catalog, cx));
                            }
                            Err(error) => {
                                host.message = format!("Emoji 暂不可用：{error}");
                                host.control
                                    .update(cx, |view, cx| view.emoji_catalog_failed(cx));
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            ControlEvent::DraftChanged { active: false, .. } => {
                self.message = "对象编辑已结束".into()
            }
            ControlEvent::ResourceDemand {
                identity,
                requests: demands,
            } => {
                if &source.read(cx).host.parsed.identity != identity {
                    return;
                }
                let mut images = Vec::new();
                let mut embeds = Vec::new();
                for demand in demands {
                    match demand {
                        ResourceRequest::Image(path) => images.push(path.clone()),
                        ResourceRequest::Embed(spec) => embeds.push(spec.clone()),
                    }
                }
                crate::images::request(images, cx);
                crate::embeds::request(embeds, cx);
            }
            ControlEvent::Failure(message) | ControlEvent::NeedsContext(message) => {
                self.message = message.clone()
            }
            ControlEvent::OpenLink(url) => {
                if url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
                {
                    cx.open_url(url);
                } else if url.starts_with('#') && self.production {
                    self.anchor(url.trim_start_matches('#').to_string(), cx);
                } else if url.starts_with('#') {
                    self.message = format!("锚点导航交给宿主：{url}");
                } else {
                    self.message = "该链接需由宿主资源接口处理".into();
                }
            }
            ControlEvent::WindowBoundary {
                identity,
                forward,
                extend_selection,
            } => {
                if self.production && self.control.read(cx).host.parsed.identity == *identity {
                    self.adjacent(*forward, *extend_selection, cx);
                } else {
                    self.message = "当前演示片段已经到达文档边界".into();
                }
            }
            _ => {}
        }
        cx.notify();
    }
    fn history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.production {
            self.session_history(redo, cx);
            return;
        }
        if let Err(error) = self.control.read(cx).prepare_unmount() {
            self.message = error.to_string();
            return;
        }
        let result = (|| {
            let revision = self.port.status()?.revision;
            let (revision, selection) = self.port.history(revision, redo)?;
            let status = self.port.status()?;
            let input = self.port.read_window(revision, 0..status.byte_length)?;
            self.control.update(cx, |view, cx| {
                let generation = view.host.parsed.identity.view_generation + 1;
                view.update_window(input, generation, WindowContext::Complete, cx)?;
                if let Some(selection) = selection {
                    view.select(selection, cx);
                }
                Ok::<_, ControlError>(())
            })
        })();
        self.message = match result {
            Ok(()) => if redo {
                "原版会话已重做"
            } else {
                "原版会话已撤销"
            }
            .into(),
            Err(error) => error.to_string(),
        };
    }
    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        if !self.hidden {
            if let Some(error)=self.viewport.mounts.iter().find_map(|m|m.view.read(cx).prepare_unmount().err()) {
                self.message=error.to_string(); cx.notify(); return;
            }
            self.viewport.saved_scroll=self.scroll.offset();
        }
        let result = self.control.update(cx, |view, cx| {
            if self.hidden {
                view.restore(cx)
            } else {
                view.hide(cx)
            }
        });
        match result {
            Ok(()) => {
                self.hidden = !self.hidden;
                if self.hidden {
                    self.pause_viewport(cx);
                    for mount in &self.viewport.mounts {
                        if mount.view != self.control { let _=mount.view.update(cx, |view,cx|view.hide(cx)); }
                    }
                } else {
                    for mount in &self.viewport.mounts {
                        if mount.view != self.control { let _=mount.view.update(cx, |view,cx|view.restore(cx)); }
                    }
                    self.scroll.set_offset(self.viewport.saved_scroll);
                    self.schedule_viewport(16,cx);
                }
                self.message = if self.hidden {
                    "窗口已隐藏"
                } else {
                    "窗口已恢复"
                }
                .into();
            }
            Err(error) => self.message = error.to_string(),
        };
        cx.notify();
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.close_prompt || self.saving || self.navigating {
            return false;
        }
        if let Err(error) = self.control.read(cx).prepare_unmount() {
            self.message = error.to_string();
            cx.notify();
            return false;
        }
        let status = match self.port.status() {
            Ok(status) => status,
            Err(error) => {
                self.message = error.to_string();
                cx.notify();
                return false;
            }
        };
        if !status.dirty {
            match self.port.close(false) {
                Ok(()) => {
                    let _ = self.control.update(cx, |view, cx| view.detach(false, cx));
                    return true;
                }
                Err(error) => {
                    self.message = error.to_string();
                    cx.notify();
                    return false;
                }
            }
        }
        self.close_prompt = true;
        self.control
            .update(cx, |view, _| view.host.set_readonly(true));
        let response = window.prompt(
            PromptLevel::Warning,
            "是否保存当前文档？",
            Some("草稿与未确认输入完成前不会卸载控件。"),
            &["保存", "不保存", "取消"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = response.await.ok();
            let _ = this.update_in(cx, |host, window, cx| {
                host.close_prompt = false;
                host.control
                    .update(cx, |view, _| view.host.set_readonly(false));
                match choice {
                    Some(0) => host.save(true, window, cx),
                    Some(1) => match host.port.close(true) {
                        Ok(()) => {
                            let _ = host.control.update(cx, |view, cx| view.detach(true, cx));
                            window.remove_window();
                        }
                        Err(error) => host.message = error.to_string(),
                    },
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        false
    }
    fn save(&mut self, close_after: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.navigating {
            return;
        }
        if let Err(error) = self.control.read(cx).prepare_unmount() {
            self.message = error.to_string();
            cx.notify();
            return;
        }
        let status = match self.port.status() {
            Ok(status) => status,
            Err(error) => {
                self.message = error.to_string();
                cx.notify();
                return;
            }
        };
        if let Some(path) = status.path {
            self.save_path(path.into(), close_after, window, cx);
            return;
        }
        let response = cx.prompt_for_new_path(
            &std::env::current_dir().unwrap_or_default(),
            Some("控件开发示例.md"),
        );
        self.close_prompt = true;
        cx.spawn_in(window, async move |this, cx| {
            let path = response.await.ok().and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |host, window, cx| {
                host.close_prompt = false;
                if let Some(path) = path {
                    host.save_path(path, close_after, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn save_path(
        &mut self,
        path: std::path::PathBuf,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.control.read(cx).prepare_unmount() {
            self.message = error.to_string();
            cx.notify();
            return;
        }
        self.saving = true;
        if close_after {
            self.control
                .update(cx, |view, _| view.host.set_readonly(true));
        }
        let port = self.port.clone();
        let task = cx
            .background_executor()
            .spawn(async move { port.save_to(path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |host, window, cx| {
                host.saving = false;
                host.control
                    .update(cx, |view, _| view.host.set_readonly(false));
                match result {
                    Ok(revision) => {
                        host.message = format!("完整会话快照已保存 · revision {revision}");
                        if close_after {
                            match host.port.close(false) {
                                Ok(()) => {
                                    let _ =
                                        host.control.update(cx, |view, cx| view.detach(false, cx));
                                    window.remove_window();
                                }
                                Err(error) => host.message = error.to_string(),
                            }
                        }
                    }
                    Err(error) => host.message = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

async fn shared_emoji_catalog() -> Result<std::sync::Arc<super::emoji::EmojiCatalog>, String> {
    use std::sync::{Arc, OnceLock, Weak};
    // 只保留弱引用：最后一个使用者释放后，元数据索引可回收；并发需求合并。
    static CATALOG: OnceLock<tokio::sync::Mutex<Weak<super::emoji::EmojiCatalog>>> =
        OnceLock::new();
    let mut cache = CATALOG
        .get_or_init(|| tokio::sync::Mutex::new(Weak::new()))
        .lock()
        .await;
    if let Some(catalog) = cache.upgrade() {
        return Ok(catalog);
    }
    let path = std::env::temp_dir()
        .join("nomo-gpui-control-resources")
        .join("emoji")
        .join("catalog-v1.json");
    let payload = super::native_emoji::load_at(path).await?;
    let catalog = Arc::new(super::emoji::EmojiCatalog::from_payload(
        payload.version.into(),
        payload.en,
        payload.zh,
        payload.shortcodes,
    )?);
    *cache = Arc::downgrade(&catalog);
    Ok(catalog)
}
impl Render for ControlDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                        for mount in &host.viewport.mounts {
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
            use super::table::{Alignment, TableIntent};
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
                    .id("control-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_8()
                    .on_scroll_wheel(cx.listener(|host, _, _, cx| host.schedule_viewport(16,cx)))
                    .children(self.viewport_children(cx)),
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
                        self.viewport.mounts.len(),
                        bytes / 1024
                    )),
            );
        result
    }
}

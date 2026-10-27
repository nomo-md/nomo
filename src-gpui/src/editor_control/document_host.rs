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
#[path = "window_panes.rs"]
mod window_panes;
#[path = "workspace_host.rs"]
mod workspace_host;
#[path = "toolbar_commands.rs"]
mod toolbar_commands;
#[path = "toolbar_global.rs"]
mod toolbar_global;
#[path = "toc_update.rs"]
mod toc_update;
#[path = "workspace_queries.rs"]
mod workspace_queries;
#[path = "outline_queries.rs"]
mod outline_queries;
pub use workspace_host::DocumentEvent;
pub(crate) use workspace_host::EditTarget;
pub use outline_queries::OutlineHeading;
pub(crate) use outline_queries::OutlineSnapshot;
#[cfg(feature = "editor-control-demo")]
#[path = "development_ui.rs"]
mod development_ui;

pub const SAMPLE: &str = "# Nomo 原生窗口控件\n\n在 **粗体**、*斜体*、~~删除线~~、<u>下划线</u>、<mark>高亮</mark>、H~2~O 和 x^2^ 中移动光标。文字保留排版，相关语法局部显示。\n\n中文与 Emoji：你好 👨‍👩‍👧‍👦 😀\n\n- [ ] 输入、选择、撤销\n- [x] 原版会话确认\n\n> [!NOTE]\n> 此入口直接复用原版 Rust 会话，加载调度仍属于宿主。\n\n| 名称 | 内容 |\n| --- | ---: |\n| 单元格 | **文字** |\n| 转义 | a\\|b |\n\n```rust\nfn main() { println!(\"Nomo\"); }\n```\n\n行内公式 $x^2+y^2=z^2$。\n";

pub struct DocumentHost {
    port: NativeWindowPort,
    embedded: bool,
    closed: bool,
    metadata: workspace_host::Metadata,
    queries: workspace_queries::Queries,
    control: Entity<EditorControl>,
    message: String,
    hidden: bool,
    close_prompt: bool,
    saving: bool,
    production: bool,
    navigating: bool,
    request: super::session_window::WindowRequest,
    total_bytes: u64,
    panes: Vec<window_panes::Pane>,
    active_pane: usize,
    split_ratio: f32,
    split_dragging: bool,
    split_focus: gpui::FocusHandle,
    focus_pending: bool,
    toolbar_marks: u8,

}
impl DocumentHost {
    #[cfg(feature = "editor-control-demo")]
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_content(SAMPLE, false, window, cx)
    }
    #[cfg(feature = "editor-control-demo")]
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
    #[cfg(feature = "editor-control-demo")]
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
        window.focus(&control.focus_handle(cx), cx);
        Self {
            metadata: workspace_host::Metadata::new(status.clone()),
            queries: Default::default(),
            embedded: false,
            closed: false,
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
            panes: vec![window_panes::Pane::new(control.clone(), subscription, ViewMode::Semantic)],
            active_pane: 0, split_ratio: 0.5, split_dragging: false, split_focus: cx.focus_handle(),
            focus_pending: false, toolbar_marks: 0,

        }
    }
    fn event(&mut self, source: Entity<EditorControl>, event: &ControlEvent, cx: &mut Context<Self>) {
        if let ControlEvent::LayoutReady { stamp, height } | ControlEvent::HeightChanged { stamp, height } = event {
            self.viewport_height(&source, stamp, *height, cx);
            return;
        }
        if source != self.control && !matches!(event, ControlEvent::ResourceDemand { .. } | ControlEvent::ResourceUsage { .. } | ControlEvent::Geometry { .. } | ControlEvent::Released(_)) { return; }
        match event {
            ControlEvent::Geometry { stamp, .. } => {
                if self.panes[self.active_pane].reveal_selection && source == self.control && stamp.identity == source.read(cx).host.parsed.identity { cx.notify(); }
                return;
            }
            ControlEvent::SelectionChanged { .. } => {
                self.panes[self.active_pane].reveal_selection = true;
                self.sync_selection(cx);
                self.refresh_toolbar_marks(cx);
                self.toc_selection_changed(cx);
            }
            ControlEvent::ResourceUsage { .. } => { self.update_resident_cost(cx); self.schedule_viewport(16,cx); }
            ControlEvent::Submit(tx) | ControlEvent::SubmitDerived(tx) => {
                let derived = matches!(event, ControlEvent::SubmitDerived(_));
                self.refresh_toolbar_marks(cx);
                if !derived {
                    self.cancel_pane_follow(self.active_pane,cx);
                    self.panes[self.active_pane].reveal_selection = true;
                }
                let port = self.port.clone();
                let tx = tx.clone();
                let edits = tx.edits.clone();
                let operation = tx.operation_id.clone();
                let task = cx
                    .background_executor()
                    .spawn(async move { if derived { port.submit_derived(tx) } else { port.submit(tx) } });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |host, cx| {
                        match result {
                            Ok(revision) => {
                                host.document_changed(cx);
                                if let Ok(status) = host.port.status() {
                                    host.total_bytes = status.byte_length;
                                }
                                host.viewport_edited(&edits, cx);
                                if let Err(error) = source
                                    .update(cx, |v, cx| v.acknowledge(&operation, revision, cx))
                                {
                                    host.message = error.to_string();
                                }
                                if source.read(cx).host.pending_count() == 0 {
                                    let identity = &source.read(cx).host.parsed.identity;
                                    match host
                                        .port
                                        .read_window(revision, identity.from_byte..identity.to_byte)
                                    {
                                        Ok(window) => {
                                            host.message = match source.update(cx, |v, _| {
                                                v.host.verify_authoritative(&window)
                                            }) {
                                                Ok(()) => {
                                                    format!("原版会话已确认 · revision {revision}")
                                                }
                                                Err(error) => error.to_string(),
                                            };
                                        }
                                        Err(error) => source
                                            .update(cx, |v, cx| v.fail(error.to_string(), cx)),
                                    }
                                }
                            }
                            Err(error) => {
                                let message = error.to_string();
                                source.update(cx, |v, cx| v.fail(message.clone(), cx));
                                if derived { host.finish_toc_check(host.status().revision, Some(message), cx); }
                            },
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
                let length=self.control.read(cx).host.projected_document_length(self.total_bytes);
                    self.control.update(cx, |view, cx| {
                        view.select(
                            DocumentSelection {
                                anchor_byte: 0,
                                head_byte: length,
                            },
                            cx,
                        )
                    });
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
                self.message = message.clone();
                cx.emit(DocumentEvent::Failure(message.clone()));
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
        if let Err(error)=self.ready(cx) { self.report(error,cx); return; }
        self.hidden=!self.hidden;
        for pane in 0..self.panes.len() {
            if self.hidden {
                self.panes[pane].viewport.saved_scroll=self.panes[pane].scroll.offset();
                self.pause_pane(pane,cx);
            }
            for mount in &self.panes[pane].viewport.mounts {
                let result=mount.view.update(cx,|view,cx| if self.hidden {view.hide(cx)} else {view.restore(cx)});
                if let Err(error)=result {self.message=error.to_string();}
            }
            if !self.hidden {
                self.panes[pane].scroll.set_offset(self.panes[pane].viewport.saved_scroll);
                self.schedule_pane(pane,16,cx);
            }
        }
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
                        host.refresh_status(cx);
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

pub(crate) async fn shared_emoji_catalog() -> Result<std::sync::Arc<super::emoji::EmojiCatalog>, String> {
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

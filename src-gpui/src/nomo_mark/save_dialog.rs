//! 保存选择器共用当前目录与首个 H1；按需查询会话索引，不持有正文或大纲副本。
use super::*;
use crate::nomo_mark::session_window::WindowRequest;
use crate::text_document::StructureQuery;
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};

#[derive(Default)]
pub(super) struct SaveDialog {
    directory: Option<PathBuf>,
    request: Option<WindowRequest>,
}

impl SaveDialog {
    pub(super) fn cancel(&mut self) {
        if let Some(request) = self.request.take() { request.cancel(); }
    }
}

impl Drop for SaveDialog {
    fn drop(&mut self) { self.cancel(); }
}

fn dialog_error(error: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(format!("无法准备文档保存窗口：{error}"))
}

fn heading_filename(title: &str) -> Option<String> {
    // 同时满足 Windows 文件名规则与常见文件系统的单名称字节限制。
    let mut name = String::new();
    for ch in title.trim().chars() {
        let ch = if ch.is_control() || "<>:\"/\\|?*".contains(ch) { '_' } else { ch };
        if name.len() + ch.len_utf8() > 200 { break; }
        name.push(ch);
    }
    let mut name = name.trim().trim_matches('.').trim().to_string();
    if name.is_empty() { return None; }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ["COM", "LPT"].iter().any(|prefix| stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        }))
    {
        name.insert(0, '_');
    }
    if !name.to_ascii_lowercase().ends_with(".md") { name.push_str(".md"); }
    Some(name)
}

fn suggested_filename(port: &NativeWindowPort, revision: u64, request: &WindowRequest) -> Result<String, ControlError> {
    request.check()?;
    let status = port.status()?;
    if status.revision != revision { return Err(ControlError::Stale); }
    let fallback = status.path.as_ref().and_then(|path| Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "未命名.md".into());
    if status.byte_length == 0 { return Ok(fallback); }
    let identity = port.ready_index(revision, request)?;
    let started = Instant::now();
    let mut cursor = None;
    loop {
        request.check()?;
        if started.elapsed() > Duration::from_secs(30) {
            return Err(ControlError::NeedsContext("保存标题查询尚未完成，请重试保存".into()));
        }
        let page = port.manager.query_structure(&port.owner, identity.clone(), StructureQuery::Headings {
            from_byte: 0, to_byte: status.byte_length, cursor,
        }).map_err(dialog_error)?;
        if page.state != "ready" { return Err(ControlError::Stale); }
        if let Some(heading) = page.items.iter().find(|heading| heading["level"].as_u64() == Some(1)) {
            let title = heading["title"].as_str().ok_or_else(|| dialog_error("一级标题缺少文本"))?;
            return Ok(heading_filename(title).unwrap_or(fallback));
        }
        if page.items.is_empty() && page.next.is_some() {
            return Err(ControlError::NeedsContext("保存标题超过索引单页预算".into()));
        }
        cursor = page.next;
        if cursor.is_none() { return Ok(fallback); }
    }
}

impl DocumentHost {
    pub(crate) fn set_save_directory(&mut self, directory: Option<&Path>) {
        if self.metadata.save_dialog.directory.as_deref() != directory {
            self.metadata.save_dialog.directory = directory.map(Path::to_path_buf);
        }
    }

    pub(crate) fn prompt_save_path(&mut self, window: &mut Window, cx: &mut Context<Self>)
        -> Result<gpui::Task<Result<Option<PathBuf>, ControlError>>, ControlError>
    {
        if self.closed || self.saving || self.close_prompt || self.metadata.save_dialog.request.is_some() {
            return Err(ControlError::Busy);
        }
        let path = self.path();
        let request = WindowRequest::default();
        self.metadata.save_dialog.request = Some(request.clone());
        Ok(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let started = Instant::now();
                loop {
                    request.check()?;
                    if started.elapsed() > Duration::from_secs(30) { return Err(ControlError::Busy); }
                    // Ctrl+S 可能紧跟最后一次输入；等待现有确认队列，不读取乐观投影或旧大纲。
                    let prepared = this.update_in(cx, |host, _, cx| {
                        if host.closed || host.saving || host.close_prompt || host.has_failed_input(cx) { return Err(ControlError::Busy); }
                        if host.path() != path { return Err(ControlError::Stale); }
                        if host.navigating || host.control.read(cx).host.pending_count() > 0 { return Ok(None); }
                        host.ready(cx)?;
                        Ok(Some((host.status().revision, host.port.clone())))
                    }).map_err(dialog_error)??;
                    let Some((revision, port)) = prepared else {
                        cx.background_executor().timer(Duration::from_millis(16)).await;
                        continue;
                    };
                    let worker = request.clone();
                    let filename = cx.background_executor().spawn(async move { suggested_filename(&port, revision, &worker) }).await;
                    request.check()?;
                    let response = this.update_in(cx, |host, _, cx| {
                        if host.path() != path { return Err(ControlError::Stale); }
                        if host.status().revision != revision || host.control.read(cx).host.pending_count() > 0 { return Ok(None); }
                        host.ready(cx)?;
                        let filename = match filename {
                            Ok(filename) => filename,
                            Err(error) => {
                                // 索引不可用时仍保留保存出口，同时明确提示无法生成标题名称。
                                host.report(error, cx);
                                path.as_deref().and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "未命名.md".into())
                            }
                        };
                        let directory = host.path().as_deref().and_then(Path::parent).map(Path::to_path_buf)
                            .or_else(|| host.metadata.save_dialog.directory.clone())
                            .map(Ok).unwrap_or_else(std::env::current_dir).map_err(dialog_error)?;
                        Ok(Some(cx.prompt_for_new_path(&directory, Some(&filename))))
                    }).map_err(dialog_error)??;
                    let Some(response) = response else { continue; };
                    let path = response.await.map_err(dialog_error)?.map_err(dialog_error)?;
                    request.check()?;
                    return Ok(path);
                }
            }.await;
            let _ = this.update_in(cx, |host, _, _| {
                if !request.is_cancelled() { host.metadata.save_dialog.request = None; }
            });
            result
        }))
    }
}

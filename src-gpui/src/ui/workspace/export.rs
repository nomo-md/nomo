//! 导出意图只持有弱文档引用和可取消快照；文档关闭、重载或窗口释放时取消。
use super::*;
use crate::document_export::{self, Appearance, ExportPermit, Snapshot, Format};
use std::sync::Arc;

pub(super) struct ExportJob {
    document: gpui::WeakEntity<DocumentHost>,
    snapshot: Arc<Snapshot>,
    request: WindowRequest,
    _task: Task<()>,
}
impl Drop for ExportJob {
    fn drop(&mut self) {
        self.request.cancel();
        if let Err(error) = self.snapshot.release() { eprintln!("取消导出时释放快照失败：{error}"); }
    }
}

impl Workspace {
    pub(super) fn cancel_export_for(&mut self, document: &Entity<DocumentHost>) {
        if self.export_job.as_ref().is_some_and(|job| job.document.upgrade().as_ref() == Some(document)) { self.export_job = None; }
    }
    pub(super) fn export_document(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = self.document() { self.export_document_format(document, format, window, cx); }
    }
    fn export_document_format(&mut self, document: Entity<DocumentHost>, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() || self.closing_window || self.application_dialogs.busy { return; }
        if self.export_job.is_some() { self.notice = Some("正在导出，请稍候…".into()); cx.notify(); return; }
        let weak = document.downgrade();
        if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
            if let Some(document) = weak.upgrade().filter(|doc| workspace.tabs.iter().any(|tab| tab.document == *doc)) {
                workspace.export_document_format(document, format, window, cx);
            }
        }) { return; }
        if !self.leave_mini(window, cx) { return; }
        let prepared = (|| -> Result<_, String> {
            let host = document.read(cx);
            host.ready(cx).map_err(|e| e.to_string())?;
            let permit = ExportPermit::acquire()?;
            let snapshot = Snapshot::capture(host.port(), host.status().revision, host.path())?;
            Ok((permit, snapshot, host.title()))
        })();
        let (permit, snapshot, title) = match prepared {
            Ok(value) => value,
            Err(error) => { self.failure_notice = Some(error.into()); cx.notify(); return; }
        };
        let directory = snapshot.path.as_deref().and_then(Path::parent).map(Path::to_path_buf)
            .or_else(|| self.tree.as_ref().map(|tree| tree.root.clone())).unwrap_or_else(|| PathBuf::from("."));
        let filename = format!("{}.{}", Path::new(&title).file_stem().unwrap_or_else(|| std::ffi::OsStr::new("未命名")).to_string_lossy(), format.extension());
        let response = cx.prompt_for_new_path(&directory, Some(&filename));
        let appearance = Appearance::capture(cx);
        let request = WindowRequest::default();
        let worker_request = request.clone();
        let worker_snapshot = snapshot.clone();
        let id = snapshot.id().to_owned();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let path = response.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                let Some(path) = path else { return Ok(None); };
                worker_request.check().map_err(|e| e.to_string())?;
                let valid = this.update_in(cx, |workspace, _, cx| {
                    let valid = workspace.export_job.as_ref().is_some_and(|job| job.snapshot.id() == id
                        && job.document.upgrade().is_some_and(|doc| workspace.tabs.iter().any(|tab| tab.document == doc)
                            && doc.read(cx).path() == worker_snapshot.path));
                    if valid { workspace.notice = Some(format!("正在导出 {}…", format.label()).into()); cx.notify(); }
                    valid
                }).map_err(|e| e.to_string())?;
                if !valid { return Err("文档已关闭、移动或导出已取消。".into()); }
                cx.background_executor().spawn(async move {
                    let _permit = permit;
                    document_export::export(format, worker_snapshot, &path, &title, appearance, worker_request).map(|report| Some((path, report)))
                }).await
            }.await;
            let _ = this.update_in(cx, |workspace, _, cx| {
                if workspace.export_job.as_ref().is_none_or(|job| job.snapshot.id() != id) { return; }
                workspace.export_job = None;
                match result {
                    Ok(Some((path, report))) => {
                        let message = format!("{} 已导出：{}", format.label(), path.display());
                        if report.warnings.is_empty() { workspace.notice = Some(message.into()); }
                        else { workspace.failure_notice = Some(format!("{message}；{}", report.warnings.join("；")).into()); }
                    }
                    Ok(None) => {}, Err(error) => workspace.failure_notice = Some(format!("{} 导出失败：{error}", format.label()).into()),
                }
                cx.notify();
            });
        });
        self.export_job = Some(ExportJob { document: document.downgrade(), snapshot, request, _task: task });
        cx.notify();
    }
}

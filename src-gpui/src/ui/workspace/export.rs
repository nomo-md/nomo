//! 导出意图只持有弱文档引用和可取消快照；文档关闭、重载或窗口释放时取消。
use super::*;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
use crate::document_export::{self, Appearance, ExportPermit, Snapshot, Format};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum OutputCommand { Export(Format), Print }

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
        if let Some(document) = self.document() { self.start_output(document, OutputCommand::Export(format), window, cx); }
    }
    pub(super) fn print_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = self.document() { self.start_output(document, OutputCommand::Print, window, cx); }
    }
    fn start_output(&mut self, document: Entity<DocumentHost>, command: OutputCommand, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() || self.closing_window || self.application_dialogs.busy { return; }
        if self.export_job.is_some() { self.notice = Some(ToolbarLocale::current(cx).app_text("export.busy").into()); cx.notify(); return; }
        let weak = document.downgrade();
        if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
            if let Some(document) = weak.upgrade().filter(|doc| workspace.tabs.iter().any(|tab| tab.document == *doc)) {
                workspace.start_output(document, command, window, cx);
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
        let response = match command {
            OutputCommand::Export(format) => {
                let directory = snapshot.path.as_deref().and_then(Path::parent)
                    .or_else(|| self.tree.as_ref().map(|tree| tree.root.as_path())).unwrap_or(Path::new("."));
                let filename = format!("{}.{}", Path::new(&title).file_stem().unwrap_or_else(|| std::ffi::OsStr::new("未命名")).to_string_lossy(), format.extension());
                Some((format, cx.prompt_for_new_path(directory, Some(&filename))))
            }
            OutputCommand::Print => None,
        };
        let appearance = Appearance::capture(cx);
        let request = WindowRequest::default();
        let worker_request = request.clone();
        let worker_snapshot = snapshot.clone();
        let id = snapshot.id().to_owned();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let target = if let Some((format, response)) = response {
                    let path = response.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                    let Some(path) = path else { return Ok(None); };
                    Some((format, path))
                } else { None };
                worker_request.check().map_err(|e| e.to_string())?;
                let valid = this.update_in(cx, |workspace, _, cx| {
                    let valid = workspace.export_job.as_ref().is_some_and(|job| job.snapshot.id() == id
                        && job.document.upgrade().is_some_and(|doc| workspace.tabs.iter().any(|tab| tab.document == doc)
                            && doc.read(cx).path() == worker_snapshot.path));
                    if valid {
                        let locale = ToolbarLocale::current(cx);
                        workspace.notice = Some(match command {
                            OutputCommand::Export(format) => locale.app_format("export.running", &[("format", format.label())]).into(),
                            OutputCommand::Print => locale.app_text("print.preparing").into(),
                        });
                        cx.notify();
                    }
                    valid
                }).map_err(|e| e.to_string())?;
                if !valid { return Err(UiMessage::new("export.cancelled")); }
                if let Some((format, path)) = target {
                    cx.background_executor().spawn(async move {
                        let _permit = permit;
                        let report = document_export::export(format, worker_snapshot, &path, &title, appearance, worker_request).map_err(UiMessage::raw)?;
                        let message = UiMessage::new("export.complete").arg("format", format.label()).arg("path", path.display());
                        Ok(Some((message, report)))
                    }).await
                } else {
                    let report = super::print::document(worker_snapshot, title, appearance, worker_request, permit, cx).await?;
                    let message = if cfg!(target_os = "linux") { "print.handedOff" } else { "print.submitted" };
                    Ok(report.map(|report| (UiMessage::new(message), report)))
                }
            }.await;
            let _ = this.update_in(cx, |workspace, _, cx| {
                if workspace.export_job.as_ref().is_none_or(|job| job.snapshot.id() != id) { return; }
                workspace.export_job = None;
                match result {
                    Ok(Some((message, report))) => {
                        let message = message.render(ToolbarLocale::current(cx));
                        if report.warnings.is_empty() { workspace.notice = Some(message.into()); }
                        else { workspace.failure_notice = Some(format!("{message}\n{}", report.warnings.join("\n")).into()); }
                    }
                    Ok(None) if matches!(command, OutputCommand::Print) => workspace.notice = None,
                    Ok(None) => {},
                    Err(error) => {
                        let message = match command {
                            OutputCommand::Export(format) => error.context("export.failed").arg("format", format.label()),
                            OutputCommand::Print => error.context("print.failed"),
                        };
                        workspace.failure_notice = Some(message.render(ToolbarLocale::current(cx)).into());
                    }
                }
                cx.notify();
            });
        });
        self.export_job = Some(ExportJob { document: document.downgrade(), snapshot, request, _task: task });
        cx.notify();
    }
}

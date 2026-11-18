//! 打印只在用户请求时生成临时 PDF，后台排版与文件导出共用同一快照和任务配额。
use crate::document_export::{self, Appearance, ExportPermit, ExportReport, Format, Snapshot};
use crate::nomo_mark::session_window::WindowRequest;
use gpui::AsyncWindowContext;
use std::sync::Arc;

#[cfg(windows)]
mod windows;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;

pub(super) async fn document(
    snapshot: Arc<Snapshot>, title: String, appearance: Appearance, request: WindowRequest,
    permit: ExportPermit, cx: &mut AsyncWindowContext,
) -> Result<Option<ExportReport>, String> {
    let appearance = appearance.for_print();
    #[cfg(target_os = "linux")]
    let (appearance, portal) = {
        let pending_parent = cx.update(|window, _| linux::parent_window(window)).map_err(|e| e.to_string())??;
        let parent = linux::resolve_parent(pending_parent).await;
        let Some((portal, sheet)) = linux::prepare(parent, &title).await? else { return Ok(None) };
        request.check().map_err(|e| e.to_string())?;
        let mut appearance = appearance;
        appearance.print_sheet = sheet;
        (appearance, portal)
    };
    let render_request = request.clone();
    let render_title = title.clone();
    let (dir, path, report, permit) = cx.background_executor().spawn(async move {
        let mut builder = tempfile::Builder::new();
        builder.prefix("nomo-print-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let dir = builder.tempdir().map_err(|e| e.to_string())?;
        let path = dir.path().join("document.pdf");
        let report = document_export::export(Format::Pdf, snapshot, &path, &render_title, appearance, render_request)?;
        Ok::<_, String>((dir, path, report, permit))
    }).await?;
    request.check().map_err(|e| e.to_string())?;

    #[cfg(windows)]
    let submitted = {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let owner = cx.update(|window, _| match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(raw) => Some(raw.hwnd.get()),
            _ => None,
        }).map_err(|e| e.to_string())?.ok_or("打印窗口不可用")?;
        let _permit = permit;
        windows::print(&path, dir, &title, owner, request).await.map_err(|e| e.to_string())?
    };
    #[cfg(target_os = "macos")]
    let submitted = {
        let owner = cx.update(|window, _| macos::owner(window)).map_err(|e| e.to_string())??;
        macos::print(&path, &title, &owner, dir, permit).await?
    };
    #[cfg(target_os = "linux")]
    let submitted = {
        let _resources = (dir, permit);
        portal.print(&path, &title).await?
    };
    Ok(submitted.then_some(report))
}

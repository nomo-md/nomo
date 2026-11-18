//! Windows asks this window's print source for preview pages and the final document.
mod pages;
mod render;

use crate::nomo_mark::session_window::WindowRequest;
use pages::{Document, PageSource};
use std::{path::Path, sync::{Arc, Mutex}};
use windows::{
    core::{factory, Error, HSTRING, Result},
    Data::Pdf::PdfDocument,
    Foundation::TypedEventHandler,
    Graphics::Printing::*,
    Storage::StorageFile,
    Win32::{Foundation::{E_ABORT, E_FAIL, HWND}, System::WinRT::Printing::IPrintManagerInterop},
};
use windows_future::IAsyncOperation;

struct Registration(PrintManager, i64);
impl Drop for Registration {
    fn drop(&mut self) {
        if let Err(error) = self.0.RemovePrintTaskRequested(self.1) {
            eprintln!("移除打印任务监听失败：{error}");
        }
    }
}

type Reply = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<bool>>>>>;

fn finish(reply: &Reply, result: Result<bool>) {
    if let Some(sender) = reply.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = sender.send(result);
    }
}

pub(super) async fn print(
    path: &Path, directory: tempfile::TempDir, title: &str, owner: isize, request: WindowRequest,
) -> Result<bool> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.await?;
    let pdf = PdfDocument::LoadFromFileAsync(&file)?.await?;
    let document = Arc::new(Document::new(pdf, directory, request)?);
    document.check()?;
    let interop: IPrintManagerInterop = factory::<PrintManager, _>()?;
    let hwnd = HWND(owner as *mut _);
    // Both interop methods run on the thread that owns the GPUI window.
    let manager: PrintManager = unsafe { interop.GetForWindow(hwnd)? };
    let title = HSTRING::from(title);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let reply = Arc::new(Mutex::new(Some(sender)));
    let cookie = manager.PrintTaskRequested(&TypedEventHandler::new(move |_, args: windows::core::Ref<PrintTaskRequestedEventArgs>| {
        let result = (|| {
            document.check()?;
            let source_document = document.clone();
            let task = args.ok()?.Request()?.CreatePrintTask(&title, &PrintTaskSourceRequestedHandler::new(move |args| {
                let source: IPrintDocumentSource = PageSource(source_document.clone()).into();
                args.ok()?.SetSource(&source)
            }))?;
            let done = reply.clone();
            let errors = document.clone();
            task.Completed(&TypedEventHandler::new(move |_, args: windows::core::Ref<PrintTaskCompletedEventArgs>| {
                let result = args.ok().and_then(|args| args.Completion()).and_then(|status| match status {
                    PrintTaskCompletion::Submitted => Ok(true),
                    PrintTaskCompletion::Canceled | PrintTaskCompletion::Abandoned => Ok(false),
                    _ => Err(errors.last_error()),
                });
                finish(&done, result);
                Ok(())
            }))?;
            let options = task.Options()?;
            let ranges = options.PageRangeOptions()?;
            ranges.SetAllowAllPages(true)?;
            ranges.SetAllowCustomSetOfPages(true)?;
            ranges.SetAllowCurrentPage(false)?;
            let displayed = options.DisplayedOptions()?;
            for option in [StandardPrintTaskOptions::MediaSize()?, StandardPrintTaskOptions::CustomPageRanges()?] {
                let mut index = 0;
                if !displayed.IndexOf(&option, &mut index)? { displayed.Append(&option)?; }
            }
            Ok(())
        })();
        if let Err(error) = &result { finish(&reply, Err(Error::new(E_FAIL, format!("创建系统打印任务失败：{error}")))); }
        result
    }))?;
    let _registration = Registration(manager, cookie);
    let shown: IAsyncOperation<bool> = unsafe { interop.ShowPrintUIForWindowAsync(hwnd)? };
    if !shown.await? { return Ok(false); }
    receiver.await.map_err(|_| Error::from_hresult(E_ABORT))?
}

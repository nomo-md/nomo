use crate::document_export::ExportPermit;
use gpui::Window;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, class, define_class, msg_send, sel};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSModalResponseCancel, NSPrintInfo, NSPrintOperation, NSPrintPanelOptions, NSView, NSWindow};
use objc2_foundation::{NSObject, NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{ffi::c_void, path::Path};
use tokio::sync::oneshot;

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" { fn PMSessionError(session: *mut c_void) -> i32; }

struct PrintJob {
    reply: oneshot::Sender<Result<bool, String>>,
    _document: Retained<AnyObject>,
    _operation: Retained<NSPrintOperation>,
    _directory: tempfile::TempDir,
    _permit: ExportPermit,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "NomoPrintCompletion"]
    #[thread_kind = MainThreadOnly]
    struct PrintCompletion;

    impl PrintCompletion {
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn finished(operation: &NSPrintOperation, success: bool, context: *mut c_void) {
            // AppKit calls this selector once, including on cancel. The class delegate outlives the sheet.
            let job = unsafe { Box::from_raw(context.cast::<PrintJob>()) };
            let result = if success {
                Ok(true)
            } else {
                let status = unsafe { PMSessionError(operation.printInfo().PMPrintSession().as_ptr()) };
                match status {
                    0 | -128 => Ok(false), // Closing the panel or kPMCancel.
                    code => Err(format!("系统打印失败（OSStatus {code}）")),
                }
            };
            let _ = job.reply.send(result);
        }
    }
);

pub(super) fn owner(window: &Window) -> Result<Retained<NSWindow>, String> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?.as_raw()
    else { return Err("当前窗口没有 AppKit 打印接口".into()); };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window().ok_or_else(|| "打印窗口已经关闭".into())
}

// Closing the originating document also cancels its pending print sheet.
struct CancelSheet<'a> { owner: &'a NSWindow, sheet: Option<Retained<NSWindow>> }
impl Drop for CancelSheet<'_> {
    fn drop(&mut self) {
        if let Some(sheet) = self.sheet.take()
            && self.owner.attachedSheet().as_deref() == Some(&*sheet) {
            self.owner.endSheet_returnCode(&sheet, NSModalResponseCancel);
        }
    }
}

pub(super) async fn print(path: &Path, title: &str, owner: &NSWindow, dir: tempfile::TempDir, permit: ExportPermit) -> Result<bool, String> {
    MainThreadMarker::new().ok_or("打印面板需要主线程")?;
    if owner.attachedSheet().is_some() { return Err("请先关闭当前窗口中的面板".into()); }
    let url = NSURL::from_file_path(path).ok_or("打印稿路径无效")?;
    let allocated: Allocated<AnyObject> = unsafe { msg_send![class!(PDFDocument), alloc] };
    let document: Option<Retained<AnyObject>> = unsafe { msg_send![allocated, initWithURL: &*url] };
    let document = document.ok_or("无法打开 PDF 打印稿")?;
    let info = NSPrintInfo::sharedPrintInfo();
    let operation: Option<Retained<NSPrintOperation>> = unsafe {
        msg_send![&*document, printOperationForPrintInfo: &*info, scalingMode: 1isize, autoRotate: true]
    };
    let operation = operation.ok_or("无法创建打印任务")?;
    operation.setJobTitle(Some(&NSString::from_str(title)));
    let panel = operation.printPanel();
    panel.setOptions(panel.options() | NSPrintPanelOptions::ShowsPreview | NSPrintPanelOptions::ShowsPageRange
        | NSPrintPanelOptions::ShowsPaperSize | NSPrintPanelOptions::ShowsOrientation | NSPrintPanelOptions::ShowsScaling);
    operation.setShowsPrintPanel(true);
    operation.setCanSpawnSeparateThread(false);
    let (reply, result) = oneshot::channel();
    let job = Box::new(PrintJob { reply, _document: document, _operation: operation.clone(), _directory: dir, _permit: permit });
    unsafe {
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(owner, Some(PrintCompletion::class().as_ref()),
            Some(sel!(printOperationDidRun:success:contextInfo:)), Box::into_raw(job).cast());
    }
    let mut guard = CancelSheet { owner, sheet: owner.attachedSheet() };
    let result = result.await.map_err(|e| e.to_string())?;
    guard.sheet = None;
    result
}

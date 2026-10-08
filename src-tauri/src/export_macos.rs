use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSPaperOrientation, NSPrintInfo, NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob,
    NSPrintingPaginationMode,
};
use objc2_foundation::{NSCopying, NSError, NSSize, NSString, NSURL};
use objc2_web_kit::{WKContentWorld, WKWebView};
use tauri::AppHandle;
use tokio::sync::oneshot;

use crate::models::ExportPdfInput;

const POINTS_PER_MILLIMETER: f64 = 72.0 / 25.4;
const DEFAULT_MARGIN_MM: f64 = 20.0;

pub(crate) async fn render_html_to_pdf(
    app: &AppHandle,
    html_path: &Path,
    output_path: &Path,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let export_window = crate::export::create_pdf_export_window(app, html_path).await?;
    let output_path = output_path.to_path_buf();
    let input = input.clone();
    let (result_tx, result_rx) = oneshot::channel();
    let result_tx = Arc::new(Mutex::new(Some(result_tx)));
    let callback_tx = Arc::clone(&result_tx);

    export_window
        .window()
        .with_webview(move |platform_webview| {
            let result = prepare_and_print_webview(
                platform_webview.inner(),
                output_path,
                input,
                callback_tx,
            );
            if let Err(error) = result {
                send_once(&result_tx, Err(error));
            }
        })
        .map_err(|error| format!("访问 PDF 导出 WKWebView 失败：{error}"))?;

    result_rx
        .await
        .map_err(|_| "等待 macOS PDF 打印时 WebView 已关闭".to_string())?
}

fn prepare_and_print_webview(
    webview_ptr: *mut std::ffi::c_void,
    output_path: std::path::PathBuf,
    input: ExportPdfInput,
    result_tx: Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>,
) -> Result<(), String> {
    if webview_ptr.is_null() {
        return Err("获取 WKWebView 实例失败".to_string());
    }

    let webview = unsafe { &*(webview_ptr.cast::<WKWebView>()) };
    let main_thread =
        MainThreadMarker::new().ok_or_else(|| "PDF 导出必须在 macOS 主线程执行".to_string())?;
    let content_world = unsafe { WKContentWorld::pageWorld(main_thread) };
    let (width, height) = paper_size_points(input.paper_size.as_deref());
    let page_width = if input.orientation.as_deref() == Some("landscape") {
        height
    } else {
        width
    };
    let margins = input.margins.as_ref();
    let content_width = (page_width
        - millimeters_to_points(margins.map(|v| v.left))
        - millimeters_to_points(margins.map(|v| v.right)))
        / POINTS_PER_MILLIMETER;
    let script = NSString::from_str(
        &r#"
if (document.getElementById('nomo-typography-options')) {
  if (!window.__NOMO_PREPARE_PRINT__) throw new Error('Typography runtime unavailable');
  await window.__NOMO_PREPARE_PRINT__(__CONTENT_WIDTH__);
}
if (document.fonts && document.fonts.ready) {
  await document.fonts.ready;
}
await Promise.all(Array.from(document.images).map((image) => {
  if (image.complete) return Promise.resolve();
  return new Promise((resolve) => {
    image.addEventListener('load', resolve, { once: true });
    image.addEventListener('error', resolve, { once: true });
  });
}));
return true;
"#
        .replace("__CONTENT_WIDTH__", &content_width.to_string()),
    );
    let handler = RcBlock::new(move |_value: *mut AnyObject, error: *mut NSError| {
        if !error.is_null() {
            let description = unsafe { &*error }.localizedDescription();
            send_once(
                &result_tx,
                Err(format!(
                    "等待 PDF 页面字体和图片完成失败：{}",
                    description.to_string()
                )),
            );
            return;
        }
        // 打印以模态方式异步运行，结果由 NomoPdfPrintDelegate 回调通知，这里只上报启动失败。
        if let Err(error) = print_webview(webview_ptr, &output_path, &input, &result_tx) {
            send_once(&result_tx, Err(error));
        }
    });
    // evaluateJavaScript 无法返回 Promise；必须由异步接口等待字体和图片加载完成。
    unsafe {
        webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &script,
            None,
            None,
            &content_world,
            Some(&handler),
        );
    }
    Ok(())
}

// 打印任务上下文：在模态打印期间持有打印操作与回调委托，打印完成后由回调统一回收。
struct PrintContext {
    result_tx: Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>,
    _operation: Retained<NSPrintOperation>,
    _delegate: Retained<NomoPdfPrintDelegate>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NomoPdfPrintDelegate"]
    struct NomoPdfPrintDelegate;

    impl NomoPdfPrintDelegate {
        // NSPrintOperation 模态运行的完成回调，在打印线程触发；只做线程安全的结果通知与资源回收。
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn print_operation_did_run(
            &self,
            _operation: &NSPrintOperation,
            success: Bool,
            context_info: *mut c_void,
        ) {
            if context_info.is_null() {
                return;
            }
            let context = unsafe { Box::from_raw(context_info.cast::<PrintContext>()) };
            let result = if success.as_bool() {
                Ok(())
            } else {
                Err("macOS 未能生成 PDF".to_string())
            };
            send_once(&context.result_tx, result);
        }
    }
);

impl NomoPdfPrintDelegate {
    fn new() -> Retained<Self> {
        unsafe { msg_send![Self::alloc(), init] }
    }
}

// runOperation 会在主线程同步阻塞到打印结束，导致整个 app 假死；
// 改用 runOperationModalForWindow 模态运行：主线程 runloop 继续处理事件，完成后经回调拿到结果。
fn print_webview(
    webview_ptr: *mut c_void,
    output_path: &Path,
    input: &ExportPdfInput,
    result_tx: &Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>,
) -> Result<(), String> {
    let webview = unsafe { &*(webview_ptr.cast::<WKWebView>()) };
    let print_info = NSPrintInfo::new();
    configure_print_info(&print_info, output_path, input)?;

    let operation = unsafe { webview.printOperationWithPrintInfo(&print_info) };
    operation.setShowsPrintPanel(false);
    operation.setShowsProgressPanel(false);

    let window = webview
        .window()
        .ok_or_else(|| "获取 PDF 导出窗口失败".to_string())?;
    let delegate = NomoPdfPrintDelegate::new();
    let context = Box::new(PrintContext {
        result_tx: Arc::clone(result_tx),
        _operation: Retained::clone(&operation),
        _delegate: Retained::clone(&delegate),
    });
    let context_ptr = Box::into_raw(context).cast::<c_void>();
    unsafe {
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &window,
            Some(&delegate),
            Some(sel!(printOperationDidRun:success:contextInfo:)),
            context_ptr,
        );
    }
    Ok(())
}

fn configure_print_info(
    print_info: &NSPrintInfo,
    output_path: &Path,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let (width, height) = paper_size_points(input.paper_size.as_deref());
    let margins = input.margins.as_ref();
    let output_path = NSString::from_str(&output_path.to_string_lossy());
    let output_url = NSURL::fileURLWithPath(&output_path);

    print_info.setPaperSize(NSSize::new(width, height));
    print_info.setOrientation(if input.orientation.as_deref() == Some("landscape") {
        NSPaperOrientation::Landscape
    } else {
        NSPaperOrientation::Portrait
    });
    print_info.setTopMargin(millimeters_to_points(margins.map(|value| value.top)));
    print_info.setRightMargin(millimeters_to_points(margins.map(|value| value.right)));
    print_info.setBottomMargin(millimeters_to_points(margins.map(|value| value.bottom)));
    print_info.setLeftMargin(millimeters_to_points(margins.map(|value| value.left)));
    print_info.setHorizontallyCentered(false);
    print_info.setVerticallyCentered(false);
    print_info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
    print_info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
    print_info.setJobDisposition(unsafe { NSPrintSaveJob });

    let settings = unsafe { print_info.dictionary() };
    let saving_url_key: &ProtocolObject<dyn NSCopying> =
        ProtocolObject::from_ref(unsafe { NSPrintJobSavingURL });
    let output_url: &AnyObject = &*output_url;
    unsafe {
        settings.setObject_forKey(output_url, saving_url_key);
    }

    Ok(())
}

fn paper_size_points(paper_size: Option<&str>) -> (f64, f64) {
    match paper_size {
        Some(value) if value.eq_ignore_ascii_case("letter") => (612.0, 792.0),
        Some(value) if !value.eq_ignore_ascii_case("a4") => {
            crate::app_logger::warn("Export", &format!("暂不支持的纸张大小：{value}，使用 A4"));
            (210.0 * POINTS_PER_MILLIMETER, 297.0 * POINTS_PER_MILLIMETER)
        }
        _ => (210.0 * POINTS_PER_MILLIMETER, 297.0 * POINTS_PER_MILLIMETER),
    }
}

fn millimeters_to_points(value: Option<f64>) -> f64 {
    let value = value.filter(|value| value.is_finite() && *value >= 0.0);
    value.unwrap_or(DEFAULT_MARGIN_MM) * POINTS_PER_MILLIMETER
}

fn send_once(
    sender: &Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>,
    result: Result<(), String>,
) {
    if let Ok(mut sender) = sender.lock() {
        if let Some(sender) = sender.take() {
            let _ = sender.send(result);
        }
    }
}

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::AppHandle;
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment6, ICoreWebView2_7, COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE,
    COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT,
};
use webview2_com::{
    CoTaskMemPWSTR, DocumentTitleChangedEventHandler, ExecuteScriptCompletedHandler,
    PrintToPdfCompletedHandler,
};
use windows_core::{Interface, HSTRING, PWSTR};

use crate::models::ExportPdfInput;

const MILLIMETERS_PER_INCH: f64 = 25.4;
const DEFAULT_MARGIN_MM: f64 = 20.0;
static DOCUMENT_READY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn render_html_to_pdf(
    app: &AppHandle,
    html_path: &Path,
    output_path: &Path,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let export_window = crate::export::create_pdf_export_window(app, html_path).await?;
    wait_for_document_ready(export_window.window(), input).await?;
    print_webview_to_pdf(export_window.window(), output_path, input).await
}

async fn wait_for_document_ready(
    window: &tauri::WebviewWindow,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let (width, height) = paper_size_inches(input.paper_size.as_deref());
    let page_width = if input.orientation.as_deref() == Some("landscape") {
        height
    } else {
        width
    };
    let margins = input.margins.as_ref();
    let content_width = (page_width
        - millimeters_to_inches(margins.map(|v| v.left))
        - millimeters_to_inches(margins.map(|v| v.right)))
        * MILLIMETERS_PER_INCH;
    let sequence = DOCUMENT_READY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let ready_title = format!("nomo-pdf-document-ready-{sequence}");
    let error_title = format!("nomo-pdf-document-error-{sequence}");
    let script = r#"
(async () => {
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
  document.title = '__READY_TITLE__';
})().catch(() => {
  document.title = '__ERROR_TITLE__';
})
"#
    .replace("__CONTENT_WIDTH__", &content_width.to_string())
    .replace("__READY_TITLE__", &ready_title)
    .replace("__ERROR_TITLE__", &error_title);
    let (result_tx, result_rx) = oneshot::channel();
    let result_tx = Arc::new(Mutex::new(Some(result_tx)));
    let callback_tx = Arc::clone(&result_tx);
    let callback_ready_title = ready_title.clone();
    let callback_error_title = error_title.clone();

    window
        .with_webview(move |platform_webview| {
            let operation = (|| -> Result<(), String> {
                let controller = platform_webview.controller();
                let core = unsafe { controller.CoreWebView2() }
                    .map_err(|error| format!("获取 WebView2 实例失败：{error}"))?;
                let title_tx = Arc::clone(&callback_tx);
                let title_handler =
                    DocumentTitleChangedEventHandler::create(Box::new(move |webview, _args| {
                        let Some(webview) = webview else {
                            return Ok(());
                        };
                        let mut title = PWSTR::null();
                        if unsafe { webview.DocumentTitle(&mut title) }.is_err() {
                            return Ok(());
                        }
                        let title = CoTaskMemPWSTR::from(title).to_string();
                        if title == callback_ready_title {
                            send_once(&title_tx, Ok(()));
                        } else if title == callback_error_title {
                            send_once(
                                &title_tx,
                                Err("等待 PDF 页面字体和图片完成失败".to_string()),
                            );
                        }
                        Ok(())
                    }));
                let mut title_token = 0;
                unsafe { core.add_DocumentTitleChanged(&title_handler, &mut title_token) }
                    .map_err(|error| format!("监听 PDF 页面资源状态失败：{error}"))?;

                let script = HSTRING::from(script);
                let execute_tx = Arc::clone(&callback_tx);
                let execute_handler =
                    ExecuteScriptCompletedHandler::create(Box::new(move |status, _result| {
                        if let Err(error) = status {
                            send_once(
                                &execute_tx,
                                Err(format!("执行 PDF 页面资源检查失败：{error}")),
                            );
                        }
                        Ok(())
                    }));
                unsafe { core.ExecuteScript(&script, &execute_handler) }
                    .map_err(|error| format!("执行 PDF 页面资源检查失败：{error}"))
            })();

            if let Err(error) = operation {
                send_once(&result_tx, Err(error));
            }
        })
        .map_err(|error| format!("访问 PDF 导出 WebView 失败：{error}"))?;

    result_rx
        .await
        .map_err(|_| "等待 PDF 页面资源时 WebView 已关闭".to_string())?
}

async fn print_webview_to_pdf(
    window: &tauri::WebviewWindow,
    output_path: &Path,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let output_path = output_path.to_path_buf();
    let input = input.clone();
    let (result_tx, result_rx) = oneshot::channel();
    let result_tx = Arc::new(Mutex::new(Some(result_tx)));
    let callback_tx = Arc::clone(&result_tx);

    window
        .with_webview(move |platform_webview| {
            let operation = (|| -> Result<(), String> {
                let controller = platform_webview.controller();
                let core = unsafe { controller.CoreWebView2() }
                    .map_err(|error| format!("获取 WebView2 实例失败：{error}"))?;
                let environment = platform_webview.environment();
                let environment6: ICoreWebView2Environment6 = environment
                    .cast()
                    .map_err(|error| format!("当前 WebView2 Runtime 不支持 PDF 打印：{error}"))?;
                let print_settings = unsafe { environment6.CreatePrintSettings() }
                    .map_err(|error| format!("创建 WebView2 打印设置失败：{error}"))?;

                configure_print_settings(&print_settings, &input)?;

                let core7: ICoreWebView2_7 = core
                    .cast()
                    .map_err(|error| format!("当前 WebView2 Runtime 不支持 PDF 打印：{error}"))?;
                let output_path = HSTRING::from(output_path.as_path());
                let handler =
                    PrintToPdfCompletedHandler::create(Box::new(move |status, succeeded| {
                        let outcome = status
                            .map_err(|error| format!("WebView2 PDF 打印失败：{error}"))
                            .and_then(|_| {
                                if succeeded {
                                    Ok(())
                                } else {
                                    Err("WebView2 未能生成 PDF".to_string())
                                }
                            });
                        send_once(&callback_tx, outcome);
                        Ok(())
                    }));

                unsafe { core7.PrintToPdf(&output_path, &print_settings, &handler) }
                    .map_err(|error| format!("启动 WebView2 PDF 打印失败：{error}"))
            })();

            if let Err(error) = operation {
                send_once(&result_tx, Err(error));
            }
        })
        .map_err(|error| format!("访问 PDF 导出 WebView 失败：{error}"))?;

    result_rx
        .await
        .map_err(|_| "等待 WebView2 PDF 打印时 WebView 已关闭".to_string())?
}

fn configure_print_settings(
    settings: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2PrintSettings,
    input: &ExportPdfInput,
) -> Result<(), String> {
    let (page_width, page_height) = paper_size_inches(input.paper_size.as_deref());
    let landscape = input.orientation.as_deref() == Some("landscape");
    let margins = input.margins.as_ref();
    let margin_top = millimeters_to_inches(margins.map(|value| value.top));
    let margin_right = millimeters_to_inches(margins.map(|value| value.right));
    let margin_bottom = millimeters_to_inches(margins.map(|value| value.bottom));
    let margin_left = millimeters_to_inches(margins.map(|value| value.left));

    unsafe {
        settings
            .SetOrientation(if landscape {
                COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE
            } else {
                COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT
            })
            .map_err(|error| format!("设置 PDF 页面方向失败：{error}"))?;
        settings
            .SetPageWidth(page_width)
            .map_err(|error| format!("设置 PDF 页面宽度失败：{error}"))?;
        settings
            .SetPageHeight(page_height)
            .map_err(|error| format!("设置 PDF 页面高度失败：{error}"))?;
        settings
            .SetMarginTop(margin_top)
            .map_err(|error| format!("设置 PDF 上边距失败：{error}"))?;
        settings
            .SetMarginRight(margin_right)
            .map_err(|error| format!("设置 PDF 右边距失败：{error}"))?;
        settings
            .SetMarginBottom(margin_bottom)
            .map_err(|error| format!("设置 PDF 下边距失败：{error}"))?;
        settings
            .SetMarginLeft(margin_left)
            .map_err(|error| format!("设置 PDF 左边距失败：{error}"))?;
        settings
            .SetShouldPrintBackgrounds(input.print_background.unwrap_or(true))
            .map_err(|error| format!("设置 PDF 背景打印失败：{error}"))?;
        settings
            .SetShouldPrintHeaderAndFooter(false)
            .map_err(|error| format!("关闭 PDF 页眉页脚失败：{error}"))?;
    }
    Ok(())
}

fn paper_size_inches(paper_size: Option<&str>) -> (f64, f64) {
    match paper_size {
        Some(value) if value.eq_ignore_ascii_case("letter") => (8.5, 11.0),
        Some(value) if !value.eq_ignore_ascii_case("a4") => {
            crate::app_logger::warn("Export", &format!("暂不支持的纸张大小：{value}，使用 A4"));
            (210.0 / MILLIMETERS_PER_INCH, 297.0 / MILLIMETERS_PER_INCH)
        }
        _ => (210.0 / MILLIMETERS_PER_INCH, 297.0 / MILLIMETERS_PER_INCH),
    }
}

fn millimeters_to_inches(value: Option<f64>) -> f64 {
    let value = value.filter(|value| value.is_finite() && *value >= 0.0);
    value.unwrap_or(DEFAULT_MARGIN_MM) / MILLIMETERS_PER_INCH
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

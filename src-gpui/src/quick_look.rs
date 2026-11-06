//! 沙箱 Quick Look 的 C ABI：仅接收传入正文；渲染可读系统字体，不打开文档、网络或编辑会话。
#![cfg(all(target_os = "macos", feature = "quick-look"))]

#[allow(dead_code)]
#[path = "embeds_native.rs"]
mod native;
mod quick_look_model;

use std::{ffi::{CString, c_char, c_void}, panic::{AssertUnwindSafe, catch_unwind}};

/// 调用方保持输入和取消上下文存活直到返回；返回值必须由 nomo_preview_free 释放。
/// 空指针只表示桥接层 panic；正常失败以 JSON error 返回，不跨 FFI 展开栈。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nomo_preview_create(
    bytes: *const u8, length: usize, dark: bool,
    cancelled: Option<unsafe extern "C" fn(*mut c_void) -> bool>, context: *mut c_void,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let result = if (bytes.is_null() && length != 0) || length > quick_look_model::MAX_INPUT {
            Err("Quick Look supports Markdown files up to 1 MiB.".into())
        } else {
            let bytes = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } };
            quick_look_model::preview(bytes, dark, || cancelled.is_some_and(|f| unsafe { f(context) }))
        };
        let json = match result {
            Ok(events) => serde_json::json!({"events": events}),
            Err(error) => serde_json::json!({"error": error}),
        };
        CString::new(json.to_string()).expect("JSON escapes NUL").into_raw()
    })).unwrap_or(std::ptr::null_mut())
}

/// 只能传入本库返回且尚未释放的指针；允许空指针。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nomo_preview_free(value: *mut c_char) {
    if !value.is_null() { drop(unsafe { CString::from_raw(value) }); }
}

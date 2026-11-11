//! 从用户触发的菜单定位文件或文件夹。
use gpui::Context;
use std::path::PathBuf;

pub(crate) fn reveal<T: 'static>(
    path: PathBuf,
    cx: &mut Context<T>,
    on_error: impl FnOnce(&mut T, String, &mut Context<T>) + 'static,
) {
    #[cfg(windows)]
    {
        use super::toolbar_i18n::ToolbarLocale;

        let task = cx
            .background_executor()
            .spawn(async move { reveal_in_explorer(&path) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                let _ = this.update(cx, |owner, cx| {
                    let message = ToolbarLocale::current(cx)
                        .app_format("workspace.openFailed", &[("error", &error)]);
                    on_error(owner, message, cx);
                });
            }
        })
        .detach();
    }

    #[cfg(not(windows))]
    {
        let _ = on_error;
        cx.reveal_path(&path);
    }
}

#[cfg(windows)]
fn reveal_in_explorer(path: &std::path::Path) -> Result<(), String> {
    use std::ptr::null_mut;
    use windows::{
        Win32::{
            System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName},
        },
        core::PCWSTR,
    };

    let path = explorer_path(path).map_err(|error| error.to_string())?;
    // GPUI 的线程池不初始化 COM，整个 Shell 调用需在同一线程内完成。
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
        .map_err(|error| error.to_string())?;
    let mut item = null_mut();
    let result = unsafe {
        SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut item, 0, None)
            // 不传子项数组时，Shell 会从目标的绝对 PIDL 打开父目录并选中目标。
            .and_then(|()| SHOpenFolderAndSelectItems(item, None, 0))
    }
    .map_err(|error| error.to_string());
    unsafe {
        CoTaskMemFree(Some(item.cast()));
        CoUninitialize();
    }
    result
}

#[cfg(windows)]
fn explorer_path(path: &std::path::Path) -> std::io::Result<Vec<u16>> {
    use std::{
        os::windows::ffi::OsStrExt,
        path::{Component, Prefix},
    };

    let path = std::path::absolute(path)?;
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        let code = windows::Win32::Foundation::ERROR_INVALID_NAME;
        return Err(std::io::Error::from_raw_os_error(code.0 as i32));
    }
    // canonicalize 产生的扩展路径前缀不被 Shell 解析器接受。
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        match prefix.kind() {
            Prefix::VerbatimDisk(_) => {
                wide.drain(..r"\\?\".len());
            }
            Prefix::VerbatimUNC(..) => {
                wide.drain(2..r"\\?\UNC\".len());
            }
            _ => {}
        }
    }
    wide.push(0);
    Ok(wide)
}

//! Stream the release tar through macOS libarchive. Never let the archive choose
//! filesystem operations (links, ACLs, xattrs, ownership or implicit directories).
use super::{DownloadCancel, UiMessage, check_cancel, validate_archive_path};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::Path,
    ptr,
};

#[link(name = "archive")]
unsafe extern "C" {
    fn archive_read_new() -> *mut c_void;
    fn archive_read_free(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_gzip(a: *mut c_void) -> c_int;
    fn archive_read_support_format_tar(a: *mut c_void) -> c_int;
    fn archive_read_open_filename(a: *mut c_void, path: *const c_char, block: usize) -> c_int;
    fn archive_read_next_header(a: *mut c_void, e: *mut *mut c_void) -> c_int;
    fn archive_entry_pathname(e: *mut c_void) -> *const c_char;
    fn archive_entry_hardlink(e: *mut c_void) -> *const c_char;
    fn archive_entry_filetype(e: *mut c_void) -> u16;
    fn archive_entry_perm(e: *mut c_void) -> u16;
    fn archive_entry_size(e: *mut c_void) -> i64;
    fn archive_read_data(a: *mut c_void, buf: *mut c_void, len: usize) -> isize;
}
struct Reader(*mut c_void);
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            archive_read_free(self.0);
        }
    }
}
fn invalid() -> UiMessage {
    UiMessage::new("installer.invalidAsset")
}

pub(super) fn extract_archive(
    path: &Path,
    root: &Path,
    cancel: &DownloadCancel,
    bytes: u64,
    entries: usize,
) -> Result<(), UiMessage> {
    extract_checked(path, root, bytes, entries, || check_cancel(Some(cancel)))
}

pub(super) fn extract_checked(
    path: &Path,
    root: &Path,
    mut bytes: u64,
    mut entries: usize,
    mut check: impl FnMut() -> Result<(), UiMessage>,
) -> Result<(), UiMessage> {
    check()?;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(UiMessage::raw)?;
    let mut buf = [0u8; 64 * 1024];
    // Reader owns the opaque handle. Entry and pathname are borrowed only until
    // the next header. No libarchive pointer is shared or retained by a file.
    unsafe {
        let raw = archive_read_new();
        if raw.is_null() {
            return Err(invalid());
        }
        let a = Reader(raw);
        if archive_read_support_filter_gzip(a.0) != 0
            || archive_read_support_format_tar(a.0) != 0
            || archive_read_open_filename(a.0, path.as_ptr(), buf.len()) != 0
        {
            return Err(invalid());
        }
        loop {
            check()?;
            let mut e = ptr::null_mut();
            match archive_read_next_header(a.0, &mut e) {
                1 => return Ok(()),
                0 if !e.is_null() => {}
                _ => return Err(invalid()),
            }
            entries = entries.checked_sub(1).ok_or_else(invalid)?;
            let name = archive_entry_pathname(e);
            if name.is_null() || !archive_entry_hardlink(e).is_null() {
                return Err(invalid());
            }
            let name = CStr::from_ptr(name).to_str().map_err(|_| invalid())?;
            validate_archive_path(name)?;
            let dest = root.join(name);
            let size = u64::try_from(archive_entry_size(e)).map_err(|_| invalid())?;
            match archive_entry_filetype(e) {
                0o040000 if size == 0 => {
                    fs::create_dir(dest).map_err(UiMessage::raw)?;
                }
                0o100000 if size <= bytes => {
                    let mut file = fs::File::create_new(dest).map_err(UiMessage::raw)?;
                    let mut left = size;
                    loop {
                        check()?;
                        let n = usize::try_from(archive_read_data(
                            a.0,
                            buf.as_mut_ptr().cast(),
                            buf.len(),
                        ))
                        .map_err(|_| invalid())?;
                        if n == 0 {
                            break;
                        }
                        left = left.checked_sub(n as u64).ok_or_else(invalid)?;
                        bytes -= n as u64;
                        file.write_all(&buf[..n]).map_err(UiMessage::raw)?;
                    }
                    if left != 0 {
                        return Err(invalid());
                    }
                    let mode = if archive_entry_perm(e) & 0o111 != 0 {
                        0o755
                    } else {
                        0o644
                    };
                    file.set_permissions(fs::Permissions::from_mode(mode))
                        .map_err(UiMessage::raw)?;
                }
                _ => return Err(invalid()),
            }
        }
    }
}

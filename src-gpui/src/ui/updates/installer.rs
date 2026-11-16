//! 只接受官方 Release 中与版本、架构、SHA-256 精确匹配的 Windows 安装包。
use super::Release;
use crate::ui::toolbar_i18n::UiMessage;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions}, io::{self, Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt}, path::{Path, PathBuf}, process::Command,
    sync::atomic::{AtomicBool, Ordering}, time::Duration,
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

const LIMIT: u64 = 128 * 1024 * 1024;
const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo";

pub(super) fn installed() -> bool {
    let Ok(exe) = std::env::current_exe().and_then(fs::canonicalize) else { return false; };
    let Ok(dir) = windows_registry::CURRENT_USER.open(UNINSTALL).and_then(|key| key.get_string("InstallLocation")) else { return false; };
    let dir = Path::new(dir.trim().trim_matches('"'));
    dir.is_absolute() && dir.join("uninstall.exe").is_file() && fs::canonicalize(dir.join("nomo.exe")).is_ok_and(|registered| registered == exe)
}

pub(super) struct Candidate { pub version: String, url: String, size: u64, digest: [u8; 32] }
impl Candidate {
    pub(super) fn from_release(release: &Release) -> Result<Self, UiMessage> {
        let (major, minor, patch) = super::version(&release.tag_name).ok_or(UiMessage::new("installer.version"))?;
        let version = format!("{major}.{minor}.{patch}");
        if release.tag_name != format!("v{version}") || Some((major,minor,patch)) <= super::version(env!("CARGO_PKG_VERSION")) {
            return Err(UiMessage::new("installer.newerOnly"));
        }
        if !cfg!(target_arch="x86_64") { return Err(UiMessage::new("installer.architecture")); }
        let name = format!("Nomo_{version}_x64-setup.exe");
        let url = format!("https://github.com/nomo-md/nomo/releases/download/v{version}/{name}");
        let mut assets = release.assets.iter().filter(|asset| asset.name == name);
        let asset = assets.next().ok_or(UiMessage::new("installer.missing"))?;
        if assets.next().is_some() || asset.browser_download_url != url || !(1..=LIMIT).contains(&asset.size) {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
        let hex = asset.digest.as_deref().and_then(|s| s.strip_prefix("sha256:"))
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).ok_or(UiMessage::new("installer.digestMissing"))?;
        let mut digest = [0; 32];
        for (i, slot) in digest.iter_mut().enumerate() { *slot = u8::from_str_radix(&hex[i*2..i*2+2], 16).expect("validated hex"); }
        Ok(Self { version, url, size: asset.size, digest })
    }
}

fn cache_directory(create: bool) -> Result<Option<PathBuf>, UiMessage> {
    let base = crate::settings::config_path().and_then(|p| p.parent().map(Path::to_path_buf)).ok_or(UiMessage::new("installer.directory"))?;
    let dir = base.join("native-updates");
    if create { fs::create_dir_all(&dir).map_err(|e| UiMessage::new("installer.createDirectory").arg("error", e))?; }
    let meta = match fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(UiMessage::raw(error)),
    };
    if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
        return Err(UiMessage::raw("更新缓存目录不是普通目录，已保留"));
    }
    Ok(Some(dir.canonicalize().map_err(UiMessage::raw)?))
}

fn cache_lease(directory: &Path) -> Result<Option<File>, UiMessage> {
    let path = directory.join(".active");
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || meta.file_attributes() & 0x400 != 0 =>
            return Err(UiMessage::raw("更新缓存锁不是普通文件，已保留")),
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(UiMessage::raw(error)),
        _ => {},
    }
    let lease = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .share_mode(FILE_SHARE_READ | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE)
        .open(path).map_err(UiMessage::raw)?;
    match lease.try_lock() {
        Ok(()) => Ok(Some(lease)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(UiMessage::raw(error)),
    }
}

pub(super) fn clean_stale() -> Result<(), UiMessage> {
    let Some(directory) = cache_directory(false)? else { return Ok(()); };
    let Some(_lease) = cache_lease(&directory)? else { return Ok(()); };
    let path = directory.join("nomo-update-x64-setup.exe");
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(UiMessage::raw(error)),
    };
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 { return Ok(()); }
    // 兼容尚无缓存租约的旧版本，避开下载结束到校验打开之间的短暂窗口。
    if meta.modified().map_err(UiMessage::raw)?.elapsed().unwrap_or_default() < Duration::from_secs(24 * 60 * 60) {
        return Ok(());
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        // Windows 拒绝删除仍在下载、校验或运行的安装包；下次启动再检查。
        Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => Ok(()),
        Err(error) => Err(UiMessage::raw(error)),
    }
}

// 租约覆盖下载、校验、待安装及本对象的清理，防止另一进程在句柄切换时删除文件。
struct StagedPath { path: PathBuf, _lease: File }
impl Drop for StagedPath {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) && error.kind() != io::ErrorKind::NotFound {
            crate::app_logger::warn("UpdateCache", &format!("更新包暂未删除，后续启动将重试：{error}"));
        }
    }
}
pub(super) struct Verified {
    pub version: String,
    // 以只读、拒绝写入和删除的方式重新打开并校验，持有到子进程创建完成。
    _lock: File,
    path: StagedPath,
}
impl Verified {
    pub(super) fn launch(&self) -> Result<(), UiMessage> {
        if !installed() { return Err(UiMessage::new("installer.locationChanged")); }
        Command::new(&self.path.path).current_dir(self.path.path.parent().expect("cache directory"))
            .arg(format!("/NOMOUPDATE={}", std::process::id())).args(["/P", "/R"])
            .spawn().map_err(|e| UiMessage::new("installer.launchFailed").arg("error", e))?;
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct DownloadCancel { flag: AtomicBool, wake: tokio::sync::Notify }
impl DownloadCancel {
    pub(super) fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        self.wake.notify_one();
    }
    pub(super) fn cancelled(&self) -> bool { self.flag.load(Ordering::Acquire) }
    fn check(&self) -> Result<(), UiMessage> {
        if self.cancelled() { Err(UiMessage::new("installer.cancelled")) } else { Ok(()) }
    }
}
pub(super) fn download(candidate: Candidate, cancel: &DownloadCancel, progress: impl Fn(u8)) -> Result<Verified, UiMessage> {
    cancel.check()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let directory = cache_directory(true)?.expect("created cache directory");
    let lease = cache_lease(&directory)?.ok_or_else(|| UiMessage::raw("另一进程正在使用更新缓存，请稍后重试"))?;
    let path = directory.join("nomo-update-x64-setup.exe");
    // 不能截断已有链接指向的文件；只解除固定文件名，再排他创建。
    if let Err(e) = fs::remove_file(&path) && e.kind() != io::ErrorKind::NotFound { return Err(UiMessage::new("installer.cleanupFailed").arg("error", e)); }
    let mut writer = OpenOptions::new().write(true).create_new(true).share_mode(0).open(&path).map_err(|e| e.to_string())?;
    let path = StagedPath { path, _lease: lease };
    let transfer = async {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(25))
            .timeout(Duration::from_secs(300)).redirect(reqwest::redirect::Policy::custom(|attempt| {
                let u = attempt.url();
                if attempt.previous().len() <= 4 && u.scheme() == "https" && u.port().is_none() && u.username().is_empty() && u.password().is_none()
                    && matches!(u.host_str(), Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")) { attempt.follow() }
                else { attempt.error(UiMessage::new("installer.redirect")) }
            })).build().map_err(|e| e.to_string())?;
        let mut response = client.get(&candidate.url).header("User-Agent", concat!("Nomo/", env!("CARGO_PKG_VERSION")))
            .send().await.map_err(download_error)?.error_for_status().map_err(download_error)?;
        if response.status() != reqwest::StatusCode::OK || response.content_length().is_some_and(|size| size != candidate.size) {
            return Err(UiMessage::new("installer.response"));
        }
        let mut total = 0u64; let mut previous = 0;
        while let Some(chunk) = response.chunk().await.map_err(download_error)? {
            cancel.check()?; total += chunk.len() as u64;
            if total > candidate.size { return Err(UiMessage::new("installer.oversized")); }
            writer.write_all(&chunk).map_err(|e| e.to_string())?;
            let percent = (total * 100 / candidate.size).min(99) as u8;
            if percent >= previous + 3 { previous = percent; progress(percent); }
        }
        if total != candidate.size { return Err(UiMessage::new("installer.incomplete")); }
        writer.flush().map_err(UiMessage::raw)
    };
    let result = runtime.block_on(async {
        let mut stopped = std::pin::pin!(cancel.wake.notified());
        let mut transfer = std::pin::pin!(transfer);
        std::future::poll_fn(|cx| {
            if stopped.as_mut().poll(cx).is_ready() { std::task::Poll::Ready(Err(UiMessage::new("installer.cancelled"))) }
            else { transfer.as_mut().poll(cx) }
        }).await
    });
    drop(writer);
    result?;
    let mut lock = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(&path.path).map_err(|e| e.to_string())?;
    if lock.metadata().map_err(|e| e.to_string())?.len() != candidate.size { return Err(UiMessage::new("installer.sizeFailed")); }
    let mut hash = Sha256::new(); let mut buf = [0u8; 32 * 1024];
    loop {
        cancel.check()?;
        let count = lock.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hash.update(&buf[..count]);
    }
    if <[u8; 32]>::from(hash.finalize()) != candidate.digest { return Err(UiMessage::new("installer.digestFailed")); }
    Ok(Verified { version: candidate.version, _lock: lock, path })
}

// 保留应用自有错误键；reqwest 的其他详情仍先剥离 URL，再交给显示层。
fn download_error(error: reqwest::Error) -> UiMessage {
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        if let Some(message) = cause.downcast_ref::<UiMessage>() { return message.clone(); }
        source = cause.source();
    }
    UiMessage::raw(error.without_url())
}

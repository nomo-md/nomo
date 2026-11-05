//! 只接受官方 Release 中与版本、架构、SHA-256 精确匹配的 Windows 安装包。
use super::Release;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions}, io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt, path::{Path, PathBuf}, process::Command,
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
    pub(super) fn from_release(release: &Release) -> Result<Self, String> {
        let (major, minor, patch) = super::version(&release.tag_name).ok_or("发布版本无效。")?;
        let version = format!("{major}.{minor}.{patch}");
        if release.tag_name != format!("v{version}") || Some((major,minor,patch)) <= super::version(env!("CARGO_PKG_VERSION")) {
            return Err("只能安装比当前版本更新的稳定版。".into());
        }
        if !cfg!(target_arch="x86_64") { return Err("此架构暂无匹配的 Windows 安装包。".into()); }
        let name = format!("Nomo_{version}_x64-setup.exe");
        let url = format!("https://github.com/nomo-md/nomo/releases/download/v{version}/{name}");
        let mut assets = release.assets.iter().filter(|asset| asset.name == name);
        let asset = assets.next().ok_or("此版本尚未发布匹配的 Windows 安装包。")?;
        if assets.next().is_some() || asset.browser_download_url != url || !(1..=LIMIT).contains(&asset.size) {
            return Err("安装包来源或大小无效，已停止更新。".into());
        }
        let hex = asset.digest.as_deref().and_then(|s| s.strip_prefix("sha256:"))
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).ok_or("安装包缺少有效的 SHA-256，已停止更新。")?;
        let mut digest = [0; 32];
        for (i, slot) in digest.iter_mut().enumerate() { *slot = u8::from_str_radix(&hex[i*2..i*2+2], 16).expect("validated hex"); }
        Ok(Self { version, url, size: asset.size, digest })
    }
}

fn cache_path() -> Result<PathBuf, String> {
    let base = crate::settings::config_path().and_then(|p| p.parent().map(Path::to_path_buf)).ok_or("无法确定更新目录。")?;
    let dir = base.join("native-updates");
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建更新目录：{e}"))?;
    Ok(dir.canonicalize().map_err(|e| e.to_string())?.join("nomo-update-x64-setup.exe"))
}

// 只清理这个受控文件；崩溃或安装器占用时留下的缓存也不超过一个安装包。
struct StagedPath(PathBuf);
impl Drop for StagedPath {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}
pub(super) struct Verified {
    pub version: String,
    // 以只读、拒绝写入和删除的方式重新打开并校验，持有到子进程创建完成。
    _lock: File,
    path: StagedPath,
}
impl Verified {
    pub(super) fn launch(&self) -> Result<(), String> {
        if !installed() { return Err("当前程序与 Windows 注册的 Nomo 安装目录不一致，已取消安装。".into()); }
        Command::new(&self.path.0).current_dir(self.path.0.parent().expect("cache directory"))
            .arg(format!("/NOMOUPDATE={}", std::process::id())).args(["/P", "/R"])
            .spawn().map_err(|e| format!("无法启动更新安装器：{e}"))?;
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
    fn check(&self) -> Result<(), String> {
        if self.cancelled() { Err("已取消下载。".into()) } else { Ok(()) }
    }
}
pub(super) fn download(candidate: Candidate, cancel: &DownloadCancel, progress: impl Fn(u8)) -> Result<Verified, String> {
    cancel.check()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let path = cache_path()?;
    // 不能截断已有链接指向的文件；只解除固定文件名，再排他创建。
    if let Err(e) = fs::remove_file(&path) && e.kind() != io::ErrorKind::NotFound { return Err(format!("更新文件正在使用或无法清理：{e}")); }
    let mut writer = OpenOptions::new().write(true).create_new(true).share_mode(0).open(&path).map_err(|e| e.to_string())?;
    let path = StagedPath(path);
    let transfer = async {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(25))
            .timeout(Duration::from_secs(300)).redirect(reqwest::redirect::Policy::custom(|attempt| {
                let u = attempt.url();
                if attempt.previous().len() <= 4 && u.scheme() == "https" && u.port().is_none() && u.username().is_empty() && u.password().is_none()
                    && matches!(u.host_str(), Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")) { attempt.follow() }
                else { attempt.error("安装包重定向不属于官方发布服务") }
            })).build().map_err(|e| e.to_string())?;
        let mut response = client.get(&candidate.url).header("User-Agent", concat!("Nomo/", env!("CARGO_PKG_VERSION")))
            .send().await.map_err(|e| e.without_url().to_string())?.error_for_status().map_err(|e| e.without_url().to_string())?;
        if response.status() != reqwest::StatusCode::OK || response.content_length().is_some_and(|size| size != candidate.size) {
            return Err("安装包响应或大小与发布信息不符。".to_owned());
        }
        let mut total = 0u64; let mut previous = 0;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
            cancel.check()?; total += chunk.len() as u64;
            if total > candidate.size { return Err("安装包超过发布大小。".into()); }
            writer.write_all(&chunk).map_err(|e| e.to_string())?;
            let percent = (total * 100 / candidate.size).min(99) as u8;
            if percent >= previous + 3 { previous = percent; progress(percent); }
        }
        if total != candidate.size { return Err("安装包下载不完整。".into()); }
        writer.flush().map_err(|e| e.to_string())
    };
    let result = runtime.block_on(async {
        let mut stopped = std::pin::pin!(cancel.wake.notified());
        let mut transfer = std::pin::pin!(transfer);
        std::future::poll_fn(|cx| {
            if stopped.as_mut().poll(cx).is_ready() { std::task::Poll::Ready(Err("已取消下载。".into())) }
            else { transfer.as_mut().poll(cx) }
        }).await
    });
    drop(writer);
    result?;
    let mut lock = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(&path.0).map_err(|e| e.to_string())?;
    if lock.metadata().map_err(|e| e.to_string())?.len() != candidate.size { return Err("安装包大小校验失败。".into()); }
    let mut hash = Sha256::new(); let mut buf = [0u8; 32 * 1024];
    loop {
        cancel.check()?;
        let count = lock.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hash.update(&buf[..count]);
    }
    if <[u8; 32]>::from(hash.finalize()) != candidate.digest { return Err("安装包 SHA-256 校验失败，已放弃此文件。".into()); }
    Ok(Verified { version: candidate.version, _lock: lock, path })
}

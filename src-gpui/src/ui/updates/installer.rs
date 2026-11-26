//! 共享安装包候选、官方优先的镜像竞速和 SHA-256 校验；安装动作由平台适配器完成。
use super::Release;
use crate::ui::toolbar_i18n::UiMessage;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "linux")]
pub(super) use linux::{LinuxPackage, restart};

#[cfg(target_os = "macos")]
mod macos;
// 在宿主测试 Windows 缓存文件名和启动命令，不执行安装器。
#[cfg(any(windows, test))]
mod windows;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

const PROXY_PREFIX: &str = "https://gh-proxy.com/";
const HEDGE_DELAY: Duration = Duration::from_secs(8);

#[cfg(any(windows, target_os = "macos"))]
pub(super) fn installed() -> bool {
    platform::installed()
}
#[cfg(target_os = "macos")]
pub(crate) fn run_command() -> Option<i32> {
    platform::run_command()
}

pub(super) struct Candidate {
    pub version: String,
    #[cfg(target_os = "linux")]
    package: LinuxPackage,
    url: String,
    size: u64,
    digest: [u8; 32],
}

impl Candidate {
    pub(super) fn from_release(release: &Release) -> Result<Self, UiMessage> {
        let (major, minor, patch) =
            super::version(&release.tag_name).ok_or(UiMessage::new("installer.version"))?;
        let version = format!("{major}.{minor}.{patch}");
        if release.tag_name != format!("v{version}")
            || Some((major, minor, patch)) <= super::version(env!("CARGO_PKG_VERSION"))
        {
            return Err(UiMessage::new("installer.newerOnly"));
        }
        #[cfg(target_os = "linux")]
        let package = release.linux_package.ok_or(UiMessage::new("installer.invalidAsset"))?;
        #[cfg(target_os = "linux")]
        let rule = package.asset_rule(&version);
        #[cfg(not(target_os = "linux"))]
        let rule = platform::asset_rule(&version)?;
        let url = format!(
            "https://github.com/nomo-md/nomo/releases/download/v{version}/{}",
            rule.name
        );
        let mut assets = release
            .assets
            .iter()
            .filter(|asset| asset.name == rule.name);
        let asset = assets.next().ok_or(UiMessage::new("installer.missing"))?;
        if assets.next().is_some()
            || asset.browser_download_url != url
            || !(1..=rule.limit).contains(&asset.size)
        {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
        let hex = asset
            .digest
            .as_deref()
            .and_then(|value| value.strip_prefix("sha256:"))
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or(UiMessage::new("installer.digestMissing"))?;
        let mut digest = [0; 32];
        for (index, slot) in digest.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("validated hex");
        }
        Ok(Self {
            version,
            #[cfg(target_os = "linux")]
            package,
            url,
            size: asset.size,
            digest,
        })
    }
}

struct AssetRule {
    name: String,
    limit: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadSource {
    Official,
    GhProxy,
}

impl DownloadSource {
    const ALL: [Self; 2] = [Self::Official, Self::GhProxy];

    fn file_tag(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::GhProxy => "gh-proxy",
        }
    }
}

fn proxy_url(official_url: &str) -> Option<String> {
    let url = url::Url::parse(official_url).ok()?;
    let allowed = url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.path().starts_with("/nomo-md/nomo/releases/download/")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    allowed.then(|| format!("{PROXY_PREFIX}{official_url}"))
}

fn cache_directory(create: bool) -> Result<Option<PathBuf>, UiMessage> {
    let base = crate::settings::config_path()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or(UiMessage::new("installer.directory"))?;
    let directory = base.join("native-updates");
    if create {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)
            .map_err(|error| UiMessage::new("installer.createDirectory").arg("error", error))?;
    }
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(UiMessage::raw(error)),
    };
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(UiMessage::raw("更新缓存目录不是普通目录，已保留"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let owner = fs::metadata("/proc/self").map_err(UiMessage::raw)?.uid();
        if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
            return Err(UiMessage::new("updates.linuxCacheUnsafe"));
        }
    }
    Ok(Some(directory.canonicalize().map_err(UiMessage::raw)?))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

struct CacheLease(File);

impl Drop for CacheLease {
    fn drop(&mut self) {
        // 最后一个使用者主动解锁，不依赖系统何时关闭最后一份文件描述符。
        if let Err(error) = self.0.unlock() {
            crate::app_logger::warn("UpdateCache", &format!("释放更新缓存租约失败：{error}"));
        }
    }
}

fn cache_lease(directory: &Path) -> Result<Option<Arc<CacheLease>>, UiMessage> {
    let path = directory.join(".active");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || is_link(&metadata) => {
            return Err(UiMessage::raw("更新缓存锁不是普通文件，已保留"));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(UiMessage::raw(error)),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
        );
    }
    let lease = options.open(path).map_err(UiMessage::raw)?;
    match lease.try_lock() {
        Ok(()) => Ok(Some(Arc::new(CacheLease(lease)))),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(UiMessage::raw(error)),
    }
}

pub(super) fn clean_stale() -> Result<(), UiMessage> {
    let Some(directory) = cache_directory(false)? else {
        return Ok(());
    };
    let Some(_lease) = cache_lease(&directory)? else {
        return Ok(());
    };
    for path in platform::stale_paths(&directory) {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(UiMessage::raw(error)),
        };
        if is_link(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
            continue;
        }
        if metadata
            .modified()
            .map_err(UiMessage::raw)?
            .elapsed()
            .unwrap_or_default()
            < Duration::from_secs(24 * 60 * 60)
        {
            continue;
        }
        let result = if metadata.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            #[cfg(windows)]
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {}
            Err(error) => return Err(UiMessage::raw(error)),
        }
    }
    Ok(())
}

struct StagedPath {
    #[cfg(target_os = "linux")]
    keep_for_manual_install: bool,
    path: PathBuf,
    _lease: Arc<CacheLease>,
}

// Windows 下先释放读锁，StagedPath 才能删除对应文件。
struct VerifiedDownload {
    _lock: File,
    staged: StagedPath,
}

impl VerifiedDownload {
    fn path(&self) -> &Path {
        &self.staged.path
    }
}

impl StagedPath {
    fn new(path: PathBuf, lease: Arc<CacheLease>) -> Self {
        Self {
            path,
            _lease: lease,
            #[cfg(target_os = "linux")]
            keep_for_manual_install: false,
        }
    }
}

impl Drop for StagedPath {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if self.keep_for_manual_install { return; }
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            crate::app_logger::warn(
                "UpdateCache",
                &format!("更新包暂未删除，后续启动将重试：{error}"),
            );
        }
    }
}

fn remove_existing(path: &Path) -> Result<(), UiMessage> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(UiMessage::new("installer.cleanupFailed").arg("error", error)),
    }
}

fn open_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

fn open_read_locked(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    options.open(path)
}

pub(super) struct DownloadCancel {
    official_activity: std::sync::Mutex<Instant>,
    cancelled: AtomicBool,
    stopped: AtomicBool,
    wake: tokio::sync::watch::Sender<u8>,
}

impl Default for DownloadCancel {
    fn default() -> Self {
        Self {
            official_activity: std::sync::Mutex::new(Instant::now()),
            cancelled: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            wake: tokio::sync::watch::channel(0).0,
        }
    }
}

impl DownloadCancel {
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify();
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.notify();
    }

    fn notify(&self) {
        let state =
            u8::from(self.cancelled()) | u8::from(self.stopped.load(Ordering::Acquire)) << 1;
        self.wake.send_replace(state);
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn record_activity(&self, source: DownloadSource, now: Instant) {
        if matches!(source, DownloadSource::Official) {
            *self.official_activity.lock().unwrap() = now;
        }
    }

    fn hedge_remaining(&self, delay: Duration, now: Instant) -> Duration {
        delay.saturating_sub(now.saturating_duration_since(*self.official_activity.lock().unwrap()))
    }

    fn stopped(&self) -> bool {
        self.cancelled() || self.stopped.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<(), UiMessage> {
        if self.stopped() {
            Err(UiMessage::new("installer.cancelled"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
struct AttemptError {
    message: UiMessage,
    retryable: bool,
}

impl AttemptError {
    fn terminal(message: impl Into<UiMessage>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
    fn retryable(message: impl Into<UiMessage>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

type AttemptResult = Result<VerifiedDownload, AttemptError>;

enum AttemptEvent {
    Progress(u8),
    Finished(DownloadSource, AttemptResult),
}

pub(super) use platform::Verified;

#[derive(Default)]
struct RaceState {
    official_done: bool,
    mirror_started: bool,
    mirror_done: bool,
}

impl RaceState {
    fn exhausted(&self) -> bool {
        self.official_done && (!self.mirror_started || self.mirror_done)
    }

    fn start_mirror(&mut self) -> bool {
        if self.mirror_started {
            return false;
        }
        self.mirror_started = true;
        true
    }

    fn failed(&mut self, source: DownloadSource, retryable: bool) -> bool {
        match source {
            DownloadSource::Official => {
                self.official_done = true;
                retryable && self.start_mirror()
            }
            DownloadSource::GhProxy => {
                self.mirror_done = true;
                false
            }
        }
    }
}

#[derive(Default)]
struct DisplayProgress {
    shown: u8,
}

impl DisplayProgress {
    fn update(&mut self, percent: u8) -> Option<u8> {
        if percent < self.shown.saturating_add(3) {
            return None;
        }
        self.shown = percent;
        Some(percent)
    }
}

pub(super) fn download(
    candidate: Candidate,
    cancel: &DownloadCancel,
    progress: impl Fn(u8),
) -> Result<Verified, UiMessage> {
    cancel.check()?;
    let directory = cache_directory(true)?.expect("created cache directory");
    let lease = cache_lease(&directory)?
        .ok_or_else(|| UiMessage::raw("另一进程正在使用更新缓存，请稍后重试"))?;
    let attempt = |source, events: &mpsc::Sender<_>| {
        download_attempt(
            &candidate,
            source,
            &directory,
            lease.clone(),
            cancel,
            events,
        )
    };
    race_and_prepare(cancel, progress, HEDGE_DELAY, attempt, |download| {
        #[cfg(target_os = "macos")]
        return platform::finish(&candidate, download, cancel);
        #[cfg(any(windows, target_os = "linux"))]
        platform::finish(&candidate, download)
    })
}

fn race_and_prepare<T>(
    cancel: &DownloadCancel,
    progress: impl Fn(u8),
    hedge_delay: Duration,
    attempt: impl Fn(DownloadSource, &mpsc::Sender<AttemptEvent>) -> AttemptResult + Sync,
    prepare: impl FnOnce(VerifiedDownload) -> Result<T, UiMessage>,
) -> Result<T, UiMessage> {
    let winner = std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let spawn = |source: DownloadSource| {
            let sender = sender.clone();
            let attempt = &attempt;
            scope.spawn(move || {
                let result = attempt(source, &sender);
                let _ = sender.send(AttemptEvent::Finished(source, result));
            });
        };
        cancel.record_activity(DownloadSource::Official, Instant::now());
        spawn(DownloadSource::Official);
        let mut state = RaceState::default();
        let mut official_error = None;
        let mut mirror_error = None;
        let mut display = DisplayProgress::default();

        loop {
            if cancel.cancelled() {
                return Err(UiMessage::new("installer.cancelled"));
            }
            if state.exhausted() {
                return Err(mirror_error
                    .or(official_error)
                    .unwrap_or_else(|| UiMessage::new("installer.response")));
            }
            let wait = if state.mirror_started || state.official_done {
                Duration::from_secs(300)
            } else {
                cancel.hedge_remaining(hedge_delay, Instant::now())
            };
            match receiver.recv_timeout(wait) {
                Ok(AttemptEvent::Progress(percent)) => {
                    if let Some(percent) = display.update(percent) {
                        progress(percent);
                    }
                }
                Ok(AttemptEvent::Finished(source, result)) => match result {
                    Ok(verified) => {
                        cancel.stop();
                        return Ok(verified);
                    }
                    Err(error) => {
                        if cancel.cancelled() {
                            return Err(UiMessage::new("installer.cancelled"));
                        }
                        let retryable = error.retryable;
                        match source {
                            DownloadSource::Official => official_error = Some(error.message),
                            DownloadSource::GhProxy => mirror_error = Some(error.message),
                        }
                        if state.failed(source, retryable) {
                            spawn(DownloadSource::GhProxy);
                        }
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout)
                    if !state.official_done && !state.mirror_started =>
                {
                    if cancel
                        .hedge_remaining(hedge_delay, Instant::now())
                        .is_zero()
                    {
                        state.start_mirror();
                        spawn(DownloadSource::GhProxy);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    cancel.stop();
                    return Err(UiMessage::new("installer.response"));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(UiMessage::new("installer.response"));
                }
            }
        }
    })?;
    // 两路均已退出，队列里的另一份包也已释放。停止竞速不等于用户取消。
    if cancel.cancelled() {
        return Err(UiMessage::new("installer.cancelled"));
    }
    prepare(winner)
}

fn download_attempt(
    candidate: &Candidate,
    source: DownloadSource,
    directory: &Path,
    lease: Arc<CacheLease>,
    cancel: &DownloadCancel,
    events: &mpsc::Sender<AttemptEvent>,
) -> AttemptResult {
    cancel.check().map_err(AttemptError::terminal)?;
    let url = match source {
        DownloadSource::Official => candidate.url.clone(),
        DownloadSource::GhProxy => proxy_url(&candidate.url)
            .ok_or_else(|| AttemptError::terminal(UiMessage::new("installer.invalidAsset")))?,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
    let path = platform::download_path(directory, source);
    remove_existing(&path).map_err(AttemptError::terminal)?;
    let mut writer =
        open_new(&path).map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
    let staged = StagedPath::new(path, lease);
    let transfer = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(25))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                let allowed = attempt.previous().len() <= 4
                    && url.scheme() == "https"
                    && url.port().is_none()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && matches!(
                        url.host_str(),
                        Some(
                            "gh-proxy.com"
                                | "github.com"
                                | "release-assets.githubusercontent.com"
                                | "objects.githubusercontent.com"
                        )
                    );
                if allowed {
                    attempt.follow()
                } else {
                    attempt.error(UiMessage::new("installer.redirect"))
                }
            }))
            .build()
            .map_err(request_error)?;
        let mut response = client
            .get(url)
            .header("User-Agent", concat!("Nomo/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(request_error)?
            .error_for_status()
            .map_err(request_error)?;
        if response.status() != reqwest::StatusCode::OK
            || response
                .content_length()
                .is_some_and(|size| size != candidate.size)
        {
            return Err(AttemptError::retryable(UiMessage::new(
                "installer.response",
            )));
        }
        let mut total = 0u64;
        let mut previous = 0u8;
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            cancel.check().map_err(AttemptError::terminal)?;
            if !chunk.is_empty() {
                cancel.record_activity(source, Instant::now());
            }
            total += chunk.len() as u64;
            if total > candidate.size {
                return Err(AttemptError::retryable(UiMessage::new(
                    "installer.oversized",
                )));
            }
            writer
                .write_all(&chunk)
                .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
            let percent = (total * 100 / candidate.size).min(99) as u8;
            if percent >= previous.saturating_add(3) {
                previous = percent;
                let _ = events.send(AttemptEvent::Progress(percent));
            }
        }
        if total != candidate.size {
            return Err(AttemptError::retryable(UiMessage::new(
                "installer.incomplete",
            )));
        }
        writer
            .flush()
            .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
        Ok(())
    };
    let result = runtime.block_on(async {
        let mut wake = cancel.wake.subscribe();
        let mut changed = std::pin::pin!(wake.changed());
        let mut transfer = std::pin::pin!(transfer);
        std::future::poll_fn(|context| {
            if cancel.stopped() {
                return std::task::Poll::Ready(Err(AttemptError::terminal(UiMessage::new(
                    "installer.cancelled",
                ))));
            }
            if changed.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(Err(AttemptError::terminal(UiMessage::new(
                    "installer.cancelled",
                ))));
            }
            transfer.as_mut().poll(context)
        })
        .await
    });
    drop(writer);
    result?;
    verify_download(candidate, staged, cancel)
}

fn verify_download(
    candidate: &Candidate,
    staged: StagedPath,
    cancel: &DownloadCancel,
) -> AttemptResult {
    let mut lock = open_read_locked(&staged.path)
        .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
    if lock
        .metadata()
        .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?
        .len()
        != candidate.size
    {
        return Err(AttemptError::retryable(UiMessage::new(
            "installer.sizeFailed",
        )));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 32 * 1024];
    loop {
        cancel.check().map_err(AttemptError::terminal)?;
        let count = lock
            .read(&mut buffer)
            .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if <[u8; 32]>::from(hash.finalize()) != candidate.digest {
        return Err(AttemptError::retryable(UiMessage::new(
            "installer.digestFailed",
        )));
    }
    Ok(VerifiedDownload {
        _lock: lock,
        staged,
    })
}

fn request_error(error: reqwest::Error) -> AttemptError {
    let retryable = error
        .status()
        .map(|status| {
            status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
        })
        .unwrap_or_else(|| !error.is_builder());
    AttemptError {
        message: download_error(error),
        retryable,
    }
}

fn download_error(error: reqwest::Error) -> UiMessage {
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        if let Some(message) = cause.downcast_ref::<UiMessage>() {
            return message.clone();
        }
        source = cause.source();
    }
    UiMessage::raw(error.without_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BYTES: &[u8] = b"verified installer fixture";

    pub(super) fn test_candidate() -> Candidate {
        Candidate {
            version: "0.8.0".into(),
            #[cfg(target_os = "linux")]
            package: LinuxPackage::Deb,
            url: String::new(),
            size: TEST_BYTES.len() as u64,
            digest: Sha256::digest(TEST_BYTES).into(),
        }
    }

    pub(super) fn test_download(
        path: PathBuf,
        lease: Arc<CacheLease>,
        candidate: &Candidate,
        cancel: &DownloadCancel,
    ) -> VerifiedDownload {
        open_new(&path).unwrap().write_all(TEST_BYTES).unwrap();
        verify_download(candidate, StagedPath::new(path, lease), cancel).unwrap()
    }

    #[test]
    fn two_verified_sources_prepare_only_the_winner_after_loser_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        let lease = cache_lease(directory).unwrap().unwrap();
        let candidate = test_candidate();
        let cancel = DownloadCancel::default();
        let verified = std::sync::Barrier::new(2);
        let winner = race_and_prepare(
            &cancel,
            |_| {},
            Duration::ZERO,
            |source, _| {
                let download = test_download(
                    platform::download_path(directory, source),
                    lease.clone(),
                    &candidate,
                    &cancel,
                );
                verified.wait(); // 两份文件都已通过真实校验，才允许任一来源报告胜出。
                Ok(download)
            },
            |winner| {
                let paths: Vec<_> = DownloadSource::ALL
                    .into_iter()
                    .map(|source| platform::download_path(directory, source))
                    .filter(|path| path.exists())
                    .collect();
                assert_eq!(paths, [winner.path()]);
                assert!(cancel.stopped() && !cancel.cancelled());
                assert!(cache_lease(directory).unwrap().is_none());
                Ok(winner)
            },
        )
        .unwrap();
        drop(lease);
        assert!(cache_lease(directory).unwrap().is_none());
        let path = winner.path().to_owned();
        drop(winner);
        assert!(!path.exists());
        assert!(cache_lease(directory).unwrap().is_some());
    }

    #[test]
    fn user_cancel_during_loser_shutdown_skips_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        let lease = cache_lease(directory).unwrap().unwrap();
        let cancel = DownloadCancel::default();
        let verified = std::sync::Barrier::new(2);
        let result = race_and_prepare(
            &cancel,
            |_| {},
            Duration::ZERO,
            |source, _| {
                let download = test_download(
                    platform::download_path(directory, source),
                    lease.clone(),
                    &test_candidate(),
                    &cancel,
                );
                let mut wake = cancel.wake.subscribe();
                verified.wait();
                if source == DownloadSource::GhProxy {
                    tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap()
                        .block_on(wake.changed())
                        .unwrap(); // 等胜者已停止竞速，再模拟用户取消。
                    cancel.cancel();
                }
                Ok(download)
            },
            |_| -> Result<(), _> { panic!("cancelled winner must not be prepared") },
        );
        assert!(result.is_err() && cancel.cancelled());
        assert!(
            DownloadSource::ALL
                .into_iter()
                .all(|source| !platform::download_path(directory, source).exists())
        );
    }

    #[test]
    fn proxy_only_accepts_canonical_release_downloads() {
        let official =
            "https://github.com/nomo-md/nomo/releases/download/v0.8.0/Nomo_0.8.0_x64-setup.exe";
        assert_eq!(
            proxy_url(official),
            Some(format!("{PROXY_PREFIX}{official}"))
        );
        assert!(
            proxy_url("https://example.com/nomo-md/nomo/releases/download/v0.8.0/a.exe").is_none()
        );
        assert!(
            proxy_url("https://github.com/nomo-md/other/releases/download/v0.8.0/a.exe").is_none()
        );
        assert!(
            proxy_url("https://github.com/nomo-md/nomo/releases/download/v0.8.0/a.exe?x=1")
                .is_none()
        );
    }

    #[test]
    fn sub_percent_activity_defers_mirror_until_actual_stall() {
        let cancel = DownloadCancel::default();
        let start = Instant::now();
        let mut display = DisplayProgress::default();
        for second in 1..=16 {
            let now = start + Duration::from_secs(second);
            cancel.record_activity(DownloadSource::Official, now);
            assert!(display.update(1).is_none());
            assert!(
                !cancel
                    .hedge_remaining(HEDGE_DELAY, now + Duration::from_secs(1))
                    .is_zero()
            );
        }
        assert!(
            cancel
                .hedge_remaining(HEDGE_DELAY, start + Duration::from_secs(24))
                .is_zero()
        );
    }

    #[test]
    fn progress_display_never_combines_sources() {
        let mut display = DisplayProgress::default();
        let updates: Vec<_> = [12, 45, 20, 70, 99]
            .into_iter()
            .filter_map(|percent| display.update(percent))
            .collect();
        assert_eq!(updates, [12, 45, 70, 99]);
        assert_eq!(display.shown, 99);
    }

    #[test]
    fn mirror_starts_once_and_terminal_failure_does_not_retry() {
        let mut state = RaceState::default();
        assert!(!state.failed(DownloadSource::Official, false));
        assert!(state.exhausted());

        let mut state = RaceState::default();
        assert!(state.start_mirror());
        assert!(!state.start_mirror());
        assert!(!state.exhausted());
    }

    #[test]
    fn retryable_failure_and_double_failure_finish_the_race() {
        let mut state = RaceState::default();
        assert!(state.failed(DownloadSource::Official, true));
        assert!(!state.exhausted());
        assert!(!state.failed(DownloadSource::GhProxy, true));
        assert!(state.exhausted());
    }
}

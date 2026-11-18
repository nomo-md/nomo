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
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

const PROXY_PREFIX: &str = "https://gh-proxy.com/";
const HEDGE_DELAY: Duration = Duration::from_secs(8);

pub(super) fn installed() -> bool {
    platform::installed()
}
#[cfg(target_os = "macos")]
pub(crate) fn run_command() -> Option<i32> {
    platform::run_command()
}

pub(super) struct Candidate {
    pub version: String,
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
    fn file_suffix(self) -> &'static str {
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
        fs::create_dir_all(&directory)
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

fn cache_lease(directory: &Path) -> Result<Option<Arc<File>>, UiMessage> {
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
        Ok(()) => Ok(Some(Arc::new(lease))),
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
    path: PathBuf,
    _lease: Arc<File>,
}

impl StagedPath {
    fn new(path: PathBuf, lease: Arc<File>) -> Self {
        Self {
            path,
            _lease: lease,
        }
    }
}

impl Drop for StagedPath {
    fn drop(&mut self) {
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
    cancelled: AtomicBool,
    stopped: AtomicBool,
    wake: tokio::sync::watch::Sender<u8>,
}

impl Default for DownloadCancel {
    fn default() -> Self {
        Self {
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

enum AttemptEvent {
    Progress(u8),
    Finished(DownloadSource, Result<Verified, AttemptError>),
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
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let spawn = |source: DownloadSource| {
            let sender = sender.clone();
            let lease = lease.clone();
            let candidate = &candidate;
            let directory = &directory;
            scope.spawn(move || {
                let result = download_attempt(candidate, source, directory, lease, cancel, &sender);
                let _ = sender.send(AttemptEvent::Finished(source, result));
            });
        };
        spawn(DownloadSource::Official);
        let mut state = RaceState::default();
        let mut official_error = None;
        let mut mirror_error = None;
        let mut display = DisplayProgress::default();
        let mut last_activity = Instant::now();
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
                HEDGE_DELAY.saturating_sub(last_activity.elapsed())
            };
            match receiver.recv_timeout(wait) {
                Ok(AttemptEvent::Progress(percent)) => {
                    last_activity = Instant::now();
                    if let Some(percent) = display.update(percent) {
                        progress(percent);
                    }
                }
                Ok(AttemptEvent::Finished(source, result)) => {
                    last_activity = Instant::now();
                    match result {
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
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    if !state.official_done && state.start_mirror() =>
                {
                    spawn(DownloadSource::GhProxy);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(UiMessage::new("installer.response"));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(UiMessage::new("installer.response"));
                }
            }
        }
    })
}

fn download_attempt(
    candidate: &Candidate,
    source: DownloadSource,
    directory: &Path,
    lease: Arc<File>,
    cancel: &DownloadCancel,
    events: &std::sync::mpsc::Sender<AttemptEvent>,
) -> Result<Verified, AttemptError> {
    cancel.check().map_err(AttemptError::terminal)?;
    let path = directory.join(format!(
        "{}.{}",
        platform::download_name(),
        source.file_suffix()
    ));
    remove_existing(&path).map_err(AttemptError::terminal)?;
    let mut writer =
        open_new(&path).map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
    let staged = StagedPath::new(path, lease);
    let url = match source {
        DownloadSource::Official => candidate.url.clone(),
        DownloadSource::GhProxy => proxy_url(&candidate.url)
            .ok_or_else(|| AttemptError::terminal(UiMessage::new("installer.invalidAsset")))?,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| AttemptError::terminal(UiMessage::raw(error)))?;
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
    platform::finish(candidate, lock, staged).map_err(AttemptError::terminal)
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

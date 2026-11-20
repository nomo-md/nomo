mod archive;
use super::{
    AssetRule, Candidate, DownloadCancel, DownloadSource, VerifiedDownload, cache_directory,
    cache_lease, is_link,
};
use crate::ui::toolbar_i18n::UiMessage;
use archive::extract_archive;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const LIMIT: u64 = 256 * 1024 * 1024;
const EXTRACT_LIMIT: u64 = 1024 * 1024 * 1024;
const COMMAND: &str = "--install-macos-update";
const STAGING_DIR: &str = "macos-staging";

struct Installation {
    app: PathBuf,
}

pub(super) fn installed() -> bool {
    current_installation().is_ok()
}

fn current_installation() -> Result<Installation, UiMessage> {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(UiMessage::raw)?;
    let app = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| UiMessage::new("installer.locationChanged"))?;
    if executable != app.join("Contents/MacOS/nomo") {
        return Err(UiMessage::new("installer.locationChanged"));
    }
    validate_installation(&app)?;
    Ok(Installation { app })
}

fn validate_installation(app: &Path) -> Result<(), UiMessage> {
    if !app.is_absolute()
        || app.file_name() != Some(OsStr::new("Nomo.app"))
        || app
            .components()
            .any(|component| component.as_os_str() == "AppTranslocation")
    {
        return Err(UiMessage::new("installer.locationChanged"));
    }
    let metadata = fs::symlink_metadata(app).map_err(UiMessage::raw)?;
    if !metadata.is_dir()
        || is_link(&metadata)
        || bundle_value(app, "CFBundleIdentifier")? != "com.nomo.desktop"
    {
        return Err(UiMessage::new("installer.locationChanged"));
    }
    let parent = app
        .parent()
        .ok_or_else(|| UiMessage::new("installer.locationChanged"))?;
    let writable = Command::new("/bin/test")
        .arg("-w")
        .arg(parent)
        .status()
        .map_err(UiMessage::raw)?
        .success();
    if !writable {
        return Err(UiMessage::new("installer.locationChanged"));
    }
    Ok(())
}

fn bundle_value(app: &Path, key: &str) -> Result<String, UiMessage> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .map_err(UiMessage::raw)?;
    if !output.status.success() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(UiMessage::raw)
}

pub(super) fn asset_rule(_version: &str) -> Result<AssetRule, UiMessage> {
    if !cfg!(target_arch = "aarch64") {
        return Err(UiMessage::new("installer.architecture"));
    }
    Ok(AssetRule {
        name: "Nomo_aarch64.app.tar.gz".into(),
        limit: LIMIT,
    })
}

pub(super) fn download_path(directory: &Path, source: DownloadSource) -> PathBuf {
    directory.join(format!(
        "nomo-update-aarch64.app.tar.gz.{}",
        source.file_tag()
    ))
}

pub(super) fn stale_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = DownloadSource::ALL
        .into_iter()
        .map(|source| download_path(directory, source))
        .collect();
    paths.extend([
        directory.join(STAGING_DIR),
        directory.join("nomo-update-helper"), // 仅用于回收旧版本留下的副本。
    ]);
    paths
}

struct StagedApp {
    root: PathBuf,
    keep: AtomicBool,
}

impl StagedApp {
    fn keep(&self) {
        self.keep.store(true, Ordering::Release);
    }
}

impl Drop for StagedApp {
    fn drop(&mut self) {
        if !self.keep.load(Ordering::Acquire)
            && let Err(error) = remove_staging(&self.root)
        {
            crate::app_logger::warn("UpdateCache", &format!("macOS 更新暂存暂未删除：{error}"));
        }
    }
}

pub(crate) struct Verified {
    pub version: String,
    app: StagedApp,
    // 先释放 StagedApp，再由下载包释放缓存租约。
    _archive: VerifiedDownload,
    target: PathBuf,
}

pub(super) fn finish(
    candidate: &Candidate,
    archive: VerifiedDownload,
    cancel: &DownloadCancel,
) -> Result<Verified, UiMessage> {
    let installation = current_installation()?;
    let root = archive
        .path()
        .parent()
        .expect("cache directory")
        .join(STAGING_DIR);
    remove_staging(&root)?;
    fs::create_dir(&root).map_err(UiMessage::raw)?;
    let app = StagedApp {
        root,
        keep: AtomicBool::new(false),
    };
    extract_archive(archive.path(), &app.root, cancel, EXTRACT_LIMIT, 4096)?;
    validate_bundle_checked(&app.root.join("Nomo.app"), &candidate.version, Some(cancel))?;
    Ok(Verified {
        version: candidate.version.clone(),
        app,
        _archive: archive,
        target: installation.app,
    })
}

impl Verified {
    pub(crate) fn launch(&self) -> Result<(), UiMessage> {
        let installation = current_installation()?;
        if installation.app != self.target {
            return Err(UiMessage::new("installer.locationChanged"));
        }
        install_command(&installation, &self.version)
            .spawn()
            .map_err(|error| UiMessage::new("installer.launchFailed").arg("error", error))?;
        self.app.keep();
        Ok(())
    }
}

fn install_command(installation: &Installation, version: &str) -> Command {
    // 主程序的签名依赖 bundle 中的 Info.plist 和资源，不能裸拷贝到缓存执行。
    let mut command = Command::new(installation.app.join("Contents/MacOS/nomo"));
    command
        .arg(COMMAND)
        .arg(std::process::id().to_string())
        .arg(&installation.app)
        .arg(version);
    command
}

fn remove_staging(path: &Path) -> Result<(), UiMessage> {
    match check_staging_directory(path) {
        Ok(()) => fs::remove_dir_all(path).map_err(UiMessage::raw),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(UiMessage::raw(error)),
    }
}

fn check_staging_directory(root: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.is_dir() && !is_link(&metadata) {
        Ok(())
    } else {
        Err(io::Error::other("更新暂存路径不是普通目录，已保留"))
    }
}

fn validate_archive_path(name: &str) -> Result<(), UiMessage> {
    let path = Path::new(name);
    let mut components = path.components();
    if components.next() != Some(Component::Normal(OsStr::new("Nomo.app")))
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(())
}

fn check_cancel(cancel: Option<&DownloadCancel>) -> Result<(), UiMessage> {
    if cancel.is_some_and(DownloadCancel::cancelled) {
        Err(UiMessage::new("installer.cancelled"))
    } else {
        Ok(())
    }
}

fn validate_tree(root: &Path, cancel: Option<&DownloadCancel>) -> Result<(), UiMessage> {
    let mut stack = vec![root.to_path_buf()];
    let mut entries = 0usize;
    let mut bytes = 0u64;
    while let Some(path) = stack.pop() {
        check_cancel(cancel)?;
        entries += 1;
        if entries > 4096 {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
        let metadata = fs::symlink_metadata(&path).map_err(UiMessage::raw)?;
        if is_link(&metadata) {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(UiMessage::raw)? {
                stack.push(entry.map_err(UiMessage::raw)?.path());
            }
        } else if metadata.is_file() {
            bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| UiMessage::new("installer.invalidAsset"))?;
            if bytes > EXTRACT_LIMIT {
                return Err(UiMessage::new("installer.invalidAsset"));
            }
        } else {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
    }
    Ok(())
}

fn validate_bundle(app: &Path, version: &str) -> Result<(), UiMessage> {
    validate_bundle_checked(app, version, None)
}

fn validate_bundle_checked(
    app: &Path,
    version: &str,
    cancel: Option<&DownloadCancel>,
) -> Result<(), UiMessage> {
    validate_tree(app, cancel)?;
    if bundle_value(app, "CFBundleIdentifier")? != "com.nomo.desktop"
        || bundle_value(app, "CFBundleShortVersionString")? != version
    {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    validate_arm64_executable(&app.join("Contents/MacOS/nomo"))?;
    check_cancel(cancel)?;
    let mut command = Command::new("/usr/bin/codesign");
    command.args(["--verify", "--deep", "--strict"]).arg(app);
    let signed = if let Some(cancel) = cancel {
        let mut child = command.spawn().map_err(UiMessage::raw)?;
        loop {
            if let Some(status) = child.try_wait().map_err(UiMessage::raw)? {
                break status;
            }
            if cancel.cancelled() {
                let _ = child.kill();
                child.wait().map_err(UiMessage::raw)?;
                return Err(UiMessage::new("installer.cancelled"));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    } else {
        command.status().map_err(UiMessage::raw)?
    };
    if !signed.success() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(())
}

fn validate_arm64_executable(path: &Path) -> Result<(), UiMessage> {
    // mach-o/loader.h、mach/machine.h：发布包只接受小端、单架构 arm64 主程序。
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const CPU_TYPE_ARM64: u32 = 0x0100_000c;
    const CPU_SUBTYPE_ARM64_ALL: u32 = 0;
    const MH_EXECUTE: u32 = 2;
    let mut header = [[0u8; 4]; 8]; // struct mach_header_64
    let header_len = header.as_flattened().len() as u64;
    let metadata = fs::symlink_metadata(path).map_err(UiMessage::raw)?;
    if !metadata.is_file() || is_link(&metadata) || metadata.len() < header_len {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    File::open(path)
        .and_then(|mut file| file.read_exact(header.as_flattened_mut()))
        .map_err(UiMessage::raw)?;
    let [magic, cpu, subtype, kind, count, bytes, _, _] = header.map(u32::from_le_bytes);
    if magic != MH_MAGIC_64
        || cpu != CPU_TYPE_ARM64
        || subtype != CPU_SUBTYPE_ARM64_ALL
        || kind != MH_EXECUTE
    {
        return Err(UiMessage::new("installer.architecture"));
    }
    // 每条加载命令至少含两个 u32；整包完整性仍由 codesign 校验。
    let bytes = u64::from(bytes);
    if count == 0 || u64::from(count) * 8 > bytes || header_len + bytes > metadata.len() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(())
}

pub(crate) fn run_command() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(COMMAND)) {
        return None;
    }
    let args = args.collect::<Vec<_>>();
    match parse_install_args(&args)
        .and_then(|(pid, target, version)| install_after_exit(pid, target, version))
    {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("macOS 更新未完成：{error}");
            Some(1)
        }
    }
}

fn parse_install_args(args: &[OsString]) -> Result<(u32, &Path, &str), String> {
    let [pid, target, version] = args else {
        return Err("更新安装命令参数无效".into());
    };
    let pid = pid
        .to_string_lossy()
        .parse::<u32>()
        .map_err(|_| "更新进程号无效")?;
    Ok((
        pid,
        Path::new(target),
        version.to_str().ok_or("更新版本无效")?,
    ))
}

fn install_after_exit(pid: u32, target: &Path, version: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let alive = Command::new("/bin/kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map_err(|error| error.to_string())?
            .success();
        if !alive {
            break;
        }
        if Instant::now() >= deadline {
            return Err("等待旧进程退出超时，未修改应用".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let cache = cache_directory(false)?.ok_or("更新缓存目录不存在")?;
    install_staged_update(&cache, target, version, open_app)
}

fn install_staged_update(
    cache: &Path,
    target: &Path,
    version: &str,
    open: impl Fn(&Path) -> bool,
) -> Result<(), String> {
    let _lease = cache_lease(cache)
        .map_err(|error| format!("更新缓存锁定失败：{error}"))?
        .ok_or("另一进程正在使用更新缓存")?;
    let root = cache.join(STAGING_DIR);
    check_staging_directory(&root).map_err(|error| format!("更新暂存目录无效：{error}"))?;
    let staged = root.join("Nomo.app");
    validate_installation(target).map_err(|error| format!("旧版应用校验失败：{error}"))?;
    validate_bundle(&staged, version).map_err(|error| format!("暂存新版校验失败：{error}"))?;
    let parent = target.parent().ok_or("应用安装路径无效")?;
    let sidecars = tempfile::Builder::new()
        .prefix(".Nomo.nomo-update-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let incoming = sidecars.path().join("incoming.app");
    let backup = sidecars.path().join("backup.app");
    let copied = Command::new("/usr/bin/ditto")
        .arg(staged)
        .arg(&incoming)
        .status()
        .map_err(|error| error.to_string())?;
    if !copied.success() {
        return Err("无法在同卷暂存新版应用".into());
    }
    if let Err(error) =
        validate_bundle(&incoming, version).map_err(|error| format!("同卷新版校验失败：{error}"))
    {
        let _ = fs::remove_dir_all(&incoming);
        return Err(error);
    }
    fs::rename(target, &backup).map_err(|error| format!("无法暂存旧版应用：{error}"))?;
    if let Err(error) = fs::rename(&incoming, target) {
        restore_backup(target, &backup, sidecars, &error.to_string())?;
        return Err(format!("无法将新版应用就位：{error}"));
    }
    if !open(target) {
        let _ = fs::remove_dir_all(target);
        restore_backup(target, &backup, sidecars, "新版启动失败")?;
        let _ = open(target);
        return Err("新版应用无法启动，已恢复旧版".into());
    }
    let _ = fs::remove_dir_all(backup);
    if let Err(error) = remove_staging(&root) {
        eprintln!("macOS 更新暂存暂未删除：{error}");
    }
    Ok(())
}

fn restore_backup(
    target: &Path,
    backup: &Path,
    sidecars: tempfile::TempDir,
    cause: &str,
) -> Result<(), String> {
    fs::rename(backup, target).map_err(|e| {
        let _ = sidecars.keep(); // 恢复失败时绝不能让临时目录析构删除唯一的旧版。
        format!(
            "{cause}；无法恢复 {}：{e}；旧版备份位于 {}",
            target.display(),
            backup.display()
        )
    })
}

fn open_app(app: &Path) -> bool {
    Command::new("/usr/bin/open")
        .arg("-n") // 安装进程仍在运行；显式启动替换后的实例。
        .arg(app)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_app(path: &Path, version: &str) {
        fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        fs::copy(
            std::env::current_exe().unwrap(),
            path.join("Contents/MacOS/nomo"),
        )
        .unwrap();
        fs::write(path.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.nomo.desktop</string><key>CFBundleShortVersionString</key><string>{version}</string><key>CFBundleExecutable</key><string>nomo</string></dict></plist>"#)).unwrap();
        assert!(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn archive_paths_stay_inside_nomo_bundle() {
        assert!(validate_archive_path("Nomo.app/Contents/MacOS/nomo").is_ok());
        assert!(validate_archive_path("../Nomo.app").is_err());
        assert!(validate_archive_path("/Nomo.app").is_err());
        assert!(validate_archive_path("Other.app/Contents/MacOS/nomo").is_err());
        assert!(validate_archive_path("Nomo.app/../escape").is_err());
    }

    #[test]
    fn signed_archive_extracts_and_validates() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source/Nomo.app");
        create_app(&source, "0.8.0");
        let archive = root.path().join("update.tar.gz");
        assert!(
            Command::new("/usr/bin/tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(source.parent().unwrap())
                .arg("Nomo.app")
                .status()
                .unwrap()
                .success()
        );

        let extract = root.path().join("extract");
        fs::create_dir(&extract).unwrap();
        extract_archive(
            &archive,
            &extract,
            &DownloadCancel::default(),
            EXTRACT_LIMIT,
            4096,
        )
        .unwrap();
        validate_bundle(&extract.join("Nomo.app"), "0.8.0").unwrap();
    }

    #[test]
    fn archive_budgets_and_mid_file_cancel_stop_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Nomo.app");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("data"), vec![0; 200_000]).unwrap();
        let archive = tmp.path().join("test.tar.gz");
        assert!(
            Command::new("/usr/bin/tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(tmp.path())
                .arg("Nomo.app")
                .status()
                .unwrap()
                .success()
        );
        for (bytes, entries) in [(100, 10), (200_000, 1)] {
            let out = tempfile::tempdir().unwrap();
            assert!(
                extract_archive(
                    &archive,
                    out.path(),
                    &DownloadCancel::default(),
                    bytes,
                    entries
                )
                .is_err()
            );
            assert!(!out.path().join("Nomo.app/data").exists());
        }
        let out = tempfile::tempdir().unwrap();
        let data = out.path().join("Nomo.app/data");
        let cancel = DownloadCancel::default();
        assert!(
            archive::extract_checked(&archive, out.path(), EXTRACT_LIMIT, 4096, || {
                if data.metadata().is_ok_and(|m| m.len() > 0) {
                    cancel.cancel();
                }
                check_cancel(Some(&cancel))
            })
            .is_err()
        );
        assert!(cancel.cancelled());
        assert!((1..200_000).contains(&data.metadata().unwrap().len()));
    }

    #[test]
    fn failed_restore_preserves_backup_and_reports_both_paths() {
        let root = tempfile::tempdir().unwrap();
        let sidecars = tempfile::tempdir_in(root.path()).unwrap();
        let backup = sidecars.path().join("backup.app");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("original"), b"old").unwrap();
        let target = root.path().join("Nomo.app");
        fs::write(&target, b"occupied").unwrap();
        let err = restore_backup(&target, &backup, sidecars, "replacement failed").unwrap_err();
        assert!(
            err.contains(&*target.to_string_lossy()) && err.contains(&*backup.to_string_lossy())
        );
        assert_eq!(fs::read(backup.join("original")).unwrap(), b"old");
    }

    #[test]
    fn replacement_installs_new_bundle_without_opening_a_window() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Nomo.app");
        let cache = root.path().join("native-updates");
        let staging = cache.join(STAGING_DIR);
        create_app(&target, "0.7.0");
        create_app(&staging.join("Nomo.app"), "0.8.0");
        let unrelated = cache.join("other");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("notes.md"), b"keep").unwrap();
        install_staged_update(&cache, &target, "0.8.0", |_| true).unwrap();
        assert_eq!(
            bundle_value(&target, "CFBundleShortVersionString").unwrap(),
            "0.8.0"
        );
        assert!(!staging.exists());
        assert_eq!(fs::read(unrelated.join("notes.md")).unwrap(), b"keep");
    }

    #[test]
    fn failed_launch_restores_old_bundle() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Nomo.app");
        let cache = root.path().join("native-updates");
        create_app(&target, "0.7.0");
        create_app(&cache.join(STAGING_DIR).join("Nomo.app"), "0.8.0");
        assert!(install_staged_update(&cache, &target, "0.8.0", |_| false).is_err());
        assert_eq!(
            bundle_value(&target, "CFBundleShortVersionString").unwrap(),
            "0.7.0"
        );
    }

    #[test]
    fn install_command_runs_inside_installed_bundle() {
        let app = PathBuf::from("/Applications With Spaces/Nomo.app");
        let installation = Installation { app: app.clone() };
        let command = install_command(&installation, "0.8.0");
        assert_eq!(
            command.get_program(),
            app.join("Contents/MacOS/nomo").as_os_str()
        );
        let mut args = command.get_args();
        assert_eq!(args.next(), Some(OsStr::new(COMMAND)));
        assert_eq!(
            parse_install_args(&args.map(OsStr::to_owned).collect::<Vec<_>>()).unwrap(),
            (std::process::id(), app.as_path(), "0.8.0")
        );
    }

    #[test]
    fn helper_does_not_accept_a_staged_path() {
        let args = ["42", "/Applications/Nomo.app", "0.8.0"].map(OsString::from);
        assert_eq!(
            parse_install_args(&args).unwrap(),
            (42, Path::new("/Applications/Nomo.app"), "0.8.0")
        );
        let mut old_args = args.to_vec();
        old_args.insert(2, "/unrelated/Nomo.app".into());
        assert!(parse_install_args(&old_args).is_err());
    }

    #[test]
    fn helper_locks_cache_first_and_rejects_linked_staging() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("native-updates");
        let unrelated = temp.path().join("documents");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("notes.md"), b"keep").unwrap();
        fs::create_dir(&cache).unwrap();
        let root = cache.join(STAGING_DIR);
        std::os::unix::fs::symlink(&unrelated, &root).unwrap();
        let install = || {
            install_staged_update(&cache, &temp.path().join("Nomo.app"), "0.8.0", |_| {
                panic!("must not launch")
            })
        };
        let lease = cache_lease(&cache).unwrap().unwrap();
        assert_eq!(install().unwrap_err(), "另一进程正在使用更新缓存");
        drop(lease);
        let error = install().unwrap_err();
        assert!(error.contains("更新暂存目录无效"), "{error}");
        assert!(remove_staging(&root).is_err());
        assert!(root.is_symlink());
        assert_eq!(fs::read(unrelated.join("notes.md")).unwrap(), b"keep");
    }

    #[test]
    fn rejects_wrong_mach_o_architectures_and_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nomo");
        // 这里只测架构探针；有效签名和实际可执行文件由归档测试覆盖。
        let header: Vec<_> = [0xfeed_facfu32, 0x0100_000c, 0, 2, 1, 8, 0, 0, 0, 8]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        fs::write(&path, &header).unwrap();
        validate_arm64_executable(&path).unwrap();
        for (offset, word) in [(0, 0xbeba_fecau32), (4, 0x0100_0007), (8, 2), (12, 6)] {
            let mut other = header.clone();
            other[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            fs::write(&path, other).unwrap();
            assert!(validate_arm64_executable(&path).is_err());
        }
        for length in [0, 16, 31, 32, 39] {
            fs::write(&path, &header[..length]).unwrap();
            assert!(validate_arm64_executable(&path).is_err());
        }
    }
}

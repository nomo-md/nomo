use super::{AssetRule, Candidate, StagedPath, cache_lease, is_link, remove_existing};
use crate::ui::toolbar_i18n::UiMessage;
use std::{
    ffi::OsStr,
    fs::{self, File},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const LIMIT: u64 = 256 * 1024 * 1024;
const EXTRACT_LIMIT: u64 = 1024 * 1024 * 1024;
const COMMAND: &str = "--install-macos-update";

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
        .filter(|path| path.file_name() == Some(OsStr::new("Nomo.app")))
        .map(Path::to_path_buf)
        .ok_or_else(|| UiMessage::new("installer.locationChanged"))?;
    validate_installation(&app)?;
    Ok(Installation { app })
}

fn validate_installation(app: &Path) -> Result<(), UiMessage> {
    if !app.is_absolute()
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

pub(super) fn download_name() -> &'static str {
    "nomo-update-aarch64.app.tar.gz"
}

pub(super) fn stale_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = ["official", "gh-proxy"]
        .iter()
        .map(|source| directory.join(format!("{}.{source}", download_name())))
        .collect();
    paths.extend([
        directory.join("macos-staging"),
        directory.join("nomo-update-helper"),
    ]);
    paths
}

struct StagedApp {
    path: PathBuf,
    keep: AtomicBool,
}

impl StagedApp {
    fn keep(&self) {
        self.keep.store(true, Ordering::Release);
    }
}

impl Drop for StagedApp {
    fn drop(&mut self) {
        if !self.keep.load(Ordering::Acquire) {
            if let Err(error) = fs::remove_dir_all(self.path.parent().expect("staging root"))
                && error.kind() != std::io::ErrorKind::NotFound
            {
                crate::app_logger::warn("UpdateCache", &format!("macOS 更新暂存暂未删除：{error}"));
            }
        }
    }
}

pub(crate) struct Verified {
    pub version: String,
    _lock: File,
    _archive: StagedPath,
    app: StagedApp,
    target: PathBuf,
}

pub(super) fn finish(
    candidate: &Candidate,
    lock: File,
    archive: StagedPath,
) -> Result<Verified, UiMessage> {
    let installation = current_installation()?;
    let root = archive
        .path
        .parent()
        .expect("cache directory")
        .join("macos-staging");
    remove_staging(&root)?;
    fs::create_dir(&root).map_err(UiMessage::raw)?;
    let app = StagedApp {
        path: root.join("Nomo.app"),
        keep: AtomicBool::new(false),
    };
    extract_archive(&archive.path, &root)?;
    validate_bundle(&app.path, &candidate.version)?;
    Ok(Verified {
        version: candidate.version.clone(),
        _lock: lock,
        _archive: archive,
        app,
        target: installation.app,
    })
}

impl Verified {
    pub(crate) fn launch(&self) -> Result<(), UiMessage> {
        let installation = current_installation()?;
        if installation.app != self.target {
            return Err(UiMessage::new("installer.locationChanged"));
        }
        let cache = self
            .app
            .path
            .parent()
            .and_then(Path::parent)
            .expect("staging cache");
        let helper = cache.join("nomo-update-helper");
        remove_existing(&helper)?;
        fs::copy(std::env::current_exe().map_err(UiMessage::raw)?, &helper)
            .map_err(UiMessage::raw)?;
        Command::new(&helper)
            .arg(COMMAND)
            .arg(std::process::id().to_string())
            .arg(&self.target)
            .arg(&self.app.path)
            .arg(&self.version)
            .spawn()
            .map_err(|error| UiMessage::new("installer.launchFailed").arg("error", error))?;
        self.app.keep();
        Ok(())
    }
}

fn remove_staging(path: &Path) -> Result<(), UiMessage> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
            fs::remove_dir_all(path).map_err(UiMessage::raw)
        }
        Ok(_) => remove_existing(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(UiMessage::raw(error)),
    }
}

fn archive_entries(archive: &Path) -> Result<Vec<(char, String)>, UiMessage> {
    let names = Command::new("/usr/bin/tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(UiMessage::raw)?;
    let verbose = Command::new("/usr/bin/tar")
        .arg("-tvzf")
        .arg(archive)
        .output()
        .map_err(UiMessage::raw)?;
    if !names.status.success() || !verbose.status.success() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    let names = String::from_utf8(names.stdout).map_err(UiMessage::raw)?;
    let verbose = String::from_utf8(verbose.stdout).map_err(UiMessage::raw)?;
    let names: Vec<_> = names.lines().map(str::to_owned).collect();
    let verbose: Vec<_> = verbose.lines().collect();
    if names.is_empty() || names.len() != verbose.len() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    names
        .into_iter()
        .zip(verbose)
        .map(|(name, detail)| {
            let kind = detail
                .chars()
                .next()
                .ok_or_else(|| UiMessage::new("installer.invalidAsset"))?;
            if !matches!(kind, '-' | 'd') || !detail.ends_with(&name) {
                return Err(UiMessage::new("installer.invalidAsset"));
            }
            validate_archive_path(&name)?;
            Ok((kind, name))
        })
        .collect()
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

fn extract_archive(archive: &Path, root: &Path) -> Result<(), UiMessage> {
    archive_entries(archive)?;
    let status = Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(root)
        .args(["--no-same-owner", "--no-same-permissions"])
        .status()
        .map_err(UiMessage::raw)?;
    if !status.success() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(())
}

fn validate_tree(root: &Path) -> Result<(), UiMessage> {
    let mut stack = vec![root.to_path_buf()];
    let mut entries = 0usize;
    let mut bytes = 0u64;
    while let Some(path) = stack.pop() {
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
    validate_tree(app)?;
    if bundle_value(app, "CFBundleIdentifier")? != "com.nomo.desktop"
        || bundle_value(app, "CFBundleShortVersionString")? != version
    {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    let executable = app.join("Contents/MacOS/nomo");
    let metadata = fs::symlink_metadata(&executable).map_err(UiMessage::raw)?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    let architectures = Command::new("/usr/bin/lipo")
        .arg("-archs")
        .arg(&executable)
        .output()
        .map_err(UiMessage::raw)?;
    if !architectures.status.success()
        || String::from_utf8(architectures.stdout)
            .map_err(UiMessage::raw)?
            .trim()
            != "arm64"
    {
        return Err(UiMessage::new("installer.architecture"));
    }
    let signed = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .status()
        .map_err(UiMessage::raw)?;
    if !signed.success() {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(())
}

pub(crate) fn run_command() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first() != Some(&OsStr::new(COMMAND).into()) {
        return None;
    }
    let result = if args.len() == 5 {
        args[1]
            .to_string_lossy()
            .parse::<u32>()
            .map_err(|_| "更新进程号无效".to_owned())
            .and_then(|pid| {
                install_after_exit(
                    pid,
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                    &args[4].to_string_lossy(),
                )
            })
    } else {
        Err("更新安装命令参数无效".into())
    };
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("macOS 更新未完成：{error}");
            Some(1)
        }
    }
}

fn install_after_exit(pid: u32, target: &Path, staged: &Path, version: &str) -> Result<(), String> {
    install_after_exit_with(pid, target, staged, version, open_app)
}

fn install_after_exit_with(
    pid: u32,
    target: &Path,
    staged: &Path,
    version: &str,
    open: impl Fn(&Path) -> bool,
) -> Result<(), String> {
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
    validate_installation(target).map_err(|error| format!("旧版应用校验失败：{error}"))?;
    validate_bundle(staged, version).map_err(|error| format!("暂存新版校验失败：{error}"))?;
    let cache = staged
        .parent()
        .and_then(Path::parent)
        .ok_or("更新缓存路径无效")?;
    let _lease = cache_lease(cache)
        .map_err(|error| format!("更新缓存锁定失败：{error}"))?
        .ok_or("另一进程正在使用更新缓存")?;
    let parent = target.parent().ok_or("应用安装路径无效")?;
    let unique = std::process::id();
    let incoming = parent.join(format!(".Nomo.nomo-update-{unique}.new"));
    let backup = parent.join(format!(".Nomo.nomo-update-{unique}.old"));
    remove_sidecar(&incoming)?;
    remove_sidecar(&backup)?;
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
        let _ = fs::rename(&backup, target);
        return Err(format!("无法将新版应用就位：{error}"));
    }
    if !open(target) {
        let _ = fs::remove_dir_all(target);
        fs::rename(&backup, target)
            .map_err(|error| format!("新版启动失败，旧版恢复也失败：{error}"))?;
        let _ = open(target);
        return Err("新版应用无法启动，已恢复旧版".into());
    }
    let _ = fs::remove_dir_all(backup);
    let _ = fs::remove_dir_all(staged.parent().expect("staging root"));
    Ok(())
}

fn remove_sidecar(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
            fs::remove_dir_all(path).map_err(|error| error.to_string())
        }
        Ok(_) => fs::remove_file(path).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn open_app(app: &Path) -> bool {
    Command::new("/usr/bin/open")
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
        archive_entries(&archive).unwrap();
        let extract = root.path().join("extract");
        fs::create_dir(&extract).unwrap();
        extract_archive(&archive, &extract).unwrap();
        validate_bundle(&extract.join("Nomo.app"), "0.8.0").unwrap();
    }

    #[test]
    fn replacement_installs_new_bundle_without_opening_a_window() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Nomo.app");
        let staged = root.path().join("native-updates/macos-staging/Nomo.app");
        create_app(&target, "0.7.0");
        create_app(&staged, "0.8.0");
        install_after_exit_with(u32::MAX, &target, &staged, "0.8.0", |_| true).unwrap();
        assert_eq!(
            bundle_value(&target, "CFBundleShortVersionString").unwrap(),
            "0.8.0"
        );
    }

    #[test]
    fn failed_launch_restores_old_bundle() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Nomo.app");
        let staged = root.path().join("native-updates/macos-staging/Nomo.app");
        create_app(&target, "0.7.0");
        create_app(&staged, "0.8.0");
        assert!(install_after_exit_with(u32::MAX, &target, &staged, "0.8.0", |_| false).is_err());
        assert_eq!(
            bundle_value(&target, "CFBundleShortVersionString").unwrap(),
            "0.7.0"
        );
    }
}

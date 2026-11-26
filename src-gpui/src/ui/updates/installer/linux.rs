//! Linux 安装版由系统包数据库识别，安装与授权交给 PackageKit。
use super::{AssetRule, Candidate, DownloadSource, VerifiedDownload};
use crate::ui::toolbar_i18n::UiMessage;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

mod packagekit;

const EXECUTABLE: &str = "/usr/bin/nomo";
const DEB_FORMAT: &str = "${Package}\\t${Architecture}\\t${Version}";
const RPM_FORMAT: &str = "%{NAME}\\t%{ARCH}\\t%{EPOCHNUM}\\t%{VERSION}\\t%{RELEASE}";

#[derive(Clone, Copy)]
pub(in crate::ui::updates) enum LinuxPackage {
    Deb,
    Rpm,
}

fn query(command: &mut Command) -> Option<String> {
    let output = command
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

impl LinuxPackage {
    pub(in crate::ui::updates) fn detect() -> Option<Self> {
        if !cfg!(target_arch = "x86_64")
            || std::env::current_exe().ok()?.as_path() != Path::new(EXECUTABLE)
        {
            return None;
        }
        let mut owners = [Self::Deb, Self::Rpm]
            .into_iter()
            .filter(|pkg| pkg.installed_version(env!("CARGO_PKG_VERSION")));
        let owner = owners.next()?;
        owners.next().is_none().then_some(owner)
    }

    fn installed_version(self, version: &str) -> bool {
        match self {
            Self::Deb => {
                let Some(owner) =
                    query(Command::new("/usr/bin/dpkg-query").args(["-S", EXECUTABLE]))
                else {
                    return false;
                };
                if !matches!(
                    owner.trim_end(),
                    "nomo: /usr/bin/nomo" | "nomo:amd64: /usr/bin/nomo"
                ) {
                    return false;
                }
                let field = format!("-f=${{Status}}\\t{DEB_FORMAT}");
                query(Command::new("/usr/bin/dpkg-query").args(["-W", &field, "nomo:amd64"]))
                    .is_some_and(|s| s == format!("install ok installed\tnomo\tamd64\t{version}"))
            }
            Self::Rpm => query(Command::new("/usr/bin/rpm").args([
                "-qf",
                "--queryformat",
                RPM_FORMAT,
                EXECUTABLE,
            ]))
            .is_some_and(|s| s == format!("nomo\tx86_64\t0\t{version}\t1")),
        }
    }

    pub(super) fn asset_rule(self, version: &str) -> AssetRule {
        let name = match self {
            Self::Deb => format!("Nomo_{version}_amd64.deb"),
            Self::Rpm => format!("Nomo-{version}-1.x86_64.rpm"),
        };
        AssetRule {
            name,
            limit: 128 * 1024 * 1024,
        }
    }

    fn matches_file(self, path: &Path, version: &str) -> bool {
        match self {
            Self::Deb => query(
                Command::new("/usr/bin/dpkg-deb")
                    .args(["--show", &format!("--showformat={DEB_FORMAT}")])
                    .arg(path),
            )
            .is_some_and(|s| s == format!("nomo\tamd64\t{version}")),
            Self::Rpm => query(
                Command::new("/usr/bin/rpm")
                    .args(["-qp", "--queryformat", RPM_FORMAT])
                    .arg(path),
            )
            .is_some_and(|s| s == format!("nomo\tx86_64\t0\t{version}\t1")),
        }
    }

    fn cache_name(self) -> &'static str {
        match self {
            Self::Deb => "nomo-update-amd64.deb",
            Self::Rpm => "nomo-update-x86_64.rpm",
        }
    }
}

pub(super) fn download_path(directory: &Path, source: DownloadSource) -> PathBuf {
    directory.join(format!("nomo-linux-{}.part", source.file_tag()))
}

pub(super) fn stale_paths(directory: &Path) -> Vec<PathBuf> {
    DownloadSource::ALL
        .into_iter()
        .map(|source| download_path(directory, source))
        .chain([LinuxPackage::Deb, LinuxPackage::Rpm].map(|pkg| directory.join(pkg.cache_name())))
        .collect()
}

pub(in crate::ui::updates) struct Verified {
    pub version: String,
    package: LinuxPackage,
    digest: [u8; 32],
    size: u64,
    download: VerifiedDownload,
}

pub(super) fn finish(
    candidate: &Candidate,
    mut download: VerifiedDownload,
) -> Result<Verified, UiMessage> {
    let path = download
        .path()
        .with_file_name(candidate.package.cache_name());
    fs::rename(download.path(), &path).map_err(UiMessage::raw)?;
    download.staged.path = path;
    if !candidate
        .package
        .matches_file(download.path(), &candidate.version)
    {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(Verified {
        version: candidate.version.clone(),
        package: candidate.package,
        digest: candidate.digest,
        size: candidate.size,
        download,
    })
}

impl Verified {
    pub(in crate::ui::updates) fn path(&self) -> &Path {
        self.download.path()
    }

    pub(in crate::ui::updates) fn keep_for_manual_install(&mut self) {
        self.download.staged.keep_for_manual_install = true;
    }

    pub(in crate::ui::updates) async fn install(&self) -> Result<(), UiMessage> {
        // 安装后的核对或桌面连接曾失败时，允许重新核对，避免重复提交安装事务。
        if self.package.installed_version(&self.version) {
            return Ok(());
        }
        if !self.package.installed_version(env!("CARGO_PKG_VERSION")) {
            return Err(UiMessage::new("updates.linuxPackageChanged"));
        }
        packagekit::install(self.path()).await?;
        if !self.package.installed_version(&self.version) {
            return Err(UiMessage::new("updates.linuxVersionMismatch"));
        }
        Ok(())
    }

    pub(in crate::ui::updates) fn verify_file(&mut self) -> Result<(), UiMessage> {
        self.download.staged.keep_for_manual_install = false;
        let original = self.download._lock.metadata().map_err(UiMessage::raw)?;
        let current = fs::symlink_metadata(self.path()).map_err(UiMessage::raw)?;
        if !current.is_file() || current.dev() != original.dev() || current.ino() != original.ino()
        {
            return Err(UiMessage::new("installer.invalidAsset"));
        }
        let file = File::open(self.path()).map_err(UiMessage::raw)?;
        let opened = file.metadata().map_err(UiMessage::raw)?;
        if opened.dev() != original.dev()
            || opened.ino() != original.ino()
            || opened.len() != self.size
        {
            return Err(UiMessage::new("installer.sizeFailed"));
        }
        let mut hash = Sha256::new();
        io::copy(&mut io::Read::take(file, self.size + 1), &mut hash).map_err(UiMessage::raw)?;
        if <[u8; 32]>::from(hash.finalize()) != self.digest {
            return Err(UiMessage::new("installer.digestFailed"));
        }
        Ok(())
    }
}

pub(in crate::ui::updates) fn restart() -> Result<(), UiMessage> {
    Command::new(EXECUTABLE)
        .spawn()
        .map_err(|e| UiMessage::new("installer.launchFailed").arg("error", e))?;
    Ok(())
}

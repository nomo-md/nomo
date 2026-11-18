use super::{Candidate, DownloadSource, VerifiedDownload};
use crate::ui::toolbar_i18n::UiMessage;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

const SETUP_STEM: &str = "nomo-update-x64-setup";

#[cfg(windows)]
pub(super) fn installed() -> bool {
    use std::fs;
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo";
    let Ok(executable) = std::env::current_exe().and_then(fs::canonicalize) else {
        return false;
    };
    let Ok(directory) = windows_registry::CURRENT_USER
        .open(UNINSTALL)
        .and_then(|key| key.get_string("InstallLocation"))
    else {
        return false;
    };
    let directory = Path::new(directory.trim().trim_matches('"'));
    directory.is_absolute()
        && directory.join("uninstall.exe").is_file()
        && fs::canonicalize(directory.join("nomo.exe"))
            .is_ok_and(|registered| registered == executable)
}

#[cfg(windows)]
pub(super) fn asset_rule(version: &str) -> Result<super::AssetRule, UiMessage> {
    if !cfg!(target_arch = "x86_64") {
        return Err(UiMessage::new("installer.architecture"));
    }
    Ok(super::AssetRule {
        name: format!("Nomo_{version}_x64-setup.exe"),
        limit: 128 * 1024 * 1024,
    })
}

pub(super) fn download_path(directory: &Path, source: DownloadSource) -> PathBuf {
    directory.join(format!("{SETUP_STEM}.{}.exe", source.file_tag()))
}

pub(super) fn stale_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths = vec![directory.join(format!("{SETUP_STEM}.exe"))];
    for source in DownloadSource::ALL {
        paths.push(download_path(directory, source));
        paths.push(directory.join(format!("{SETUP_STEM}.exe.{}", source.file_tag())));
    }
    paths
}

pub(crate) struct Verified {
    pub version: String,
    download: VerifiedDownload,
}

pub(super) fn finish(
    candidate: &Candidate,
    download: VerifiedDownload,
) -> Result<Verified, UiMessage> {
    let path = download.path();
    if !path.is_absolute() || path.extension() != Some(OsStr::new("exe")) {
        return Err(UiMessage::new("installer.invalidAsset"));
    }
    Ok(Verified {
        version: candidate.version.clone(),
        download,
    })
}

impl Verified {
    fn command(&self) -> Command {
        let path = self.download.path();
        let mut command = Command::new(path);
        command
            .current_dir(path.parent().expect("cache directory"))
            .arg(format!("/NOMOUPDATE={}", std::process::id()))
            .args(["/P", "/R"]);
        command
    }

    #[cfg(windows)]
    pub(crate) fn launch(&self) -> Result<(), UiMessage> {
        if !installed() {
            return Err(UiMessage::new("installer.locationChanged"));
        }
        self.command()
            .spawn()
            .map_err(|error| UiMessage::new("installer.launchFailed").arg("error", error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        DownloadCancel, cache_lease,
        tests::{test_candidate, test_download},
    };
    use super::*;

    #[test]
    fn both_commands_use_the_exact_verified_exe_path() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().canonicalize().unwrap();
        let lease = cache_lease(&directory).unwrap().unwrap();
        let candidate = test_candidate();
        let stale = stale_paths(&directory);
        assert!(stale.contains(&directory.join("nomo-update-x64-setup.exe")));
        let mut downloads = Vec::new();
        for source in DownloadSource::ALL {
            let path = download_path(&directory, source);
            let verified = finish(
                &candidate,
                test_download(
                    path.clone(),
                    lease.clone(),
                    &candidate,
                    &DownloadCancel::default(),
                ),
            )
            .unwrap();
            assert_eq!(verified.version, candidate.version);
            assert!(path.is_absolute());
            assert_eq!(path.extension(), Some(OsStr::new("exe")));
            assert_eq!(verified.command().get_program(), path.as_os_str());
            assert!(stale.contains(&path));
            assert!(stale.contains(
                &directory.join(format!("nomo-update-x64-setup.exe.{}", source.file_tag()))
            ));
            #[cfg(windows)]
            {
                assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
                assert!(std::fs::remove_file(&path).is_err());
            }
            downloads.push(verified);
        }
        assert_ne!(downloads[0].download.path(), downloads[1].download.path());
        let legacy = directory.join("nomo-update-x64-setup.exe.official");
        assert!(
            finish(
                &candidate,
                test_download(legacy, lease, &candidate, &DownloadCancel::default())
            )
            .is_err()
        );
        drop(downloads);
        assert!(
            DownloadSource::ALL
                .into_iter()
                .all(|source| !download_path(&directory, source).exists())
        );
    }
}

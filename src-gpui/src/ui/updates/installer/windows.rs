use super::{AssetRule, Candidate, StagedPath};
use crate::ui::toolbar_i18n::UiMessage;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
};

const LIMIT: u64 = 128 * 1024 * 1024;
const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo";

pub(super) fn installed() -> bool {
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

pub(super) fn asset_rule(version: &str) -> Result<AssetRule, UiMessage> {
    if !cfg!(target_arch = "x86_64") {
        return Err(UiMessage::new("installer.architecture"));
    }
    Ok(AssetRule {
        name: format!("Nomo_{version}_x64-setup.exe"),
        limit: LIMIT,
    })
}

pub(super) fn download_name() -> &'static str {
    "nomo-update-x64-setup.exe"
}

pub(super) fn stale_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = ["official", "gh-proxy"]
        .iter()
        .map(|source| directory.join(format!("{}.{source}", download_name())))
        .collect();
    paths.push(directory.join(download_name()));
    paths
}

pub(crate) struct Verified {
    pub version: String,
    _lock: File,
    path: StagedPath,
}

pub(super) fn finish(
    candidate: &Candidate,
    lock: File,
    path: StagedPath,
) -> Result<Verified, UiMessage> {
    Ok(Verified {
        version: candidate.version.clone(),
        _lock: lock,
        path,
    })
}

impl Verified {
    pub(crate) fn launch(&self) -> Result<(), UiMessage> {
        if !installed() {
            return Err(UiMessage::new("installer.locationChanged"));
        }
        Command::new(&self.path.path)
            .current_dir(self.path.path.parent().expect("cache directory"))
            .arg(format!("/NOMOUPDATE={}", std::process::id()))
            .args(["/P", "/R"])
            .spawn()
            .map_err(|error| UiMessage::new("installer.launchFailed").arg("error", error))?;
        Ok(())
    }
}

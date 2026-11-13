//! 当前用户的 Windows 文件集成；MSIX 菜单由包清单管理，非打包版沿用经典菜单。
use std::{fs, io, os::windows::fs::MetadataExt, path::PathBuf, ptr::{null, null_mut}};
use windows::Win32::{System::Com::CoTaskMemFree, UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_NO_PACKAGE_REDIRECTION, SHGetKnownFolderPath}};
use windows_registry::{CURRENT_USER, Key};
use windows_sys::Win32::{
    Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
    Storage::{FileSystem::FILE_ATTRIBUTE_REPARSE_POINT, Packaging::Appx::GetCurrentPackageFamilyName},
    UI::{
        Shell::{SHChangeNotify, ShellExecuteW, SHCNE_ASSOCCHANGED, SHCNF_FLUSHNOWAIT},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

const CLASSES: &str = r"Software\Classes";
const CAPABILITIES: &str = r"Software\Nomo\Capabilities";
const PROG_ID: &str = "Nomo.Markdown";
// Negative icon indices select a resource ID. Keep this in sync with build.rs and open-with.nsh.
const EXPLORER_ICON: &str = "-6";
const MENU_DISABLED_MARKER: &str = "shell-context-menu.disabled";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageIdentity { package_family_name: String, application_id: String, store_product_id: String }

fn package_identity() -> Result<PackageIdentity, String> {
    serde_json::from_str(include_str!("../../scripts/packaging/msix/package-identity.json"))
        .map_err(|e| format!("无法读取 Nomo 的包身份配置：{e}"))
}

/// 无包身份才允许使用经典入口；查询失败或身份不符不能降级为注册表写入。
pub(crate) fn current_package_family_name() -> Result<Option<String>, String> {
    // Windows 的包系列名称最多 64 个 UTF-16 单元；固定缓冲区避免按外部长度分配。
    let mut buffer = [0u16; 128];
    let mut length = buffer.len() as u32;
    let result = unsafe { GetCurrentPackageFamilyName(&mut length, buffer.as_mut_ptr()) };
    if result == APPMODEL_ERROR_NO_PACKAGE { return Ok(None); }
    if result != ERROR_SUCCESS {
        let detail = if result == ERROR_INSUFFICIENT_BUFFER { "包系列名称超出长度限制".to_owned() }
            else { io::Error::from_raw_os_error(result as i32).to_string() };
        return Err(format!("无法查询当前应用包身份：{detail}"));
    }
    let length = length as usize;
    if length < 2 || length > buffer.len() || buffer[length - 1] != 0 { return Err("Windows 返回的包系列名称无效。".into()); }
    let family = String::from_utf16(&buffer[..length - 1]).map_err(|_| "包系列名称不是有效的 Unicode。")?;
    let identity = package_identity()?;
    if family != identity.package_family_name || identity.application_id != "Nomo" {
        return Err("当前应用包身份与 Nomo 配置不一致，已停止系统集成操作。".into());
    }
    Ok(Some(family))
}

fn package_state_directory(family: &str) -> Result<PathBuf, String> {
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_NO_PACKAGE_REDIRECTION, None) }
        .map_err(|e| format!("无法读取本地应用数据目录：{e}"))?;
    let local = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0.cast())); }
    let local = PathBuf::from(local.map_err(|e| format!("应用数据目录无效：{e}"))?);
    if !local.is_absolute() { return Err("应用数据目录必须是绝对路径。".into()); }
    let packages = local.join("Packages");
    let package = packages.join(family);
    for directory in [&packages, &package] {
        let metadata = fs::symlink_metadata(directory).map_err(|e| format!("无法检查当前应用包目录：{e}"))?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("当前应用包目录不是普通目录，已停止修改菜单状态。".into());
        }
    }
    let state = package.join("LocalState");
    match fs::create_dir(&state) {
        Ok(()) => {},
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(format!("无法创建当前应用包状态目录：{e}")),
    }
    let metadata = fs::symlink_metadata(&state).map_err(|e| format!("无法检查应用包状态目录：{e}"))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("当前应用包状态目录不是普通目录，已停止修改菜单状态。".into());
    }
    Ok(state)
}

fn set_package_menu_enabled(family: &str, enabled: bool) -> Result<(), String> {
    let marker = package_state_directory(family)?.join(MENU_DISABLED_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("右键菜单状态文件不是普通文件，已停止修改。".into());
            }
            if enabled { fs::remove_file(&marker).map_err(|e| format!("无法启用当前包的右键菜单：{e}"))?; }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if !enabled {
                // 标记只以“存在”表示禁用，排他创建无需改写文件内容或保留临时文件。
                fs::OpenOptions::new().write(true).create_new(true).open(&marker)
                    .map_err(|e| format!("无法禁用当前包的右键菜单：{e}"))?;
            }
        }
        Err(e) => return Err(format!("无法读取右键菜单状态：{e}")),
    }
    Ok(())
}
// 沿用原版的 verb 名称，便携版移动后再次注册即可更新位置，不重复添加菜单。
const MENU_VERBS: [(&str, &str, &str); 4] = [
    (r"SystemFileAssociations\.md\shell\Nomo.Open", "用 Nomo 打开", "%1"),
    (r"SystemFileAssociations\.markdown\shell\Nomo.Open", "用 Nomo 打开", "%1"),
    (r"Directory\shell\Nomo.OpenFolder", "用 Nomo 打开文件夹", r"%1\."),
    (r"Directory\Background\shell\Nomo.OpenFolder", "用 Nomo 打开文件夹", r"%V\."),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellAction { BindMarkdown, RegisterMenu, RemoveMenu }

impl ShellAction {
    pub(crate) fn apply(self) -> Result<&'static str, String> {
        if let Some(family) = current_package_family_name()? {
            let message = match self {
                Self::BindMarkdown => "shell.chooseDefault",
                Self::RegisterMenu => { set_package_menu_enabled(&family, true)?; "shell.packageEnabled" },
                Self::RemoveMenu => { set_package_menu_enabled(&family, false)?; "shell.packageDisabled" },
            };
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_FLUSHNOWAIT, null(), null()); }
            return Ok(message);
        }
        let path = std::env::current_exe().map_err(|e| format!("无法读取 Nomo 的位置：{e}"))?;
        let exe = path.to_str().ok_or("Nomo 的启动路径不是有效的 Unicode 路径。")?;
        let message = match self {
            Self::BindMarkdown => register_markdown(CURRENT_USER, exe)
                .map(|()| "shell.chooseDefault"),
            Self::RegisterMenu => change_menu(CURRENT_USER, exe, true)
                .map(|_| "shell.menuRegistered"),
            Self::RemoveMenu => change_menu(CURRENT_USER, exe, false)
                .map(|removed| if removed > 0 { "shell.menuRemoved" } else { "shell.noMenu" }),
        }.map_err(|e| format!("Windows 注册表操作未完成：{e}"))?;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_FLUSHNOWAIT, null(), null()); }
        Ok(message)
    }
}

fn register_markdown(root: &Key, exe: &str) -> windows_registry::Result<()> {
    let prog = root.create(format!(r"{CLASSES}\{PROG_ID}"))?;
    prog.set_string("", "Markdown Document")?;
    prog.create("DefaultIcon")?.set_string("", format!("\"{exe}\",{EXPLORER_ICON}"))?;
    prog.create(r"shell\open\command")?.set_string("", format!("\"{exe}\" \"%1\""))?;

    let caps = root.create(CAPABILITIES)?;
    caps.set_string("ApplicationName", "Nomo")?;
    caps.set_string("ApplicationDescription", "Nomo Markdown 编辑器")?;
    let types = caps.create("FileAssociations")?;
    for extension in [".md", ".markdown"] {
        types.set_string(extension, PROG_ID)?;
        root.create(format!(r"{CLASSES}\{extension}\OpenWithProgids"))?.set_string(PROG_ID, "")?;
    }
    // 只发布候选；扩展名默认值和 Explorer 的 UserChoice 均归系统与用户管理。
    root.create(r"Software\RegisteredApplications")?.set_string("Nomo", CAPABILITIES)
}

fn change_menu(root: &Key, exe: &str, register: bool) -> windows_registry::Result<usize> {
    let mut changed = 0;
    for (verb, label, argument) in MENU_VERBS {
        let key = format!(r"{CLASSES}\{verb}");
        // 目录以 \. 结尾，即使 %V 是磁盘根目录，其末尾的反斜杠也不会转义闭引号。
        let command = format!("\"{exe}\" \"{argument}\"");
        if register {
            let menu = root.create(&key)?;
            menu.create("command")?.set_string("", &command)?;
            menu.set_string("Icon", format!("\"{exe}\",{EXPLORER_ICON}"))?;
            menu.set_string("MUIVerb", label)?;
        } else {
            let existing = root.open(format!(r"{key}\command")).and_then(|k| k.get_string(""));
            let existing = match existing {
                Ok(value) => value,
                Err(e) if [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND].into_iter()
                    .any(|code| e.code() == windows::core::HRESULT::from_win32(code)) => continue,
                Err(e) => return Err(e),
            };
            let legacy = format!("\"{exe}\" \"{}\"", argument.trim_end_matches(r"\."));
            if !existing.eq_ignore_ascii_case(&command) && !existing.eq_ignore_ascii_case(&legacy) { continue; }
            root.remove_tree(&key)?;
        }
        changed += 1;
    }
    Ok(changed)
}

pub(crate) fn open_default_apps() -> Result<(), String> {
    let target = if let Some(family) = current_package_family_name()? {
        let mut uri = url::Url::parse("ms-settings:defaultapps").expect("fixed Windows settings URI");
        uri.query_pairs_mut().append_pair("registeredAUMID", &format!("{family}!Nomo"));
        uri.to_string()
    } else { "ms-settings:defaultapps?registeredAppUser=Nomo".into() };
    // Windows 11 支持直达当前用户的应用页；旧系统仍可使用通用默认应用页。
    for uri in [target.as_str(), "ms-settings:defaultapps"] {
        if open_system_uri(uri).is_ok() { return Ok(()); }
    }
    Err("Nomo 已加入候选列表，但未能打开系统设置。请手动打开 Windows 设置 → 应用 → 默认应用，为 .md 选择 Nomo。".into())
}

fn open_system_uri(uri: &str) -> Result<(), String> {
    let uri: Vec<_> = uri.encode_utf16().chain(Some(0)).collect();
    let result = unsafe { ShellExecuteW(null_mut(), null(), uri.as_ptr(), null(), null(), SW_SHOWNORMAL) } as isize;
    if result > 32 { Ok(()) } else { Err(format!("Windows 未能打开系统入口，错误码：{result}")) }
}

pub(crate) fn open_store_updates() -> Result<(), String> {
    current_package_family_name()?.ok_or("当前 Nomo 没有应用包身份。")?;
    let identity = package_identity()?;
    if identity.store_product_id.len() != 12 || !identity.store_product_id.bytes().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("Nomo 的 Microsoft Store 产品标识无效。".into());
    }
    open_system_uri(&format!("ms-windows-store://pdp/?ProductId={}", identity.store_product_id))
        .map_err(|e| format!("无法打开 Microsoft Store，请从商店的库中检查 Nomo 更新。{e}"))
}

//! Windows 当前用户的 Markdown 打开候选和资源管理器经典菜单。只在设置中显式操作。
use std::ptr::{null, null_mut};
use windows_registry::{CURRENT_USER, Key};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND},
    UI::{
        Shell::{SHChangeNotify, ShellExecuteW, SHCNE_ASSOCCHANGED, SHCNF_FLUSHNOWAIT},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

const CLASSES: &str = r"Software\Classes";
const CAPABILITIES: &str = r"Software\Nomo\Capabilities";
const PROG_ID: &str = "Nomo.Markdown";
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
    prog.create("DefaultIcon")?.set_string("", format!("\"{exe}\",0"))?;
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
            menu.set_string("Icon", format!("\"{exe}\",0"))?;
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
    // Windows 11 支持直达当前用户的应用页；旧系统仍可使用通用默认应用页。
    for uri in ["ms-settings:defaultapps?registeredAppUser=Nomo", "ms-settings:defaultapps"] {
        let uri: Vec<_> = uri.encode_utf16().chain(Some(0)).collect();
        let result = unsafe { ShellExecuteW(null_mut(), null(), uri.as_ptr(), null(), null(), SW_SHOWNORMAL) };
        if result as isize > 32 { return Ok(()); }
    }
    Err("Nomo 已加入候选列表，但未能打开系统设置。请手动打开 Windows 设置 → 应用 → 默认应用，为 .md 选择 Nomo。".into())
}

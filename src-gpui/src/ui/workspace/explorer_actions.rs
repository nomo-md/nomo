//! 目录树菜单及文件操作。磁盘操作由用户触发，运行于后台，并保护打开的文档。
use super::*;
use std::io::Write;

#[derive(Clone, Copy)]
pub(super) enum TreeAction { NewFile, NewFolder, Refresh, Collapse, Open, Pin, Rename, CopyFile, CopyPath, Reveal, Delete }

pub(super) struct TreeMenu {
    path: PathBuf,
    directory: bool,
    root: bool,
    position: gpui::Point<gpui::Pixels>,
}

#[derive(Clone, Copy)]
pub(super) enum EditKind { NewFile, NewFolder, Rename }

pub(super) struct TreeEdit {
    pub(super) kind: EditKind,
    pub(super) path: PathBuf,
    pub(super) field: Entity<TextField>,
    _subscription: Subscription,
    _blur: Subscription,
}

impl Workspace {
    pub(super) fn tree_create_insertion(&self, rows: &[crate::ui::file_tree::Row]) -> Option<usize> {
        let edit = self.explorer.edit.as_ref()?;
        if matches!(edit.kind, EditKind::Rename) { return None; }
        let tree = self.tree.as_ref()?;
        if edit.path == tree.root { return tree.root_expanded().then_some(1); }
        rows.iter().position(|row| row.entry.path == edit.path && row.expanded).map(|index| index + 2)
    }

    pub(super) fn open_tree_menu(&mut self, path: PathBuf, directory: bool, root: bool, position: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        self.explorer.menu = Some(TreeMenu { path, directory, root, position });
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_tree_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.explorer.menu.as_ref()?;
        let mut items = Vec::new();
        if menu.directory {
            items.extend([(TreeAction::NewFile, "新建文件"), (TreeAction::NewFolder, "新建文件夹")]);
        } else {
            items.extend([(TreeAction::Open, "预览打开"), (TreeAction::Pin, "固定打开")]);
        }
        if !menu.root { items.push((TreeAction::Rename, "重命名")); }
        if !menu.directory { items.push((TreeAction::CopyFile, "复制文件")); }
        items.extend([(TreeAction::CopyPath, "复制路径"), (TreeAction::Reveal, "在文件管理器中显示")]);
        if menu.directory { items.push((TreeAction::Refresh, "刷新")); }
        if menu.root { items.push((TreeAction::Collapse, "全部折叠")); }
        else { items.push((TreeAction::Delete, "删除")); }
        let mut panel = div().id("tree-context-menu").w(px(198.0)).py(px(5.0)).flex().flex_col()
            .rounded(px(8.0)).bg(theme.surface).border_1().border_color(theme.border).shadow_lg().occlude()
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| { workspace.explorer.menu = None; cx.notify(); }));
        for (index, (action, label)) in items.into_iter().enumerate() {
            let path = menu.path.clone();
            let disabled = matches!(action, TreeAction::CopyFile) && !cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux"));
            panel = panel.child(div().id(("tree-menu-item", index)).h(px(29.0)).px(px(14.0)).flex().items_center()
                .text_size(px(13.0)).text_color(theme.fg).when(disabled, |el| el.opacity(0.45).tooltip(tooltip("当前平台暂不支持复制文件")))
                .when(!disabled, |el| el.cursor_pointer().hover(|style| style.bg(theme.hover)))
                .child(label).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |workspace, _, window, cx| {
                    cx.stop_propagation();
                    workspace.explorer.menu = None;
                    if !disabled { workspace.run_tree_action(action, path.clone(), window, cx); }
                    cx.notify();
                })));
        }
        Some(gpui::deferred(gpui::anchored().position(menu.position).snap_to_window_with_margin(px(8.0)).child(panel)).with_priority(3).into_any_element())
    }

    pub(super) fn run_tree_action(&mut self, action: TreeAction, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.explorer.mutating.iter().any(|busy| path.starts_with(busy) || busy.starts_with(&path)) {
            self.notice = Some("文件操作正在进行，请稍后重试。".into());
            cx.notify();
            return;
        }
        match action {
            TreeAction::NewFile => self.start_tree_edit(EditKind::NewFile, path, window, cx),
            TreeAction::NewFolder => self.start_tree_edit(EditKind::NewFolder, path, window, cx),
            TreeAction::Rename => self.start_tree_edit(EditKind::Rename, path, window, cx),
            TreeAction::Refresh => self.refresh_tree(cx),
            TreeAction::Collapse => {
                if let Some(tree) = &mut self.tree { tree.collapse_all(); }
                self.cancel_hidden_tree_scan();
                self.sync_tree_selection(false, cx);
                cx.notify();
            }
            TreeAction::Open => self.open_path_with_preview(&path, true, window, cx),
            TreeAction::Pin => self.open_path_with_preview(&path, false, window, cx),
            TreeAction::CopyPath => cx.write_to_clipboard(gpui::ClipboardItem::new_string(display_tree_path(&path))),
            TreeAction::Reveal => cx.reveal_path(&path),
            TreeAction::Delete => self.confirm_tree_delete(path, window, cx),
            TreeAction::CopyFile => {
                if let Err(error) = copy_file_to_clipboard(&path, cx) { self.notice = Some(error.into()); cx.notify(); }
            },
        }
    }

    fn start_tree_edit(&mut self, kind: EditKind, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(kind, EditKind::Rename) {
            if let Some(tree) = &mut self.tree {
                if let Err(error) = tree.expand(&path) { self.notice = Some(error.into()); cx.notify(); return; }
            }
            self.queue_tree_scan(path.clone(), false, cx);
        }
        let field = cx.new(|cx| TextField::new("名称", cx));
        if matches!(kind, EditKind::Rename) {
            let name = path.file_name().map(|v| v.to_string_lossy().into_owned()).unwrap_or_default();
            let is_dir = self.tree.as_ref().is_some_and(|tree| tree.rows().iter().any(|row| row.entry.path == path && row.entry.is_dir));
            field.update(cx, |field, cx| {
                field.set_text(&name, cx);
                if !is_dir { field.select_file_stem(cx); }
            });
        }
        let subscription = cx.subscribe_in(&field, window, |workspace, field, event, window, cx| {
            if field.read(cx).is_composing() { return; }
            match event {
                FieldEvent::Submit { .. } => workspace.commit_tree_edit(window, cx),
                FieldEvent::Cancel => { workspace.explorer.edit = None; cx.notify(); },
                FieldEvent::Changed => {},
            }
        });
        let focus = field.focus_handle(cx);
        let editing = field.clone();
        let blur = cx.on_blur(&focus, window, move |workspace, _, cx| {
            if workspace.explorer.edit.as_ref().is_some_and(|edit| edit.field == editing) {
                workspace.explorer.edit = None;
                cx.notify();
            }
        });
        self.explorer.edit = Some(TreeEdit { kind, path: path.clone(), field, _subscription: subscription, _blur: blur });
        if let Some(tree) = &self.tree {
            let row = tree.rows().iter().position(|row| row.entry.path == path).map_or(0, |i| i + 1);
            self.tree_scroll.scroll_to_item(row + usize::from(!matches!(kind, EditKind::Rename)), ScrollStrategy::Bottom);
        }
        window.focus(&focus, cx);
        cx.notify();
    }

    fn commit_tree_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.explorer.edit.take() else { return; };
        let name = edit.field.read(cx).content.trim().to_string();
        if name.is_empty() { cx.notify(); return; }
        if let Err(error) = validate_name(&name) { self.notice = Some(error.into()); self.explorer.edit = Some(edit); cx.notify(); return; }
        let Some(root) = self.tree.as_ref().map(|tree| tree.root.clone()) else { return; };
        let generation = self.explorer.generation();
        let path = edit.path;
        let kind = edit.kind;
        if matches!(kind, EditKind::Rename) {
            let Some(manager) = self.services.clone() else { self.notice = Some("文档会话不可用，无法安全重命名。".into()); cx.notify(); return; };
            let busy_paths = self.tree_busy_paths(&path);
            self.explorer.mutating.extend(busy_paths.iter().cloned());
            let task = cx.background_executor().spawn(async move { rename_tree_path(&root, &path, &name, &manager) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |workspace, cx| {
                    for path in &busy_paths { workspace.explorer.mutating.remove(path); }
                    if let Err(error) = &result { workspace.notice = Some(error.clone().into()); }
                    cx.notify();
                });
                if let Ok(report) = result {
                    let _ = cx.update(|app| broadcast_tree_rename(app, &report.paths, report.warning.as_deref()));
                }
            }).detach();
            return;
        }
        let task = cx.background_executor().spawn(async move { apply_tree_edit(&root, &path, kind, &name) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |workspace, window, cx| {
                if workspace.explorer.generation() != generation { return; }
                match result {
                    Ok(path) => {
                        workspace.refresh_tree(cx);
                        if matches!(kind, EditKind::NewFile) { workspace.open_path(&path, window, cx); }
                    }
                    Err(error) => workspace.notice = Some(error.into()),
                }
                cx.notify();
            });
        }).detach();
    }

    fn tree_path_open(&self, path: &Path, cx: &App) -> bool {
        let resolved = self.tree.as_ref().and_then(|tree| tree.rows().iter().find(|row| row.entry.path == path)
            .and_then(|row| row.entry.resolved_path.clone()));
        let matches = |open: &Path| open == path || open.starts_with(path)
            || resolved.as_deref().is_some_and(|resolved| open == resolved || open.starts_with(resolved));
        self.tabs.iter().filter_map(|tab| tab.document.read(cx).path()).any(|open| matches(&open))
            || self.opening.iter().filter_map(|opening| opening.path.as_deref()).any(matches)
    }

    fn tree_busy_paths(&self, path: &Path) -> Vec<PathBuf> {
        let mut paths = vec![path.to_path_buf()];
        if let Some(resolved) = self.tree.as_ref().and_then(|tree| tree.rows().iter()
            .find(|row| row.entry.path == path).and_then(|row| row.entry.resolved_path.clone())) {
            if resolved != path { paths.push(resolved); }
        }
        paths
    }

    fn confirm_tree_delete(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree_path_open(&path, cx) {
            self.notice = Some("请先关闭该文件或目录内已打开的文档，再删除。".into()); cx.notify(); return;
        }
        let Some(root) = self.tree.as_ref().map(|tree| tree.root.clone()) else { return; };
        let generation = self.explorer.generation();
        let response = window.prompt(PromptLevel::Warning, &format!("确定删除“{}”？", path.display()), Some("此操作会永久删除，无法撤销。"), &["删除", "取消"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if response.await.ok() != Some(0) { return; }
            let _ = this.update_in(cx, |workspace, window, cx| {
                if workspace.explorer.generation() != generation { return; }
                if workspace.tree_path_open(&path, cx) {
                    workspace.notice = Some("请先关闭该文件或目录内已打开的文档，再删除。".into()); cx.notify(); return;
                }
                let busy_paths = workspace.tree_busy_paths(&path);
                workspace.explorer.mutating.extend(busy_paths.iter().cloned());
                let manager = workspace.services.clone();
                let task = cx.background_executor().spawn(async move {
                    let manager = manager.ok_or("无法检查其他窗口的文档，删除已取消。")?;
                    manager.with_unopened_path(&path, || delete_tree_path(&root, &path))
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    let _ = this.update_in(cx, |workspace, _, cx| {
                        for path in &busy_paths { workspace.explorer.mutating.remove(path); }
                        if workspace.explorer.generation() != generation { return; }
                        if let Err(error) = result { workspace.notice = Some(error.into()); }
                        workspace.refresh_tree(cx);
                        cx.notify();
                    });
                }).detach();
            });
        }).detach();
    }
}

struct TreeRenameReport {
    paths: Vec<(PathBuf, PathBuf)>,
    warning: Option<String>,
}

fn rename_tree_path(root: &Path, path: &Path, name: &str, manager: &MarkdownDocumentSessionManager) -> Result<TreeRenameReport, String> {
    validate_name(name)?;
    let source = checked_existing(root, path)?;
    let destination = source.parent().ok_or("目标没有父目录。")?.join(name);
    if source == destination { return Ok(TreeRenameReport { paths: Vec::new(), warning: None }); }
    let outcome = manager.rename_path(&source, &destination, || {
        rename_no_replace(&source, &destination).map_err(|error| format!("重命名失败：{error}"))
    })?;
    let mut paths = vec![(source, destination)];
    let logical = (path.to_path_buf(), path.with_file_name(name));
    if paths[0] != logical { paths.push(logical); }
    Ok(TreeRenameReport { paths, warning: outcome.warning })
}

fn broadcast_tree_rename(app: &mut App, paths: &[(PathBuf, PathBuf)], warning: Option<&str>) {
    if paths.is_empty() { return; }
    for handle in app.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
        if let Err(error) = handle.update(app, |workspace, window, cx| {
            let mut changed = false;
            for tab in &workspace.tabs {
                changed |= tab.document.update(cx, |host, cx| host.refresh_renamed_path(cx));
            }
            for opening in &mut workspace.opening {
                if let Some(path) = &mut opening.path { remap_tree_path(path, paths); }
            }
            if let Some((path, _)) = &mut workspace.deferred_open { remap_tree_path(path, paths); }
            workspace.apply_tree_rename(paths, cx);
            if changed {
                workspace.sync_tree_selection(false, cx);
                if let Some(tab) = workspace.tab() {
                    window.set_window_title(&format!("{} — Nomo", tab.document.read(cx).title()));
                }
            }
            if let Some(warning) = warning { workspace.notice = Some(warning.to_string().into()); }
            cx.notify();
        }) {
            crate::app_logger::warn("FileTree", &format!("同步重命名后的窗口失败：{error}"));
        }
    }
}

fn remap_tree_path(path: &mut PathBuf, paths: &[(PathBuf, PathBuf)]) {
    for (old, new) in paths {
        if let Ok(suffix) = path.strip_prefix(old) {
            *path = if suffix.as_os_str().is_empty() { new.clone() } else { new.join(suffix) };
            break;
        }
    }
}

pub(super) fn display_tree_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(target_os = "windows")]
    {
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") { return format!(r"\\{rest}"); }
        if let Some(rest) = value.strip_prefix(r"\\?\") { return rest.to_string(); }
    }
    value.into_owned()
}

#[cfg(target_os = "windows")]
fn copy_file_to_clipboard(path: &Path, _: &mut App) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::c_void;
    #[repr(C)]
    struct DropFiles { offset: u32, x: i32, y: i32, non_client: i32, wide: i32 }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetActiveWindow() -> *mut c_void;
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }
    if !path.is_file() { return Err("无法复制：文件不存在或不可访问。".into()); }
    let original = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut wide = if original.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
        let mut value = vec![92, 92]; value.extend_from_slice(&original[8..]); value
    } else if original.starts_with(&[92, 92, 63, 92]) { original[4..].to_vec() } else { original };
    wide.extend([0, 0]);
    let bytes = std::mem::size_of::<DropFiles>() + wide.len() * 2;
    unsafe {
        let memory = GlobalAlloc(2, bytes);
        if memory.is_null() { return Err("分配剪贴板内存失败。".into()); }
        let data = GlobalLock(memory);
        if data.is_null() { GlobalFree(memory); return Err("无法访问剪贴板内存。".into()); }
        data.cast::<DropFiles>().write(DropFiles { offset: std::mem::size_of::<DropFiles>() as u32, x: 0, y: 0, non_client: 0, wide: 1 });
        std::ptr::copy_nonoverlapping(wide.as_ptr(), (data as *mut u8).add(std::mem::size_of::<DropFiles>()).cast::<u16>(), wide.len());
        GlobalUnlock(memory);
        let owner = GetActiveWindow();
        if owner.is_null() || OpenClipboard(owner) == 0 { GlobalFree(memory); return Err("无法打开系统剪贴板。".into()); }
        let ok = EmptyClipboard() != 0 && !SetClipboardData(15, memory).is_null();
        CloseClipboard();
        if !ok { GlobalFree(memory); return Err("复制文件到剪贴板失败。".into()); }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct FileClipboard(arboard::Clipboard);
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl gpui::Global for FileClipboard {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn copy_file_to_clipboard(path: &Path, cx: &mut App) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() { return Err("无法复制：文件不存在或不可访问。".into()); }
    if cx.try_global::<FileClipboard>().is_none() {
        let clipboard = arboard::Clipboard::new().map_err(|error| format!("无法打开系统剪贴板：{error}"))?;
        cx.set_global(FileClipboard(clipboard));
    }
    cx.global_mut::<FileClipboard>().0.set().file_list(&[path])
        .map_err(|error| format!("复制文件到剪贴板失败：{error}"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn copy_file_to_clipboard(_: &Path, _: &mut App) -> Result<(), String> { Err("当前平台暂不支持复制文件。".into()) }

fn validate_name(name: &str) -> Result<(), String> {
    if name == "." || name == ".." || name.chars().any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch)) || name.ends_with([' ', '.']) {
        return Err("名称包含不允许的字符。".into());
    }
    Ok(())
}

fn checked_parent(root: &Path, parent: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| format!("无法访问根目录：{error}"))?;
    let parent = parent.canonicalize().map_err(|error| format!("无法访问目录：{error}"))?;
    if !parent.starts_with(&root) { return Err("目标不在当前根目录中。".into()); }
    Ok(parent)
}

fn checked_existing(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path == root { return Err("不能修改根目录。".into()); }
    let parent = checked_parent(root, path.parent().ok_or("目标没有父目录。")?)?;
    let name = path.file_name().ok_or("目标名称无效。")?;
    let target = parent.join(name);
    let metadata = std::fs::symlink_metadata(&target).map_err(|error| format!("无法访问目标：{error}"))?;
    if metadata.file_type().is_symlink() { return Err("不能通过符号链接执行此操作。".into()); }
    Ok(target)
}

fn apply_tree_edit(root: &Path, path: &Path, kind: EditKind, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    if matches!(kind, EditKind::Rename) { return Err("重命名必须通过文档会话执行。".into()); }
    let parent = checked_parent(root, path)?;
    let name = if matches!(kind, EditKind::NewFile) && !name.to_ascii_lowercase().ends_with(".md") { format!("{name}.md") } else { name.to_string() };
    let base = if matches!(kind, EditKind::NewFile) { &name[..name.len() - 3] } else { &name };
    for suffix in 0..10_000 {
        let candidate = if suffix == 0 { name.clone() } else if matches!(kind, EditKind::NewFile) { format!("{base} ({suffix}).md") } else { format!("{name} ({suffix})") };
        let target = parent.join(&candidate);
        let result = if matches!(kind, EditKind::NewFolder) {
            std::fs::create_dir(&target)
        } else {
            std::fs::OpenOptions::new().write(true).create_new(true).open(&target).and_then(|mut file| {
                file.write_all(format!("# {}\n\n", &candidate[..candidate.len() - 3]).as_bytes())
            })
        };
        match result {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("创建失败：{error}")),
        }
    }
    Err("同名文件过多，无法创建。".into())
}

fn delete_tree_path(root: &Path, path: &Path) -> Result<(), String> {
    let target = checked_existing(root, path)?;
    let metadata = std::fs::symlink_metadata(&target).map_err(|error| format!("无法访问目标：{error}"))?;
    if metadata.is_dir() { std::fs::remove_dir_all(&target) } else { std::fs::remove_file(&target) }
        .map_err(|error| format!("删除失败：{error}"))
}

#[cfg(target_os = "windows")]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" { fn MoveFileW(existing: *const u16, new: *const u16) -> i32; }
    let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(target_os = "linux")]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" { fn renameat2(oldfd: i32, old: *const i8, newfd: i32, new: *const i8, flags: u32) -> i32; }
    let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
    if unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(target_os = "macos")]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" { fn renamex_np(old: *const i8, new: *const i8, flags: u32) -> i32; }
    let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
    if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 4) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

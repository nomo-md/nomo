//! 目录树菜单及文件操作。磁盘操作由用户触发，运行于后台，并保护打开的文档。
use super::*;
use crate::file_system::rename_no_replace;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
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
    pub(super) fn place_tree_entry(&mut self, item: crate::ui::path_drag::PathDrag, placement: crate::ui::tree_drag::Placement, cx: &mut Context<Self>) {
        if placement.parent.as_os_str().is_empty() || item.path.parent() == Some(&placement.parent) {
            self.library.update(cx, |library, cx| library.change(LibraryChange::Place { placement, path: item.path }, cx));
        } else {
            self.move_tree_entry(item, placement.root.clone(), placement.parent.clone(), Some(placement), cx);
        }
    }

    pub(super) fn move_tree_entry(&mut self, item: crate::ui::path_drag::PathDrag, root: PathBuf, destination: PathBuf, placement: Option<crate::ui::tree_drag::Placement>, cx: &mut Context<Self>) {
        let source = item.path.clone();
        let blocked = self.explorer.mutating.iter().any(|busy| source.starts_with(busy) || busy.starts_with(&source) || destination.starts_with(busy));
        let Some(manager) = self.services.clone().filter(|_| !blocked) else {
            let error = UiMessage::new(if blocked { "tree.busy" } else { "tree.sessionUnavailable" });
            self.home.update(cx, |home, cx| home.move_finished(&source, &destination, Some(error.clone()), cx));
            self.explorer.drag.end(self.explorer.reduce_motion);
            self.notice = Some(error.render(ToolbarLocale::current(cx)).into()); cx.notify(); return;
        };
        self.explorer.mutating.insert(source.clone());
        let target = destination.clone();
        let task = cx.background_executor().spawn(async move { move_tree_path(&item, &root, &target, &manager) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Ok(report) = &result { let _ = cx.update(|app| broadcast_tree_rename(app, &report.paths, report.warning.as_deref())); }
            let _ = this.update(cx, |workspace, cx| {
                workspace.explorer.mutating.remove(&source);
                if result.is_ok() {
                    if let (Some(placement), Some(name)) = (placement, source.file_name()) {
                        let path = destination.join(name);
                        workspace.library.update(cx, |library, cx| library.change(LibraryChange::Place { placement, path }, cx));
                    }
                } else { workspace.explorer.drag.end(workspace.explorer.reduce_motion); }
                let error = result.err();
                if let Some(error) = &error { workspace.notice = Some(error.render(ToolbarLocale::current(cx)).into()); }
                workspace.home.update(cx, |home, cx| home.move_finished(&source, &destination, error, cx));
                workspace.refresh_tree(cx);
                cx.notify();
            });
        }).detach();
    }

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
        let locale = ToolbarLocale::current(cx);
        let mut items = Vec::new();
        if menu.directory {
            items.extend([(TreeAction::NewFile, "tree.newFile"), (TreeAction::NewFolder, "tree.newFolder")]);
        } else {
            items.extend([(TreeAction::Open, "tree.preview"), (TreeAction::Pin, "tree.pin")]);
        }
        if !menu.root { items.push((TreeAction::Rename, "tree.rename")); }
        if !menu.directory { items.push((TreeAction::CopyFile, "tree.copyFile")); }
        items.extend([(TreeAction::CopyPath, "tree.copyPath"), (TreeAction::Reveal, "tree.reveal")]);
        if menu.directory { items.push((TreeAction::Refresh, "tree.refresh")); }
        if menu.root { items.push((TreeAction::Collapse, "tree.collapseAll")); }
        else { items.push((TreeAction::Delete, "tree.delete")); }
        let mut panel = div().id("tree-context-menu").w(px(if locale.is_chinese() { 198.0 } else { 252.0 })).py(px(5.0)).flex().flex_col()
            .rounded(px(8.0)).bg(theme.surface).border_1().border_color(theme.border).shadow_lg().occlude()
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| { workspace.explorer.menu = None; cx.notify(); }));
        for (index, (action, label)) in items.into_iter().enumerate() {
            let path = menu.path.clone();
            let disabled = matches!(action, TreeAction::CopyFile) && !cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux"));
            panel = panel.child(div().id(("tree-menu-item", index)).h(px(29.0)).px(px(14.0)).flex().flex_none().items_center()
                .text_size(px(13.0)).text_color(theme.fg).when(disabled, |el| el.opacity(0.45).tooltip(tooltip(locale.app_text("tree.copyUnsupported"))))
                .when(!disabled, |el| el.cursor_pointer().hover(|style| style.bg(theme.hover)))
                .child(locale.app_text(label)).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |workspace, _, window, cx| {
                    cx.stop_propagation();
                    workspace.explorer.menu = None;
                    if !disabled { workspace.run_tree_action(action, path.clone(), window, cx); }
                    cx.notify();
                })));
        }
        Some(gpui::deferred(popup("tree-menu-enter",
            PopupAnchor::Bounds(gpui::Bounds::new(menu.position, gpui::size(px(0.0), px(0.0)))),
            PopupPlacement::BelowLeft, panel, self.explorer.reduce_motion)).with_priority(3).into_any_element())
    }

    pub(super) fn run_tree_action(&mut self, action: TreeAction, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.explorer.mutating.iter().any(|busy| path.starts_with(busy) || busy.starts_with(&path)) {
            self.notice = Some(ToolbarLocale::current(cx).app_text("tree.busy").into());
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
            TreeAction::Reveal => Self::reveal_in_file_manager(path, cx),
            TreeAction::Delete => self.confirm_tree_delete(path, window, cx),
            TreeAction::CopyFile => {
                if let Err(error) = copy_file_to_clipboard(&path, cx) { self.notice = Some(error.render(ToolbarLocale::current(cx)).into()); cx.notify(); }
            },
        }
    }

    pub(super) fn reveal_in_file_manager(path: PathBuf, cx: &mut Context<Self>) {
        crate::ui::file_manager::reveal(path, cx, |workspace, message, cx| {
            workspace.failure_notice = Some(message.into());
            cx.notify();
        });
    }

    fn start_tree_edit(&mut self, kind: EditKind, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(kind, EditKind::Rename) {
            if let Some(tree) = &mut self.tree {
                if let Err(error) = tree.expand_message(&path) { self.notice = Some(error.render(ToolbarLocale::current(cx)).into()); cx.notify(); return; }
            }
            self.queue_tree_scan(path.clone(), false, cx);
        }
        let field = cx.new(|cx| TextField::new(ToolbarLocale::current(cx).app_text("tree.name"), cx));
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
        if let Err(error) = validate_name(&name) { self.notice = Some(error.render(ToolbarLocale::current(cx)).into()); self.explorer.edit = Some(edit); cx.notify(); return; }
        let Some(root) = self.tree.as_ref().map(|tree| tree.root.clone()) else { return; };
        let generation = self.explorer.generation();
        let path = edit.path;
        let kind = edit.kind;
        if matches!(kind, EditKind::Rename) {
            let Some(manager) = self.services.clone() else { self.notice = Some(ToolbarLocale::current(cx).app_text("tree.sessionUnavailable").into()); cx.notify(); return; };
            let busy_paths = self.tree_busy_paths(&path);
            self.explorer.mutating.extend(busy_paths.iter().cloned());
            let task = cx.background_executor().spawn(async move { rename_tree_path(&root, &path, &name, &manager) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |workspace, cx| {
                    for path in &busy_paths { workspace.explorer.mutating.remove(path); }
                    if let Err(error) = &result { workspace.notice = Some(error.render(ToolbarLocale::current(cx)).into()); }
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
                        if matches!(kind, EditKind::NewFile) {
                            workspace.pending_tree_click.take();
                            workspace.deferred_open = None;
                            workspace.enqueue_open_request(OpenRequest { path: Some(path), newly_created: true, ..Default::default() }, cx);
                            workspace.drain_open_requests(window, cx);
                        }
                    }
                    Err(error) => workspace.notice = Some(error.render(ToolbarLocale::current(cx)).into()),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn tree_busy_paths(&self, path: &Path) -> Vec<PathBuf> {
        let mut paths = vec![path.to_path_buf()];
        if let Some(resolved) = self.tree.as_ref().and_then(|tree| tree.rows().iter()
            .find(|row| row.entry.path == path).and_then(|row| row.entry.resolved_path.clone())) {
            if resolved != path { paths.push(resolved); }
        }
        paths
    }

}

struct TreeRenameReport {
    paths: Vec<(PathBuf, PathBuf)>,
    warning: Option<String>,
}

fn move_tree_path(item: &crate::ui::path_drag::PathDrag, root: &Path, directory: &Path, manager: &MarkdownDocumentSessionManager) -> Result<TreeRenameReport, UiMessage> {
    let source = checked_existing(&item.root, &item.path)?;
    let parent = checked_parent(root, directory)?;
    // 以解析后的绝对路径再次核验，目录别名不能绕过自身/子目录检查。
    if source == parent || parent.starts_with(&source) { return Err(UiMessage::new("home.moveInsideSelf")); }
    if source.parent() == Some(parent.as_path()) { return Err(UiMessage::new("home.moveSameFolder")); }
    let name = source.file_name().ok_or_else(|| UiMessage::new("tree.invalidTarget"))?;
    let destination = parent.join(name);
    let outcome = manager.rename_path(&source, &destination, || rename_no_replace(&source, &destination).map_err(|error| error.to_string()))
        .map_err(|error| UiMessage::raw(error).context("home.moveFailed"))?;
    let logical = (item.path.clone(), directory.join(name));
    let mut paths = vec![(source, destination)];
    if paths[0] != logical { paths.push(logical); }
    Ok(TreeRenameReport { paths, warning: outcome.warning })
}

fn rename_tree_path(root: &Path, path: &Path, name: &str, manager: &MarkdownDocumentSessionManager) -> Result<TreeRenameReport, UiMessage> {
    validate_name(name)?;
    let source = checked_existing(root, path)?;
    let destination = source.parent().ok_or(UiMessage::new("tree.noParent"))?.join(name);
    if source == destination { return Ok(TreeRenameReport { paths: Vec::new(), warning: None }); }
    let outcome = manager.rename_path(&source, &destination, || {
        rename_no_replace(&source, &destination).map_err(|error| error.to_string())
    }).map_err(|error| UiMessage::raw(error).context("tree.renameFailed"))?;
    let mut paths = vec![(source, destination)];
    let logical = (path.to_path_buf(), path.with_file_name(name));
    if paths[0] != logical { paths.push(logical); }
    Ok(TreeRenameReport { paths, warning: outcome.warning })
}

fn broadcast_tree_rename(app: &mut App, paths: &[(PathBuf, PathBuf)], warning: Option<&str>) {
    if paths.is_empty() { return; }
    crate::ui::home::library::shared(app).update(app, |library, cx| library.change(crate::ui::home::library::Change::Rename(paths.to_vec()), cx));
    for handle in app.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
        if let Err(error) = handle.update(app, |workspace, window, cx| {
            let mut changed = false;
            for tab in &workspace.tabs {
                changed |= tab.document.update(cx, |host, cx| host.refresh_renamed_path(cx));
            }
            for opening in &mut workspace.opening {
                if let Some(path) = &mut opening.path { remap_tree_path(path, paths); }
            }
            if let Some(path) = workspace.deferred_open.as_mut().and_then(|request| request.path.as_mut()) {
                remap_tree_path(path, paths);
            }
            for request in &mut workspace.open_requests.pending {
                if let Some(path) = &mut request.path { remap_tree_path(path, paths); }
            }
            workspace.remap_saved_paths(|path|remap_tree_path(path,paths));
            workspace.home.update(cx, |home, cx| home.paths_renamed(paths, cx));
            workspace.apply_tree_rename(paths, cx);
            workspace.record_layout(cx);
            if changed {
                workspace.sync_tree_selection(false, cx);
                if let Some(tab) = workspace.tab() {
                    window.set_window_title(&format!("{} — Nomo", tab.document.read(cx).display_title(cx)));
                }
            }
            if let Some(warning) = warning { workspace.notice = Some(ToolbarLocale::current(cx).app_format("tree.renameWarning", &[("error", warning)]).into()); }
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
        use std::path::{Component, Prefix};

        let mut value = value.replace('/', "\\");
        if let Some(Component::Prefix(prefix)) = Path::new(&value).components().next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(_) => value.replace_range(..r"\\?\".len(), ""),
                Prefix::VerbatimUNC(..) => value.replace_range(..r"\\?\UNC\".len(), r"\\"),
                _ => {}
            }
        }
        value
    }
    #[cfg(not(target_os = "windows"))]
    value.into_owned()
}

#[cfg(target_os = "windows")]
fn copy_file_to_clipboard(path: &Path, _: &mut App) -> Result<(), UiMessage> {
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
    if !path.is_file() { return Err(UiMessage::new("tree.fileUnavailable")); }
    let original = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut wide = if original.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
        let mut value = vec![92, 92]; value.extend_from_slice(&original[8..]); value
    } else if original.starts_with(&[92, 92, 63, 92]) { original[4..].to_vec() } else { original };
    wide.extend([0, 0]);
    let bytes = std::mem::size_of::<DropFiles>() + wide.len() * 2;
    unsafe {
        let memory = GlobalAlloc(2, bytes);
        if memory.is_null() { return Err(UiMessage::new("tree.clipboardMemory")); }
        let data = GlobalLock(memory);
        if data.is_null() { GlobalFree(memory); return Err(UiMessage::new("tree.clipboardAccess")); }
        data.cast::<DropFiles>().write(DropFiles { offset: std::mem::size_of::<DropFiles>() as u32, x: 0, y: 0, non_client: 0, wide: 1 });
        std::ptr::copy_nonoverlapping(wide.as_ptr(), (data as *mut u8).add(std::mem::size_of::<DropFiles>()).cast::<u16>(), wide.len());
        GlobalUnlock(memory);
        let owner = GetActiveWindow();
        if owner.is_null() || OpenClipboard(owner) == 0 { GlobalFree(memory); return Err(UiMessage::new("tree.clipboardOpen")); }
        let ok = EmptyClipboard() != 0 && !SetClipboardData(15, memory).is_null();
        CloseClipboard();
        if !ok { GlobalFree(memory); return Err(UiMessage::new("tree.clipboardCopy")); }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct FileClipboard(arboard::Clipboard);
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl gpui::Global for FileClipboard {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn copy_file_to_clipboard(path: &Path, cx: &mut App) -> Result<(), UiMessage> {
    if !path.is_absolute() || !path.is_file() { return Err(UiMessage::new("tree.fileUnavailable")); }
    if cx.try_global::<FileClipboard>().is_none() {
        let clipboard = arboard::Clipboard::new().map_err(|error| UiMessage::new("tree.clipboardOpenFailed").arg("error", error))?;
        cx.set_global(FileClipboard(clipboard));
    }
    cx.global_mut::<FileClipboard>().0.set().file_list(&[path])
        .map_err(|error| UiMessage::new("tree.clipboardCopyFailed").arg("error", error))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn copy_file_to_clipboard(_: &Path, _: &mut App) -> Result<(), UiMessage> { Err(UiMessage::new("tree.copyUnsupported")) }

fn validate_name(name: &str) -> Result<(), UiMessage> {
    if name == "." || name == ".." || name.chars().any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch)) || name.ends_with([' ', '.']) {
        return Err(UiMessage::new("tree.invalidName"));
    }
    Ok(())
}

fn checked_parent(root: &Path, parent: &Path) -> Result<PathBuf, UiMessage> {
    let root = root.canonicalize().map_err(|error| UiMessage::new("tree.rootAccess").arg("error", error))?;
    let parent = parent.canonicalize().map_err(|error| UiMessage::new("tree.directoryAccess").arg("error", error))?;
    if !parent.starts_with(&root) { return Err(UiMessage::new("tree.outsideRoot")); }
    Ok(parent)
}

pub(super) fn checked_existing(root: &Path, path: &Path) -> Result<PathBuf, UiMessage> {
    if path == root { return Err(UiMessage::new("tree.rootMutation")); }
    let parent = checked_parent(root, path.parent().ok_or(UiMessage::new("tree.noParent"))?)?;
    let name = path.file_name().ok_or(UiMessage::new("tree.invalidTarget"))?;
    let target = parent.join(name);
    let metadata = std::fs::symlink_metadata(&target).map_err(|error| UiMessage::new("tree.targetAccess").arg("error", error))?;
    if metadata.file_type().is_symlink() { return Err(UiMessage::new("tree.symlink")); }
    Ok(target)
}

fn apply_tree_edit(root: &Path, path: &Path, kind: EditKind, name: &str) -> Result<PathBuf, UiMessage> {
    validate_name(name)?;
    if matches!(kind, EditKind::Rename) { return Err(UiMessage::new("tree.renameSession")); }
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
            Err(error) => return Err(UiMessage::new("tree.createFailed").arg("error", error)),
        }
    }
    Err(UiMessage::new("tree.tooManyNames"))
}

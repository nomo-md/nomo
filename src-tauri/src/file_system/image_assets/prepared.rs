//! 原生剪贴板快照只在磁盘暂存。标识归属窗口，借用中的文件由 Arc 延迟到任务结束再删除。
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

static IMAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static CLIPBOARD_ENCODING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
// 只限制尚未释放的磁盘快照，不保存像素缓存。额度耗尽时明确失败，不淘汰正在使用的图片。
const MAX_PREPARED_IMAGES: usize = 32;
const MAX_PREPARED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct TemporaryImage {
    pub(super) path: PathBuf,
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::app_logger::warn("ImageInsertion", &format!("清理临时图片失败：{error}"));
            }
        }
    }
}

struct PreparedEntry {
    owner: String,
    file: Option<Arc<TemporaryImage>>,
    bytes: u64,
}

#[derive(Clone)]
pub(crate) struct PreparedImages {
    root: Arc<PathBuf>,
    entries: Arc<Mutex<HashMap<String, PreparedEntry>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedClipboardImage {
    asset_id: String,
    file_name: String,
    width: u32,
    height: u32,
}

impl PreparedImages {
    pub(crate) fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|error| format!("创建图片暂存目录失败：{error}"))?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        // 应用单实例 setup 内执行；只清理本功能的普通文件，不递归遍历链接或其他目录。
        for entry in fs::read_dir(&root)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            if entry.file_name().to_string_lossy().starts_with("image-")
                && entry.file_type().is_ok_and(|kind| kind.is_file())
            {
                fs::remove_file(entry.path())
                    .map_err(|error| format!("清理上次图片暂存失败：{error}"))?;
            }
        }
        Ok(Self {
            root: Arc::new(root),
            entries: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(super) fn create_file(
        &self,
        file_name: &str,
    ) -> Result<(Arc<TemporaryImage>, File), String> {
        let safe_name = super::sanitize_image_file_name(file_name);
        loop {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let sequence = IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = self.root.join(format!(
                "image-{}-{stamp}-{sequence}-{safe_name}",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Arc::new(TemporaryImage { path }), file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("创建临时图片失败：{error}")),
            }
        }
    }

    fn reserve(&self, owner: &str) -> Result<Reservation, String> {
        let mut entries = self.entries.lock().map_err(|error| error.to_string())?;
        if entries.len() >= MAX_PREPARED_IMAGES {
            return Err("待处理的剪贴板图片过多，请等待当前插入完成".into());
        }
        let id = format!(
            "{}-{}",
            std::process::id(),
            IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        entries.insert(
            id.clone(),
            PreparedEntry {
                owner: owner.into(),
                file: None,
                bytes: 0,
            },
        );
        Ok(Reservation {
            store: self.clone(),
            owner: owner.into(),
            id,
            published: false,
        })
    }

    pub(super) fn acquire(&self, owner: &str, id: &str) -> Result<Arc<TemporaryImage>, String> {
        let entries = self.entries.lock().map_err(|error| error.to_string())?;
        let entry = entries.get(id).ok_or("原生图片资源已失效")?;
        if entry.owner != owner {
            return Err("图片资源不属于当前窗口".into());
        }
        entry
            .file
            .clone()
            .ok_or_else(|| "图片资源尚未准备好".into())
    }

    fn release(&self, owner: &str, id: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().map_err(|error| error.to_string())?;
        if entries.get(id).is_some_and(|entry| entry.owner != owner) {
            return Err("图片资源不属于当前窗口".into());
        }
        let removed = entries.remove(id);
        drop(entries);
        drop(removed);
        Ok(())
    }

    pub(crate) fn release_owner(&self, owner: &str) {
        let removed = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let ids: Vec<_> = entries
                .iter()
                .filter(|(_, entry)| entry.owner == owner)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id))
                .collect::<Vec<_>>()
        };
        // WindowEvent 回调只改元数据，删除文件交给阻塞池。
        tauri::async_runtime::spawn_blocking(move || drop(removed));
    }

    pub(crate) fn clear(&self) {
        let removed = std::mem::take(
            &mut *self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        drop(removed);
    }
}

struct Reservation {
    store: PreparedImages,
    owner: String,
    id: String,
    published: bool,
}

impl Reservation {
    fn publish(
        mut self,
        file: Arc<TemporaryImage>,
        width: u32,
        height: u32,
    ) -> Result<PreparedClipboardImage, String> {
        let bytes = fs::metadata(&file.path)
            .map_err(|error| error.to_string())?
            .len();
        {
            let mut entries = self
                .store
                .entries
                .lock()
                .map_err(|error| error.to_string())?;
            let occupied: u64 = entries.values().map(|entry| entry.bytes).sum();
            if bytes > MAX_PREPARED_BYTES.saturating_sub(occupied) {
                return Err("剪贴板图片暂存空间已满，请等待当前插入完成".into());
            }
            let entry = entries
                .get_mut(&self.id)
                .ok_or("来源窗口已关闭，图片插入已取消")?;
            entry.file = Some(file);
            entry.bytes = bytes;
        }
        self.published = true;
        Ok(PreparedClipboardImage {
            asset_id: self.id.clone(),
            file_name: "clipboard.png".into(),
            width,
            height,
        })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.published {
            let _ = self.store.release(&self.owner, &self.id);
        }
    }
}

fn encode_rgba(
    store: &PreparedImages,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Arc<TemporaryImage>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err("剪贴板图片尺寸或像素数据无效".into());
    }
    let (temporary, file) = store.create_file("clipboard.png")?;
    let mut output = BufWriter::new(file);
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("PNG 编码失败：{error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("PNG 编码失败：{error}"))?;
        writer
            .finish()
            .map_err(|error| format!("PNG 编码失败：{error}"))?;
    }
    output
        .flush()
        .map_err(|error| format!("写入临时图片失败：{error}"))?;
    Ok(temporary)
}

#[tauri::command]
pub(crate) async fn prepare_clipboard_image(
    window: WebviewWindow,
    state: State<'_, PreparedImages>,
) -> Result<Option<PreparedClipboardImage>, String> {
    if window
        .app_handle()
        .get_webview_window(window.label())
        .is_none()
    {
        return Err("来源窗口已关闭".into());
    }
    let reservation = state.reserve(window.label())?;
    let store = state.inner().clone();
    let permit = CLIPBOARD_ENCODING
        .acquire()
        .await
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if window
            .app_handle()
            .get_webview_window(window.label())
            .is_none()
        {
            return Err("来源窗口已关闭".into());
        }
        // 插件的原生读图不能运行于 UI 线程；此处不会创建 WebView Image Resource。
        let image = match window.clipboard().read_image() {
            Ok(image) => image,
            Err(error) => {
                crate::app_logger::debug(
                    "ImageInsertion",
                    &format!("原生读图不可用，尝试文本：{error}"),
                );
                return Ok(None);
            }
        };
        let width = image.width();
        let height = image.height();
        let temporary = encode_rgba(&store, width, height, image.rgba())?;
        drop(image);
        reservation.publish(temporary, width, height).map(Some)
    })
    .await
    .map_err(|error| format!("剪贴板图片任务失败：{error}"))?
}

#[tauri::command]
pub(crate) async fn release_prepared_image(
    window: WebviewWindow,
    state: State<'_, PreparedImages>,
    asset_id: String,
) -> Result<(), String> {
    let owner = window.label().to_owned();
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.release(&owner, &asset_id))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> PreparedImages {
        PreparedImages::new(std::env::temp_dir().join(format!(
            "nomo-prepared-test-{name}-{}-{}",
            std::process::id(),
            IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
        .unwrap()
    }

    fn remove_test_directory(store: &PreparedImages) {
        // Windows 的文件删除可能短暂处于 pending 状态；只重试已验证资源释放后的空目录清理。
        for _ in 0..100 {
            match fs::remove_dir(&*store.root) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("清理图片测试目录失败：{error}"),
            }
        }
        fs::remove_dir(&*store.root).unwrap();
    }

    #[test]
    fn preserves_rgba_and_keeps_borrowed_file_until_upload_finishes() {
        let store = store("rgba");
        let rgba = [255, 0, 0, 0, 10, 20, 30, 127, 0, 255, 0, 255];
        let file = encode_rgba(&store, 3, 1, &rgba).unwrap();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(File::open(&file.path).unwrap()))
                .read_info()
                .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut decoded).unwrap();
        assert_eq!(decoded, rgba);
        drop(reader);
        let snapshot = store.reserve("main").unwrap().publish(file, 3, 1).unwrap();
        assert!(store.acquire("other", &snapshot.asset_id).is_err());
        assert!(store.release("other", &snapshot.asset_id).is_err());
        let lease = store.acquire("main", &snapshot.asset_id).unwrap();
        let path = lease.path.clone();
        store.release("main", &snapshot.asset_id).unwrap();
        store.release("main", &snapshot.asset_id).unwrap();
        assert!(path.exists());
        drop(lease);
        assert!(!path.exists());
        remove_test_directory(&store);
    }

    #[test]
    fn failed_encoding_and_abandoned_reservations_leave_no_resources() {
        let store = store("failure");
        assert!(encode_rgba(&store, 1, 1, &[0]).is_err());
        let reservation = store.reserve("main").unwrap();
        let id = reservation.id.clone();
        drop(reservation);
        assert!(store.acquire("main", &id).is_err());
        assert_eq!(fs::read_dir(&*store.root).unwrap().count(), 0);
        remove_test_directory(&store);
    }

    #[test]
    fn pending_snapshot_limits_do_not_evict_live_handles() {
        let store = store("budget");
        let reservations: Vec<_> = (0..MAX_PREPARED_IMAGES)
            .map(|_| store.reserve("main").unwrap())
            .collect();
        assert!(store.reserve("main").is_err());
        drop(reservations);
        let first = store.reserve("main").unwrap();
        store
            .entries
            .lock()
            .unwrap()
            .get_mut(&first.id)
            .unwrap()
            .bytes = MAX_PREPARED_BYTES;
        let next = store.reserve("main").unwrap();
        let image = encode_rgba(&store, 1, 1, &[1, 2, 3, 4]).unwrap();
        let path = image.path.clone();
        assert!(next.publish(image, 1, 1).is_err());
        assert!(!path.exists());
        assert!(store.entries.lock().unwrap().contains_key(&first.id));
        drop(first);
        assert!(store.entries.lock().unwrap().is_empty());
        remove_test_directory(&store);
    }

    #[test]
    fn closed_owner_cannot_publish_late_snapshot_and_startup_only_removes_owned_files() {
        let store = store("owner");
        let reservation = store.reserve("main").unwrap();
        let file = encode_rgba(&store, 1, 1, &[1, 2, 3, 4]).unwrap();
        let path = file.path.clone();
        store.release_owner("main");
        assert!(reservation.publish(file, 1, 1).is_err());
        assert!(!path.exists());
        fs::write(store.root.join("image-stale.png"), [0]).unwrap();
        fs::write(store.root.join("keep.txt"), [0]).unwrap();
        let restarted = PreparedImages::new((*store.root).clone()).unwrap();
        assert!(!restarted.root.join("image-stale.png").exists());
        assert!(restarted.root.join("keep.txt").exists());
        fs::remove_file(restarted.root.join("keep.txt")).unwrap();
        remove_test_directory(&store);
    }
}

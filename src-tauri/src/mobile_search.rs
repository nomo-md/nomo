//! Android 文档抽屉搜索：串行只读扫描，取消旧请求，只返回有限摘要。
mod scanner;
use crate::text_document::DocumentSessionManager;
use scanner::{scan_file, scan_snapshot, Snippet};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};

const SEARCH_EVENT: &str = "nomo://mobile-search";

#[derive(Clone, Default)]
pub(crate) struct MobileSearchState {
    active: Arc<Mutex<Option<(String, Arc<AtomicBool>)>>>,
    serial: Arc<Mutex<()>>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchSource {
    key: String,
    version: u64,
    path: Option<String>,
    session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchRequest {
    task_id: String,
    query: String,
    sources: Vec<SearchSource>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchEvent {
    task_id: String,
    key: Option<String>,
    version: Option<u64>,
    status: &'static str,
    snippet: Option<Snippet>,
    error: Option<String>,
    completed: usize,
    total: usize,
}

#[tauri::command]
pub(crate) fn start_mobile_document_search(
    app: AppHandle,
    state: State<MobileSearchState>,
    request: SearchRequest,
) -> Result<(), String> {
    if request.task_id.is_empty()
        || request.task_id.len() > 128
        || request.sources.len() > 1024
        || request.query.trim().is_empty()
        || request.query.len() > scanner::CHUNK_BYTES
    {
        return Err("invalid-search-request".into());
    }
    let state = state.inner().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.active.lock().map_err(|_| "search-lock-failed")?;
        if let Some((_, old)) = active.take() {
            old.store(true, Ordering::Release);
        }
        *active = Some((request.task_id.clone(), cancel.clone()));
    }
    let manager = app.state::<DocumentSessionManager>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(_serial) = state.serial.lock() else {
            return;
        };
        run_search(&request, &manager, &cancel, |event| {
            app.emit_to("main", SEARCH_EVENT, event).map_err(|_| ())
        });
        if let Ok(mut active) = state.active.lock() {
            if active
                .as_ref()
                .is_some_and(|(id, _)| id == &request.task_id)
            {
                *active = None;
            }
        }
    });
    Ok(())
}

/// One sequential worker per task. Cancellation is checked before scheduling every source.
fn run_search(
    request: &SearchRequest,
    manager: &DocumentSessionManager,
    cancel: &AtomicBool,
    mut emit: impl FnMut(SearchEvent) -> Result<(), ()>,
) {
    let mut event = SearchEvent {
        task_id: request.task_id.clone(),
        key: None,
        version: None,
        status: "running",
        snippet: None,
        error: None,
        completed: 0,
        total: request.sources.len(),
    };
    for source in &request.sources {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let result = search_source(source, &request.query, manager, cancel);
        if cancel.load(Ordering::Acquire) {
            break;
        }
        event.key = Some(source.key.clone());
        event.version = Some(source.version);
        event.completed += 1;
        event.status = "result";
        match result {
            Ok(snippet) => {
                event.snippet = snippet;
                event.error = None;
            }
            Err(error) => {
                event.snippet = None;
                event.error = Some(error);
            }
        }
        if emit(event.clone()).is_err() {
            return;
        }
    }
    event.status = if cancel.load(Ordering::Acquire) {
        "cancelled"
    } else {
        "completed"
    };
    event.key = None;
    event.version = None;
    event.snippet = None;
    event.error = None;
    let _ = emit(event);
}

fn search_source(
    source: &SearchSource,
    query: &str,
    manager: &DocumentSessionManager,
    cancel: &AtomicBool,
) -> Result<Option<Snippet>, String> {
    if let Some(id) = &source.session_id {
        let readers = manager.mobile_search_snapshot_readers(id, source.version)?;
        let result = scan_snapshot(readers, query, cancel)?;
        if manager
            .session_status(id)
            .map_err(|_| "session-closed")?
            .revision
            != source.version
        {
            return Err("revision-changed".into());
        }
        return Ok(result);
    }
    let path = Path::new(source.path.as_deref().ok_or("missing-source")?);
    if !matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "txt" | "json"
    ) {
        return Err("unsupported-file".into());
    }
    scan_file(path, query, cancel)
}

#[tauri::command]
pub(crate) fn cancel_mobile_document_search(
    state: State<MobileSearchState>,
    task_id: String,
) -> Result<(), String> {
    let active = state.active.lock().map_err(|_| "search-lock-failed")?;
    if let Some((id, cancel)) = active.as_ref() {
        if id == &task_id {
            cancel.store(true, Ordering::Release);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_document::{SegmentedEdit, SegmentedEditBatch};
    use std::{fs, path::PathBuf, sync::atomic::AtomicU64};

    static NEXT_ROOT_ID: AtomicU64 = AtomicU64::new(0);

    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            loop {
                let path = std::env::temp_dir().join(format!(
                    "nomo-mobile-search-{}-{}",
                    std::process::id(),
                    NEXT_ROOT_ID.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot reserve search test directory: {error}"),
                }
            }
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mobile_search_scans_200_closed_documents_and_cancels_remaining_sources() {
        let root = Root::new();
        let manager = DocumentSessionManager::new(root.0.join("sessions")).unwrap();
        let sources = (0..200)
            .map(|index| {
                let path = root.0.join(format!("doc-{index}.md"));
                fs::write(&path, format!("document {index} needle")).unwrap();
                SearchSource {
                    key: index.to_string(),
                    version: 1,
                    path: Some(path.to_string_lossy().into_owned()),
                    session_id: None,
                }
            })
            .collect();
        let request = SearchRequest {
            task_id: "task-1".into(),
            query: "needle".into(),
            sources,
        };
        let cancel = AtomicBool::new(false);
        let mut results = Vec::new();
        run_search(&request, &manager, &cancel, |event| {
            results.push(event);
            Ok(())
        });
        assert_eq!(results.len(), 201);
        assert!(results[..200]
            .iter()
            .all(|event| event.snippet.is_some() && event.error.is_none()));
        assert_eq!(results.last().unwrap().completed, 200);
        results.clear();
        run_search(&request, &manager, &cancel, |event| {
            if event.key.is_some() {
                cancel.store(true, Ordering::Release);
            }
            results.push(event);
            Ok(())
        });
        assert_eq!(results.len(), 2);
        assert_eq!(results.last().unwrap().status, "cancelled");
        assert_eq!(results.last().unwrap().completed, 1);
    }

    #[test]
    fn mobile_search_reads_unsaved_segmented_snapshot_and_rejects_stale_revision() {
        let root = Root::new();
        let manager = DocumentSessionManager::new(root.0.join("sessions")).unwrap();
        for extension in ["txt", "json"] {
            let path = root.0.join(format!("note.{extension}"));
            fs::write(&path, "original").unwrap();
            let opened = manager.open_document(path.clone(), None).unwrap();
            let applied = manager
                .apply_edits(SegmentedEditBatch {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    edits: vec![SegmentedEdit {
                        from_byte: 0,
                        to_byte: 8,
                        inserted_text: "unsaved needle".into(),
                    }],
                })
                .unwrap();
            let mut source = SearchSource {
                key: extension.into(),
                version: applied.revision,
                path: Some(path.to_string_lossy().into_owned()),
                session_id: Some(opened.session_id.clone()),
            };
            let cancel = AtomicBool::new(false);
            assert!(search_source(&source, "needle", &manager, &cancel)
                .unwrap()
                .is_some());
            assert_eq!(fs::read_to_string(&path).unwrap(), "original");
            source.version = opened.revision;
            assert_eq!(
                search_source(&source, "needle", &manager, &cancel).unwrap_err(),
                "revision-changed"
            );
        }
    }

    #[test]
    fn mobile_search_preserves_encoding_matches_in_readonly_segmented_snapshots() {
        let root = Root::new();
        let manager = DocumentSessionManager::new(root.0.join("sessions")).unwrap();
        let text = "中文目标";
        let utf16: Vec<u8> = [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat();
        for (index, bytes) in [encoding_rs::GBK.encode(text).0.into_owned(), utf16]
            .into_iter()
            .enumerate()
        {
            let path = root.0.join(format!("encoded-{index}.txt"));
            fs::write(&path, bytes).unwrap();
            let opened = manager.open_document(path.clone(), None).unwrap();
            let source = SearchSource {
                key: index.to_string(),
                version: opened.revision,
                path: Some(path.to_string_lossy().into_owned()),
                session_id: Some(opened.session_id),
            };
            assert!(
                search_source(&source, "目标", &manager, &AtomicBool::new(false))
                    .unwrap()
                    .is_some()
            );
        }
    }
}

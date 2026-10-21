//! TXT/JSON 分段文档引擎。
//!
//! Tauri 命令只负责协议适配；所有文件、revision 和后台任务语义都通过
//! [`DocumentSessionManager`] 这一稳定 seam 暴露，避免前端或测试穿透内部结构。

mod chunk_reader;
pub(crate) mod commands;
mod edit_journal;
mod encoding;
mod line_index;
pub(crate) mod markdown_codec;
pub(crate) mod markdown_commands;
mod markdown_history;
pub(crate) mod markdown_session;
mod piece_tree;
mod session;
mod task_runner;

use serde::{Deserialize, Serialize};

pub(crate) use session::DocumentSessionManager;

pub(crate) type EventSink = std::sync::Arc<dyn Fn(SegmentedEvent) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SegmentedDocumentKind {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TextEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    #[serde(rename = "unsupported")]
    Unsupported,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LineEnding {
    #[default]
    Lf,
    Crlf,
    Mixed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenSegmentedDocumentResult {
    pub(crate) session_id: String,
    pub(crate) revision: u64,
    pub(crate) persisted_revision: u64,
    pub(crate) document_kind: SegmentedDocumentKind,
    pub(crate) encoding: TextEncoding,
    pub(crate) line_ending: LineEnding,
    pub(crate) byte_length: u64,
    /// 源文件系统权限是否只读；不包含 baseline/编码校验期间的临时编辑门禁。
    pub(crate) filesystem_readonly: bool,
    pub(crate) readonly: bool,
    pub(crate) first_window: SegmentedWindow,
    /// 原文件基线变化时保留的恢复日志路径；补丁不会自动套到新基线。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_conflict_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<u64>,
    pub(crate) revision: u64,
    pub(crate) start_byte: u64,
    pub(crate) end_byte: u64,
    pub(crate) start_line: u64,
    pub(crate) text: String,
    pub(crate) leading_partial_line: bool,
    pub(crate) trailing_partial_line: bool,
    pub(crate) long_line: bool,
    pub(crate) index_progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) utf16_byte_offsets: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) json_lexical_state: Option<JsonLexicalState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JsonLexicalState {
    pub(crate) mode: JsonLexicalMode,
    pub(crate) escaped: bool,
}

impl Default for JsonLexicalState {
    fn default() -> Self {
        Self {
            mode: JsonLexicalMode::Default,
            escaped: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum JsonLexicalMode {
    Default,
    String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedEdit {
    pub(crate) from_byte: u64,
    pub(crate) to_byte: u64,
    pub(crate) inserted_text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedEditBatch {
    pub(crate) session_id: String,
    pub(crate) base_revision: u64,
    pub(crate) edits: Vec<SegmentedEdit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplySegmentedEditsResult {
    pub(crate) revision: u64,
    pub(crate) persisted_revision: u64,
    pub(crate) dirty: bool,
    pub(crate) invalidated_from_byte: u64,
    pub(crate) invalidated_to_byte: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedHistoryResult {
    pub(crate) changed: bool,
    pub(crate) revision: u64,
    pub(crate) persisted_revision: u64,
    pub(crate) byte_length: u64,
    pub(crate) dirty: bool,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlushSegmentedJournalResult {
    pub(crate) revision: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSegmentedRevisionResult {
    pub(crate) session_id: String,
    pub(crate) saved_revision: u64,
    pub(crate) current_revision: u64,
    pub(crate) persisted_revision: u64,
    pub(crate) dirty: bool,
    /// 保存目标的文件系统权限是否只读；与会话临时编辑门禁分开上报。
    pub(crate) filesystem_readonly: bool,
    pub(crate) readonly: bool,
    pub(crate) modified_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartSegmentedTaskRequest {
    pub(crate) session_id: String,
    pub(crate) base_revision: u64,
    pub(crate) task: SegmentedTask,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum SegmentedTask {
    Search {
        query: String,
        #[serde(default, rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(default, rename = "wholeWord")]
        whole_word: bool,
        #[serde(default, rename = "wrapAround")]
        wrap_around: bool,
        /// 分页锚点只参与筛选结果窗口，全文匹配总数仍按完整快照统计。
        #[serde(default, rename = "anchorByte")]
        anchor_byte: Option<u64>,
        #[serde(default)]
        direction: Option<SegmentedSearchDirection>,
    },
    ReplaceAll {
        query: String,
        replacement: String,
        #[serde(default, rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(default, rename = "wholeWord")]
        whole_word: bool,
    },
    SelectAllCopy,
    JsonValidate,
    JsonFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SegmentedSearchDirection {
    /// 返回锚点之后的下一页，当前项为页首。
    Forward,
    /// 返回锚点之前的最近一页，当前项为最接近锚点的前一条。
    Backward,
}

impl SegmentedTask {
    pub(crate) fn kind(&self) -> SegmentedTaskKind {
        match self {
            Self::Search { .. } => SegmentedTaskKind::Search,
            Self::ReplaceAll { .. } => SegmentedTaskKind::ReplaceAll,
            Self::SelectAllCopy => SegmentedTaskKind::SelectAllCopy,
            Self::JsonValidate => SegmentedTaskKind::JsonValidate,
            Self::JsonFormat => SegmentedTaskKind::JsonFormat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SegmentedTaskKind {
    Search,
    ReplaceAll,
    SelectAllCopy,
    JsonValidate,
    JsonFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SegmentedTaskState {
    Running,
    Completed,
    Cancelled,
    Conflict,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartSegmentedTaskResult {
    pub(crate) task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelSegmentedTaskResult {
    pub(crate) task_id: String,
    pub(crate) cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SegmentedExternalChangeKind {
    None,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckSegmentedExternalChangeResult {
    pub(crate) session_id: String,
    pub(crate) revision: u64,
    pub(crate) change: SegmentedExternalChangeKind,
    /// 同一外部身份稳定、下一次真实身份变化即改变；前端可用它抑制“忽略”后的重复提示。
    pub(crate) change_token: String,
    pub(crate) modified_at: i64,
    pub(crate) dirty: bool,
    pub(crate) save_in_progress: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedSessionStatus {
    pub(crate) session_id: String,
    pub(crate) revision: u64,
    pub(crate) persisted_revision: u64,
    pub(crate) byte_length: u64,
    pub(crate) indexed_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) estimated_lines: u64,
    pub(crate) completed: bool,
    pub(crate) encoding: TextEncoding,
    pub(crate) line_ending: LineEnding,
    /// 源文件系统权限是否只读；不包含后台准备阶段的临时只读状态。
    pub(crate) filesystem_readonly: bool,
    pub(crate) readonly: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) baseline_error: Option<String>,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexProgressEvent {
    pub(crate) session_id: String,
    pub(crate) task_id: String,
    pub(crate) request_id: String,
    pub(crate) revision: u64,
    pub(crate) indexed_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) estimated_lines: u64,
    pub(crate) completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encoding: Option<TextEncoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) line_ending: Option<LineEnding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) baseline_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskProgressEvent {
    pub(crate) session_id: String,
    pub(crate) task_id: String,
    pub(crate) base_revision: u64,
    pub(crate) revision: u64,
    pub(crate) request_id: String,
    pub(crate) kind: SegmentedTaskKind,
    pub(crate) state: SegmentedTaskState,
    pub(crate) processed_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) match_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_match: Option<SegmentedTaskMatch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) nearby_matches: Vec<SegmentedTaskMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_byte_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) persisted_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentedTaskMatch {
    pub(crate) start_byte: u64,
    pub(crate) end_byte: u64,
}

#[derive(Debug, Clone)]
pub(crate) enum SegmentedEvent {
    Index(IndexProgressEvent),
    Task(TaskProgressEvent),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextDocumentError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actual_revision: Option<u64>,
    #[serde(rename = "recoveryPath", skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_path: Option<String>,
}

impl TextDocumentError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            actual_revision: None,
            recovery_path: None,
        }
    }

    pub(crate) fn revision_conflict(expected: u64, actual: u64) -> Self {
        Self {
            code: "revision-conflict",
            message: format!("文档 revision 已变化：expected={expected}, actual={actual}"),
            actual_revision: Some(actual),
            recovery_path: None,
        }
    }

    pub(crate) fn source_missing_recovered(path: &std::path::Path) -> Self {
        Self {
            code: "source-missing-recovered",
            message: format!("源文件不存在，未保存内容已恢复到 {}", path.display()),
            actual_revision: None,
            recovery_path: Some(path.to_string_lossy().into_owned()),
        }
    }
}

impl std::fmt::Display for TextDocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TextDocumentError {}

impl From<std::io::Error> for TextDocumentError {
    fn from(error: std::io::Error) -> Self {
        Self::new("io-error", error.to_string())
    }
}

pub(crate) type TextDocumentResult<T> = Result<T, TextDocumentError>;

#[cfg(test)]
mod tests {
    use super::chunk_reader::{reset_read_range_metrics, total_read_range_bytes};
    use super::{
        DocumentSessionManager, JsonLexicalMode, LineEnding, SegmentedEdit, SegmentedEditBatch,
        SegmentedEvent, SegmentedExternalChangeKind, SegmentedSearchDirection, SegmentedTask,
        SegmentedTaskState, StartSegmentedTaskRequest, TextEncoding,
    };
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
        sync::{Arc, Condvar, Mutex},
        time::{Duration, Instant, SystemTime},
    };

    #[test]
    fn manager_opens_utf8_bom_without_splitting_a_multibyte_character() {
        let root = unique_test_dir("open-utf8-bom");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, b"\xEF\xBB\xBFa\xE4\xB8\xADb\nnext").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");

        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open document");
        let window = manager
            .read_window(&opened.session_id, opened.revision, 1, 2)
            .expect("read aligned window");

        assert_eq!(opened.encoding, TextEncoding::Utf8Bom);
        assert_eq!(opened.byte_length, 10);
        assert_eq!(window.start_byte, 1);
        assert_eq!(window.text, "中");
        assert_eq!(window.end_byte, 4);
        cleanup(root);
    }

    #[test]
    fn manager_rejects_small_file_replacement_between_probe_and_identity_verification() {
        let root = unique_test_dir("open-probe-identity-race");
        let document_path = root.join("sample.txt");
        let replacement_path = root.join("replacement.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "OLD!").expect("write source");
        fs::write(&replacement_path, "NEW!").expect("write replacement");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        manager.enable_open_probe_test_pause();
        let open_manager = manager.clone();
        let open_path = document_path.clone();
        let open_thread = std::thread::spawn(move || open_manager.open_document(open_path, None));
        assert!(manager.wait_until_open_probe_test_paused());

        replace_path_for_test(&replacement_path, &document_path);
        manager.release_open_probe_test_pause();
        let error = open_thread
            .join()
            .expect("open thread")
            .expect_err("probe race must fail");
        assert_eq!(error.code, "file-changed-during-open");

        let reopened = manager
            .open_document(document_path, None)
            .expect("open stable replacement");
        assert_eq!(reopened.first_window.text, "NEW!");
        cleanup(root);
    }

    #[test]
    fn manager_applies_revisioned_edits_and_stream_saves_bom_and_crlf() {
        let root = unique_test_dir("edit-save");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, b"\xEF\xBB\xBFa\r\nb").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open document");

        let applied = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 3,
                    to_byte: 4,
                    inserted_text: "中\n末".into(),
                }],
            })
            .expect("apply");
        let stale = manager
            .read_window(&opened.session_id, opened.revision, 0, 64)
            .expect_err("stale read must fail");
        let current = manager
            .read_window(&opened.session_id, applied.revision, 0, 64)
            .expect("current window");

        assert_eq!(stale.code, "revision-conflict");
        assert_eq!(current.text, "a\r\n中\r\n末");
        assert!(applied.dirty);

        let saved = manager
            .save_revision(&opened.session_id, applied.revision, None, false)
            .expect("save");
        assert!(!saved.dirty);
        assert_eq!(
            fs::read(&document_path).expect("saved bytes"),
            b"\xEF\xBB\xBFa\r\n\xE4\xB8\xAD\r\n\xE6\x9C\xAB"
        );
        cleanup(root);
    }

    #[test]
    fn manager_streams_search_across_task_chunks() {
        let root = unique_test_dir("search-task");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        let mut body = vec![b'a'; 64 * 1024 - 3];
        body.extend_from_slice(b"needle-tail");
        for _ in 0..40 {
            body.extend_from_slice(b"|needle");
        }
        fs::write(&document_path, body).expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(document_path, None)
            .expect("open document");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();

        let initial_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::Search {
                        query: "needle".into(),
                        case_sensitive: true,
                        whole_word: false,
                        wrap_around: false,
                        anchor_byte: None,
                        direction: None,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start task");
        let initial = wait_for_task(&events, &initial_task.task_id);
        assert_eq!(initial.match_count, 41);
        assert_eq!(
            initial.current_match.expect("current match").start_byte,
            (64 * 1024 - 3) as u64
        );
        assert_eq!(initial.nearby_matches.len(), 16);
        assert_eq!(initial.nearby_matches[0].start_byte, (64 * 1024 - 3) as u64);

        let anchor = initial
            .nearby_matches
            .last()
            .expect("initial page")
            .start_byte;
        let forward_events = Arc::new(Mutex::new(Vec::new()));
        let captured = forward_events.clone();
        let forward_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::Search {
                        query: "needle".into(),
                        case_sensitive: true,
                        whole_word: false,
                        wrap_around: false,
                        anchor_byte: Some(anchor),
                        direction: Some(SegmentedSearchDirection::Forward),
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start forward page");
        let forward = wait_for_task(&forward_events, &forward_task.task_id);
        assert_eq!(forward.match_count, 41);
        assert_eq!(forward.nearby_matches.len(), 16);
        assert!(forward
            .nearby_matches
            .iter()
            .all(|found| found.start_byte > anchor));
        let forward_current = forward.current_match.expect("forward current").start_byte;
        assert_eq!(forward_current, forward.nearby_matches[0].start_byte);

        let backward_events = Arc::new(Mutex::new(Vec::new()));
        let captured = backward_events.clone();
        let backward_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id,
                    base_revision: opened.revision,
                    task: SegmentedTask::Search {
                        query: "needle".into(),
                        case_sensitive: true,
                        whole_word: false,
                        wrap_around: false,
                        anchor_byte: Some(forward_current),
                        direction: Some(SegmentedSearchDirection::Backward),
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start backward page");
        let backward = wait_for_task(&backward_events, &backward_task.task_id);
        assert_eq!(backward.match_count, 41);
        assert_eq!(backward.nearby_matches.len(), 16);
        assert!(backward
            .nearby_matches
            .iter()
            .all(|found| found.start_byte < forward_current));
        assert_eq!(
            backward.current_match.expect("backward current").start_byte,
            backward
                .nearby_matches
                .last()
                .expect("backward page")
                .start_byte
        );
        cleanup(root);
    }

    #[test]
    fn search_request_deserializes_camel_case_page_anchor() {
        let request: StartSegmentedTaskRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "session",
            "baseRevision": 7,
            "task": {
                "type": "search",
                "query": "needle",
                "anchorByte": 42,
                "direction": "backward"
            }
        }))
        .expect("deserialize search page");

        let SegmentedTask::Search {
            anchor_byte,
            direction,
            ..
        } = request.task
        else {
            panic!("search task expected");
        };
        assert_eq!(anchor_byte, Some(42));
        assert_eq!(direction, Some(SegmentedSearchDirection::Backward));
    }

    #[test]
    fn manager_reports_clean_dirty_modified_and_deleted_external_state() {
        let root = unique_test_dir("external-change");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "alpha").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open document");

        let clean = manager
            .check_external_change(&opened.session_id)
            .expect("clean state");
        assert_eq!(clean.change, SegmentedExternalChangeKind::None);
        assert!(!clean.dirty);
        assert!(!clean.save_in_progress);

        manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 5,
                    to_byte: 5,
                    inserted_text: " local".into(),
                }],
            })
            .expect("local edit");
        fs::write(&document_path, "external replacement").expect("external write");
        let modified = manager
            .check_external_change(&opened.session_id)
            .expect("modified state");
        assert_eq!(modified.change, SegmentedExternalChangeKind::Modified);
        assert!(modified.dirty);

        fs::remove_file(&document_path).expect("delete");
        let deleted = manager
            .check_external_change(&opened.session_id)
            .expect("deleted state");
        assert_eq!(deleted.change, SegmentedExternalChangeKind::Deleted);
        cleanup(root);
    }

    #[test]
    fn manager_detects_same_size_same_mtime_atomic_replacement() {
        let root = unique_test_dir("external-strong-identity");
        let document_path = root.join("sample.txt");
        let replacement_path = root.join("replacement.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "AAAA").expect("write source");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open document");
        let clean = manager
            .check_external_change(&opened.session_id)
            .expect("clean identity");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("local edit");
        let original_modified = fs::metadata(&document_path)
            .expect("source metadata")
            .modified()
            .expect("source mtime");
        fs::write(&replacement_path, "BBBB").expect("write replacement");
        fs::OpenOptions::new()
            .write(true)
            .open(&replacement_path)
            .expect("open replacement metadata")
            .set_modified(original_modified)
            .expect("preserve replacement mtime");
        replace_path_for_test(&replacement_path, &document_path);
        let replaced_metadata = fs::metadata(&document_path).expect("replacement metadata");
        assert_eq!(replaced_metadata.len(), 4);
        assert_eq!(
            replaced_metadata.modified().expect("replacement mtime"),
            original_modified
        );

        let changed = manager
            .check_external_change(&opened.session_id)
            .expect("changed identity");
        assert_eq!(changed.change, SegmentedExternalChangeKind::Modified);
        assert_ne!(changed.change_token, clean.change_token);
        let error = manager
            .save_revision(&opened.session_id, edited.revision, None, false)
            .expect_err("same metadata replacement must block save");
        assert_eq!(error.code, "external-file-changed");
        assert_eq!(fs::read_to_string(document_path).expect("disk"), "BBBB");
        cleanup(root);
    }

    #[test]
    fn manager_reports_external_change_while_save_is_in_progress() {
        let root = unique_test_dir("external-during-save");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "base").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open");
        let applied = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager.enable_save_test_pause();
        let save_manager = manager.clone();
        let save_session = opened.session_id.clone();
        let save_thread = std::thread::spawn(move || {
            save_manager.save_revision(&save_session, applied.revision, None, false)
        });
        assert!(manager.wait_until_save_test_paused());
        let saving = manager
            .check_external_change(&opened.session_id)
            .expect("saving state");
        assert!(saving.save_in_progress);
        assert!(saving.dirty);

        fs::write(&document_path, "external").expect("external write");
        let changed = manager
            .check_external_change(&opened.session_id)
            .expect("changed while saving");
        assert_eq!(changed.change, SegmentedExternalChangeKind::Modified);
        assert!(changed.save_in_progress);
        manager.release_save_test_pause();
        let error = save_thread
            .join()
            .expect("save thread")
            .expect_err("save conflict");
        assert_eq!(error.code, "external-file-changed", "{error:?}");
        assert!(
            manager
                .check_external_change(&opened.session_id)
                .expect("after save")
                .dirty
        );
        cleanup(root);
    }

    #[test]
    fn manager_keeps_newer_revision_dirty_when_editing_during_save() {
        let root = unique_test_dir("edit-during-save");
        let document_path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open");
        let revision_one = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " one".into(),
                }],
            })
            .expect("revision one");
        manager.enable_save_test_pause();
        let save_manager = manager.clone();
        let save_session = opened.session_id.clone();
        let save_thread = std::thread::spawn(move || {
            save_manager.save_revision(&save_session, revision_one.revision, None, false)
        });
        assert!(manager.wait_until_save_test_paused());
        let revision_two = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: revision_one.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 8,
                    to_byte: 8,
                    inserted_text: " two".into(),
                }],
            })
            .expect("revision two");
        manager.release_save_test_pause();
        let saved = save_thread
            .join()
            .expect("save thread")
            .expect("save revision one");

        assert_eq!(saved.saved_revision, revision_one.revision);
        assert_eq!(saved.current_revision, revision_two.revision);
        assert_eq!(saved.persisted_revision, revision_one.revision);
        assert!(saved.dirty);
        assert_eq!(
            fs::read_to_string(&document_path).expect("disk"),
            "base one"
        );
        let current = manager
            .read_window(&opened.session_id, revision_two.revision, 0, 64)
            .expect("current");
        assert_eq!(current.text, "base one two");
        manager
            .close_session(&opened.session_id, false)
            .expect("preserve newer recovery");
        let recovered_manager = DocumentSessionManager::new(state_root).expect("manager");
        let recovered = recovered_manager
            .open_document(document_path, None)
            .expect("recover revision two");
        assert_eq!(recovered.first_window.text, "base one two");
        cleanup(root);
    }

    #[test]
    fn manager_relocates_recovery_journal_after_save_as() {
        let root = unique_test_dir("save-as-recovery");
        let original = root.join("original.txt");
        let saved_as = root.join("saved-as.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&original, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(original.clone(), None).expect("open");
        let first = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " one".into(),
                }],
            })
            .expect("first edit");
        manager
            .save_revision(
                &opened.session_id,
                first.revision,
                Some(saved_as.clone()),
                false,
            )
            .expect("save as");
        fs::write(&original, "evil").expect("mutate old source after save as");
        let second = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: first.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 8,
                    to_byte: 8,
                    inserted_text: " two".into(),
                }],
            })
            .expect("second edit");
        manager
            .flush_journal(&opened.session_id, second.revision)
            .expect("flush");
        manager
            .close_session(&opened.session_id, false)
            .expect("close");

        let recovered_manager = DocumentSessionManager::new(state_root).expect("manager");
        let recovered = recovered_manager
            .open_document(saved_as, None)
            .expect("recover saved-as path");
        assert_eq!(recovered.first_window.text, "base one two");
        cleanup(root);
    }

    #[test]
    fn manager_recovers_flushed_edits_and_discard_removes_recovery() {
        let root = unique_test_dir("journal-recovery");
        let document_path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open");
        let applied = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, applied.revision)
            .expect("flush");
        manager
            .close_session(&opened.session_id, false)
            .expect("close preserving recovery");

        let recovered_manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let recovered = recovered_manager
            .open_document(document_path.clone(), None)
            .expect("recover");
        let window = recovered_manager
            .read_window(&recovered.session_id, recovered.revision, 0, 64)
            .expect("read recovered");
        assert_eq!(window.text, "base local");
        assert_eq!(recovered.revision, 1);

        recovered_manager
            .close_session(&recovered.session_id, true)
            .expect("discard recovery");
        let clean_manager = DocumentSessionManager::new(state_root).expect("manager");
        let clean = clean_manager
            .open_document(document_path, None)
            .expect("clean reopen");
        assert_eq!(clean.first_window.text, "base");
        cleanup(root);
    }

    #[test]
    fn manager_replace_all_commits_one_revision_and_json_format_streams() {
        let root = unique_test_dir("write-tasks");
        fs::create_dir_all(&root).expect("create root");
        let text_path = root.join("sample.txt");
        fs::write(&text_path, "foo xx foo").expect("write text");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(text_path, None).expect("open text");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("replace all");
        let replaced = wait_for_task(&events, &task.task_id);
        assert_eq!(replaced.match_count, 2);
        assert_eq!(replaced.result_revision, Some(1));
        let window = manager
            .read_window(&opened.session_id, 1, 0, 64)
            .expect("read replacement");
        assert_eq!(window.text, "bar xx bar");

        let json_path = root.join("sample.json");
        fs::write(&json_path, r#"{"a":[1,true]}"#).expect("write json");
        let json = manager.open_document(json_path, None).expect("open json");
        let json_events = Arc::new(Mutex::new(Vec::new()));
        let captured = json_events.clone();
        let format_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: json.session_id.clone(),
                    base_revision: json.revision,
                    task: SegmentedTask::JsonFormat,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("format json");
        let formatted = wait_for_task(&json_events, &format_task.task_id);
        let formatted_revision = formatted.result_revision.expect("result revision");
        let window = manager
            .read_window(&json.session_id, formatted_revision, 0, 1024)
            .expect("read formatted json");
        assert!(window.text.contains('\n'));
        serde_json::from_str::<serde_json::Value>(&window.text).expect("valid formatted json");
        cleanup(root);
    }

    #[test]
    fn manager_reports_streaming_json_progress_without_format_phase_regression() {
        let root = unique_test_dir("json-task-progress");
        fs::create_dir_all(&root).expect("create root");
        let json_path = root.join("large.json");
        write_large_json_fixture(&json_path, br#"{"name":"value","ok":true}"#, 512 * 1024);
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(json_path, None).expect("open json");
        wait_for_index(&manager, &opened.session_id);

        let validate_events = Arc::new(Mutex::new(Vec::new()));
        let captured = validate_events.clone();
        let validate_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::JsonValidate,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("validate json");
        let validated = wait_for_task(&validate_events, &validate_task.task_id);
        let validate_progress = task_processed_bytes(&validate_events, &validate_task.task_id);
        let validate_running =
            task_running_processed_bytes(&validate_events, &validate_task.task_id);
        assert_eq!(validated.processed_bytes, opened.byte_length);
        assert!(validate_progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(
            validate_running
                .iter()
                .any(|processed| *processed > 0 && *processed < opened.byte_length),
            "校验必须在完成前报告流式进度"
        );

        let format_events = Arc::new(Mutex::new(Vec::new()));
        let captured = format_events.clone();
        let format_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id,
                    base_revision: opened.revision,
                    task: SegmentedTask::JsonFormat,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("format json");
        let formatted = wait_for_task(&format_events, &format_task.task_id);
        let format_progress = task_processed_bytes(&format_events, &format_task.task_id);
        let format_running = task_running_processed_bytes(&format_events, &format_task.task_id);
        let phase_boundary = opened.byte_length / 2;
        assert_eq!(formatted.processed_bytes, opened.byte_length);
        assert!(format_progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(format_running
            .iter()
            .any(|processed| *processed > 0 && *processed <= phase_boundary));
        assert!(format_running
            .iter()
            .any(|processed| *processed > phase_boundary));
        cleanup(root);
    }

    #[test]
    fn manager_copy_fallback_reports_path_keeps_latest_and_preserves_other_task_assets() {
        let root = unique_test_dir("copy-fallback");
        let state_root = root.join("state");
        let tasks_root = state_root.join("tasks");
        fs::create_dir_all(&tasks_root).expect("create tasks");
        fs::write(tasks_root.join("stale.txt"), "stale").expect("stale fallback");
        for asset in ["replace.replace", "undo.undo", "format.json"] {
            fs::write(tasks_root.join(asset), asset).expect("task asset");
        }
        let manager = DocumentSessionManager::new(state_root).expect("manager");
        assert!(!tasks_root.join("stale.txt").exists());
        for asset in ["replace.replace", "undo.undo", "format.json"] {
            assert!(tasks_root.join(asset).exists(), "must preserve {asset}");
        }

        let document_path = root.join("sample.txt");
        fs::write(&document_path, "first").expect("write document");
        let opened = manager
            .open_document(document_path, None)
            .expect("open document");
        wait_for_index(&manager, &opened.session_id);
        manager.force_copy_fallback(true);

        let first_events = Arc::new(Mutex::new(Vec::new()));
        let captured = first_events.clone();
        let first_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::SelectAllCopy,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("first copy");
        let first = wait_for_task(&first_events, &first_task.task_id);
        let first_path = PathBuf::from(first.output_path.expect("fallback path"));
        assert_eq!(
            fs::read_to_string(&first_path).expect("first fallback"),
            "first"
        );
        assert!(first
            .message
            .expect("fallback message")
            .contains(&first_path.to_string_lossy().into_owned()));

        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: opened.byte_length,
                    to_byte: opened.byte_length,
                    inserted_text: " second".into(),
                }],
            })
            .expect("edit document");
        let second_events = Arc::new(Mutex::new(Vec::new()));
        let captured = second_events.clone();
        let second_task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id,
                    base_revision: edited.revision,
                    task: SegmentedTask::SelectAllCopy,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("second copy");
        let second = wait_for_task(&second_events, &second_task.task_id);
        let second_path = PathBuf::from(second.output_path.expect("fallback path"));
        assert_ne!(first_path, second_path);
        assert!(!first_path.exists(), "older fallback must be removed");
        assert_eq!(
            fs::read_to_string(&second_path).expect("second fallback"),
            "first second"
        );
        for asset in ["replace.replace", "undo.undo", "format.json"] {
            assert!(tasks_root.join(asset).exists(), "must preserve {asset}");
        }
        cleanup(root);
    }

    #[test]
    fn manager_rejects_utf8_and_crlf_splits_and_edits_across_pieces() {
        let root = unique_test_dir("edit-boundaries");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "a中\r\nb").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");

        let utf8_error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 2,
                    to_byte: 2,
                    inserted_text: "x".into(),
                }],
            })
            .expect_err("must reject UTF-8 split");
        assert_eq!(utf8_error.code, "utf8-boundary-split");
        let crlf_error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 5,
                    to_byte: 5,
                    inserted_text: "x".into(),
                }],
            })
            .expect_err("must reject CRLF split");
        assert_eq!(crlf_error.code, "crlf-boundary-split");

        let first = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 1,
                    to_byte: 4,
                    inserted_text: "文".into(),
                }],
            })
            .expect("first piece edit");
        let second = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: first.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 6,
                    inserted_text: "X".into(),
                }],
            })
            .expect("cross-piece edit");
        let window = manager
            .read_window(&opened.session_id, second.revision, 0, 64)
            .expect("read");
        assert_eq!(window.text, "Xb");
        cleanup(root);
    }

    #[test]
    fn manager_opens_invalid_utf8_readonly_and_preserves_dirty_on_replace_failure() {
        let root = unique_test_dir("readonly-save-failure");
        fs::create_dir_all(&root).expect("create root");
        let invalid_path = root.join("invalid.txt");
        fs::write(&invalid_path, [0xff, 0xfe]).expect("write invalid");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let invalid = manager
            .open_document(invalid_path, None)
            .expect("open invalid");
        assert!(invalid.readonly);
        let error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: invalid.session_id,
                base_revision: invalid.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "x".into(),
                }],
            })
            .expect_err("readonly edit");
        assert_eq!(error.code, "document-readonly");

        let path = root.join("valid.txt");
        fs::write(&path, "base").expect("write valid");
        let valid = manager.open_document(path, None).expect("open valid");
        let applied = manager
            .apply_edits(SegmentedEditBatch {
                session_id: valid.session_id.clone(),
                base_revision: valid.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " dirty".into(),
                }],
            })
            .expect("edit");
        let invalid_target = root.join("non-empty-directory");
        fs::create_dir_all(&invalid_target).expect("target dir");
        fs::write(invalid_target.join("child"), "keep non-empty").expect("child");
        let save_error = manager
            .save_revision(
                &valid.session_id,
                applied.revision,
                Some(invalid_target),
                false,
            )
            .expect_err("replace must fail");
        assert_eq!(save_error.code, "save-replace-failed");
        let status = manager
            .check_external_change(&valid.session_id)
            .expect("status");
        assert!(status.dirty);
        cleanup(root);
    }

    #[test]
    fn manager_preserves_original_dirty_state_and_recovery_after_save_write_enospc() {
        let root = unique_test_dir("save-write-enospc");
        let state_root = root.join("state");
        let document_path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&document_path, "base").expect("write document");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager
            .open_document(document_path.clone(), None)
            .expect("open document");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: opened.byte_length,
                    to_byte: opened.byte_length,
                    inserted_text: " dirty".into(),
                }],
            })
            .expect("edit document");

        manager.fail_next_save_write_after(2);
        let error = manager
            .save_revision(&opened.session_id, edited.revision, None, false)
            .expect_err("injected ENOSPC must fail");
        assert_eq!(error.code, "save-write-failed");
        assert_eq!(
            fs::read_to_string(&document_path).expect("original"),
            "base"
        );
        let current = manager
            .read_window(&opened.session_id, edited.revision, 0, 64)
            .expect("dirty window");
        assert_eq!(current.text, "base dirty");
        assert!(
            manager
                .check_external_change(&opened.session_id)
                .expect("dirty status")
                .dirty
        );

        // save 在临时写入前已刷新 recovery；模拟进程重启后仍能重放未保存 revision。
        drop(manager);
        let recovered_manager = DocumentSessionManager::new(state_root).expect("restart manager");
        let recovered = recovered_manager
            .open_document(document_path, None)
            .expect("recover dirty revision");
        assert_eq!(recovered.revision, edited.revision);
        assert_eq!(recovered.first_window.text, "base dirty");
        cleanup(root);
    }

    #[test]
    fn manager_cancels_read_task_and_write_task_conflicts_with_new_revision() {
        let root = unique_test_dir("task-cancel-conflict");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        write_large_fixture(&path, b"foo-", 2 * 1024 * 1024);
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");

        let cancel_events = Arc::new(Mutex::new(Vec::new()));
        let captured = cancel_events.clone();
        let cancel_manager = manager.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: 0,
                    task: SegmentedTask::Search {
                        query: "missing".into(),
                        case_sensitive: true,
                        whole_word: false,
                        wrap_around: false,
                        anchor_byte: None,
                        direction: None,
                    },
                },
                Some(Arc::new(move |event| {
                    if let SegmentedEvent::Task(progress) = &event {
                        if progress.processed_bytes > 0 {
                            let _ = cancel_manager.cancel_task(&progress.task_id);
                        }
                    }
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start search");
        let cancelled = wait_for_task_terminal(&cancel_events, &task.task_id);
        assert_eq!(cancelled.state, SegmentedTaskState::Cancelled);

        let conflict_events = Arc::new(Mutex::new(Vec::new()));
        let captured = conflict_events.clone();
        let edit_manager = manager.clone();
        let session_id = opened.session_id.clone();
        let edited = Arc::new(AtomicBool::new(false));
        let edit_once = edited.clone();
        let replace = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: 0,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    if let SegmentedEvent::Task(progress) = &event {
                        if progress.processed_bytes > 0 && !edit_once.swap(true, Ordering::AcqRel) {
                            edit_manager
                                .apply_edits(SegmentedEditBatch {
                                    session_id: session_id.clone(),
                                    base_revision: 0,
                                    edits: vec![SegmentedEdit {
                                        from_byte: 0,
                                        to_byte: 0,
                                        inserted_text: "local-".into(),
                                    }],
                                })
                                .expect("concurrent edit");
                        }
                    }
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start replace");
        let conflict = wait_for_task_terminal(&conflict_events, &replace.task_id);
        assert_eq!(conflict.state, SegmentedTaskState::Conflict);
        assert!(edited.load(Ordering::Acquire));
        cleanup(root);
    }

    #[test]
    fn manager_validates_tail_encoding_before_enabling_writes() {
        let root = unique_test_dir("tail-encoding");
        let path = root.join("late-invalid.txt");
        fs::create_dir_all(&root).expect("create root");
        let mut bytes = vec![b'a'; 300 * 1024];
        bytes.push(0xff);
        fs::write(&path, bytes).expect("write invalid tail");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        manager.set_validation_chunk_delay(100);
        let opened = manager.open_document(path, None).expect("open");

        assert!(opened.readonly, "全文编码校验完成前必须保持只读");
        let pending_window = manager
            .read_window(&opened.session_id, opened.revision, 300 * 1024, 8)
            .expect("pending validation uses lossy readonly window");
        assert_eq!(pending_window.text, "\u{fffd}");
        assert_eq!(pending_window.utf16_byte_offsets, Some(vec![0, 1]));
        let status = wait_for_encoding(&manager, &opened.session_id, TextEncoding::Unsupported);
        assert_eq!(status.encoding, TextEncoding::Unsupported);
        assert!(status.readonly);
        assert!(wait_for_index(&manager, &opened.session_id).readonly);
        let error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id,
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "unsafe".into(),
                }],
            })
            .expect_err("invalid tail must stay readonly");
        assert_eq!(error.code, "document-readonly");
        cleanup(root);
    }

    #[test]
    fn manager_marks_a_tail_window_when_it_intersects_a_long_line() {
        let root = unique_test_dir("long-line-tail-window");
        let path = root.join("long-line.txt");
        fs::create_dir_all(&root).expect("create root");
        let mut bytes = vec![b'a'; 150 * 1024];
        bytes.extend_from_slice(b"\nshort\n");
        fs::write(&path, bytes).expect("write long line");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");

        let window = manager
            .read_window(&opened.session_id, opened.revision, 140 * 1024, 4 * 1024)
            .expect("read tail block of long line");
        assert!(
            window.long_line,
            "窗口虽然只覆盖长行尾部，仍必须通过有界相邻扫描报告 longLine"
        );
        cleanup(root);
    }

    #[test]
    fn manager_returns_json_lexical_state_for_window_inside_long_string() {
        let root = unique_test_dir("json-lexical-checkpoint");
        let path = root.join("long.json");
        fs::create_dir_all(&root).expect("create root");
        let mut bytes = b"{\"value\":\"".to_vec();
        bytes.extend(std::iter::repeat_n(b'a', 1200 * 1024));
        bytes.extend_from_slice(b"\",\"tail\":true}");
        fs::write(&path, bytes).expect("write json");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        wait_for_exact_index(&manager, &opened.session_id);
        wait_for_writable_validation(&manager, &opened.session_id);

        let start = 1024 * 1024 + 73;
        let window = manager
            .read_window(&opened.session_id, opened.revision, start, 64)
            .expect("read inside string");
        let lexical = window.json_lexical_state.expect("json lexical state");
        assert_eq!(lexical.mode, JsonLexicalMode::String);
        assert!(!lexical.escaped);

        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "\n".into(),
                }],
            })
            .expect("edit before long string");
        let immediate = manager
            .read_window(&opened.session_id, edited.revision, start + 1, 64)
            .expect("read with invalidated JSON checkpoint");
        assert!(
            immediate.json_lexical_state.is_none(),
            "失效点后的远端 JSON 状态必须标记未知，等待 idle exact rebuild"
        );
        cleanup(root);
    }

    #[test]
    fn manager_undoes_and_redoes_replace_all_as_one_transaction() {
        let root = unique_test_dir("history-task");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "foo foo").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start replace");
        let replaced = wait_for_task(&events, &task.task_id);
        let replaced_revision = replaced.result_revision.expect("replacement revision");
        assert_eq!(
            manager
                .read_window(&opened.session_id, replaced_revision, 0, 64)
                .expect("replaced")
                .text,
            "bar bar"
        );

        let undone = manager
            .undo_revision(&opened.session_id, replaced_revision)
            .expect("undo transaction");
        assert_eq!(
            manager
                .read_window(&opened.session_id, undone.revision, 0, 64)
                .expect("undone")
                .text,
            "foo foo"
        );
        assert!(undone.can_redo);
        let redone = manager
            .redo_revision(&opened.session_id, undone.revision)
            .expect("redo transaction");
        assert_eq!(
            manager
                .read_window(&opened.session_id, redone.revision, 0, 64)
                .expect("redone")
                .text,
            "bar bar"
        );
        assert!(redone.can_undo);
        cleanup(root);
    }

    #[test]
    fn manager_preserves_crlf_in_replace_and_json_format_tasks() {
        let root = unique_test_dir("task-crlf");
        fs::create_dir_all(&root).expect("create root");
        let text_path = root.join("sample.txt");
        fs::write(&text_path, b"foo\r\nfoo").expect("write text");
        let manager = DocumentSessionManager::new(root.join("state-text")).expect("manager");
        let opened = manager.open_document(text_path, None).expect("open text");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "x\ny".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("replace");
        let replaced = wait_for_task(&events, &task.task_id);
        let text = manager
            .read_window(
                &opened.session_id,
                replaced.result_revision.expect("revision"),
                0,
                128,
            )
            .expect("read")
            .text;
        assert_eq!(text, "x\r\ny\r\nx\r\ny");

        let json_path = root.join("sample.json");
        fs::write(&json_path, b"{\r\n\"items\":[1,2]\r\n}").expect("write json");
        let json_manager = DocumentSessionManager::new(root.join("state-json")).expect("manager");
        let json = json_manager
            .open_document(json_path, None)
            .expect("open json");
        assert_eq!(json.line_ending, LineEnding::Crlf);
        let json_events = Arc::new(Mutex::new(Vec::new()));
        let captured = json_events.clone();
        let task = json_manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: json.session_id.clone(),
                    base_revision: json.revision,
                    task: SegmentedTask::JsonFormat,
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("format");
        let formatted = wait_for_task(&json_events, &task.task_id);
        let bytes = json_manager
            .read_window(
                &json.session_id,
                formatted.result_revision.expect("revision"),
                0,
                256,
            )
            .expect("formatted")
            .text
            .into_bytes();
        assert!(bytes.windows(2).any(|pair| pair == b"\r\n"));
        assert!(bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte != b'\n' || index > 0 && bytes[index - 1] == b'\r'));
        cleanup(root);
    }

    #[test]
    fn manager_save_as_unlocks_filesystem_readonly_utf8_session() {
        let root = unique_test_dir("readonly-save-as");
        fs::create_dir_all(&root).expect("create root");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, "base").expect("write");
        let mut permissions = fs::metadata(&source).expect("metadata").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).expect("readonly");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(source, None).expect("open");
        assert!(opened.readonly);

        let saved = manager
            .save_revision(&opened.session_id, opened.revision, Some(target), false)
            .expect("save as");
        assert!(!saved.readonly);
        manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id,
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " writable".into(),
                }],
            })
            .expect("edit after save as");
        cleanup(root);
    }

    #[test]
    fn manager_discard_close_prevents_late_write_task_recovery() {
        let root = unique_test_dir("close-task-race");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        write_large_fixture(&path, b"foo-", 2 * 1024 * 1024);
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager
            .open_document(path.clone(), None)
            .expect("open document");
        wait_for_index(&manager, &opened.session_id);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("start replace");
        manager
            .close_session(&opened.session_id, true)
            .expect("discard close");
        let terminal = wait_for_task_terminal(&events, &task.task_id);
        assert!(matches!(
            terminal.state,
            SegmentedTaskState::Cancelled | SegmentedTaskState::Failed
        ));

        let reopened_manager = DocumentSessionManager::new(state_root).expect("manager");
        let reopened = reopened_manager.open_document(path, None).expect("reopen");
        assert!(reopened.first_window.text.starts_with("foo-"));
        cleanup(root);
    }

    #[test]
    fn manager_rejects_duplicate_open_and_save_as_into_another_session() {
        let root = unique_test_dir("session-path-registry");
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&first_path, "first").expect("write first");
        fs::write(&second_path, "second").expect("write second");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let first = manager
            .open_document(first_path.clone(), None)
            .expect("open first");
        let duplicate = manager
            .open_document(first_path, None)
            .expect_err("duplicate path");
        assert_eq!(duplicate.code, "session-already-open");
        manager
            .open_document(second_path.clone(), None)
            .expect("open second");
        let save_error = manager
            .save_revision(&first.session_id, first.revision, Some(second_path), false)
            .expect_err("occupied save target");
        assert_eq!(save_error.code, "save-target-open");
        cleanup(root);
    }

    #[test]
    fn manager_maps_lossy_utf16_boundaries_to_original_window_bytes() {
        let root = unique_test_dir("lossy-byte-map");
        let path = root.join("invalid.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, b"a\xffb").expect("write invalid UTF-8");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");

        let opened = manager.open_document(path, None).expect("open readonly");

        assert_eq!(opened.encoding, TextEncoding::Unsupported);
        assert!(opened.readonly);
        assert_eq!(opened.first_window.text, "a\u{fffd}b");
        assert_eq!(
            opened.first_window.utf16_byte_offsets,
            Some(vec![0, 1, 2, 3])
        );
        cleanup(root);
    }

    #[test]
    fn manager_undoes_multi_edit_transactions_and_blocks_redo_when_encoding_turns_readonly() {
        let root = unique_test_dir("history-multi-readonly");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "0123456789").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");

        let first = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![
                    SegmentedEdit {
                        from_byte: 1,
                        to_byte: 3,
                        inserted_text: "A".into(),
                    },
                    SegmentedEdit {
                        from_byte: 6,
                        to_byte: 8,
                        inserted_text: "BCDE".into(),
                    },
                ],
            })
            .expect("multi edit");
        let second = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: first.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "prefix-".into(),
                }],
            })
            .expect("second edit");
        assert_eq!(
            manager
                .read_window(&opened.session_id, second.revision, 0, 64)
                .expect("edited")
                .text,
            "prefix-0A345BCDE89"
        );

        // 模拟尾部后台校验刚发现非法 UTF-8：既有补丁仍可逐步撤回，但不得前进或新增。
        manager
            .force_unsupported_for_test(&opened.session_id)
            .expect("force readonly");
        let undo_second = manager
            .undo_revision(&opened.session_id, second.revision)
            .expect("undo while readonly");
        assert_eq!(
            manager
                .read_window(&opened.session_id, undo_second.revision, 0, 64)
                .expect("first transaction remains")
                .text,
            "0A345BCDE89"
        );
        let undo_first = manager
            .undo_revision(&opened.session_id, undo_second.revision)
            .expect("undo to baseline");
        assert_eq!(
            manager
                .read_window(&opened.session_id, undo_first.revision, 0, 64)
                .expect("baseline")
                .text,
            "0123456789"
        );
        let redo_error = manager
            .redo_revision(&opened.session_id, undo_first.revision)
            .expect_err("redo must remain readonly");
        assert_eq!(redo_error.code, "document-readonly");
        let edit_error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id,
                base_revision: undo_first.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "x".into(),
                }],
            })
            .expect_err("new edit must remain readonly");
        assert_eq!(edit_error.code, "document-readonly");
        cleanup(root);
    }

    #[test]
    fn manager_rejects_oversized_direct_edit_before_allocating_inverse_text() {
        let root = unique_test_dir("large-direct-edit");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("create root");
        let length = 8 * 1024 * 1024 + 1;
        fs::write(&path, vec![b'a'; length]).expect("write fixture");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        wait_for_writable_validation(&manager, &opened.session_id);

        let error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id,
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: length as u64,
                    inserted_text: String::new(),
                }],
            })
            .expect_err("large direct delete must use a task");

        assert_eq!(error.code, "edit-transaction-too-large");
        cleanup(root);
    }

    #[test]
    fn manager_task_history_swaps_assets_without_copying_full_text_into_added_store() {
        let root = unique_test_dir("task-history-assets");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "foo foo").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("replace");
        let replaced = wait_for_task(&events, &task.task_id);
        let replaced_revision = replaced.result_revision.expect("revision");
        assert_eq!(manager.added_len(&opened.session_id).expect("added len"), 0);

        let undone = manager
            .undo_revision(&opened.session_id, replaced_revision)
            .expect("undo");
        assert_eq!(manager.added_len(&opened.session_id).expect("added len"), 0);
        manager
            .redo_revision(&opened.session_id, undone.revision)
            .expect("redo");
        assert_eq!(manager.added_len(&opened.session_id).expect("added len"), 0);
        cleanup(root);
    }

    #[test]
    fn manager_save_as_ignores_external_replacement_of_source_and_returns_session_id() {
        let root = unique_test_dir("save-as-external-source");
        let source = root.join("source.txt");
        let external = root.join("external.tmp");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&source, "base").expect("write source");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(source.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        fs::write(&external, "external replacement").expect("write replacement");
        #[cfg(not(target_os = "windows"))]
        fs::rename(&external, &source).expect("atomically replace source");
        #[cfg(target_os = "windows")]
        {
            fs::remove_file(&source).expect("remove source");
            fs::rename(&external, &source).expect("replace source");
        }

        let saved = manager
            .save_revision(
                &opened.session_id,
                edited.revision,
                Some(target.clone()),
                false,
            )
            .expect("save as despite source change");

        assert_eq!(saved.session_id, opened.session_id);
        assert_eq!(fs::read_to_string(target).expect("target"), "base local");
        assert_eq!(
            fs::read_to_string(source).expect("source"),
            "external replacement"
        );
        cleanup(root);
    }

    #[test]
    fn manager_save_as_preserves_undo_for_a_concurrent_higher_revision() {
        let root = unique_test_dir("save-as-history-race");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&source, "base").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(source, None).expect("open");
        let revision_one = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " one".into(),
                }],
            })
            .expect("revision one");
        manager.enable_save_test_pause();
        let save_manager = manager.clone();
        let save_session = opened.session_id.clone();
        let saved_revision = revision_one.revision;
        let save_thread = std::thread::spawn(move || {
            save_manager.save_revision(&save_session, saved_revision, Some(target), false)
        });
        assert!(manager.wait_until_save_test_paused());
        let revision_two = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: revision_one.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 8,
                    to_byte: 8,
                    inserted_text: " two".into(),
                }],
            })
            .expect("revision two");
        manager.release_save_test_pause();
        let saved = save_thread.join().expect("save thread").expect("save as");
        assert_eq!(saved.session_id, opened.session_id);
        assert!(saved.dirty);

        let undone = manager
            .undo_revision(&opened.session_id, revision_two.revision)
            .expect("undo newer revision");
        assert_eq!(
            manager
                .read_window(&opened.session_id, undone.revision, 0, 64)
                .expect("read undone")
                .text,
            "base one"
        );
        assert!(undone.can_undo, "save as must retain earlier history too");
        cleanup(root);
    }

    #[test]
    fn manager_dirty_close_keeps_only_task_asset_referenced_by_recovery() {
        let root = unique_test_dir("dirty-close-task-assets");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "foo foo").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let task = manager
            .start_task(
                StartSegmentedTaskRequest {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    task: SegmentedTask::ReplaceAll {
                        query: "foo".into(),
                        replacement: "bar".into(),
                        case_sensitive: true,
                        whole_word: false,
                    },
                },
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("replace");
        let replaced = wait_for_task(&events, &task.task_id);
        let output_path = manager.task_output_path(&task.task_id, "replace");
        let undo_path = manager.task_output_path(&task.task_id, "undo");
        assert!(output_path.is_file());
        assert!(undo_path.is_file());

        manager
            .close_session(&opened.session_id, false)
            .expect("dirty close");
        assert!(
            output_path.is_file(),
            "journal still references task output"
        );
        assert!(
            !undo_path.exists(),
            "in-memory-only undo asset must be removed"
        );

        let recovered_manager = DocumentSessionManager::new(state_root).expect("manager");
        let recovered = recovered_manager
            .open_document(path, None)
            .expect("recover task output");
        assert_eq!(recovered.first_window.text, "bar bar");
        recovered_manager
            .save_revision(
                &recovered.session_id,
                replaced.result_revision.expect("revision"),
                None,
                false,
            )
            .expect("save recovered text");
        assert!(!output_path.exists(), "save prunes obsolete recovery asset");
        cleanup(root);
    }

    #[test]
    fn manager_merges_edits_into_one_original_scan_before_idle_exact_rebuild() {
        let root = unique_test_dir("original-index-delta-merge");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("create root");
        write_large_fixture(&path, b"line-value\n", 4 * 1024 * 1024);
        let original = fs::read(&path).expect("fixture bytes");
        let original_lines = original.iter().filter(|byte| **byte == b'\n').count() as u64;
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let edited = Arc::new(AtomicBool::new(false));
        let edit_once = edited.clone();
        let validation_unlocked_before_index = Arc::new(AtomicBool::new(false));
        let unlocked = validation_unlocked_before_index.clone();
        let latest_revision = Arc::new(AtomicU64::new(0));
        let latest = latest_revision.clone();
        let event_manager = manager.clone();
        let sink: super::EventSink = Arc::new(move |event| {
            if let SegmentedEvent::Index(progress) = &event {
                if progress.indexed_bytes == 0 && progress.task_id.starts_with("index-") {
                    event_manager
                        .set_index_chunk_delay(&progress.session_id, 20)
                        .expect("slow initial index");
                }
                if progress.task_id.starts_with("validation-")
                    && progress.readonly == Some(false)
                    && !edit_once.swap(true, Ordering::AcqRel)
                {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while event_manager
                        .initial_index_build_starts(&progress.session_id)
                        .expect("initial build count")
                        == 0
                    {
                        assert!(Instant::now() < deadline, "initial scan did not start");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let status = event_manager
                        .session_status(&progress.session_id)
                        .expect("status during validation");
                    unlocked.store(!status.readonly && !status.completed, Ordering::Release);
                    let mut revision = progress.revision;
                    for _ in 0..40 {
                        revision = event_manager
                            .apply_edits(SegmentedEditBatch {
                                session_id: progress.session_id.clone(),
                                base_revision: revision,
                                edits: vec![SegmentedEdit {
                                    from_byte: 0,
                                    to_byte: 0,
                                    inserted_text: "x\n".into(),
                                }],
                            })
                            .expect("edit during original scan")
                            .revision;
                    }
                    latest.store(revision, Ordering::Release);
                    event_manager
                        .set_index_chunk_delay(&progress.session_id, 0)
                        .expect("release initial index");
                }
            }
            captured.lock().expect("events").push(event);
        });
        let opened = manager
            .open_document(path, Some(sink))
            .expect("open large document");
        assert!(
            opened.readonly,
            "validation pending must initially gate writes"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while !edited.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "validation did not unlock");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(validation_unlocked_before_index.load(Ordering::Acquire));
        let revision = latest_revision.load(Ordering::Acquire);
        assert_eq!(revision, 40);
        let merged = wait_for_index(&manager, &opened.session_id);
        assert_eq!(
            manager
                .index_revision(&opened.session_id)
                .expect("index revision"),
            40
        );
        assert_eq!(merged.estimated_lines, original_lines + 40);
        assert_eq!(
            manager
                .initial_index_build_starts(&opened.session_id)
                .expect("initial builds"),
            1
        );
        assert!(
            events.lock().expect("events").iter().any(|event| matches!(
                event,
                SegmentedEvent::Index(progress) if progress.completed && progress.revision == 40
            )),
            "original scan completion must publish the latest merged revision"
        );

        let start = opened.byte_length / 2 + 80;
        let window = manager
            .read_window(&opened.session_id, revision, start, 128)
            .expect("far window after delta merge");
        let original_prefix = (window.start_byte - 80) as usize;
        let expected_line = 40
            + original[..original_prefix]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64;
        assert_eq!(window.start_line, expected_line);

        wait_for_exact_index(&manager, &opened.session_id);
        assert_eq!(
            manager
                .index_build_starts(&opened.session_id)
                .expect("all builds"),
            2,
            "one original scan plus one coalesced idle exact rebuild"
        );
        assert_eq!(
            manager
                .index_worker_starts(&opened.session_id)
                .expect("workers"),
            1,
            "the same worker must carry merge and idle rebuild"
        );
        cleanup(root);
    }

    #[test]
    fn manager_reuses_one_index_worker_for_a_rapid_edit_burst() {
        let root = unique_test_dir("index-worker-burst");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base\n").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        wait_for_index(&manager, &opened.session_id);
        std::thread::sleep(Duration::from_millis(20));
        let starts_before = manager
            .index_worker_starts(&opened.session_id)
            .expect("worker starts");
        let builds_before = manager
            .index_build_starts(&opened.session_id)
            .expect("index builds");
        let mut revision = opened.revision;
        for _ in 0..100 {
            revision = manager
                .apply_edits(SegmentedEditBatch {
                    session_id: opened.session_id.clone(),
                    base_revision: revision,
                    edits: vec![SegmentedEdit {
                        from_byte: 0,
                        to_byte: 0,
                        inserted_text: "x".into(),
                    }],
                })
                .expect("rapid edit")
                .revision;
        }
        wait_for_exact_index(&manager, &opened.session_id);
        let starts_after = manager
            .index_worker_starts(&opened.session_id)
            .expect("worker starts");
        let builds_after = manager
            .index_build_starts(&opened.session_id)
            .expect("index builds");
        assert_eq!(starts_after, starts_before + 1);
        assert_eq!(
            builds_after,
            builds_before + 1,
            "rapid input must trigger only the final idle rebuild"
        );
        cleanup(root);
    }

    #[test]
    fn manager_far_window_reads_bounded_prefix_chunks_without_cloning_index() {
        let root = unique_test_dir("bounded-window-index");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("create root");
        write_large_fixture(&path, b"line-value\n", 5 * 1024 * 1024);
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        wait_for_index(&manager, &opened.session_id);
        std::thread::sleep(Duration::from_millis(20));
        let index_before = manager
            .index_identity(&opened.session_id)
            .expect("index identity");
        super::chunk_reader::reset_read_range_metrics();

        manager
            .read_window(
                &opened.session_id,
                opened.revision,
                4 * 1024 * 1024 + 123,
                64 * 1024,
            )
            .expect("far window");

        assert!(super::chunk_reader::max_read_range_request() <= 256 * 1024);
        assert_eq!(
            manager
                .index_identity(&opened.session_id)
                .expect("index identity"),
            index_before
        );
        cleanup(root);
    }

    #[test]
    fn manager_reload_failure_preserves_the_dirty_old_session() {
        let root = unique_test_dir("reload-failure");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        fs::remove_file(path).expect("remove source");

        manager
            .reload_session(&opened.session_id, None)
            .expect_err("reload candidate must fail");
        assert_eq!(
            manager
                .read_window(&opened.session_id, edited.revision, 0, 64)
                .expect("old session remains usable")
                .text,
            "base local"
        );
        cleanup(root);
    }

    #[test]
    fn manager_reload_atomically_replaces_the_old_session_after_candidate_succeeds() {
        let root = unique_test_dir("reload-success");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        fs::write(&path, "disk replacement").expect("external write");

        let reloaded = manager
            .reload_session(&opened.session_id, None)
            .expect("reload");

        assert_ne!(reloaded.session_id, opened.session_id);
        assert_eq!(reloaded.revision, 0);
        assert_eq!(reloaded.first_window.text, "disk replacement");
        let old_error = manager
            .read_window(&opened.session_id, edited.revision, 0, 64)
            .expect_err("old id removed");
        assert_eq!(old_error.code, "session-not-found");
        let recovery_entries = fs::read_dir(state_root.join("recovery"))
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0);
        assert_eq!(recovery_entries, 0, "reload discards old dirty recovery");
        cleanup(root);
    }

    #[test]
    fn manager_reload_stays_committed_when_old_asset_cleanup_fails() {
        let root = unique_test_dir("reload-cleanup-failure");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        fs::write(&path, "disk replacement").expect("external write");
        manager.fail_reload_cleanup_for_test();

        let reloaded = manager
            .reload_session(&opened.session_id, None)
            .expect("cleanup failure must not roll back committed reload");

        assert_eq!(reloaded.first_window.text, "disk replacement");
        assert_eq!(
            manager
                .session_status(&opened.session_id)
                .expect_err("old session was committed out")
                .code,
            "session-not-found"
        );
        assert!(
            fs::read_dir(state_root.join("recovery"))
                .expect("recovery directory")
                .next()
                .is_some(),
            "injected cleanup failure should leave the old asset as evidence"
        );
        cleanup(root);
    }

    #[test]
    fn manager_surfaces_baseline_mismatch_as_a_readable_recovery_conflict_path() {
        let root = unique_test_dir("recovery-conflict");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        let later = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: edited.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 10,
                    to_byte: 10,
                    inserted_text: " tail".into(),
                }],
            })
            .expect("edit after recovery snapshot");
        manager
            .flush_journal(&opened.session_id, later.revision)
            .expect("flush forward record");
        manager
            .close_session(&opened.session_id, false)
            .expect("preserve dirty recovery");
        fs::write(&path, "external replacement").expect("change baseline");

        let reopened_manager = DocumentSessionManager::new(state_root).expect("manager");
        let reopened = reopened_manager
            .open_document(path, None)
            .expect("open changed baseline");

        assert_eq!(reopened.revision, 0, "old patch must not be auto-applied");
        assert_eq!(reopened.first_window.text, "external replacement");
        let conflict_path = PathBuf::from(
            reopened
                .recovery_conflict_path
                .expect("conflict path must be explicit"),
        );
        assert!(conflict_path.is_file());
        assert_eq!(
            conflict_path.extension().and_then(|value| value.to_str()),
            Some("txt")
        );
        assert!(conflict_path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("sample.recovered-")));
        assert_eq!(
            fs::read_to_string(conflict_path).expect("read recovered document"),
            "base local tail"
        );
        cleanup(root);
    }

    #[test]
    fn manager_materializes_recovery_when_the_source_was_deleted() {
        let root = unique_test_dir("deleted-source-recovery");
        let path = root.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        manager
            .close_session(&opened.session_id, false)
            .expect("dirty close");
        fs::remove_file(&path).expect("delete source");

        let reopened_manager = DocumentSessionManager::new(state_root).expect("manager");
        let error = reopened_manager
            .open_document(path, None)
            .expect_err("missing source returns recovered path");

        assert_eq!(error.code, "source-missing-recovered");
        let recovered = PathBuf::from(error.recovery_path.expect("recovery path"));
        assert_eq!(
            recovered.extension().and_then(|value| value.to_str()),
            Some("txt")
        );
        assert_eq!(
            fs::read_to_string(recovered).expect("recovered"),
            "base local"
        );
        cleanup(root);
    }

    #[test]
    fn manager_falls_back_to_state_directory_when_source_directory_is_readonly() {
        let root = unique_test_dir("readonly-source-recovery");
        let source_dir = root.join("source");
        let path = source_dir.join("sample.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&source_dir).expect("create source root");
        fs::write(&path, "base").expect("write");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path.clone(), None).expect("open");
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: " local".into(),
                }],
            })
            .expect("edit");
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        manager
            .close_session(&opened.session_id, false)
            .expect("dirty close");
        fs::write(&path, "external").expect("change baseline");
        let mut permissions = fs::metadata(&source_dir).expect("metadata").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source_dir, permissions.clone()).expect("readonly directory");

        let reopened_manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let reopened = reopened_manager
            .open_document(path, None)
            .expect("open with fallback recovery");

        let recovered = PathBuf::from(
            reopened
                .recovery_conflict_path
                .expect("fallback recovery path"),
        );
        assert!(recovered.starts_with(state_root.join("recovered")));
        assert_eq!(
            fs::read_to_string(recovered).expect("recovered"),
            "base local"
        );
        permissions.set_readonly(false);
        fs::set_permissions(&source_dir, permissions).expect("restore permissions");
        cleanup(root);
    }

    #[test]
    #[ignore = "显式大文件性能基准；设置 NOMO_SEGMENTED_PERF_MIB 调整规模"]
    fn segmented_manager_large_file_benchmark() {
        let root = unique_test_dir("large-benchmark");
        fs::create_dir_all(&root).expect("create root");
        let mib = std::env::var("NOMO_SEGMENTED_PERF_MIB")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(16);
        let target_bytes = mib * 1024 * 1024;
        // 四类夹具分别覆盖 §14 的性能场景；TXT 只写完整 pattern，避免中文尾部被截成非法 UTF-8。
        let fixtures = [
            (
                "multiline.txt",
                b"ordinary line 0123456789\n".as_slice(),
                false,
            ),
            (
                "multiline.json",
                "{\n  \"key\": \"中文\",\n  \"value\": 12345\n}".as_bytes(),
                true,
            ),
            (
                "compressed-single-line.json",
                r#"{"key":"中文","value":12345}"#.as_bytes(),
                true,
            ),
            (
                "chinese.txt",
                "大量中文字符用于分段读写性能验证。\n".as_bytes(),
                false,
            ),
        ];

        for (name, pattern, is_json) in fixtures {
            let path = root.join(name);
            if is_json {
                write_large_json_fixture(&path, pattern, target_bytes);
            } else {
                let complete_pattern_bytes = target_bytes - (target_bytes % pattern.len());
                write_large_fixture(&path, pattern, complete_pattern_bytes);
            }
            let manager =
                DocumentSessionManager::new(root.join(format!("state-{name}"))).expect("manager");

            let open_started = Instant::now();
            let opened = manager
                .open_document(path.clone(), None)
                .expect("open large document");
            let open_elapsed = open_started.elapsed();
            assert!(!opened.first_window.text.is_empty());

            // 索引计时必须紧接 open；若先做 96 次读取，后台索引早已完成，会得到虚假的 0ms。
            let index_started = Instant::now();
            wait_for_index(&manager, &opened.session_id);
            let index_elapsed = index_started.elapsed();
            let file_mib = opened.byte_length as f64 / 1024.0 / 1024.0;
            let index_throughput_mib = file_mib / index_elapsed.as_secs_f64().max(0.000_001);

            let read_started = Instant::now();
            for step in 0..96_u64 {
                let start = (step * 128 * 1024).min(opened.byte_length.saturating_sub(1));
                manager
                    .read_window(&opened.session_id, opened.revision, start, 128 * 1024)
                    .expect("read window");
            }
            let read_elapsed = read_started.elapsed();
            let (cache_bytes, cache_capacity) = manager
                .cache_usage(&opened.session_id)
                .expect("cache usage");
            assert!(cache_bytes <= cache_capacity);

            let edit_started = Instant::now();
            let applied = manager
                .apply_edits(SegmentedEditBatch {
                    session_id: opened.session_id.clone(),
                    base_revision: opened.revision,
                    edits: vec![SegmentedEdit {
                        from_byte: 0,
                        to_byte: 0,
                        inserted_text: "benchmark-prefix\n".into(),
                    }],
                })
                .expect("edit large document");
            let edit_elapsed = edit_started.elapsed();

            let output = root.join(format!("saved-{name}"));
            let save_started = Instant::now();
            manager
                .save_revision(&opened.session_id, applied.revision, Some(output), false)
                .expect("save large document");
            let save_elapsed = save_started.elapsed();
            let throughput_mib = file_mib / save_elapsed.as_secs_f64().max(0.000_001);
            eprintln!(
                "segmented-perf dataset={name} sizeMiB={mib} openMs={} read96Ms={} indexMs={} indexMiBps={index_throughput_mib:.2} editUs={} cacheBytes={cache_bytes}/{cache_capacity} saveMiBps={throughput_mib:.2}",
                open_elapsed.as_millis(),
                read_elapsed.as_millis(),
                index_elapsed.as_millis(),
                edit_elapsed.as_micros(),
            );
            manager
                .close_session(&opened.session_id, true)
                .expect("close benchmark session");
        }
        cleanup(root);
    }

    #[test]
    fn manager_returns_first_window_before_forced_baseline_copy_and_unlocks_after_publish() {
        let root = unique_test_dir("baseline-copy-fallback");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("root");
        write_large_fixture(&path, b"fallback line\n", 4 * 1024 * 1024);
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        manager.force_baseline_copy_fallback(true, 15);

        let started = Instant::now();
        let opened = manager.open_document(path, None).expect("open fallback");
        let elapsed = started.elapsed();

        assert!(opened.readonly, "baseline 未发布前必须只读");
        assert!(
            !manager
                .baseline_ready(&opened.session_id)
                .expect("baseline state"),
            "带每块延迟的全文 copy 不应阻塞首窗返回"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "open took {elapsed:?}"
        );
        let error = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "blocked".into(),
                }],
            })
            .expect_err("copy pending must gate edits");
        assert_eq!(error.code, "document-readonly");

        wait_for_writable_validation(&manager, &opened.session_id);
        let status = wait_for_index(&manager, &opened.session_id);
        assert!(status.completed);
        assert!(manager.baseline_ready(&opened.session_id).expect("ready"));
        manager
            .close_session(&opened.session_id, true)
            .expect("close");
        cleanup(root);
    }

    #[test]
    fn manager_keeps_dirty_baseline_after_in_place_overwrite_for_recovery_and_both_save_modes() {
        let root = unique_test_dir("immutable-in-place-overwrite");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("root");
        let original_body = format!("HEAD\n{}\nTAIL\n", "a".repeat(300 * 1024));

        let recovery_source = root.join("recovery.txt");
        fs::write(&recovery_source, &original_body).expect("source");
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager
            .open_document(recovery_source.clone(), None)
            .expect("open recovery source");
        wait_for_writable_validation(&manager, &opened.session_id);
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "LOCAL\n".into(),
                }],
            })
            .expect("edit");
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&recovery_source)
            .expect("in-place open")
            .write_all(b"EXTERNAL\n")
            .expect("in-place overwrite");
        let external = manager
            .check_external_change(&opened.session_id)
            .expect("external state");
        let repeated = manager
            .check_external_change(&opened.session_id)
            .expect("repeated external state");
        assert_eq!(external.change, SegmentedExternalChangeKind::Modified);
        assert_eq!(external.change_token, repeated.change_token);
        fs::write(&recovery_source, "SECOND EXTERNAL CHANGE\n").expect("second external change");
        let changed_again = manager
            .check_external_change(&opened.session_id)
            .expect("second external identity");
        assert_ne!(external.change_token, changed_again.change_token);
        let far = manager
            .read_window(
                &opened.session_id,
                edited.revision,
                edited.invalidated_to_byte + original_body.len() as u64 - 32,
                64,
            )
            .expect("read immutable far window");
        assert!(far.text.contains("TAIL"));
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush from immutable baseline");
        manager
            .close_session(&opened.session_id, false)
            .expect("dirty close");
        let reopened_manager = DocumentSessionManager::new(state_root).expect("reopen manager");
        let reopened = reopened_manager
            .open_document(recovery_source, None)
            .expect("open external source with conflict");
        let recovered_path = PathBuf::from(
            reopened
                .recovery_conflict_path
                .clone()
                .expect("recovery conflict path"),
        );
        let recovered = fs::read_to_string(recovered_path).expect("recovered document");
        assert!(recovered.starts_with("LOCAL\nHEAD\n"));
        assert!(recovered.ends_with("TAIL\n"));
        reopened_manager
            .close_session(&reopened.session_id, true)
            .expect("close reopened");

        let save_as_source = root.join("save-as-source.txt");
        let save_as_target = root.join("save-as-target.txt");
        fs::write(&save_as_source, "base-save-as").expect("save-as source");
        let save_as_manager =
            DocumentSessionManager::new(root.join("save-as-state")).expect("save-as manager");
        let save_as_opened = save_as_manager
            .open_document(save_as_source.clone(), None)
            .expect("open save-as");
        let save_as_edit = save_as_manager
            .apply_edits(SegmentedEditBatch {
                session_id: save_as_opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: "-local".into(),
                }],
            })
            .expect("save-as edit");
        fs::write(&save_as_source, "external").expect("in-place save-as overwrite");
        save_as_manager
            .save_revision(
                &save_as_opened.session_id,
                save_as_edit.revision,
                Some(save_as_target.clone()),
                false,
            )
            .expect("save as from immutable baseline");
        assert_eq!(
            fs::read_to_string(save_as_target).expect("saved as"),
            "base-local-save-as"
        );

        let overwrite_source = root.join("overwrite.txt");
        fs::write(&overwrite_source, "base-overwrite").expect("overwrite source");
        let overwrite_manager =
            DocumentSessionManager::new(root.join("overwrite-state")).expect("overwrite manager");
        let overwrite_opened = overwrite_manager
            .open_document(overwrite_source.clone(), None)
            .expect("open overwrite");
        let overwrite_edit = overwrite_manager
            .apply_edits(SegmentedEditBatch {
                session_id: overwrite_opened.session_id.clone(),
                base_revision: 0,
                edits: vec![SegmentedEdit {
                    from_byte: 4,
                    to_byte: 4,
                    inserted_text: "-local".into(),
                }],
            })
            .expect("overwrite edit");
        fs::write(&overwrite_source, "external replacement").expect("external overwrite");
        overwrite_manager
            .save_revision(
                &overwrite_opened.session_id,
                overwrite_edit.revision,
                None,
                true,
            )
            .expect("overwrite external from immutable baseline");
        assert_eq!(
            fs::read_to_string(overwrite_source).expect("overwritten"),
            "base-local-overwrite"
        );
        cleanup(root);
    }

    #[test]
    fn first_journal_flush_reuses_baseline_without_scanning_the_document() {
        let root = unique_test_dir("journal-baseline-reuse");
        let path = root.join("large.txt");
        let state_root = root.join("state");
        fs::create_dir_all(&root).expect("root");
        write_large_fixture(&path, b"durable baseline\n", 4 * 1024 * 1024);
        let manager = DocumentSessionManager::new(state_root.clone()).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        wait_for_index(&manager, &opened.session_id);
        wait_for_writable_validation(&manager, &opened.session_id);
        let edited = manager
            .apply_edits(SegmentedEditBatch {
                session_id: opened.session_id.clone(),
                base_revision: opened.revision,
                edits: vec![SegmentedEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "local\n".into(),
                }],
            })
            .expect("edit");
        reset_read_range_metrics();
        manager
            .flush_journal(&opened.session_id, edited.revision)
            .expect("flush");
        assert_eq!(
            total_read_range_bytes(),
            0,
            "first flush must not materialize an O(file size) snapshot"
        );
        let journal = fs::read_dir(state_root.join("recovery"))
            .expect("recovery dir")
            .find_map(|entry| {
                let path = entry.ok()?.path();
                (path.extension().and_then(|value| value.to_str()) == Some("journal"))
                    .then_some(path)
            })
            .expect("journal");
        assert!(fs::read_to_string(journal)
            .expect("journal payload")
            .contains(".snapshot"));
        manager
            .close_session(&opened.session_id, true)
            .expect("close");
        cleanup(root);
    }

    #[test]
    fn pending_index_publishes_remote_checkpoints_and_reuses_foreground_scan() {
        let root = unique_test_dir("pending-index-checkpoint");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("root");
        write_large_fixture(&path, b"0123456789abcdef\n", 16 * 1024 * 1024);
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let captured = gate.clone();
        let sink = Arc::new(move |event: SegmentedEvent| {
            let SegmentedEvent::Index(progress) = event else {
                return;
            };
            if progress.completed || progress.indexed_bytes < 2 * 1024 * 1024 {
                return;
            }
            let (lock, condition) = &*captured;
            let mut state = lock.lock().expect("gate");
            if state.0 {
                return;
            }
            state.0 = true;
            condition.notify_all();
            while !state.1 {
                state = condition.wait(state).expect("release index");
            }
        });
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager
            .open_document(path.clone(), Some(sink))
            .expect("open");
        {
            let (lock, condition) = &*gate;
            let state = lock.lock().expect("gate");
            let (state, _) = condition
                .wait_timeout_while(state, Duration::from_secs(5), |state| !state.0)
                .expect("wait progress");
            assert!(state.0, "index did not reach progressive checkpoint");
        }

        let start = 1536 * 1024;
        reset_read_range_metrics();
        let first = manager
            .read_window(&opened.session_id, opened.revision, start, 128)
            .expect("remote pending window");
        let first_read_bytes = total_read_range_bytes();
        reset_read_range_metrics();
        let second = manager
            .read_window(&opened.session_id, opened.revision, start, 128)
            .expect("repeat remote pending window");
        let repeated_read_bytes = total_read_range_bytes();
        {
            let (lock, condition) = &*gate;
            let mut state = lock.lock().expect("gate");
            state.1 = true;
            condition.notify_all();
        }
        let bytes = fs::read(path).expect("fixture");
        let expected_line = bytes[..first.start_byte as usize]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
        assert_eq!(first.start_line, expected_line);
        assert_eq!(second.start_line, expected_line);
        assert!(first.index_progress < 1.0);
        assert!(
            first_read_bytes < 1024 * 1024,
            "progressive checkpoint should bound prefix scan, got {first_read_bytes}"
        );
        assert!(
            repeated_read_bytes < 400 * 1024,
            "foreground checkpoint should deduplicate repeated O(offset) scan, got {repeated_read_bytes}"
        );
        cleanup(root);
    }

    #[test]
    fn manager_counts_cr_only_and_lone_cr_lines_consistently() {
        let root = unique_test_dir("cr-only-lines");
        let path = root.join("sample.txt");
        fs::create_dir_all(&root).expect("root");
        fs::write(&path, b"a\rb\r\nc\rd").expect("fixture");
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        let opened = manager.open_document(path, None).expect("open");
        assert_eq!(opened.line_ending, LineEnding::Mixed);
        let status = wait_for_index(&manager, &opened.session_id);
        assert_eq!(status.estimated_lines, 3);
        let after_crlf = manager
            .read_window(&opened.session_id, opened.revision, 5, 1)
            .expect("after CRLF");
        let after_lone_cr = manager
            .read_window(&opened.session_id, opened.revision, 7, 1)
            .expect("after lone CR");
        assert_eq!(after_crlf.start_line, 2);
        assert!(!after_crlf.leading_partial_line);
        assert_eq!(after_lone_cr.start_line, 3);
        assert!(!after_lone_cr.leading_partial_line);
        cleanup(root);
    }

    #[test]
    fn baseline_copy_failure_is_visible_in_status_and_event() {
        let root = unique_test_dir("baseline-copy-error");
        let path = root.join("large.txt");
        fs::create_dir_all(&root).expect("root");
        write_large_fixture(&path, b"baseline error\n", 2 * 1024 * 1024);
        let manager = DocumentSessionManager::new(root.join("state")).expect("manager");
        manager.force_baseline_copy_fallback(true, 10);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let opened = manager
            .open_document(
                path.clone(),
                Some(Arc::new(move |event| {
                    captured.lock().expect("events").push(event);
                })),
            )
            .expect("open");
        std::thread::sleep(Duration::from_millis(30));
        fs::write(&path, "changed during copy").expect("change source");
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            let status = manager.session_status(&opened.session_id).expect("status");
            if status.baseline_error.is_some() {
                break status;
            }
            assert!(Instant::now() < deadline, "baseline error was not surfaced");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.readonly);
        assert!(status
            .baseline_error
            .as_deref()
            .is_some_and(|message| message.contains("baseline")));
        assert!(events.lock().expect("events").iter().any(|event| matches!(
            event,
            SegmentedEvent::Index(progress) if progress.baseline_error.is_some()
        )));
        cleanup(root);
    }

    #[test]
    fn manager_startup_removes_unpublished_session_baseline_assets() {
        let root = unique_test_dir("baseline-startup-cleanup");
        let state_root = root.join("state");
        let sessions = state_root.join("sessions");
        fs::create_dir_all(&sessions).expect("sessions");
        let partial = sessions.join("crashed.baseline");
        let added = sessions.join("crashed.added");
        fs::write(&partial, "partial baseline").expect("partial");
        fs::write(&added, "partial added").expect("added");

        let _manager = DocumentSessionManager::new(state_root).expect("manager");

        assert!(!partial.exists());
        assert!(!added.exists());
        cleanup(root);
    }

    fn wait_for_task(
        events: &Arc<Mutex<Vec<SegmentedEvent>>>,
        task_id: &str,
    ) -> super::TaskProgressEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(progress) = events.lock().expect("events").iter().find_map(|event| {
                let SegmentedEvent::Task(progress) = event else {
                    return None;
                };
                (progress.task_id == task_id && progress.state == SegmentedTaskState::Completed)
                    .then_some(progress.clone())
            }) {
                return progress;
            }
            assert!(Instant::now() < deadline, "task {task_id} timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn task_processed_bytes(events: &Arc<Mutex<Vec<SegmentedEvent>>>, task_id: &str) -> Vec<u64> {
        events
            .lock()
            .expect("events")
            .iter()
            .filter_map(|event| {
                let SegmentedEvent::Task(progress) = event else {
                    return None;
                };
                (progress.task_id == task_id).then_some(progress.processed_bytes)
            })
            .collect()
    }

    fn task_running_processed_bytes(
        events: &Arc<Mutex<Vec<SegmentedEvent>>>,
        task_id: &str,
    ) -> Vec<u64> {
        events
            .lock()
            .expect("events")
            .iter()
            .filter_map(|event| {
                let SegmentedEvent::Task(progress) = event else {
                    return None;
                };
                (progress.task_id == task_id && progress.state == SegmentedTaskState::Running)
                    .then_some(progress.processed_bytes)
            })
            .collect()
    }

    fn wait_for_index(
        manager: &DocumentSessionManager,
        session_id: &str,
    ) -> super::SegmentedSessionStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.session_status(session_id).expect("session status");
            if status.completed {
                return status;
            }
            assert!(Instant::now() < deadline, "index {session_id} timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_exact_index(
        manager: &DocumentSessionManager,
        session_id: &str,
    ) -> super::SegmentedSessionStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.session_status(session_id).expect("session status");
            if manager.index_is_exact(session_id).expect("exact index") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "exact index {session_id} timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_writable_validation(
        manager: &DocumentSessionManager,
        session_id: &str,
    ) -> super::SegmentedSessionStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.session_status(session_id).expect("session status");
            if !status.readonly {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "validation {session_id} timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_encoding(
        manager: &DocumentSessionManager,
        session_id: &str,
        encoding: TextEncoding,
    ) -> super::SegmentedSessionStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.session_status(session_id).expect("session status");
            if status.encoding == encoding {
                return status;
            }
            assert!(Instant::now() < deadline, "encoding {session_id} timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_task_terminal(
        events: &Arc<Mutex<Vec<SegmentedEvent>>>,
        task_id: &str,
    ) -> super::TaskProgressEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(progress) = events.lock().expect("events").iter().find_map(|event| {
                let SegmentedEvent::Task(progress) = event else {
                    return None;
                };
                (progress.task_id == task_id && progress.state != SegmentedTaskState::Running)
                    .then_some(progress.clone())
            }) {
                return progress;
            }
            assert!(Instant::now() < deadline, "task {task_id} timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn write_large_fixture(path: &std::path::Path, pattern: &[u8], target_bytes: usize) {
        let mut file = fs::File::create(path).expect("create fixture");
        let mut written = 0_usize;
        while written < target_bytes {
            let count = pattern.len().min(target_bytes - written);
            file.write_all(&pattern[..count]).expect("write fixture");
            written += count;
        }
        file.sync_all().expect("sync fixture");
    }

    fn write_large_json_fixture(path: &std::path::Path, object: &[u8], target_bytes: usize) {
        let mut file = fs::File::create(path).expect("create json fixture");
        file.write_all(b"[").expect("json open");
        let mut written = 1_usize;
        let mut first = true;
        while written + object.len() + 1 < target_bytes {
            if !first {
                file.write_all(b",").expect("json separator");
                written += 1;
            }
            file.write_all(object).expect("json object");
            written += object.len();
            first = false;
        }
        file.write_all(b"]").expect("json close");
        file.sync_all().expect("sync json fixture");
    }

    fn replace_path_for_test(replacement: &std::path::Path, target: &std::path::Path) {
        #[cfg(not(target_os = "windows"))]
        fs::rename(replacement, target).expect("atomic replace");
        #[cfg(target_os = "windows")]
        {
            // Windows std::fs::rename 不覆盖已有目标；删除后换入仍会产生新的 file index。
            fs::remove_file(target).expect("remove replace target");
            fs::rename(replacement, target).expect("replace target");
        }
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("nomo-segmented-{name}-{nonce}"))
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}

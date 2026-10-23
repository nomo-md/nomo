use super::{
    markdown_session::{MarkdownDocumentSessionManager, MarkdownTransaction},
    TextDocumentError, TextDocumentResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{State, WebviewWindow};

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum MarkdownRequest {
    WritingStats {
        #[serde(flatten)]
        query: super::writing_stats::StatsQuery,
    },
    CancelWritingStats {
        request_id: Option<u64>,
    },
    Open {
        path: Option<String>,
        text: Option<String>,
        #[serde(default)]
        fresh: bool,
    },
    Recover {
        document_id: String,
    },
    Status {
        document_id: String,
    },
    ReadWindow {
        document_id: String,
        revision: u64,
        from_byte: u64,
        to_byte: u64,
    },
    Apply {
        transaction: MarkdownTransaction,
    },
    Undo {
        document_id: String,
        revision: u64,
    },
    Redo {
        document_id: String,
        revision: u64,
    },
    CaptureSnapshot {
        document_id: String,
    },
    CaptureOriginal {
        document_id: String,
    },
    ReadSnapshot {
        snapshot_id: String,
        from_byte: u64,
        to_byte: u64,
    },
    ReleaseSnapshot {
        snapshot_id: String,
    },
    SaveSnapshot {
        snapshot_id: String,
        path: String,
        overwrite_external: bool,
    },
    Close {
        document_id: String,
        discard: bool,
    },
    CloseMany {
        documents: Vec<CloseDocument>,
    },
    Detach {
        document_id: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseDocument {
    pub document_id: String,
    pub discard: bool,
}
fn value(result: impl Serialize) -> TextDocumentResult<Value> {
    serde_json::to_value(result)
        .map_err(|e| TextDocumentError::new("protocol-serialize", e.to_string()))
}

/// 窗口身份由 Tauri 注入，不能由前端指定或绕过会话所有权。
#[tauri::command]
pub(crate) async fn markdown_document_command(
    window: WebviewWindow,
    manager: State<'_, MarkdownDocumentSessionManager>,
    request: MarkdownRequest,
) -> TextDocumentResult<Value> {
    let owner = window.label().to_owned();
    let manager = manager.inner().clone();
    let request = match request {
        MarkdownRequest::WritingStats { query } => return value(manager.writing_stats(owner, query).await?),
        MarkdownRequest::CancelWritingStats { request_id } => {
            manager.cancel_writing_stats(&owner, request_id);
            return value(());
        }
        request => request,
    };
    tauri::async_runtime::spawn_blocking(move || match request {
        MarkdownRequest::WritingStats { .. } | MarkdownRequest::CancelWritingStats { .. } => unreachable!(),
        MarkdownRequest::Open { path, text, fresh } => {
            value(if fresh {
                manager.open_with_policy(&owner, path.map(Into::into), text, true)?
            } else {
                manager.open(&owner, path.map(Into::into), text)?
            })
        }
        MarkdownRequest::Recover { document_id } => value(manager.recover(&owner, &document_id)?),
        MarkdownRequest::Detach { document_id } => value(manager.detach(&owner, &document_id)?),
        MarkdownRequest::Status { document_id } => value(manager.status(&owner, &document_id)?),
        MarkdownRequest::ReadWindow {
            document_id,
            revision,
            from_byte,
            to_byte,
        } => value(manager.read_window(&owner, &document_id, revision, from_byte, to_byte)?),
        MarkdownRequest::Apply { transaction } => value(manager.apply(&owner, transaction)?),
        MarkdownRequest::Undo {
            document_id,
            revision,
        } => value(manager.history(&owner, &document_id, revision, false)?),
        MarkdownRequest::Redo {
            document_id,
            revision,
        } => value(manager.history(&owner, &document_id, revision, true)?),
        MarkdownRequest::CaptureSnapshot { document_id } => {
            value(manager.capture(&owner, &document_id)?)
        }
        MarkdownRequest::CaptureOriginal { document_id } => {
            value(manager.capture_original(&owner, &document_id)?)
        }
        MarkdownRequest::ReadSnapshot {
            snapshot_id,
            from_byte,
            to_byte,
        } => value(manager.read_snapshot(&owner, &snapshot_id, from_byte, to_byte)?),
        MarkdownRequest::ReleaseSnapshot { snapshot_id } => {
            value(manager.release(&owner, &snapshot_id)?)
        }
        MarkdownRequest::SaveSnapshot {
            snapshot_id,
            path,
            overwrite_external,
        } => value(manager.save(&owner, &snapshot_id, path.into(), overwrite_external)?),
        MarkdownRequest::Close {
            document_id,
            discard,
        } => value(manager.close(&owner, &document_id, discard)?),
        MarkdownRequest::CloseMany { documents } => value(manager.close_many(&owner, documents.into_iter().map(|document| (document.document_id, document.discard)).collect())?),
    })
    .await
    .map_err(|e| TextDocumentError::new("background-join-failed", e.to_string()))?
}

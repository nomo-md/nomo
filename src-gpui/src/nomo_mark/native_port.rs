//! 直接调用原版 MarkdownDocumentSessionManager；窗口范围始终由调用方提供。
use super::protocol::*;
use crate::text_document::{
    MarkdownSelection,
    markdown_session::{
        MarkdownDocumentSessionManager, MarkdownEdit, MarkdownStatus, MarkdownTransaction, SnapshotHandle,
    },
};
use std::{ops::Range, path::PathBuf};

#[derive(Clone)]
pub struct NativeWindowPort {
    pub manager: MarkdownDocumentSessionManager,
    pub owner: String,
    pub document_id: String,
}
fn convert_error(error: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(error.to_string())
}
impl NativeWindowPort {
    pub fn open(
        root: PathBuf,
        owner: String,
        path: Option<PathBuf>,
        text: Option<String>,
    ) -> Result<Self, ControlError> {
        let manager = MarkdownDocumentSessionManager::new(root).map_err(convert_error)?;
        Self::open_in(manager, owner, path, text)
    }
    /// 正式 Workspace 的所有标签共享服务及预算，只为新文档创建会话。
    pub(crate) fn open_in(
        manager: MarkdownDocumentSessionManager,
        owner: String,
        path: Option<PathBuf>,
        text: Option<String>,
    ) -> Result<Self, ControlError> {
        let status = manager.open(&owner, path, text).map_err(convert_error)?;
        Ok(Self {
            manager,
            owner,
            document_id: status.document_id,
        })
    }
    pub fn status(&self) -> Result<MarkdownStatus, ControlError> {
        self.manager
            .status(&self.owner, &self.document_id)
            .map_err(convert_error)
    }
    pub fn read_window(
        &self,
        revision: u64,
        range: Range<u64>,
    ) -> Result<DocumentWindow, ControlError> {
        let w = self
            .manager
            .read_window(
                &self.owner,
                &self.document_id,
                revision,
                range.start,
                range.end,
            )
            .map_err(convert_error)?;
        Ok(DocumentWindow {
            inline_templates: w.inline_templates,
            document_id: w.document_id,
            revision: w.revision,
            from_byte: w.from_byte,
            to_byte: w.to_byte,
            text: w.text,
        })
    }
    pub fn submit(&self, transaction: DocumentTransaction) -> Result<u64, ControlError> {
        self.submit_with_kind(transaction, false)
    }
    pub(crate) fn submit_derived(&self, transaction: DocumentTransaction) -> Result<u64, ControlError> {
        self.submit_with_kind(transaction, true)
    }
    fn submit_with_kind(&self, transaction: DocumentTransaction, derived: bool) -> Result<u64, ControlError> {
        let selection = |s: DocumentSelection| {
            Some(MarkdownSelection {
                anchor_byte: s.anchor_byte,
                head_byte: s.head_byte,
            })
        };
        let transaction = MarkdownTransaction {
            template_update: transaction.template_update,
            derived,
            document_id: self.document_id.clone(),
            base_revision: transaction.base_revision,
            operation_id: transaction.operation_id,
            history_group_id: transaction.history_group_id,
            edits: transaction
                .edits
                .into_iter()
                .map(|e| MarkdownEdit {
                    from_byte: e.from_byte,
                    to_byte: e.to_byte,
                    inserted_text: e.inserted_text,
                })
                .collect(),
            selection_before: selection(transaction.selection_before),
            selection_after: selection(transaction.selection_after),
        };
        self.manager
            .apply(&self.owner, transaction)
            .map(|status| status.revision)
            .map_err(convert_error)
    }
    pub fn history(
        &self,
        revision: u64,
        redo: bool,
    ) -> Result<(u64, Option<DocumentSelection>), ControlError> {
        let status = self
            .manager
            .history(&self.owner, &self.document_id, revision, redo)
            .map_err(convert_error)?;
        Ok((
            status.revision,
            status.selection.map(|s| DocumentSelection {
                anchor_byte: s.anchor_byte,
                head_byte: s.head_byte,
            }),
        ))
    }
    /// 同步捕获已完成目录派生的修订，避免后台保存与后续输入交错。
    pub(crate) fn capture_for_save(&self, revision: u64) -> Result<SnapshotHandle, ControlError> {
        let snapshot = self.manager.capture(&self.owner, &self.document_id).map_err(convert_error)?;
        if snapshot.revision != revision {
            self.manager.release(&self.owner, &snapshot.snapshot_id).map_err(convert_error)?;
            return Err(ControlError::Stale);
        }
        Ok(snapshot)
    }
    pub(crate) fn save_captured(&self, snapshot: SnapshotHandle, target: Option<PathBuf>, snapshot_before_save: bool) -> Result<u64, ControlError> {
        let saved = (|| {
            if let Some(path) = target {
                if snapshot_before_save && path.exists() {
                    crate::buffer::write_snapshot(&path).map_err(|e| ControlError::HostFailure(format!("创建保存前快照失败：{e}")))?;
                }
                self.manager.save(&self.owner, &snapshot.snapshot_id, path, false).map_err(convert_error)
            } else {
                self.manager.save_current(&self.owner, &snapshot.snapshot_id, snapshot_before_save).map_err(convert_error)
            }
        })();
        let released = self.manager.release(&self.owner, &snapshot.snapshot_id).map_err(convert_error);
        let saved = saved?;
        released?;
        Ok(saved.revision)
    }
    /// 保存始终使用完整会话快照，绝不使用控件局部 source。
    pub fn save_to(&self, target: PathBuf) -> Result<u64, ControlError> {
        let snapshot = self
            .manager
            .capture(&self.owner, &self.document_id)
            .map_err(convert_error)?;
        let saved = self
            .manager
            .save(&self.owner, &snapshot.snapshot_id, target, false);
        let released = self.manager.release(&self.owner, &snapshot.snapshot_id);
        let saved = saved.map_err(convert_error)?;
        released.map_err(convert_error)?;
        Ok(saved.revision)
    }
    pub(crate) fn save_current(&self, snapshot_before_save: bool) -> Result<u64, ControlError> {
        let snapshot = self.manager.capture(&self.owner, &self.document_id).map_err(convert_error)?;
        let saved = self.manager.save_current(&self.owner, &snapshot.snapshot_id, snapshot_before_save);
        let released = self.manager.release(&self.owner, &snapshot.snapshot_id);
        let saved = saved.map_err(convert_error)?;
        released.map_err(convert_error)?;
        Ok(saved.revision)
    }
    pub fn close(&self, discard: bool) -> Result<(), ControlError> {
        self.manager
            .close(&self.owner, &self.document_id, discard)
            .map_err(convert_error)
    }
    pub(crate) fn release_unmounted(&self)->Result<(),ControlError> {
        match self.manager.status(&self.owner,&self.document_id) {
            Ok(status) if status.dirty=>self.manager.detach(&self.owner,&self.document_id).map_err(convert_error),
            Ok(_)=>self.close(false),
            Err(error) if error.code=="document-closed"=>Ok(()),
            Err(error)=>Err(convert_error(error)),
        }
    }
    pub(crate) fn save_external_captured(&self, snapshot: SnapshotHandle, change: &crate::text_document::markdown_session::ExternalChange, snapshot_before_save: bool) -> Result<u64, ControlError> {
        let result = self.manager.save_after_external(&self.owner, &snapshot.snapshot_id, change, snapshot_before_save).map(|status| status.revision).map_err(convert_error);
        let released = self.manager.release(&self.owner, &snapshot.snapshot_id).map_err(convert_error);
        let revision = result?; released?; Ok(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nomo_mark::{
        commands::{self, EditIntent},
        host::WindowHostAdapter,
        parse::InlineFormat,
    };
    #[test]
    fn partial_window_edits_use_original_history_and_full_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "nomo-mark-contract-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let full = "before\r\n**中文**\nafter\r\n";
        let port = NativeWindowPort::open(
            root.join("sessions"),
            "gpui-test".into(),
            None,
            Some(full.into()),
        )
        .unwrap();
        let status = port.status().unwrap();
        let start = full.find("**").unwrap() as u64;
        let end = start + "**中文**".len() as u64;
        let mut host = WindowHostAdapter::new(
            port.read_window(status.revision, start..end).unwrap(),
            "piece",
            0,
            WindowContext::Complete,
        )
        .unwrap();
        host.mark_layout_ready(
            LayoutStamp {
                identity: host.parsed.identity.clone(),
                width: 400.0,
                font_generation: 0,
                resource_generation: 0,
                zoom: 1.0,
            },
            20.0,
        )
        .unwrap();
        host.selection = DocumentSelection::caret(start + 2);
        let result = commands::apply(
            &host.parsed,
            host.selection,
            EditIntent::ToggleInline(InlineFormat::Strong),
        )
        .unwrap();
        host.queue_edit(result.edits, result.selection, "format-1")
            .unwrap();
        let Some(ControlEvent::Submit(transaction)) = host.next_submission() else {
            panic!("submission")
        };
        let revision = port.submit(transaction.clone()).unwrap();
        // 丢失确认后用同一 operationId 重试，原版会话必须返回相同结果。
        assert_eq!(port.submit(transaction.clone()).unwrap(), revision);
        host.acknowledge(&transaction.operation_id, revision)
            .unwrap();
        let saved = root.join("result.md");
        port.save_to(saved.clone()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&saved).unwrap(),
            "before\r\n中文\nafter\r\n"
        );
        let (undo, _) = port.history(revision, false).unwrap();
        assert_eq!(port.read_window(undo, start..end).unwrap().text, "**中文**");
        assert!(port.close(false).is_err()); // dirty 不能由控件卸载静默丢弃。
        let (redo, _) = port.history(undo, true).unwrap();
        assert_eq!(port.read_window(redo, start..end - 4).unwrap().text, "中文");
        assert!(!port.status().unwrap().dirty); // redo 回到已保存快照。
        port.close(false).unwrap();
        drop(port);
        std::fs::remove_dir_all(root).unwrap();
    }
}

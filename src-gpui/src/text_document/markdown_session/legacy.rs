//! 旧草稿仅在用户恢复时转成原生会话；先发布完整会话，再记录已处理标记。
use super::*;
use crate::legacy_records::{Draft, DRAFT_BYTES};

impl MarkdownDocumentSessionManager {
    pub(super) fn adopt_legacy_under_path_lock(&self, owner: &str, draft: &Draft, as_copy: bool, cancelled: &dyn Fn() -> bool) -> TextDocumentResult<MarkdownStatus> {
        let _lease = SessionLease::acquire(&draft.root.join("gpui-legacy-records"))?;
        let target = self.root.join(&draft.id);
        if !target.exists() {
            if draft.handled() { return Err(error("legacy-handled", "旧草稿已处理，请重新打开恢复列表")); }
            fs::create_dir_all(&self.root)?;
            let staging = tempfile::Builder::new().prefix("legacy-staging-").tempdir_in(&self.root)?;
            let raw_path = staging.path().join("raw");
            let mut raw = File::create(&raw_path)?;
            // 原文件只作为保存点读取；旧版没有可靠的文件身份，不将当前磁盘身份当成旧草稿的基线。
            if !as_copy && let Some(path) = &draft.path {
                match File::open(path) {
                    Ok(mut source) => {
                        let before = BaselineIdentity::read_file(path, &source)?;
                        if before.physical_len > DRAFT_BYTES { return Err(error("legacy-budget", "磁盘文件超过 256 MiB，请使用另存草稿恢复")); }
                        let length = crate::legacy_records::copy_draft(&mut source, &mut raw, cancelled).map_err(|cause|error("legacy-baseline",cause))?;
                        if length != before.physical_len || before != BaselineIdentity::read_file(path, &source)? || before != BaselineIdentity::read(path)? {
                            return Err(error("source-changed", "磁盘文件在恢复时发生变化，旧草稿已保留"));
                        }
                    }
                    Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {},
                    Err(cause) => return Err(error("legacy-baseline", format!("无法读取磁盘保存点，请使用另存草稿：{cause}"))),
                }
            }
            raw.sync_all()?;
            drop(raw);
            let map = markdown_codec::prepare_working_source(&raw_path, &staging.path().join("working"))?;
            let added_path = staging.path().join("added");
            let mut added = File::create(&added_path)?;
            let length = draft.copy_to(&mut added, cancelled).map_err(|cause| error("legacy-draft", cause))?;
            added.sync_all()?;
            drop(added);
            // 草稿一直是无编码转换的 UTF-8 字符串，保留前导 BOM 字符、CRLF 和尾部空白。
            markdown_codec::write_encoded(File::open(&added_path)?, &mut std::io::sink(), MarkdownEncoding::Utf8)?;
            let mut history = DiskHistory::open(&staging.path().join("history"))?;
            let event = history.append(&HistoryRecord {
                previous_event: None, group: "legacy-import".into(), parent_undo: None, previous_redo: None,
                edits: vec![TreeEdit { from: 0, to: map.utf8_len, pieces: if length == 0 {Vec::new()} else {vec![Piece {source: PieceSource::Added {offset: 0, length}, line_breaks: None}]} }],
                inverse: Vec::new(), group_entries: Vec::new(), selection_before: None, selection_after: None, templates_before: None, templates_after: None,
            })?;
            drop(history);
            let manifest = Manifest { legacy_unconfirmed: !as_copy && draft.path.is_some(), version: 1, document_id: draft.id.clone(), path: if as_copy {None} else {draft.path.clone()}, identity: None,
                source_map: map, revision: 1, event: Some(event), undo: None, redo: None, selection: None, saved_tree: None, inline_templates: Vec::new(), derived: false };
            let bytes = serde_json::to_vec(&manifest).map_err(|cause| error("legacy-manifest", cause.to_string()))?;
            crate::file_system::write_file_atomically(&staging.path().join("manifest.json"), &bytes).map_err(|cause| error("legacy-manifest", cause))?;
            if cancelled() {return Err(error("request-cancelled", "旧草稿恢复已取消，原记录仍保留"));}
            fs::rename(staging.path(), &target)?;
        }
        // 即使挂载失败或窗口在异步完成前关闭，已发布的原生会话仍可在下次启动恢复。
        draft.mark_handled().map_err(|cause| error("legacy-receipt", cause))?;
        self.recover_under_path_lock(owner, &draft.id, as_copy)
    }

    /// 旧阅读记录保存的是源码行号；后台顺序扫描，不把整篇正文带入布局状态。
    pub(crate) fn legacy_line_byte(&self, owner: &str, document_id: &str, line: u64, cancelled: impl Fn() -> bool) -> TextDocumentResult<u64> {
        let snapshot = { let session = self.session(document_id)?; let session = lock(&session)?; session.owner(owner)?; session.current.clone() };
        let mut reader = snapshot.reader();
        let mut buffer = [0u8; 64 * 1024];
        let mut offset = 0;
        let mut current = 1;
        let mut previous_cr = false;
        loop {
            if cancelled() { return Err(error("request-cancelled", "阅读位置恢复已取消")); }
            let read = reader.read(&mut buffer)?;
            if read == 0 { return Ok(offset); }
            for &byte in &buffer[..read] {
                // CRLF 算一行，并把位置放在整个换行符之后。
                if current >= line && !(previous_cr && byte == b'\n') { return Ok(offset); }
                offset += 1;
                if byte == b'\r' || byte == b'\n' && !previous_cr { current += 1; }
                previous_cr = byte == b'\r';
            }
        }
    }
}

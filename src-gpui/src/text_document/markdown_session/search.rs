//! 搜索快照和批量事务接入。扫描、候选树、dirty 比较均在会话锁外进行。
#[cfg(test)]
mod tests;
use super::*;
use crate::text_document::markdown_search::{
    check_cancel, Checkpoint, ProgressSink, ReplaceQuery, ReplaceResult, SearchPage, SearchQuery,
    SearchTask, TaskIdentity, TaskProgress,
};

const HISTORY_BATCH_ITEMS: usize = 4096;
const HISTORY_BATCH_BYTES: usize = 1024 * 1024;

pub(in crate::text_document) struct PreparedReplacement {
    operation_id: String,
    tree: PieceTree,
    head: Option<u64>,
    count: u64,
    changed: u64,
    selection: Option<MarkdownSelection>,
    anchors: Vec<u64>,
    saved_key: Option<String>,
    dirty: bool,
    inline_templates: Vec<InlineTemplate>,
}

struct Batch {
    group: String,
    head: Option<u64>,
    undo: Option<u64>,
    redo: Option<u64>,
    forward: Vec<TreeEdit>,
    inverse: Vec<TreeEdit>,
    bytes: usize,
    selection_before: Option<MarkdownSelection>,
    templates_before: Vec<InlineTemplate>,
    templates: Vec<InlineTemplate>,
}
impl Batch {
    fn push(
        &mut self,
        manager: &MarkdownDocumentSessionManager,
        task: &SearchTask,
        forward: TreeEdit,
        inverse: TreeEdit,
    ) -> TextDocumentResult<()> {
        // 本路径每个基本动作至多引用一个片段。保守估算含 JSON 字段和 u64 的最大编码长度。
        let size = 256 + (forward.pieces.len() + inverse.pieces.len()) * 160;
        if !self.forward.is_empty()
            && (self.forward.len() >= HISTORY_BATCH_ITEMS
                || self.bytes + size >= HISTORY_BATCH_BYTES)
        {
            self.flush(manager, task, None)?;
        }
        markdown_templates::map_templates(&mut self.templates,
            [(forward.from, forward.to, forward.pieces.iter().map(Piece::len).sum())], false);
        self.bytes += size;
        self.forward.push(forward);
        self.inverse.push(inverse);
        Ok(())
    }
    fn flush(
        &mut self,
        manager: &MarkdownDocumentSessionManager,
        task: &SearchTask,
        selection_after: Option<MarkdownSelection>,
    ) -> TextDocumentResult<()> {
        if self.forward.is_empty() {
            return Ok(());
        }
        check_cancel(&task.cancel)?;
        let handle = manager.session(&task.query.document_id)?;
        let mut session = lock(&handle)?;
        session.owner(&task.owner)?;
        if session.current.revision != task.query.base_revision {
            return Err(TextDocumentError::revision_conflict(
                task.query.base_revision,
                session.current.revision,
            ));
        }
        self.inverse.reverse();
        let record = HistoryRecord {
            templates_before: (self.templates_before != self.templates).then(|| self.templates_before.clone()),
            templates_after: (self.templates_before != self.templates).then(|| self.templates.clone()),
            previous_event: self.head,
            group: self.group.clone(),
            parent_undo: self.undo,
            previous_redo: self.redo,
            edits: std::mem::take(&mut self.forward),
            inverse: std::mem::take(&mut self.inverse),
            group_entries: vec![],
            selection_before: self.selection_before.take(),
            selection_after,
        };
        // 正文批次原预算不变；稀疏来源元数据另计，历史热缓存仍按完整序列化字节计量。
        let metadata_bytes = serde_json::to_vec(&(&record.templates_before, &record.templates_after)).unwrap().len() + 128;
        debug_assert!(serde_json::to_vec(&record).unwrap().len() <= HISTORY_BATCH_BYTES + metadata_bytes);
        self.templates_before = self.templates.clone();
        let offset = session.history.append_staged(&record)?;
        self.head = Some(offset);
        self.undo = Some(offset);
        self.bytes = 0;
        Ok(())
    }
}

struct MappedPoint {
    original: u64,
    association: i64,
    mapped: Option<u64>,
}
impl MappedPoint {
    fn apply(&mut self, from: u64, to: u64, inserted: u64, delta: i64) {
        if self.mapped.is_some() {
            return;
        }
        if self.original < from {
            self.mapped = Some(self.original.saturating_add_signed(delta));
        } else if self.original < to {
            self.mapped = Some(
                from.saturating_add_signed(delta) + if self.association > 0 { inserted } else { 0 },
            );
        }
    }
    fn finish(&self, delta: i64) -> u64 {
        self.mapped
            .unwrap_or_else(|| self.original.saturating_add_signed(delta))
    }
}

impl MarkdownDocumentSessionManager {
    fn search_task(
        &self,
        owner: &str,
        query: SearchQuery,
        sink: Option<ProgressSink>,
    ) -> TextDocumentResult<Arc<SearchTask>> {
        let handle = self.session(&query.document_id)?;
        let snapshot = {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if session.current.revision != query.base_revision {
                return Err(TextDocumentError::revision_conflict(
                    query.base_revision,
                    session.current.revision,
                ));
            }
            session.current.clone()
        };
        self.search
            .create(owner.into(), query, snapshot, Box::new(handle), sink)
    }
    pub(crate) fn start_search(
        &self,
        owner: &str,
        query: SearchQuery,
        sink: Option<ProgressSink>,
    ) -> TextDocumentResult<TaskIdentity> {
        let task = self.search_task(owner, query, sink)?;
        let identity = task.identity.clone();
        self.search.start_search(task);
        Ok(identity)
    }
    pub(crate) fn search_page(
        &self,
        owner: &str,
        task_id: &str,
        from_index: u64,
        limit: usize,
    ) -> TextDocumentResult<SearchPage> {
        let page = self.search.page(owner, task_id, from_index, limit)?;
        let status = self.status(owner, &page.identity.document_id)?;
        if status.revision != page.identity.base_revision {
            return Err(TextDocumentError::revision_conflict(
                page.identity.base_revision,
                status.revision,
            ));
        }
        Ok(page)
    }
    pub(crate) fn search_progress(
        &self,
        owner: &str,
        task_id: &str,
    ) -> TextDocumentResult<TaskProgress> {
        let task = self.search.get(owner, task_id)?;
        let progress = lock(&task.data)?.progress.clone();
        Ok(progress)
    }
    pub(crate) fn release_search(&self, owner: &str, task_id: &str) {
        self.search.release(owner, task_id);
    }
    pub(crate) fn prepare_replace_all(
        &self,
        owner: &str,
        request: ReplaceQuery,
        sink: Option<ProgressSink>,
    ) -> TextDocumentResult<TaskIdentity> {
        if request.operation_id.is_empty() || request.anchors.len() > 64 {
            return Err(error("invalid-transaction", "替换操作身份或阅读锚点无效"));
        }
        let task = self.search_task(owner, request.search.clone(), sink)?;
        if let Err(error) = validate_selection(&task.snapshot, request.selection.as_ref()) {
            self.search.cancel(owner, &task.identity.task_id);
            return Err(error);
        }
        if request
            .anchors
            .iter()
            .any(|byte| *byte > task.snapshot.len())
        {
            self.search.cancel(owner, &task.identity.task_id);
            return Err(error("invalid-selection", "阅读位置超出文档范围"));
        }
        let identity = task.identity.clone();
        let manager = self.clone();
        self.search
            .start(task, move |task| manager.build_replacement(&task, request));
        Ok(identity)
    }
    fn build_replacement(
        &self,
        task: &SearchTask,
        request: ReplaceQuery,
    ) -> TextDocumentResult<()> {
        let handle = self.session(&task.query.document_id)?;
        let (mut batch, history_file) = {
            let session = lock(&handle)?;
            session.owner(&task.owner)?;
            if session.current.revision != task.query.base_revision {
                return Err(TextDocumentError::revision_conflict(
                    task.query.base_revision,
                    session.current.revision,
                ));
            }
            (
                Batch {
                    group: id("replace-all"),
                    head: session.manifest.event,
                    undo: session.manifest.undo,
                    redo: session.manifest.redo,
                    forward: vec![],
                    inverse: vec![],
                    bytes: 0,
                    selection_before: request.selection.clone(),
                    templates_before: session.manifest.inline_templates.clone(),
                    templates: session.manifest.inline_templates.clone(),
                },
                session.history.durability_handle()?,
            )
        };
        let mut points: Vec<MappedPoint> = vec![];
        if let Some(selection) = &request.selection {
            let forward = selection.anchor_byte < selection.head_byte;
            points.push(MappedPoint {
                original: selection.anchor_byte,
                association: if forward { -1 } else { 1 },
                mapped: None,
            });
            points.push(MappedPoint {
                original: selection.head_byte,
                association: if selection.anchor_byte > selection.head_byte {
                    -1
                } else {
                    1
                },
                mapped: None,
            });
        }
        points.extend(request.anchors.iter().map(|byte| MappedPoint {
            original: *byte,
            association: 1,
            mapped: None,
        }));
        let mut tree = task.snapshot.tree.clone();
        let mut replacement_piece: Option<Piece> = None;
        let replacement = request.replacement.as_bytes();
        let mut delta = 0_i64;
        let mut changed = 0;
        let scanned = task.matcher.scan(
            &task.snapshot,
            Checkpoint::default(),
            &task.cancel,
            false,
            |matched, actual| {
                if actual.as_bytes() == replacement {
                    return Ok(true);
                }
                let (from, to) = matched.raw();
                for point in &mut points {
                    point.apply(from, to, replacement.len() as u64, delta);
                }
                if replacement_piece.is_none() && !replacement.is_empty() {
                    replacement_piece = Some(Piece::added(
                        task.snapshot.added.append(replacement)?,
                        replacement.len() as u64,
                        replacement.iter().filter(|byte| **byte == b'\n').count() as u64,
                    ));
                }
                // 基本动作按片段从右往左删除，再插入共享替换片段。无需展开长范围的片段数组；
                // CRLF 中点只存在于未发布候选树中，最终正文严格等于字面量替换的结果。
                let mut end = to;
                while end > from {
                    check_cancel(&task.cancel)?;
                    let (piece, relative) = task
                        .snapshot
                        .tree
                        .piece_at(end - 1)
                        .ok_or_else(|| error("invalid-piece-tree", "替换片段缺失"))?;
                    let begin = from.max(end - relative - 1);
                    let length = end - begin;
                    let old = match piece.source {
                        PieceSource::Original { offset, .. } => Piece {
                            source: PieceSource::Original {
                                offset: offset + relative + 1 - length,
                                length,
                            },
                            line_breaks: None,
                        },
                        PieceSource::Added { offset, .. } => Piece {
                            source: PieceSource::Added {
                                offset: offset + relative + 1 - length,
                                length,
                            },
                            line_breaks: None,
                        },
                    };
                    let at = begin.saturating_add_signed(delta);
                    // 最左片段的删除与插入合成一个历史动作；常见单片段命中只需一条。
                    let inserted = if begin == from { replacement_piece.clone() } else { None };
                    let forward = TreeEdit {
                        from: at,
                        to: end.saturating_add_signed(delta),
                        pieces: inserted.iter().cloned().collect(),
                    };
                    let inverse = TreeEdit {
                        from: at,
                        to: at + inserted.as_ref().map_or(0, Piece::len),
                        pieces: vec![old],
                    };
                    tree = tree.replace_range(forward.from, forward.to, inserted)?;
                    batch.push(self, task, forward, inverse)?;
                    end = begin;
                }
                delta += replacement.len() as i64 - (to - from) as i64;
                changed += 1;
                Ok(true)
            },
            task.progress_reporter(),
        )?;
        check_cancel(&task.cancel)?;
        let mut candidate = task.snapshot.clone();
        candidate.tree = tree.clone();
        let normalize = |point: &MappedPoint| -> TextDocumentResult<u64> {
            let mut byte = point.finish(delta);
            while byte < candidate.len() && !boundary(&candidate, byte)? {
                byte += 1;
            }
            Ok(byte)
        };
        let selection = if request.selection.is_some() {
            Some(MarkdownSelection {
                anchor_byte: normalize(&points[0])?,
                head_byte: normalize(&points[1])?,
            })
        } else {
            None
        };
        let anchors = points
            .iter()
            .skip(if selection.is_some() { 2 } else { 0 })
            .map(normalize)
            .collect::<TextDocumentResult<Vec<_>>>()?;
        batch.flush(self, task, selection.clone())?;
        // 同步准备好的内容和日志时不持有 Session 锁，普通输入仍可继续。
        if changed > 0 {
            task.snapshot.added.sync()?;
            history_file.sync_all()?;
        }
        let (saved, saved_key) = {
            let session = lock(&handle)?;
            (session.saved.clone(), session.manifest.saved_tree.clone())
        };
        let dirty = changed > 0 && !snapshots_equal_cancelled(&candidate, &saved, &task.cancel)?;
        check_cancel(&task.cancel)?;
        lock(&task.data)?.prepared = Some(PreparedReplacement {
            operation_id: request.operation_id,
            tree,
            head: batch.head,
            count: scanned.count,
            changed,
            selection,
            anchors,
            saved_key,
            dirty,
            inline_templates: batch.templates,
        });
        task.emit("prepared", task.snapshot.len(), scanned.count, None);
        Ok(())
    }
    pub(crate) fn commit_replace_all(
        &self,
        owner: &str,
        document_id: &str,
        task_id: &str,
        operation_id: &str,
    ) -> TextDocumentResult<ReplaceResult> {
        let handle = self.session(document_id)?;
        {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if let Some((_, result)) = session.search_ack.iter().find(|(id, _)| id == operation_id)
            {
                if result.identity.task_id != task_id {
                    return Err(error("operation-conflict", "同一操作身份用于不同替换"));
                }
                return Ok(result.clone());
            }
        }
        let task = self.search.get(owner, task_id)?;
        if task.query.document_id != document_id {
            return Err(error("operation-conflict", "替换目标文档不一致"));
        }
        let mut data = lock(&task.data)?;
        if let Some(result) = &data.completed {
            if data.completed_operation.as_deref() != Some(operation_id) {
                return Err(error("operation-conflict", "替换操作身份不一致"));
            }
            return Ok(result.clone());
        }
        let prepared = data
            .prepared
            .as_ref()
            .ok_or_else(|| error("search-not-ready", "替换尚未准备完成"))?;
        if prepared.operation_id != operation_id {
            return Err(error("operation-conflict", "替换操作身份不一致"));
        }
        let mut candidate = task.snapshot.clone();
        candidate.tree = prepared.tree.clone();
        loop {
            check_cancel(&task.cancel)?;
            let (saved, saved_key) = {
                let session = lock(&handle)?;
                session.owner(owner)?;
                if session.current.revision != task.query.base_revision {
                    return Err(TextDocumentError::revision_conflict(
                        task.query.base_revision,
                        session.current.revision,
                    ));
                }
                (session.saved.clone(), session.manifest.saved_tree.clone())
            };
            let dirty = if saved_key == prepared.saved_key {
                prepared.dirty
            } else if prepared.changed > 0 {
                !snapshots_equal_cancelled(&candidate, &saved, &task.cancel)?
            } else {
                false
            };
            check_cancel(&task.cancel)?;
            let mut session = lock(&handle)?;
            session.owner(owner)?;
            if session.current.revision != task.query.base_revision {
                return Err(TextDocumentError::revision_conflict(
                    task.query.base_revision,
                    session.current.revision,
                ));
            }
            if saved_key != session.manifest.saved_tree {
                continue;
            }
            check_cancel(&task.cancel)?;
            // 从这里进入不可取消的发布点。失败只留下不可达日志，原 manifest 和正文保持不变。
            if prepared.changed > 0 {
                let mut manifest = session.manifest.clone();
                manifest.revision += 1;
                manifest.event = prepared.head;
                manifest.undo = prepared.head;
                manifest.redo = None;
                manifest.selection = prepared.selection.clone();
                manifest.inline_templates = prepared.inline_templates.clone();
                session.persist(&manifest)?;
                session.current.tree = prepared.tree.clone();
                session.current.revision = manifest.revision;
                session.manifest = manifest;
                session.dirty = dirty;
                session.active_group = None;
            }
            let result = ReplaceResult {
                identity: task.identity.clone(),
                status: session.status(),
                count: prepared.count,
                changed_count: prepared.changed,
                anchors: prepared.anchors.clone(),
            };
            session
                .search_ack
                .push_back((operation_id.into(), result.clone()));
            while session.search_ack.len() > 128 {
                session.search_ack.pop_front();
            }
            data.completed = Some(result.clone());
            data.completed_operation = Some(operation_id.into());
            data.prepared = None;
            if result.changed_count > 0 {
                self.structure.edited(document_id, result.status.revision, &[]);
            }
            drop(session);
            drop(data);
            if result.changed_count > 0 {
                self.writing_stats.invalidate(document_id, false);
                self.search.invalidate(document_id, Some(task_id));
            }
            task.emit("completed", task.snapshot.len(), result.count, None);
            return Ok(result);
        }
    }
}

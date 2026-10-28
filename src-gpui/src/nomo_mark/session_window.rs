//! GPUI 的原版窗口会话适配。沿用原生索引分页；原文分片仅探测边界，不扫描全文建立索引。
//! 分组容量与 Web 宿主共用 viewportPolicy.json；全局服务始终调用原版会话。
use super::{native_port::NativeWindowPort, protocol::*};
use crate::text_document::{StructureIdentity, StructureQuery};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportPolicy {
    pub base_screens: usize,
    pub forward_screens: usize,
    pub retain_screens: usize,
    pub quiet_ms: u64,
    pub batch_ms: u64,
    pub group_bytes: u64,
    pub group_blocks: usize,
}
pub fn policy() -> &'static ViewportPolicy {
    static POLICY: std::sync::OnceLock<ViewportPolicy> = std::sync::OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../resources/viewportPolicy.json"
        ))
        .expect("shared viewport policy")
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Block {
    from_byte: u64,
    to_byte: u64,
    mode: String,
    kind: String,
    semantic_complete: bool,
    references: Vec<String>,
}
pub struct LoadedWindow {
    pub input: DocumentWindow,
    pub context: WindowContext,
    pub total_bytes: u64,
    pub index: StructureIdentity,
}
#[derive(Clone, Default)]
pub struct WindowRequest {
    cancelled: Arc<AtomicBool>,
}
impl WindowRequest {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub(crate) fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::Acquire) }
    pub(crate) fn check(&self) -> Result<(), ControlError> {
        if self.is_cancelled() {
            Err(ControlError::Stale)
        } else {
            Ok(())
        }
    }
}
fn host_error(error: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(error.to_string())
}

impl NativeWindowPort {
    /// 读取索引分页，给原文结构补充分片几何；每个切分点至多探测一个 UTF-8/CRLF 边界。
    pub fn window_groups(&self, revision: u64, retained: &[(u64, u64)], request: &WindowRequest) -> Result<Vec<super::viewport_model::Group>, ControlError> {
        let index = self.ready_index(revision, request)?;
        let total = self.status()?.byte_length;
        if total == 0 { return Ok(Vec::new()); }
        let mut cursor = None;
        let mut blocks = Vec::new();
        let mut bytes = 0usize;
        loop {
            request.check()?;
            let page = self.manager.query_structure(&self.owner, index.clone(), StructureQuery::Blocks { from_byte: 0, to_byte: total, cursor }).map_err(host_error)?;
            if page.fragment.is_some() {
                return Err(ControlError::NeedsContext("结构元数据需要续取，保留当前可编辑窗口".into()));
            }
            for value in page.items {
                let block: super::viewport_model::BlockSummary = serde_json::from_value(value).map_err(host_error)?;
                bytes += std::mem::size_of_val(&block) + block.id.len() + block.kind.len() + block.mode.len();
                if bytes > 32 * 1024 * 1024 { return Err(ControlError::NeedsContext("结构元数据超过宿主临时预算，保留当前窗口并等待分页几何适配".into())); }
                blocks.push(block);
            }
            cursor = page.next;
            if cursor.is_none() { break; }
        }
        request.check()?;
        if self.status()?.revision != revision { return Err(ControlError::Stale); }
        let mut bounded = Vec::new();
        let chunk_bytes = policy().group_bytes.saturating_sub(8).max(8);
        let mut source_bytes = 0usize;
        for block in blocks {
            if block.mode == "semantic" && block.semantic_complete && block.kind != "native_region" {
                source_bytes += std::mem::size_of_val(&block) + block.id.len() + block.kind.len() + block.mode.len();
                if source_bytes > 32 * 1024 * 1024 {
                    return Err(ControlError::NeedsContext("窗口元数据超过宿主预算".into()));
                }
                bounded.push(block);
                continue;
            }
            let mut from = block.from_byte;
            while from < block.to_byte {
                request.check()?;
                let mut nominal = (block.from_byte + ((from - block.from_byte) / chunk_bytes + 1) * chunk_bytes).min(block.to_byte);
                let mut to = self.source_boundary(revision, nominal)?;
                if to <= from {
                    nominal = nominal.saturating_add(chunk_bytes).min(block.to_byte);
                    to = self.source_boundary(revision, nominal)?;
                }
                for &(start, end) in retained {
                    if end.saturating_sub(start) > policy().group_bytes { continue; }
                    for boundary in [start, end] {
                        if boundary > from && boundary < to { to = boundary; }
                    }
                }
                if to <= from { return Err(ControlError::InvalidRange); }
                let id = format!("{}:source:{from}", block.id);
                source_bytes += std::mem::size_of::<super::viewport_model::BlockSummary>() + id.len() + "source_window".len() + 3;
                if source_bytes > 32 * 1024 * 1024 {
                    return Err(ControlError::NeedsContext("原文窗口元数据超过宿主预算".into()));
                }
                let span = block.to_byte - block.from_byte;
                let lines = block.to_line.saturating_sub(block.from_line);
                bounded.push(super::viewport_model::BlockSummary {
                    id, from_byte: from, to_byte: to,
                    from_line: block.from_line + lines.saturating_mul(from - block.from_byte) / span,
                    to_line: block.from_line + lines.saturating_mul(to - block.from_byte) / span,
                    mode: "raw".into(), kind: "source_window".into(), semantic_complete: false,
                    layout_units: 1,
                });
                from = to;
            }
        }
        super::viewport_model::groups(bounded, retained)
    }
    /// 仅探测切分点所在的 UTF-8/CRLF 边界，不读取或扫描整块正文。
    fn source_boundary(&self, revision: u64, byte: u64) -> Result<u64, ControlError> {
        Ok(self.read_window(revision, byte..byte)?.from_byte)
    }
    fn load_source_group(
        &self, revision: u64, byte: u64, block: Range<u64>, limit: Option<u64>,
        request: &WindowRequest, index: StructureIdentity,
    ) -> Result<LoadedWindow, ControlError> {
        request.check()?;
        let chunk_bytes = policy().group_bytes.saturating_sub(8).max(8);
        let (from, to) = if let Some(to) = limit {
            (byte, to)
        } else {
            let mut nominal = block.start + (byte.saturating_sub(block.start) / chunk_bytes) * chunk_bytes;
            let mut from = self.source_boundary(revision, nominal)?;
            let mut to = self.source_boundary(revision, nominal.saturating_add(chunk_bytes).min(block.end))?;
            if to <= byte && to < block.end {
                nominal = nominal.saturating_add(chunk_bytes);
                from = to;
                to = self.source_boundary(revision, nominal.saturating_add(chunk_bytes).min(block.end))?;
            }
            (from, to)
        };
        if from >= to || from < block.start || to > block.end || to - from > policy().group_bytes {
            return Err(ControlError::InvalidRange);
        }
        self.source_window(revision, from..to, request, index)
    }
    pub(crate) fn load_source_range(&self, revision: u64, range: Range<u64>, request: &WindowRequest) -> Result<LoadedWindow, ControlError> {
        let index = self.ready_index(revision, request)?;
        self.source_window(revision, range, request, index)
    }
    fn source_window(&self, revision: u64, range: Range<u64>, request: &WindowRequest, index: StructureIdentity) -> Result<LoadedWindow, ControlError> {
        request.check()?;
        if range.start >= range.end || range.end - range.start > policy().group_bytes { return Err(ControlError::InvalidRange); }
        let (from, to) = (range.start, range.end);
        let input = self.read_window(revision, range)?;
        if input.from_byte != from || input.to_byte != to { return Err(ControlError::InvalidUtf8Boundary); }
        request.check()?;
        let status = self.status()?;
        if status.revision != revision { return Err(ControlError::Stale); }
        Ok(LoadedWindow {
            input, context: WindowContext::Resolved(ParseContext { source_only: true, ..Default::default() }),
            total_bytes: status.byte_length, index,
        })
    }
    pub(crate) fn ready_index(
        &self,
        revision: u64,
        request: &WindowRequest,
    ) -> Result<StructureIdentity, ControlError> {
        self.wait_index(revision, None, request)
    }
    fn wait_index(
        &self,
        revision: u64,
        through_byte: Option<u64>,
        request: &WindowRequest,
    ) -> Result<StructureIdentity, ControlError> {
        request.check()?;
        let identity = self
            .manager
            .start_structure(&self.owner, &self.document_id, revision)
            .map_err(host_error)?;
        let started = Instant::now();
        loop {
            request.check()?;
            let progress = self
                .manager
                .query_structure(&self.owner, identity.clone(), StructureQuery::Progress)
                .map_err(host_error)?;
            match progress.state.as_str() {
                "ready" => return Ok(identity),
                "building" if through_byte.is_some_and(|through| {
                    progress.indexed_bytes >= through.min(progress.total_bytes)
                }) => return Ok(identity),
                "failed" | "cancelled" => {
                    return Err(host_error(progress.failure.unwrap_or(progress.state)));
                }
                _ => {}
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err(ControlError::NeedsContext(
                    "原版结构索引仍在准备，可重试当前定位".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// 首屏只等待首个有界范围发布；全局索引仍由原会话继续构建。
    pub(crate) fn load_initial_group(
        &self,
        revision: u64,
        request: &WindowRequest,
    ) -> Result<LoadedWindow, ControlError> {
        let index = self.wait_index(revision, Some(policy().group_bytes), request)?;
        self.group_from_index(revision, 0, None, request, index)
    }
    /// byte/limit 属于宿主需求；语义窗口保留完整结构，超大结构返回有界原文窗口。
    pub fn load_group(
        &self,
        revision: u64,
        byte: u64,
        limit: Option<u64>,
        request: &WindowRequest,
    ) -> Result<LoadedWindow, ControlError> {
        let index = self.ready_index(revision, request)?;
        self.group_from_index(revision, byte, limit, request, index)
    }
    fn group_from_index(
        &self,
        revision: u64,
        byte: u64,
        limit: Option<u64>,
        request: &WindowRequest,
        index: StructureIdentity,
    ) -> Result<LoadedWindow, ControlError> {
        request.check()?;
        let status = self.status()?;
        if status.revision != revision {
            return Err(ControlError::Stale);
        }
        if status.byte_length == 0 {
            return Ok(LoadedWindow {
                input: self.read_window(revision, 0..0)?,
                context: WindowContext::Complete,
                total_bytes: 0,
                index,
            });
        }
        let byte = byte.min(status.byte_length - 1);
        let response = self
            .manager
            .query_structure(
                &self.owner,
                index.clone(),
                StructureQuery::Blocks {
                    from_byte: byte,
                    to_byte: (byte + policy().group_bytes).min(status.byte_length),
                    cursor: None,
                },
            )
            .map_err(host_error)?;
        if response.fragment.is_some() {
            return Err(ControlError::NeedsContext(
                "原版结构项需要分页续取，当前窗口暂缓挂载".into(),
            ));
        }
        let blocks = response
            .items
            .into_iter()
            .map(serde_json::from_value::<Block>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(host_error)?;
        let first = blocks
            .first()
            .ok_or_else(|| ControlError::NeedsContext("目标结构尚未发布".into()))?;
        if first.mode != "semantic" || !first.semantic_complete || first.kind == "native_region" {
            return self.load_source_group(revision, byte, first.from_byte..first.to_byte, limit, request, index);
        }
        let from = first.from_byte;
        let mut to = first.to_byte;
        let mut references = BTreeSet::new();
        for (i, block) in blocks.iter().enumerate() {
            if i > 0
                && (i >= policy().group_blocks
                    || block.mode != "semantic"
                    || !block.semantic_complete
                    || block.to_byte - from > policy().group_bytes
                    || limit.is_some_and(|limit| block.to_byte > limit))
            {
                break;
            }
            to = block.to_byte;
            references.extend(block.references.iter().cloned());
        }
        if limit.is_some_and(|limit| to > limit) {
            return Err(ControlError::NeedsContext(
                "目标与已挂载窗口重叠，需重新定位完整边界".into(),
            ));
        }
        if to - from > 1_048_576 {
            return Err(ControlError::NeedsContext(
                "完整结构超过原版单次窗口限制".into(),
            ));
        }
        let context = ParseContext {
            structural_boundary: true,
            reference_keys: references.into_iter().collect(),
            ..Default::default()
        };
        // 只查当前窗口用到的定义；不复制全局引用表。
        let context = self.reference_context(revision, index.clone(), context, request)?;
        request.check()?;
        let input = self.read_window(revision, from..to)?;
        Ok(LoadedWindow {
            input,
            context: WindowContext::Resolved(context),
            total_bytes: status.byte_length,
            index,
        })
    }
    /// 只补齐当前窗口的引用元数据；不等待完整索引，也不重新读取正文。
    pub(crate) fn refresh_references(&self, revision: u64, context: ParseContext, request: &WindowRequest) -> Result<ParseContext, ControlError> {
        request.check()?;
        let index = self.manager.start_structure(&self.owner, &self.document_id, revision).map_err(host_error)?;
        self.reference_context(revision, index, context, request)
    }
    fn reference_context(&self, revision: u64, index: StructureIdentity, mut context: ParseContext, request: &WindowRequest) -> Result<ParseContext, ControlError> {
        context.references.clear();
        context.pending_references = false;
        for batch in context.reference_keys.chunks(64) {
            request.check()?;
            let response = self
                .manager
                .query_structure(
                    &self.owner,
                    index.clone(),
                    StructureQuery::References {
                        keys: batch.to_vec(),
                    },
                )
                .map_err(host_error)?;
            if matches!(response.state.as_str(), "failed" | "cancelled") {
                return Err(host_error(response.failure.unwrap_or(response.state)));
            }
            // 未扫描到定义不等于不存在；只暂停该引用的交互，不降级整个窗口。
            context.pending_references |= response.state != "ready" && response.items.len() < batch.len();
            for item in response.items {
                let key = item["key"].as_str().ok_or(ControlError::InvalidRange)?;
                let target = &item["definition"];
                context.references.insert(
                    key.to_string(),
                    ReferenceTarget {
                        destination: target["href"]
                            .as_str()
                            .ok_or(ControlError::InvalidRange)?
                            .into(),
                        title: target["title"].as_str().unwrap_or_default().into(),
                    },
                );
            }
        }
        request.check()?;
        if self.status()?.revision != revision { return Err(ControlError::Stale); }
        Ok(context)
    }
    pub fn resolve_anchor(
        &self,
        revision: u64,
        anchor: &str,
        request: &WindowRequest,
    ) -> Result<u64, ControlError> {
        let index = self.ready_index(revision, request)?;
        let (namespace, key) = anchor
            .strip_prefix("fn-")
            .map_or(("slug", anchor), |key| ("footnote", key));
        let response = self
            .manager
            .query_structure(
                &self.owner,
                index,
                StructureQuery::Resolve {
                    namespace: namespace.into(),
                    key: key.into(),
                },
            )
            .map_err(host_error)?;
        response
            .items
            .first()
            .and_then(|item| {
                item["target"]["byte"]
                    .as_u64()
                    .or_else(|| item["target"]["fromByte"].as_u64())
            })
            .ok_or_else(|| ControlError::NeedsContext(format!("未找到锚点 {anchor}")))
    }
    /// 全文复制读取同一个快照，分片大小遵守原版 read_window 上限。
    /// 剪切由调用方先验证系统剪贴板成功，再提交与该快照修订绑定的删除事务。
    pub fn copy_range(&self, revision: u64, range: Range<u64>) -> Result<String, ControlError> {
        let snapshot = self
            .manager
            .capture(&self.owner, &self.document_id)
            .map_err(host_error)?;
        let result = (|| {
            if snapshot.revision != revision {
                return Err(ControlError::Stale);
            }
            if range.start > range.end || range.end > snapshot.byte_length {
                return Err(ControlError::InvalidRange);
            }
            let mut text = String::new();
            let mut at = range.start;
            while at < range.end {
                let part = self
                    .manager
                    .read_snapshot(
                        &self.owner,
                        &snapshot.snapshot_id,
                        at,
                        (at + 1_048_572).min(range.end),
                    )
                    .map_err(host_error)?;
                if part.from_byte != at || part.to_byte <= at || part.to_byte > range.end {
                    return Err(ControlError::InvalidRange);
                }
                at = part.to_byte;
                text.push_str(&part.text);
            }
            Ok(text)
        })();
        let released = self
            .manager
            .release(&self.owner, &snapshot.snapshot_id)
            .map_err(host_error);
        let text = result?;
        released?;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_groups_cover_real_session_and_retain_changed_window() {
        let root=std::env::temp_dir().join(format!("nomo-viewport-groups-{}",std::process::id()));
        let text=(0..180).map(|i|format!("段落 {i} 中文 😀\r\n\r\n")).collect::<String>();
        let port=NativeWindowPort::open(root.clone(),"viewport-test".into(),None,Some(text.clone())).unwrap();
        let request=WindowRequest::default();
        let groups=port.window_groups(0,&[],&request).unwrap();
        assert!(groups.len()>=3);
        let mut restored=String::new();
        for group in &groups {
            let loaded=port.load_group(0,group.from,Some(group.to),&request).unwrap();
            assert_eq!((loaded.input.from_byte,loaded.input.to_byte),(group.from,group.to));
            restored.push_str(&loaded.input.text);
        }
        assert_eq!(restored,text);
        let group=&groups[1];
        let at=group.from+"段落".len() as u64;
        let insert=" 新增\r\n\r\n第二段";
        let revision=port.submit(DocumentTransaction {base_revision:0,operation_id:"viewport-edit".into(),history_group_id:"viewport-edit".into(),edits:vec![SourceEdit{from_byte:at,to_byte:at,inserted_text:insert.into()}],selection_before:DocumentSelection::caret(at),selection_after:DocumentSelection::caret(at+insert.len() as u64),template_update:None}).unwrap();
        let retained=(group.from,group.to+insert.len() as u64);
        let updated=port.window_groups(revision,&[retained],&request).unwrap();
        assert!(updated.iter().any(|g|(g.from,g.to)==retained));
        assert_eq!(updated[0].from,0);
        assert_eq!(updated.last().unwrap().to,text.len() as u64+insert.len() as u64);
        assert!(updated.windows(2).all(|w|w[0].to==w[1].from));
        port.close(true).unwrap();drop(port);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn clipboard_reads_one_snapshot_across_unicode_and_crlf_chunks() {
        let root =
            std::env::temp_dir().join(format!("nomo-production-copy-{}", std::process::id()));
        let text = "中🙂\r\n".repeat(180_000);
        let port =
            NativeWindowPort::open(root.clone(), "copy-test".into(), None, Some(text.clone()))
                .unwrap();
        assert_eq!(port.copy_range(0, 0..text.len() as u64).unwrap(), text);
        assert!(matches!(
            port.copy_range(1, 0..text.len() as u64),
            Err(ControlError::Stale)
        ));
        port.close(true).unwrap();
        drop(port);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn real_file_windows_resolve_remote_references_and_share_history() {
        let root =
            std::env::temp_dir().join(format!("nomo-production-windows-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let text = format!(
            "[remote]: https://example.com \"远处\"\r\n\r\n{}\n[target][remote]\n\nlast\n",
            "前段落🙂\r\n\r\n".repeat(10_000)
        );
        let file = root.join("input.md");
        std::fs::write(&file, &text).unwrap();
        let port = NativeWindowPort::open(
            root.join("session"),
            "production-test".into(),
            Some(file.clone()),
            None,
        )
        .unwrap();
        let at = text.find("[target]").unwrap() as u64;
        let group = port
            .load_group(0, at, None, &WindowRequest::default())
            .unwrap();
        assert!(group.input.from_byte > 0);
        assert!(group.input.text.contains("[target][remote]"));
        let WindowContext::Resolved(context) = group.context else {
            panic!("resolved context")
        };
        assert_eq!(
            context.references.get("REMOTE").unwrap().destination,
            "https://example.com"
        );
        let rev = port
            .submit(DocumentTransaction {
                base_revision: 0,
                operation_id: "edit-last".into(),
                history_group_id: "edit-last".into(),
                edits: vec![SourceEdit {
                    from_byte: at + 1,
                    to_byte: at + 7,
                    inserted_text: "修改".into(),
                }],
                selection_before: DocumentSelection::caret(at),
                selection_after: DocumentSelection::caret(at),
                template_update: None,
            })
            .unwrap();
        let copy = port
            .copy_range(rev, 0..port.status().unwrap().byte_length)
            .unwrap();
        assert_eq!(copy, text.replacen("[target]", "[修改]", 1));
        port.save_to(file.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), copy);
        let (revision, _) = port.history(rev, false).unwrap();
        assert_eq!(
            port.copy_range(revision, 0..text.len() as u64).unwrap(),
            text
        );
        let cancelled = WindowRequest::default();
        cancelled.cancel();
        assert!(matches!(
            port.load_group(revision, at, None, &cancelled),
            Err(ControlError::Stale)
        ));
        port.close(true).unwrap();
        drop(port);
        std::fs::remove_dir_all(root).unwrap();
    }
}

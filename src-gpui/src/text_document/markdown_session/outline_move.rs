//! 大纲章节移动：复用不可变 Piece，作为一次原子历史操作发布。
use super::*;
use crate::text_document::structure_index::{Identity, Query};

const WORK_BUDGET: usize = 8 * 1024 * 1024;
const MAX_HEADING_LINE: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum OutlinePlacement { Before, Inside, After }
#[derive(Clone, Debug, Serialize)]
pub(crate) struct OutlineMove {
    pub identity: Identity,
    pub source: String,
    pub target: String,
    pub placement: OutlinePlacement,
    pub operation: String,
    pub selection: Option<MarkdownSelection>,
}
#[derive(Clone, Debug)]
pub(crate) struct OutlineMoveResult { pub status: MarkdownStatus, pub byte: u64 }
struct Section { from: u64, to: u64, level: u8 }
struct Patch { from: u64, to: u64, text: String }

fn section(manager: &MarkdownDocumentSessionManager, owner: &str, identity: &Identity, key: &str) -> TextDocumentResult<Section> {
    let response = manager.query_structure(owner, identity.clone(), Query::Resolve { namespace: "heading".into(), key: key.into() })?;
    if response.state != "ready" || !response.semantic_complete { return Err(error("outline-incomplete", "结构索引尚不完整，不能移动章节。")); }
    let target = response.items.first().and_then(|item| item.get("target")).filter(|item| item.is_object()).ok_or_else(|| error("outline-stale", "标题已变化，请重新选择。"))?;
    Ok(Section {
        from: target["fromByte"].as_u64().ok_or_else(|| error("outline-invalid", "标题位置无效"))?,
        to: target["toByte"].as_u64().ok_or_else(|| error("outline-invalid", "章节范围无效"))?,
        level: target["level"].as_u64().and_then(|v| u8::try_from(v).ok()).filter(|v| (1..=6).contains(v)).ok_or_else(|| error("outline-invalid", "标题层级无效"))?,
    })
}
fn check_boundary(manager: &MarkdownDocumentSessionManager, owner: &str, identity: &Identity, byte: u64, total: u64) -> TextDocumentResult<()> {
    if byte == total { return Ok(()); }
    let response = manager.query_structure(owner, identity.clone(), Query::Blocks { from_byte: byte, to_byte: (byte + 1).min(total), cursor: None })?;
    let block = response.items.first().ok_or_else(|| error("outline-boundary", "章节边界尚不可用。"))?;
    if !response.semantic_complete || response.fragment.is_some() || block["fromByte"].as_u64() != Some(byte) || block["semanticComplete"].as_bool() != Some(true) {
        return Err(error("outline-boundary", "章节边界位于不可安全拆分的容器中，文档未修改。"));
    }
    Ok(())
}
fn line(snapshot: &DocumentSnapshot, start: u64) -> TextDocumentResult<(Vec<u8>, u64)> {
    let length = (snapshot.len().saturating_sub(start)).min(MAX_HEADING_LINE as u64 + 1) as usize;
    let bytes = snapshot.read_range(start, length)?;
    let end = bytes.iter().position(|b| *b == b'\r' || *b == b'\n');
    if end.is_none() && length > MAX_HEADING_LINE { return Err(error("outline-heading-too-long", "标题行超过移动工作预算。")); }
    let end = end.unwrap_or(bytes.len());
    let mut next = end;
    if bytes.get(next) == Some(&b'\r') { next += 1; }
    if bytes.get(next) == Some(&b'\n') { next += 1; }
    Ok((bytes[..end].to_vec(), start + next as u64))
}
fn marker_start(bytes: &[u8]) -> usize {
    let mut i = 0;
    loop {
        while bytes.get(i).is_some_and(|b| *b == b' ' || *b == b'\t') { i += 1; }
        if bytes.get(i) == Some(&b'>') { i += 1; continue; }
        let list = matches!(bytes.get(i), Some(b'-' | b'+' | b'*')) && bytes.get(i + 1).is_some_and(u8::is_ascii_whitespace);
        if list { i += 1; continue; }
        let begin = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) { i += 1; }
        if i > begin && matches!(bytes.get(i), Some(b'.' | b')')) && bytes.get(i + 1).is_some_and(u8::is_ascii_whitespace) { i += 1; continue; }
        return begin;
    }
}
fn marker_patches(snapshot: &DocumentSnapshot, at: u64, old: u8, new: u8) -> TextDocumentResult<Vec<Patch>> {
    if old == new { return Ok(vec![]); }
    let (first, next) = line(snapshot, at)?;
    let start = marker_start(&first);
    let count = first[start..].iter().take_while(|b| **b == b'#').count();
    if count == old as usize && first.get(start + count).is_none_or(u8::is_ascii_whitespace) {
        return Ok(vec![Patch { from: at + start as u64, to: at + (start + count) as u64, text: "#".repeat(new as usize) }]);
    }
    if next >= snapshot.len() { return Err(error("outline-heading-syntax", "无法安全识别标题格式，文档未修改。")); }
    let (underline, after) = line(snapshot, next)?;
    let underline_start = marker_start(&underline);
    let marker = if old == 1 { b'=' } else { b'-' };
    let mut end = underline_start;
    while underline.get(end) == Some(&marker) { end += 1; }
    if old > 2 || end == underline_start || underline[end..].iter().any(|b| !b.is_ascii_whitespace()) {
        return Err(error("outline-heading-syntax", "无法安全识别 Setext 标题，文档未修改。"));
    }
    if new <= 2 {
        return Ok(vec![Patch { from: next + underline_start as u64, to: next + end as u64, text: (if new == 1 { "=" } else { "-" }).repeat(end - underline_start) }]);
    }
    // 容器内的 Setext 标题可能依赖下一行前缀；不能在不完整上下文中重写。
    if start != 0 || underline_start != 0 { return Err(error("outline-heading-syntax", "容器中的 Setext 标题无法安全调整层级。")); }
    Ok(vec![Patch { from: at, to: at, text: format!("{} ", "#".repeat(new as usize)) }, Patch { from: next, to: after, text: String::new() }])
}
fn append_range(tree: &PieceTree, mut from: u64, to: u64, pieces: &mut Vec<Piece>, used: &mut usize) -> TextDocumentResult<()> {
    while from < to {
        let (piece, relative) = tree.piece_at(from).ok_or_else(|| error("outline-piece", "正文片段缺失"))?;
        let length = (piece.len() - relative).min(to - from);
        let source = match piece.source {
            PieceSource::Original { offset, .. } => PieceSource::Original { offset: offset + relative, length },
            PieceSource::Added { offset, .. } => PieceSource::Added { offset: offset + relative, length },
        };
        *used += std::mem::size_of::<Piece>();
        if *used > WORK_BUDGET { return Err(error("outline-budget", "章节移动超过工作预算，文档未修改。")); }
        pieces.push(Piece { source, line_breaks: None });
        from += length;
    }
    Ok(())
}
fn apply_edit(tree: &mut PieceTree, forward: &mut Vec<TreeEdit>, inverse: &mut Vec<TreeEdit>, from: u64, to: u64, pieces: Vec<Piece>, used: &mut usize) -> TextDocumentResult<()> {
    let mut original = Vec::new(); append_range(tree, from, to, &mut original, used)?;
    let inserted = pieces.iter().map(Piece::len).sum::<u64>();
    let edit = TreeEdit { from, to, pieces };
    *tree = apply_tree_edits(tree.clone(), std::slice::from_ref(&edit))?;
    forward.push(edit);
    inverse.insert(0, TreeEdit { from, to: from + inserted, pieces: original });
    Ok(())
}
fn append_text(session: &mut Session, text: &str) -> TextDocumentResult<Option<Piece>> {
    if text.is_empty() { return Ok(None); }
    Ok(Some(Piece::added(session.current.added.append(text.as_bytes())?, text.len() as u64, text.bytes().filter(|b| *b == b'\n').count() as u64)))
}
fn byte(snapshot: &DocumentSnapshot, at: u64) -> TextDocumentResult<u8> {
    snapshot.read_range(at, 1)?.first().copied().ok_or_else(|| error("outline-boundary", "无法读取章节边界"))
}

impl MarkdownDocumentSessionManager {
    pub(crate) fn move_outline_section(&self, owner: &str, request: OutlineMove) -> TextDocumentResult<OutlineMoveResult> {
        if request.operation.is_empty() { return Err(error("invalid-transaction", "章节移动缺少操作身份")); }
        let fingerprint: [u8; 32] = Sha256::digest(serde_json::to_vec(&request).map_err(|e| error("invalid-transaction", e.to_string()))?).into();
        let handle = self.session(&request.identity.document_id)?;
        let snapshot = {
            let session = lock(&handle)?; session.owner(owner)?;
            if let Some((old, status)) = session.ack.get(&request.operation) {
                if old != &fingerprint { return Err(error("operation-conflict", "同一操作身份携带了不同内容")); }
                return Ok(OutlineMoveResult { status: status.clone(), byte: status.selection.as_ref().map_or(0, |s| s.head_byte) });
            }
            if session.current.revision != request.identity.revision { return Err(TextDocumentError::revision_conflict(request.identity.revision, session.current.revision)); }
            session.current.clone()
        };
        let source = section(self, owner, &request.identity, &request.source)?;
        let target = section(self, owner, &request.identity, &request.target)?;
        if source.from <= target.from && target.from < source.to { return Err(error("outline-descendant", "不能把章节移动到自身或其子章节。")); }
        let destination = match request.placement { OutlinePlacement::Before => target.from, OutlinePlacement::Inside | OutlinePlacement::After => target.to };
        let desired = target.level + u8::from(matches!(request.placement, OutlinePlacement::Inside));
        if desired > 6 { return Err(error("outline-level", "移动后标题会超过 H6。")); }
        let delta = desired as i16 - source.level as i16;
        if delta == 0 && (destination == source.from || destination == source.to) { return Err(error("outline-no-change", "章节位置没有变化。")); }
        for boundary in [source.from, source.to, destination] { check_boundary(self, owner, &request.identity, boundary, snapshot.len())?; }
        let mut patches = Vec::new(); let mut cursor = None;
        loop {
            let page = self.query_structure(owner, request.identity.clone(), Query::Headings { from_byte: source.from, to_byte: source.to, cursor })?;
            if !page.semantic_complete || page.fragment.is_some() { return Err(error("outline-incomplete", "章节标题不完整，文档未修改。")); }
            if page.items.is_empty() && page.next.is_some() { return Err(error("outline-heading-too-long", "章节标题超过分页预算，文档未修改。")); }
            for item in page.items {
                let at = item["byte"].as_u64().ok_or_else(|| error("outline-invalid", "标题位置无效"))?;
                let level = item["level"].as_u64().ok_or_else(|| error("outline-invalid", "标题层级无效"))? as i16;
                let next = level + delta;
                if !(1..=6).contains(&next) { return Err(error("outline-level", "移动后标题会超过 H1–H6。")); }
                patches.extend(marker_patches(&snapshot, at, level as u8, next as u8)?);
                if patches.len() * std::mem::size_of::<Patch>() > WORK_BUDGET / 2 { return Err(error("outline-budget", "章节标题超过移动工作预算。")); }
            }
            cursor = page.next;
            if cursor.is_none() { break; }
        }
        patches.sort_by_key(|p| (p.from, p.to));
        if patches.windows(2).any(|w| w[0].to > w[1].from) || patches.iter().any(|p| p.from < source.from || p.to > source.to) { return Err(error("outline-invalid", "章节标题范围重叠。")); }
        let mut session = lock(&handle)?; session.owner(owner)?;
        if session.current.revision != request.identity.revision { return Err(TextDocumentError::revision_conflict(request.identity.revision, session.current.revision)); }
        if session.manifest.inline_templates.iter().any(|t| (t.from_byte < source.to && t.to_byte > source.from) || (t.from_byte < destination && destination < t.to_byte)) { return Err(error("outline-template", "章节或目标包含活动格式模板，请先完成编辑再移动。")); }
        let mut used = patches.len() * std::mem::size_of::<Patch>();
        let mut pieces = Vec::new(); let mut at = source.from;
        for patch in &patches {
            append_range(&snapshot.tree, at, patch.from, &mut pieces, &mut used)?;
            if let Some(piece) = append_text(&mut session, &patch.text)? { pieces.push(piece); }
            at = patch.to;
        }
        append_range(&snapshot.tree, at, source.to, &mut pieces, &mut used)?;
        let had_newline = snapshot.len() > 0 && matches!(byte(&snapshot, snapshot.len() - 1)?, b'\r' | b'\n');
        let (_, first_end) = line(&snapshot, source.from)?;
        let ending_at = if first_end > source.from && matches!(byte(&snapshot, first_end - 1)?, b'\r' | b'\n') { first_end } else { source.from };
        let newline = if ending_at >= 2 && snapshot.read_range(ending_at - 2, 2)? == b"\r\n" { "\r\n" }
            else if ending_at > 0 && byte(&snapshot, ending_at - 1)? == b'\r' { "\r" } else { "\n" };
        let mut tree = snapshot.tree.clone(); let mut forward = Vec::new(); let mut inverse = Vec::new();
        apply_edit(&mut tree, &mut forward, &mut inverse, source.from, source.to, vec![], &mut used)?;
        let insertion = if destination > source.from { destination - (source.to - source.from) } else { destination };
        let mut before = snapshot.clone(); before.tree = tree.clone();
        let prefix = insertion > 0 && !matches!(byte(&before, insertion - 1)?, b'\r' | b'\n');
        if prefix { if let Some(piece) = append_text(&mut session, newline)? { pieces.insert(0, piece); } }
        let inserted = pieces.iter().map(Piece::len).sum::<u64>();
        apply_edit(&mut tree, &mut forward, &mut inverse, insertion, insertion, pieces, &mut used)?;
        let mut candidate = snapshot.clone(); candidate.tree = tree.clone();
        if insertion + inserted < candidate.len() && inserted > 0 && !matches!(byte(&candidate, insertion + inserted - 1)?, b'\r' | b'\n') {
            let piece = append_text(&mut session, newline)?.into_iter().collect();
            apply_edit(&mut tree, &mut forward, &mut inverse, insertion + inserted, insertion + inserted, piece, &mut used)?;
            candidate.tree = tree.clone();
        }
        if candidate.len() > 0 && !had_newline && matches!(byte(&candidate, candidate.len() - 1)?, b'\r' | b'\n') {
            let end = candidate.len(); let start = if end >= 2 && byte(&candidate, end - 1)? == b'\n' && byte(&candidate, end - 2)? == b'\r' { end - 2 } else { end - 1 };
            apply_edit(&mut tree, &mut forward, &mut inverse, start, end, vec![], &mut used)?;
        }
        // 合并为原文坐标中的保守变更区间，区间外的结构块仍可增量复用。
        let changed_from = source.from.min(destination);
        let changed_to = source.to.max(destination);
        let unchanged = snapshot.len() - (changed_to - changed_from);
        let changed_inserted = tree.len().checked_sub(unchanged).ok_or_else(|| error("outline-invalid", "章节变更范围无效。"))?;
        let mut templates = session.manifest.inline_templates.clone();
        markdown_templates::map_templates(&mut templates, forward.iter().map(|e| (e.from, e.to, e.pieces.iter().map(Piece::len).sum())), false);
        let selection = Some(MarkdownSelection { anchor_byte: insertion + if prefix { newline.len() as u64 } else { 0 }, head_byte: insertion + if prefix { newline.len() as u64 } else { 0 } });
        let group = id("outline-move");
        let record = HistoryRecord { previous_event: session.manifest.event, group: group.clone(), parent_undo: session.manifest.undo,
            previous_redo: session.manifest.redo, edits: forward, inverse, group_entries: vec![], selection_before: request.selection,
            selection_after: selection.clone(), templates_before: Some(session.manifest.inline_templates.clone()), templates_after: Some(templates) };
        let status = session.publish(record, tree, HistoryPointer::This, HistoryPointer::Existing(None))?;
        session.active_group = Some((group.clone(), group));
        session.ack.insert(request.operation.clone(), (fingerprint, status.clone())); session.ack_order.push_back(request.operation);
        if session.ack_order.len() > 128 { if let Some(old) = session.ack_order.pop_front() { session.ack.remove(&old); } }
        drop(session);
        self.writing_stats.invalidate(&status.document_id, false); self.structure.edited(&status.document_id, &[(changed_from, changed_to, changed_inserted)]); self.search.invalidate(&status.document_id, None);
        Ok(OutlineMoveResult { byte: selection.map_or(insertion, |s| s.head_byte), status })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn indexed(manager: &MarkdownDocumentSessionManager, status: &MarkdownStatus) -> Identity {
        let identity = manager.start_structure("main", &status.document_id, status.revision).unwrap();
        let start = std::time::Instant::now();
        loop {
            let progress = manager.query_structure("main", identity.clone(), Query::Progress).unwrap();
            assert_ne!(progress.state, "failed", "{:?}", progress.failure);
            if progress.state == "ready" { break; }
            assert!(start.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        identity
    }
    fn keys(manager: &MarkdownDocumentSessionManager, identity: &Identity) -> Vec<String> {
        let page = manager.query_structure("main", identity.clone(), Query::Outline { cursor: None }).unwrap();
        page.items.iter().map(|item| format!("{}:{}", item["blockId"].as_str().unwrap(), item["blockHeadingIndex"].as_u64().unwrap())).collect()
    }
    fn contents(manager: &MarkdownDocumentSessionManager, status: &MarkdownStatus) -> String {
        manager.read_window("main", &status.document_id, status.revision, 0, status.byte_length).unwrap().text
    }
    #[test]
    fn outline_move_subtree_crlf_and_undo_redo() {
        let root = std::env::temp_dir().join(id("outline-move-test"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let original = "# A\r\nAlpha\r\n## Sub\r\nB\r\n# C\r\n中文🙂";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let request = OutlineMove { identity, source:headings[0].clone(), target:headings[2].clone(), placement:OutlinePlacement::Inside, operation:"move".into(), selection:None };
        let result = manager.move_outline_section("main", request.clone()).unwrap();
        assert_eq!(contents(&manager, &result.status), "# C\r\n中文🙂\r\n## A\r\nAlpha\r\n### Sub\r\nB");
        assert_eq!(manager.move_outline_section("main", request).unwrap().status.revision, result.status.revision);
        let undone = manager.history("main", &status.document_id, result.status.revision, false).unwrap();
        assert_eq!(contents(&manager, &undone), original);
        let redone = manager.history("main", &status.document_id, undone.revision, true).unwrap();
        assert_eq!(contents(&manager, &redone), "# C\r\n中文🙂\r\n## A\r\nAlpha\r\n### Sub\r\nB");
        manager.detach("main", &status.document_id).unwrap();
        let recovered = manager.recover("main", &status.document_id).unwrap();
        assert_eq!(contents(&manager, &recovered), "# C\r\n中文🙂\r\n## A\r\nAlpha\r\n### Sub\r\nB");
        assert!(!recovered.can_undo);
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn outline_move_setext_and_invalid_operations() {
        let root = std::env::temp_dir().join(id("outline-move-setext"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let original = "## Target\nbody\n\nAlpha\n=====\ncontent\n";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let request = OutlineMove { identity:identity.clone(), source:headings[1].clone(), target:headings[0].clone(), placement:OutlinePlacement::Inside, operation:"setext".into(), selection:None };
        let result = manager.move_outline_section("main", request).unwrap();
        assert_eq!(contents(&manager, &result.status), "## Target\nbody\n\n### Alpha\ncontent\n");
        let stale = OutlineMove { identity, source:headings[0].clone(), target:headings[0].clone(), placement:OutlinePlacement::Before, operation:"stale".into(), selection:None };
        assert!(manager.move_outline_section("main", stale).is_err());
        assert_eq!(contents(&manager, &result.status), "## Target\nbody\n\n### Alpha\ncontent\n");
        let current = indexed(&manager, &result.status); let headings = keys(&manager, &current);
        let invalid = OutlineMove { identity:current, source:headings[0].clone(), target:headings[1].clone(), placement:OutlinePlacement::Inside, operation:"invalid".into(), selection:None };
        assert!(manager.move_outline_section("main", invalid).is_err());
        assert_eq!(contents(&manager, &result.status), "## Target\nbody\n\n### Alpha\ncontent\n");
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn outline_move_final_heading_preserves_crlf_and_rejects_multiline_setext() {
        let root = std::env::temp_dir().join(id("outline-move-lines"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let original = "# A\r\ncontent\r\n# B";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let result = manager.move_outline_section("main", OutlineMove { identity, source:headings[1].clone(), target:headings[0].clone(), placement:OutlinePlacement::Before, operation:"last".into(), selection:None }).unwrap();
        assert_eq!(contents(&manager, &result.status), "# B\r\n# A\r\ncontent");
        let undone = manager.history("main", &status.document_id, result.status.revision, false).unwrap();
        assert_eq!(contents(&manager, &undone), original);
        manager.close("main", &status.document_id, true).unwrap();

        let original = "## Target\nbody\nAlpha\n=====\ncontent\n";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let request = OutlineMove { identity, source:headings[1].clone(), target:headings[0].clone(), placement:OutlinePlacement::Inside, operation:"multiline".into(), selection:None };
        assert!(manager.move_outline_section("main", request).is_err());
        assert_eq!(contents(&manager, &status), original);
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn outline_move_rejects_level_overflow_without_changes() {
        let root = std::env::temp_dir().join(id("outline-move-overflow"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let original = "# A\n###### deep\n# B\n";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let request = OutlineMove { identity, source:headings[0].clone(), target:headings[2].clone(), placement:OutlinePlacement::Inside, operation:"overflow".into(), selection:None };
        assert!(manager.move_outline_section("main", request).is_err());
        assert_eq!(contents(&manager, &status), original);
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn outline_move_blockquote_heading_with_complete_boundary() {
        let root = std::env::temp_dir().join(id("outline-move-container"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let original = "> # A\n> content\n\n# B\n";
        let status = manager.open("main", None, Some(original.into())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        assert_eq!(headings.len(), 2);
        let result = manager.move_outline_section("main", OutlineMove { identity, source:headings[0].clone(), target:headings[1].clone(), placement:OutlinePlacement::Inside, operation:"container".into(), selection:None }).unwrap();
        assert_eq!(contents(&manager, &result.status), "# B\n> ## A\n> content\n\n");
        let undone = manager.history("main", &status.document_id, result.status.revision, false).unwrap();
        assert_eq!(contents(&manager, &undone), original);
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn outline_move_duplicate_and_large_section_without_whole_text_edit() {
        let root = std::env::temp_dir().join(id("outline-move-large"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let body = "line\n\n".repeat(50_000);
        let original = format!("# Same\n{body}# Same\nend\n");
        let status = manager.open("main", None, Some(original.clone())).unwrap();
        let identity = indexed(&manager, &status); let headings = keys(&manager, &identity);
        let result = manager.move_outline_section("main", OutlineMove { identity, source:headings[1].clone(), target:headings[0].clone(), placement:OutlinePlacement::Before, operation:"large".into(), selection:None }).unwrap();
        assert_eq!(contents(&manager, &result.status), format!("# Same\nend\n# Same\n{body}"));
        let undone = manager.history("main", &status.document_id, result.status.revision, false).unwrap();
        assert_eq!(contents(&manager, &undone), original);
        manager.close("main", &status.document_id, true).unwrap(); drop(manager); let _ = fs::remove_dir_all(root);
    }
}

//! 自动目录派生只读不可变快照；最终提交复用原生事务、幂等确认与历史。
use super::*;
use crate::text_document::structure_index::{parser::Block, scanner::Scanner, Identity, Query};

const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Baseline {
    signature: Vec<u8>,
    ranges: Vec<Range>,
    complete: bool,
}
#[derive(Serialize, Deserialize)]
struct Range {
    from: u64,
    to: u64,
    text: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TocResult {
    pub status: MarkdownStatus,
    pub changed: bool,
    pub native_edits: Option<Vec<serde_json::Value>>,
}
fn fail(message: &str) -> TextDocumentError {
    error("automatic-toc-incomplete", message)
}
fn signature(hash: &mut Sha256, level: u8, title: &str) {
    hash.update([level]);
    hash.update((title.len() as u64).to_le_bytes());
    hash.update(title.as_bytes());
}
fn ranges(snapshot: &DocumentSnapshot, block: &Block) -> TextDocumentResult<Vec<Range>> {
    if block.kind != "toc_block" && block.kind != "html_block" {
        return Ok(vec![]);
    }
    if block.mode != "semantic" {
        return Err(fail("目录所在语法块不完整，不能生成部分目录。"));
    }
    let text = String::from_utf8(
        snapshot.read_range(block.from_byte, (block.to_byte - block.from_byte) as usize)?,
    )
    .map_err(|_| fail("目录原文不是有效 UTF-8。"))?;
    let mut result = vec![];
    let mut start = None;
    let mut offset = 0;
    while offset < text.len() {
        let mut end = text[offset..].find(['\r', '\n']).map_or(text.len(), |at| offset + at);
        if text.as_bytes().get(end) == Some(&b'\r') { end += 1; }
        if text.as_bytes().get(end) == Some(&b'\n') { end += 1; }
        let line = &text[offset..end];
        let content = line.trim_end_matches(['\r', '\n']);
        match content.trim() {
            "<!-- toc -->" => start = Some(offset),
            "<!-- /toc -->" => {
                if let Some(from) = start.take() {
                    let to = offset + content.len();
                    result.push(Range {
                        from: block.from_byte + from as u64,
                        to: block.from_byte + to as u64,
                        text: text[from..to].into(),
                    });
                }
            }
            _ => {}
        }
        offset = end;
    }
    Ok(result)
}
fn escape(text: &str) -> String {
    let mut result = String::new();
    for (offset, ch) in text.char_indices() {
        let entity = ch == '&'
            && text[offset + 1..].find(';').is_some_and(|end| {
                let body = &text[offset + 1..offset + 1 + end];
                !body.is_empty()
                    && body
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '#')
            });
        if "\\`*_~^<>[]$".contains(ch) || entity {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}
fn map_position(position: u64, edits: &[MarkdownEdit], right: bool) -> u64 {
    let mut delta = 0_i64;
    for edit in edits {
        if position < edit.from_byte {
            break;
        }
        if position > edit.to_byte || (position == edit.to_byte && edit.from_byte != edit.to_byte) {
            delta += edit.inserted_text.len() as i64 - (edit.to_byte - edit.from_byte) as i64;
        } else {
            return edit.from_byte.saturating_add_signed(delta)
                + if right {
                    edit.inserted_text.len() as u64
                } else {
                    0
                };
        }
    }
    position.saturating_add_signed(delta)
}
impl MarkdownDocumentSessionManager {
    pub(crate) fn materialize_structure_toc(
        &self,
        owner: &str,
        identity: Identity,
        operation_id: String,
        edit_generation: u64,
        selection: Option<MarkdownSelection>,
    ) -> TextDocumentResult<TocResult> {
        let handle = self.session(&identity.document_id)?;
        let fingerprint: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&(&identity, edit_generation, &selection))
                .map_err(|_| fail("目录事务参数无效。"))?,
        )
        .into();
        let (snapshot, original, has_derived) = {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if let Some((_, old, status)) = session
                .toc_ack
                .iter()
                .find(|(id, _, _)| *id == operation_id)
            {
                if *old != fingerprint {
                    return Err(error("operation-conflict", "目录操作身份已用于其它参数"));
                }
                return Ok(TocResult {
                    native_edits: None,
                    changed: status.revision != identity.revision,
                    status: status.clone(),
                });
            }
            if session.current.revision != identity.revision {
                return Err(TextDocumentError::revision_conflict(
                    identity.revision,
                    session.current.revision,
                ));
            }
            let mut original = session.current.clone();
            original.tree = PieceTree::from_original(session.manifest.source_map.utf8_len);
            original.revision = 0;
            (session.current.clone(), original, session.manifest.derived)
        };
        let progress = self.query_structure(owner, identity.clone(), Query::Progress)?;
        if progress.state != "ready" {
            return Err(fail("结构索引尚未完成。"));
        }
        let mut current_ranges = vec![];
        let mut range_bytes = 0;
        let mut cursor = None;
        loop {
            let page =
                self.query_structure(owner, identity.clone(), Query::TocBlocks { cursor })?;
            if page.fragment.is_some() {
                return Err(fail("单块结构详情过大，目录派生尚不能处理此块。"));
            }
            for item in page.items {
                let block: Block =
                    serde_json::from_value(item).map_err(|_| fail("目录结构记录无效。"))?;
                for range in ranges(&snapshot, &block)? {
                    range_bytes += range.text.len() + std::mem::size_of::<Range>();
                    if range_bytes > LIMIT { return Err(fail("目录原文超过派生工作预算。")); }
                    current_ranges.push(range);
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let unchanged = |status| {
            Ok(TocResult {
                native_edits: Some(vec![]),
                status,
                changed: false,
            })
        };
        if current_ranges.is_empty() || identity.revision == 0 {
            return unchanged(self.status(owner, &identity.document_id)?);
        }
        if !progress.semantic_complete {
            return Err(fail("部分语法尚未完整识别，不能写入不完整目录。"));
        }
        let baseline: Baseline =
            if let Some(value) = self.structure.toc_baseline(owner, &identity, None)? {
                serde_json::from_str(&value).map_err(|_| fail("原始目录记录损坏。"))?
            } else {
                let mut scanner = Scanner::new(0, 0, 1);
                let mut offset = 0;
                let mut hash = Sha256::new();
                let mut baseline = Baseline {
                    signature: vec![],
                    ranges: vec![],
                    complete: true,
                };
                let mut bytes = 0;
                loop {
                    self.status(owner, &identity.document_id)
                        .and_then(|status| {
                            if status.revision == identity.revision {
                                Ok(status)
                            } else {
                                Err(TextDocumentError::revision_conflict(
                                    identity.revision,
                                    status.revision,
                                ))
                            }
                        })?;
                    let chunk = original
                        .read_range(offset, 16384.min((original.len() - offset) as usize))?;
                    let length = std::str::from_utf8(&chunk)
                        .map(|s| s.len())
                        .unwrap_or_else(|e| e.valid_up_to());
                    if length == 0 && offset < original.len() {
                        return Err(fail("原始目录读取没有前进。"));
                    }
                    offset += length as u64;
                    let batch = scanner
                        .push(
                            std::str::from_utf8(&chunk[..length])
                                .map_err(|_| fail("原始目录编码无效。"))?,
                            offset == original.len(),
                        )
                        .map_err(|e| fail(&e))?;
                    for block in batch.blocks {
                        baseline.complete &= block.semantic_complete;
                        for heading in &block.headings {
                            signature(&mut hash, heading.level, &heading.title);
                        }
                        for range in ranges(&original, &block)? {
                            bytes += range.text.len();
                            baseline.ranges.push(range);
                        }
                    }
                    if bytes > LIMIT {
                        return Err(fail("原始目录超过派生事务预算。"));
                    }
                    if offset == original.len() {
                        break;
                    }
                }
                baseline.signature = hash.finalize().to_vec();
                let serialized = serde_json::to_string(&baseline)
                    .map_err(|_| fail("原始目录记录序列化失败。"))?;
                self.structure
                    .toc_baseline(owner, &identity, Some(&serialized))?;
                baseline
            };
        if !baseline.complete {
            return Err(fail("原始标题语义不完整，不能确认目录恢复条件。"));
        }
        let mut hash = Sha256::new();
        let mut content = String::new();
        let mut cursor = None;
        loop {
            let page = self.query_structure(owner, identity.clone(), Query::Outline { cursor })?;
            for heading in page.items {
                let title = heading["title"]
                    .as_str()
                    .ok_or_else(|| fail("标题内容缺失。"))?;
                let level = heading["level"].as_u64().unwrap_or(1) as u8;
                signature(&mut hash, level, title);
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&format!(
                    "{}- [{}](#{})",
                    "  ".repeat(level.saturating_sub(1) as usize),
                    escape(title),
                    heading["id"]
                        .as_str()
                        .ok_or_else(|| fail("标题锚点缺失。"))?
                ));
                if content.len() > LIMIT {
                    return Err(fail("生成目录超过派生事务预算。"));
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let same = hash.finalize().as_slice() == baseline.signature;
        if same && !has_derived && current_ranges.len() == baseline.ranges.len() {
            return unchanged(self.status(owner, &identity.document_id)?);
        }
        let mut edits = vec![];
        let mut generated_bytes = 0;
        for (i, range) in current_ranges.into_iter().enumerate() {
            let newline = if range.text.contains("\r\n") {
                "\r\n"
            } else if range.text.contains('\n') {
                "\n"
            } else if range.text.contains('\r') {
                "\r"
            } else {
                "\n"
            };
            let generated = format!(
                "<!-- toc -->{newline}{}{}<!-- /toc -->",
                content.replace('\n', newline),
                if content.is_empty() { "" } else { newline }
            );
            let inserted_text = if same {
                baseline
                    .ranges
                    .get(i)
                    .map(|range| range.text.clone())
                    .unwrap_or(generated)
            } else {
                generated
            };
            generated_bytes += inserted_text.len();
            if generated_bytes > LIMIT {
                return Err(fail("目录派生事务超过预算。"));
            }
            if inserted_text != range.text {
                edits.push(MarkdownEdit {
                    from_byte: range.from,
                    to_byte: range.to,
                    inserted_text,
                });
            }
        }
        let mapped = selection.as_ref().map(|selection| MarkdownSelection {
            anchor_byte: map_position(
                selection.anchor_byte,
                &edits,
                selection.anchor_byte >= selection.head_byte,
            ),
            head_byte: map_position(
                selection.head_byte,
                &edits,
                selection.head_byte >= selection.anchor_byte,
            ),
        });
        let native_edits = (edits.len() <= 256).then(|| edits.iter().map(|edit| serde_json::json!({
            "fromByte":edit.from_byte,"toByte":edit.to_byte,"insertedBytes":edit.inserted_text.len()
        })).collect());
        let status = self.apply(
            owner,
            MarkdownTransaction {
                template_update: None,
                derived: true,
                document_id: identity.document_id.clone(),
                base_revision: identity.revision,
                operation_id: operation_id.clone(),
                history_group_id: "automatic-toc".into(),
                edits,
                selection_before: selection,
                selection_after: mapped,
            },
        )?;
        let mut session = lock(&handle)?;
        session.owner(owner)?;
        session
            .toc_ack
            .push_back((operation_id, fingerprint, status.clone()));
        while session.toc_ack.len() > 64 {
            session.toc_ack.pop_front();
        }
        Ok(TocResult {
            native_edits,
            changed: status.revision != identity.revision,
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn indexed(manager: &MarkdownDocumentSessionManager, status: &MarkdownStatus) -> Identity {
        let identity = manager
            .start_structure("main", &status.document_id, status.revision)
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            let result = manager
                .query_structure("main", identity.clone(), Query::Progress)
                .unwrap();
            assert_ne!(result.state, "failed");
            if result.state == "ready" {
                break;
            }
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        identity
    }
    #[test]
    fn native_toc_preserves_original_restores_history_and_retries_idempotently() {
        let root = std::env::temp_dir().join(id("nomo-native-toc"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let text =
            "# Alpha\r\n\r\n<!-- toc -->\r\n  - [custom](#alpha)\r\n<!-- /toc -->\r\n\r\nbody\r\n";
        let initial = manager.open("main", None, Some(text.into())).unwrap();
        let body = manager
            .apply(
                "main",
                MarkdownTransaction {
                    template_update: None,
                    derived: false,
                    document_id: initial.document_id.clone(),
                    base_revision: 0,
                    operation_id: "body".into(),
                    history_group_id: "body".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: initial.byte_length,
                        to_byte: initial.byte_length,
                        inserted_text: "more\r\n".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let no_change = manager
            .materialize_structure_toc("main", indexed(&manager, &body), "toc-body".into(), 1, None)
            .unwrap();
        assert!(!no_change.changed);
        assert!(!no_change.status.has_derived_edits);
        let renamed = manager
            .apply(
                "main",
                MarkdownTransaction {
                    template_update: None,
                    derived: false,
                    document_id: initial.document_id.clone(),
                    base_revision: body.revision,
                    operation_id: "rename".into(),
                    history_group_id: "rename".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: 2,
                        to_byte: 7,
                        inserted_text: "中文".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let identity = indexed(&manager, &renamed);
        let result = manager
            .materialize_structure_toc(
                "main",
                identity.clone(),
                "toc-rename".into(),
                2,
                Some(MarkdownSelection {
                    anchor_byte: renamed.byte_length,
                    head_byte: renamed.byte_length,
                }),
            )
            .unwrap();
        assert!(result.changed);
        let content = manager
            .read_window(
                "main",
                &result.status.document_id,
                result.status.revision,
                0,
                result.status.byte_length,
            )
            .unwrap()
            .text;
        assert!(content.contains("<!-- toc -->\r\n- [中文](#中文)\r\n<!-- /toc -->"));
        assert_eq!(
            result.status.selection.as_ref().unwrap().head_byte,
            result.status.byte_length
        );
        let retry = manager
            .materialize_structure_toc(
                "main",
                identity,
                "toc-rename".into(),
                2,
                Some(MarkdownSelection {
                    anchor_byte: renamed.byte_length,
                    head_byte: renamed.byte_length,
                }),
            )
            .unwrap();
        assert_eq!(retry.status.revision, result.status.revision);
        let undone = manager
            .history(
                "main",
                &result.status.document_id,
                result.status.revision,
                false,
            )
            .unwrap();
        let restored = manager
            .materialize_structure_toc(
                "main",
                indexed(&manager, &undone),
                "toc-restore".into(),
                3,
                None,
            )
            .unwrap();
        let content = manager
            .read_window(
                "main",
                &restored.status.document_id,
                restored.status.revision,
                0,
                restored.status.byte_length,
            )
            .unwrap()
            .text;
        assert_eq!(content, format!("{text}more\r\n"));
        manager.close("main", &initial.document_id, true).unwrap();
        drop(manager);
        let _ = fs::remove_dir_all(root);
    }
}

//! 对象草稿具有独立确认/取消边界；只保存局部草稿，不创建第二套撤销历史。
use super::{commands::EditResult, parse::ParsedWindow, protocol::*};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Code,
    InlineMath,
    InsertInlineMath,
    InsertInlineComment,
    InlineComment,
    InsertBlockMath,
    InsertBlockComment,
    BlockMath,
    BlockComment,
    FrontMatter,
    Mermaid,
    Image,
    Link,
    Details,
    Html,
    TableSize,
}
pub struct ObjectDraft {
    pub identity: WindowIdentity,
    pub kind: ObjectKind,
    pub range: Range<usize>,
    pub text: String,
    original: String,
}
/// 只隔离当前块的正文；围栏和换行仍属于同一笔宿主事务。
pub struct BlockFrame {
    prefix: String,
    suffix: String,
    marker: Option<(char, usize)>,
    kind: ObjectKind,
}
impl BlockFrame {
    pub fn split(text: &str, kind: ObjectKind) -> Option<(Self, String)> {
        let (start, end, marker) = match kind {
            ObjectKind::Code | ObjectKind::Mermaid => {
                let first = text.find('\n')?;
                let header = text[..first].trim();
                let marker = header.chars().next()?;
                if !matches!(marker, '`' | '~') { return None; }
                let count = header.chars().take_while(|ch| *ch == marker).count();
                if count < 3 { return None; }
                let trimmed = text.trim_end_matches(['\r', '\n']);
                let last = trimmed.rfind('\n')? + 1;
                let closing = trimmed[last..].trim();
                if closing.len() < count || !closing.chars().all(|ch| ch == marker) { return None; }
                let mut end = last;
                if end > first + 1 {
                    end -= 1;
                    if text[..end].ends_with('\r') { end -= 1; }
                }
                (first + 1, end, Some((marker, count)))
            }
            ObjectKind::BlockMath => {
                let start = text.find("$$")? + 2;
                let end = text.rfind("$$")?;
                if end < start { return None; }
                let body = &text[start..end];
                let leading = if body.starts_with("\r\n") { 2 } else { usize::from(body.starts_with('\n')) };
                let trailing = if body.ends_with("\r\n") { 2 } else { usize::from(body.ends_with('\n')) };
                (start + leading, end.saturating_sub(trailing).max(start + leading), None)
            }
            ObjectKind::InlineMath => {
                if !text.starts_with('$') || !text.ends_with('$') || text.len() < 2 { return None; }
                (1, text.len() - 1, None)
            }
            ObjectKind::BlockComment => {
                let start = text.find("<!--")? + 4;
                let end = text.rfind("-->")?;
                let body = &text[start..end];
                let leading = if body.starts_with("\r\n") { 2 } else { usize::from(body.starts_with('\n')) };
                let trailing = if body.ends_with("\r\n") { 2 } else { usize::from(body.ends_with('\n')) };
                (start + leading, end.saturating_sub(trailing).max(start + leading), None)
            }
            ObjectKind::FrontMatter => {
                let start = text.find('\n')? + 1;
                let trimmed = text.trim_end_matches(['\r', '\n']);
                let closing = trimmed.rfind('\n')? + 1;
                let end = text[..closing].trim_end_matches(['\r', '\n']).len().max(start);
                (start, end, None)
            }
            _ => return None,
        };
        if start > end || end > text.len() { return None; }
        Some((Self { prefix: text[..start].into(), suffix: text[end..].into(), marker, kind }, text[start..end].into()))
    }
    pub fn body_start(&self) -> usize { self.prefix.len() }
    pub fn language(&self) -> &str {
        self.prefix.trim().trim_start_matches(['`', '~']).trim()
    }
    pub fn set_language(&mut self, language: &str) {
        if let Some((marker, count)) = self.marker {
            if let Some(start) = self.prefix.find(marker) {
                let end = self.prefix.trim_end_matches(['\r', '\n']).len();
                self.prefix.replace_range(start + count..end, language);
            }
        }
    }
    pub fn estimated_bytes(&self) -> usize { self.prefix.capacity() + self.suffix.capacity() }
    pub fn join(&self, body: &str) -> Result<String, ControlError> {
        let mut prefix = self.prefix.clone();
        let mut suffix = self.suffix.clone();
        if self.kind == ObjectKind::BlockMath && body.is_empty() && prefix.ends_with('\n') {
            // 清空已有公式后仍保持一个可识别的空块，不留下分隔段落的空行。
            if suffix.starts_with("\r\n") { suffix.drain(..2); }
            else if suffix.starts_with('\n') { suffix.remove(0); }
        }
        if self.marker.is_none() && prefix.ends_with('\n') && !body.is_empty() && !body.ends_with('\n') && suffix.starts_with("$$") {
            suffix.insert_str(0, if prefix.ends_with("\r\n") { "\r\n" } else { "\n" });
        }
        if let Some((marker, count)) = self.marker {
            if !body.is_empty() && !body.ends_with('\n') && !suffix.starts_with(['\r', '\n']) {
                suffix.insert_str(0, if prefix.ends_with("\r\n") { "\r\n" } else { "\n" });
            }
            let longest = body.lines().map(|line| line.trim_start().chars().take_while(|c| *c == marker).count()).max().unwrap_or(0);
            if longest >= count {
                let fence = marker.to_string().repeat(longest + 1);
                if let Some(start) = prefix.find(marker) {
                    prefix.replace_range(start..start + count, &fence);
                }
                if let Some(start) = suffix.find(marker) {
                    let length = suffix[start..].chars().take_while(|c| *c == marker).count();
                    suffix.replace_range(start..start + length, &fence);
                }
            }
        } else if self.kind == ObjectKind::BlockMath && body.contains("$$") {
            return Err(ControlError::Unsupported("公式内容不能包含块围栏".into()));
        }
        if self.kind == ObjectKind::BlockComment && body.contains("-->") {
            return Err(ControlError::Unsupported("注释内容不能包含 -->".into()));
        }
        if self.kind == ObjectKind::FrontMatter && body.lines().any(|line| matches!(line.trim(), "---" | "...")) {
            return Err(ControlError::Unsupported("属性内容不能包含独立的 Front Matter 围栏".into()));
        }
        if self.kind == ObjectKind::FrontMatter && !body.is_empty() && !body.ends_with('\n') && !suffix.starts_with(['\r', '\n']) {
            suffix.insert_str(0, if prefix.ends_with("\r\n") { "\r\n" } else { "\n" });
        }
        if self.kind == ObjectKind::InlineMath {
            if body.trim().is_empty() || body.contains(['\r', '\n']) { return Err(ControlError::Unsupported("请输入非空单行公式".into())); }
            let mut tex = String::new(); let mut slashes = 0;
            for ch in body.chars() {
                if ch == '$' && slashes % 2 == 0 { tex.push('\\'); }
                tex.push(ch); slashes = if ch == '\\' { slashes + 1 } else { 0 };
            }
            return Ok(format!("{prefix}{tex}{suffix}"));
        }
        Ok(format!("{prefix}{body}{suffix}"))
    }
}
impl ObjectDraft {
    pub fn begin(
        parsed: &ParsedWindow,
        range: Range<usize>,
        kind: ObjectKind,
    ) -> Result<Self, ControlError> {
        let source = parsed
            .source
            .get(range.clone())
            .ok_or(ControlError::InvalidRange)?;
        Ok(Self {
            identity: parsed.identity.clone(),
            kind,
            range,
            text: source.into(),
            original: source.into(),
        })
    }
    pub fn changed(&self) -> bool {
        self.text != self.original
    }
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.text.capacity()
            + self.original.capacity()
            + self.identity.document_id.capacity()
            + self.identity.window_id.capacity()
    }
    pub fn confirm(&self, parsed: &ParsedWindow) -> Result<EditResult, ControlError> {
        self.confirm_text(parsed, &self.text)
    }
    pub fn confirm_text(
        &self,
        parsed: &ParsedWindow,
        text: &str,
    ) -> Result<EditResult, ControlError> {
        if parsed.identity != self.identity {
            return Err(ControlError::Stale);
        }
        if parsed.source.get(self.range.clone()) != Some(&self.original) {
            return Err(ControlError::Stale);
        }
        let start = parsed.identity.from_byte + self.range.start as u64;
        Ok(EditResult {
            template_update: None,
            edits: if text != self.original {
                vec![SourceEdit {
                    from_byte: start,
                    to_byte: parsed.identity.from_byte + self.range.end as u64,
                    inserted_text: text.to_string(),
                }]
            } else {
                Vec::new()
            },
            selection: DocumentSelection::caret(start + text.len() as u64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_draft_cancel_is_noop_and_stale_confirm_is_rejected() {
        let mut parsed = crate::nomo_mark::parse::fixture("$x$ untouched\r\n");
        let mut draft = ObjectDraft::begin(&parsed, 0..3, ObjectKind::InlineMath).unwrap();
        assert!(draft.confirm(&parsed).unwrap().edits.is_empty());
        draft.text = "$y$".into();
        assert_eq!(draft.confirm(&parsed).unwrap().edits.len(), 1);
        parsed.identity.view_generation += 1;
        assert_eq!(draft.confirm(&parsed), Err(ControlError::Stale));
        drop(draft);
        assert_eq!(&*parsed.source, "$x$ untouched\r\n");
    }
}

//! 对象草稿具有独立确认/取消边界；只保存局部草稿，不创建第二套撤销历史。
use super::{commands::EditResult, parse::ParsedWindow, protocol::*};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Code,
    InlineMath,
    BlockMath,
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
        let mut parsed = crate::editor_control::parse::fixture("$x$ untouched\r\n");
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

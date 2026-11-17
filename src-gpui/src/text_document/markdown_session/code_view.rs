//! 会话内的代码显示偏好：只保存不可变片段位置，不持有正文或视图，也不写入编辑历史。
use super::*;
use std::collections::BTreeMap;

// 两种显示状态共用条目；连同 BTree 节点保守计入 256 KiB 上限，不淘汰用户的撤销记录。
const MAX_CODE_VIEW_STATES: usize = 4096;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CodeViewState {
    pub wrapped: bool,
    pub expanded: bool,
}

fn source_key(session: &Session, at: u64) -> TextDocumentResult<(bool, u64)> {
    let (piece, relative) = session.current.tree.piece_at(at)
        .ok_or_else(|| error("invalid-range", "代码块位置已失效"))?;
    Ok(match piece.source {
        PieceSource::Original { offset, .. } => (false, offset + relative),
        PieceSource::Added { offset, .. } => (true, offset + relative),
    })
}

impl MarkdownDocumentSessionManager {
    pub(crate) fn code_view_states_at(&self, owner: &str, document_id: &str, revision: u64, starts: &[u64]) -> TextDocumentResult<BTreeMap<u64, CodeViewState>> {
        let handle = self.session(document_id)?;
        let session = lock(&handle)?;
        session.owner(owner)?;
        if session.current.revision != revision {
            return Err(TextDocumentError::revision_conflict(revision, session.current.revision));
        }
        if session.code_view_states.is_empty() { return Ok(BTreeMap::new()); }
        starts.iter().filter_map(|at| match source_key(&session, *at) {
            Ok(key) => session.code_view_states.get(&key).map(|state| Ok((*at, *state))),
            Err(error) => Some(Err(error)),
        }).collect()
    }

    pub(crate) fn set_code_wrapped(&self, owner: &str, document_id: &str, revision: u64, at: u64, wrapped: bool) -> TextDocumentResult<()> {
        self.update_code_view_state(owner, document_id, revision, at, |state| state.wrapped = wrapped)
    }

    pub(crate) fn set_code_expanded(&self, owner: &str, document_id: &str, revision: u64, at: u64, expanded: bool) -> TextDocumentResult<()> {
        self.update_code_view_state(owner, document_id, revision, at, |state| state.expanded = expanded)
    }

    fn update_code_view_state(&self, owner: &str, document_id: &str, revision: u64, at: u64, update: impl FnOnce(&mut CodeViewState)) -> TextDocumentResult<()> {
        let handle = self.session(document_id)?;
        let mut session = lock(&handle)?;
        session.owner(owner)?;
        if session.current.revision != revision {
            return Err(TextDocumentError::revision_conflict(revision, session.current.revision));
        }
        let key = source_key(&session, at)?;
        let mut state = session.code_view_states.get(&key).copied().unwrap_or_default();
        update(&mut state);
        if state != CodeViewState::default() {
            if session.code_view_states.len() >= MAX_CODE_VIEW_STATES && !session.code_view_states.contains_key(&key) {
                return Err(error("view-state-budget", "当前会话的代码块显示状态已达上限，请先将部分代码块收起并关闭自动换行"));
            }
            session.code_view_states.insert(key, state);
        } else {
            session.code_view_states.remove(&key);
        }
        Ok(())
    }
}

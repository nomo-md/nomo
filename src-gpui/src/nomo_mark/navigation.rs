//! 键盘边界查询只使用当前片段，跨窗口时由宿主读取有界上下文。
use super::{native_port::NativeWindowPort, protocol::ControlError, session_window::WindowRequest};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorUnit { Grapheme, Word, Line }

pub fn boundary(text: &str, at: usize, unit: CursorUnit, forward: bool) -> usize {
    match (unit, forward) {
        (CursorUnit::Grapheme, true) => text.grapheme_indices(true).map(|(at, _)| at).find(|next| *next > at).unwrap_or(text.len()),
        (CursorUnit::Grapheme, false) => text[..at].grapheme_indices(true).next_back().map_or(0, |(at, _)| at),
        (CursorUnit::Word, true) => text[at..].split_word_bound_indices().find(|(_, word)| !word.trim().is_empty()).map_or(text.len(), |(offset, word)| at + offset + word.len()),
        (CursorUnit::Word, false) => text[..at].split_word_bound_indices().rev().find(|(_, word)| !word.trim().is_empty()).map_or(0, |(at, _)| at),
        (CursorUnit::Line, true) => text[at..].find(['\r', '\n']).map_or(text.len(), |offset| at + offset),
        (CursorUnit::Line, false) => text[..at].rfind('\n').map_or(0, |at| at + 1),
    }
}

impl NativeWindowPort {
    pub(crate) fn cursor_boundary(&self, revision: u64, at: u64, unit: CursorUnit, forward: bool, request: &WindowRequest) -> Result<u64, ControlError> {
        request.check()?;
        let status = self.status()?;
        if status.revision != revision { return Err(ControlError::Stale); }
        let total = status.byte_length;
        if at > total { return Err(ControlError::InvalidRange); }
        let limit = 64 * 1024;
        let range = if forward { at.saturating_sub(16)..at.saturating_add(limit).min(total) } else { at.saturating_sub(limit)..at.saturating_add(16).min(total) };
        let window = self.read_window(revision, range)?;
        let local = (at - window.from_byte) as usize;
        if !window.text.is_char_boundary(local) { return Err(ControlError::InvalidUtf8Boundary); }
        let next = boundary(&window.text, local, unit, forward);
        if (forward && next == window.text.len() && window.to_byte < total) || (!forward && next == 0 && window.from_byte > 0) {
            return Err(ControlError::NeedsContext("键盘边界超过 64 KiB 查询预算；原选区已保留".into()));
        }
        request.check()?;
        Ok(window.from_byte + next as u64)
    }
}

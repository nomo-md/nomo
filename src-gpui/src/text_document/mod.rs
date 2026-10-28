//! GPUI 开发宿主的原生文档会话入口；不依赖窗口平台，也不决定窗口范围。
mod native_types;
pub(crate) use native_types::{SegmentedEdit, TextDocumentError, TextDocumentResult};
mod chunk_reader;
mod edit_journal;
mod markdown_codec;
mod markdown_history;
mod markdown_search;
pub(crate) mod markdown_session;
mod piece_tree;
mod structure_index;
mod writing_stats;

pub(crate) use markdown_history::MarkdownSelection;
pub(crate) use structure_index::{Identity as StructureIdentity, Query as StructureQuery};
pub(crate) use structure_index::parser::{MAX_SEMANTIC_BYTES, normalize_label as normalize_reference_label};
pub(crate) use markdown_search::{SearchQuery, SearchMatch, ReplaceQuery};
pub(crate) use writing_stats::{StatsQuery, DocumentStats};

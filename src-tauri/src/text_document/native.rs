//! GPUI 开发宿主的窄接入入口。直接编译原版实现，不维护第二份会话或加载器。
//! 正式 Tauri 仍从 mod.rs 进入；这里不注册 Tauri 命令，也不决定窗口范围。
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

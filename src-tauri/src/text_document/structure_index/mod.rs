//! 唯一原生结构索引。解析、磁盘分页和调度独立于 WebView 与会话锁。
mod invalidation;
pub(super) mod parser;
pub(super) mod scanner;
mod service;
mod source_anchors;
mod store;
pub(super) use invalidation::Invalidation;
pub(super) use service::StructureService;
pub(crate) use store::{Identity, Query, Response};

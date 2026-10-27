//! Nomo 内部的窗口编辑控件。只消费宿主提供的片段，不读取文件或持有全文历史。
#[cfg(all(test, feature = "editor-control-demo"))]
mod acceptance;
pub mod mapping;
pub mod parse;
pub mod protocol;
// 只复用原版的纯语法边界函数，不链接或重建全局结构索引。
pub mod commands;
#[cfg(feature = "editor-control-demo")]
pub mod demo;
pub mod document_host;
mod dialect;
pub mod emoji;
pub mod host;
pub mod html_table;
mod image_properties;
pub mod interaction;
mod inline_commands;
mod semantic_selection;
mod semantic_global;
pub(crate) mod link_edit;
mod list_commands;
mod template_projection;
pub mod menu;
mod native_emoji;
pub mod native_port;
pub mod session_window;
mod viewport_model;
mod resident_budget;
#[path = "../text_document/structure_index/syntax.rs"]
pub mod nomo_syntax;
pub mod paint;
pub mod resources;
pub mod source_view;
pub mod table;
pub mod table_widths;
pub mod view;
pub mod widgets;

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
mod dialect;
pub mod emoji;
pub mod host;
pub mod html_table;
mod image_properties;
pub mod interaction;
mod inline_commands;
mod list_commands;
mod template_projection;
pub mod menu;
#[cfg(feature = "editor-control-demo")]
#[path = "../../../src-tauri/src/emoji_catalog/native.rs"]
mod native_emoji;
#[cfg(feature = "editor-control-demo")]
pub mod native_port;
#[cfg(feature = "editor-control-demo")]
pub mod session_window;
#[cfg(feature = "editor-control-demo")]
mod viewport_model;
#[cfg(feature = "editor-control-demo")]
mod resident_budget;
#[path = "../../../src-tauri/src/text_document/structure_index/syntax.rs"]
pub mod nomo_syntax;
pub mod paint;
pub mod resources;
pub mod source_view;
pub mod table;
pub mod table_widths;
pub mod view;
pub mod widgets;

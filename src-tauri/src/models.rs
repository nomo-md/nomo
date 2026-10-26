use serde::{Deserialize, Serialize};

#[path = "markdown_encoding.rs"]
mod markdown_encoding;
pub(crate) use markdown_encoding::MarkdownEncoding;

#[derive(Debug, Serialize)]
pub(crate) struct DocumentPayload {
    pub(crate) path: String,
    pub(crate) file_name: String,
    pub(crate) markdown: String,
    pub(crate) encoding: MarkdownEncoding,
    pub(crate) modified_at: i64,
    pub(crate) size_bytes: i64,
    pub(crate) readonly: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RecentEntryType {
    #[serde(rename = "file")]
    File,
    #[serde(rename = "folder")]
    Folder,
}

impl RecentEntryType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            RecentEntryType::File => "file",
            RecentEntryType::Folder => "folder",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecentEntry {
    pub(crate) path: String,
    pub(crate) entry_type: String,
    pub(crate) title: Option<String>,
    pub(crate) modified_at: i64,
    pub(crate) word_count: i64,
    pub(crate) opened_at: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecentEntryInput {
    pub(crate) path: String,
    pub(crate) entry_type: RecentEntryType,
    pub(crate) title: Option<String>,
    pub(crate) word_count: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotInput {
    pub(crate) path: String,
    pub(crate) markdown: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SnapshotRecord {
    pub(crate) id: String,
    pub(crate) document_path: String,
    pub(crate) content_hash: String,
    pub(crate) markdown: String,
    pub(crate) created_at: i64,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredSnapshotRecord {
    pub(crate) id: String,
    pub(crate) document_path: String,
    pub(crate) content_hash: String,
    pub(crate) created_at: i64,
    pub(crate) reason: String,
    #[serde(default, skip_serializing)]
    pub(crate) markdown: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceDraftInput {
    pub(crate) draft_id: Option<String>,
    pub(crate) markdown: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceDraftPayload {
    pub(crate) draft_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) markdown: Option<String>,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SettingInput {
    pub(crate) key: String,
    pub(crate) value_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SettingRecord {
    pub(crate) key: String,
    pub(crate) value_json: String,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct FileStatus {
    pub(crate) path: String,
    pub(crate) exists: bool,
    pub(crate) is_file: bool,
    pub(crate) modified_at: i64,
    pub(crate) size_bytes: i64,
    pub(crate) readonly: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct WindowStateInput {
    pub(crate) x: Option<i32>,
    pub(crate) y: Option<i32>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) maximized: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FolderFileInfo {
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExportHtmlInput {
    pub(crate) html_content: String,
    pub(crate) file_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PdfMarginsInput {
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
    pub(crate) left: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PdfOutlineEntry {
    pub(crate) marker_uri: String,
    pub(crate) title: String,
    pub(crate) level: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExportPdfInput {
    pub(crate) html_content: String,
    pub(crate) file_path: String,
    pub(crate) paper_size: Option<String>,
    pub(crate) orientation: Option<String>,
    pub(crate) margins: Option<PdfMarginsInput>,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) print_background: Option<bool>,
    #[serde(default)]
    pub(crate) outline: Vec<PdfOutlineEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportResult {
    pub(crate) file_path: String,
    pub(crate) bytes_written: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadFileInput {
    pub(crate) path: String,
    pub(crate) max_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Base64FileResult {
    pub(crate) data_url: String,
    pub(crate) mime_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FileTreeEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    pub(crate) has_children: bool,
    pub(crate) children_loaded: bool,
    pub(crate) children: Vec<FileTreeEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageAssetInput {
    pub(crate) document_path: String,
    pub(crate) document_file_name: String,
    pub(crate) strategy: String,
    pub(crate) file_name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageAssetPayload {
    pub(crate) markdown_src: String,
    pub(crate) absolute_path: String,
    pub(crate) reused: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageResolveInput {
    pub(crate) document_path: Option<String>,
    pub(crate) src: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageResolvePayload {
    pub(crate) src: String,
    pub(crate) display_src: String,
    pub(crate) exists: bool,
    pub(crate) absolute_path: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageDeleteInput {
    pub(crate) document_path: Option<String>,
    pub(crate) src: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageDeletePayload {
    pub(crate) src: String,
    pub(crate) removed: bool,
    pub(crate) skipped: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PicgoCoreUploadInput {
    pub(crate) file_name: String,
    pub(crate) command: String,
    pub(crate) config_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PicgoServerUploadInput {
    pub(crate) file_name: String,
    pub(crate) server_url: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageUploadPayload {
    pub(crate) url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PicgoConnectionTestInput {
    pub(crate) provider: String,
    pub(crate) server_url: Option<String>,
    pub(crate) command: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DesktopActionPayload {
    pub(crate) ok: bool,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct MarkdownAssociationStatus {
    pub(crate) supported: bool,
    pub(crate) registered: bool,
    pub(crate) is_default: bool,
    pub(crate) default_prog_id: Option<String>,
    #[serde(rename = "managedByPackage")]
    pub(crate) managed_by_package: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WindowLabelPayload {
    #[serde(rename = "windowLabel")]
    pub(crate) window_label: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WindowsContextMenuStatus {
    pub(crate) supported: bool,
    pub(crate) registered: bool,
    pub(crate) enabled: bool,
    #[serde(rename = "managedByPackage")]
    pub(crate) managed_by_package: bool,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyInstallerNotice {
    pub(crate) should_prompt: bool,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MarkdownEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    #[serde(rename = "utf-16le-bom")]
    Utf16LeBom,
    #[serde(rename = "utf-16be-bom")]
    Utf16BeBom,
    #[serde(rename = "gbk")]
    Gbk,
}

impl Default for MarkdownEncoding {
    fn default() -> Self {
        Self::Utf8
    }
}

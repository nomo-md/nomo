//! 与原版 markdown-document/protocol.ts 对齐的窗口、事务及生命周期契约。
pub use crate::markdown_templates::{InlineTemplate, TemplateUpdate};
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEdit {
    pub from_byte: u64,
    pub to_byte: u64,
    pub inserted_text: String,
}
/// 原版原生全文操作返回的范围变化；正文仍由原版会话持有。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRangeEdit {
    pub from_byte: u64,
    pub to_byte: u64,
    pub inserted_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSelection {
    pub anchor_byte: u64,
    pub head_byte: u64,
}
impl DocumentSelection {
    pub fn range(self) -> Range<u64> {
        self.anchor_byte.min(self.head_byte)..self.anchor_byte.max(self.head_byte)
    }
    pub fn caret(at: u64) -> Self {
        Self {
            anchor_byte: at,
            head_byte: at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentWindow {
    pub document_id: String,
    pub revision: u64,
    pub from_byte: u64,
    pub to_byte: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_templates: Vec<InlineTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowIdentity {
    pub document_id: String,
    pub window_id: String,
    pub revision: u64,
    pub view_generation: u64,
    /// 控件内部的乐观投影代数；不得推进宿主拥有的 viewGeneration。
    pub projection_generation: u64,
    pub from_byte: u64,
    pub to_byte: u64,
}
impl WindowIdentity {
    pub fn from_window(
        window: &DocumentWindow,
        window_id: impl Into<String>,
        view_generation: u64,
    ) -> Self {
        Self {
            document_id: window.document_id.clone(),
            window_id: window_id.into(),
            revision: window.revision,
            view_generation,
            projection_generation: 0,
            from_byte: window.from_byte,
            to_byte: window.to_byte,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTransaction {
    pub base_revision: u64,
    pub operation_id: String,
    pub history_group_id: String,
    pub edits: Vec<SourceEdit>,
    pub selection_before: DocumentSelection,
    pub selection_after: DocumentSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_update: Option<TemplateUpdate>,
}

/// 必须由宿主提供完整边界或上下文；不得根据加载片段猜测容器起点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowContext {
    Complete,
    Resolved(ParseContext),
    Missing { reason: String },
}
impl Default for WindowContext {
    fn default() -> Self {
        Self::Missing {
            reason: "宿主尚未提供解析上下文".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParseContext {
    /// 只有宿主明确保证片段位于可独立解析的边界时才允许语义编辑。
    pub structural_boundary: bool,
    pub references: std::collections::BTreeMap<String, ReferenceTarget>,
    pub ancestors: Vec<AncestorContainer>,
    pub inline_templates: Vec<InlineTemplate>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceTarget {
    pub destination: String,
    pub title: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AncestorContainer {
    Quote,
    ListItem { number: Option<u64> },
    Details { summary: String, open: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Preparing,
    Editable,
    ReadOnly,
    Suspended,
    Failed,
    Detached,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    InvalidRange,
    InvalidUtf8Boundary,
    NeedsContext(String),
    Stale,
    ReadOnly,
    Busy,
    Detached,
    HostFailure(String),
    Unsupported(String),
}
impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange => f.write_str("窗口或编辑范围无效"),
            Self::InvalidUtf8Boundary => f.write_str("位置不在 UTF-8 字符边界"),
            Self::NeedsContext(reason) => write!(f, "窗口上下文未就绪：{reason}"),
            Self::Stale => f.write_str("窗口映射已过期"),
            Self::ReadOnly => f.write_str("窗口当前不可编辑"),
            Self::Busy => f.write_str("窗口仍有组字、草稿或未确认编辑"),
            Self::Detached => f.write_str("窗口已卸载"),
            Self::HostFailure(reason) => write!(f, "宿主提交失败：{reason}"),
            Self::Unsupported(reason) => write!(f, "当前控件尚不支持：{reason}"),
        }
    }
}
impl std::error::Error for ControlError {}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutStamp {
    pub identity: WindowIdentity,
    pub width: f32,
    pub font_generation: u64,
    pub resource_generation: u64,
    pub zoom: f32,
}

/// GPUI 逻辑像素、当前应用窗口坐标。与 stamp 一起使用，禁止跨代复用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometryRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlEvent {
    Submit(DocumentTransaction),
    SelectionChanged {
        identity: WindowIdentity,
        selection: DocumentSelection,
    },
    Navigate {
        identity: WindowIdentity,
        source_byte: u64,
        focus: bool,
    },
    EditOutsideWindow {
        identity: WindowIdentity,
        selection: DocumentSelection,
        intent: super::commands::EditIntent,
    },
    Undo,
    Redo,
    SelectAll,
    WindowBoundary {
        identity: WindowIdentity,
        forward: bool,
        extend_selection: bool,
    },
    OpenLink(String),
    DraftChanged {
        identity: WindowIdentity,
        active: bool,
    },
    EmojiCatalogRequested {
        identity: WindowIdentity,
    },
    ClipboardRequest {
        identity: WindowIdentity,
        selection: DocumentSelection,
        cut: bool,
    },
    ResourceUsage {
        identity: WindowIdentity,
        parsed_bytes: usize,
        layout_bytes: usize,
        pending_bytes: usize,
    },
    ResourceDemand {
        identity: WindowIdentity,
        requests: Vec<super::resources::ResourceRequest>,
    },
    Released(WindowIdentity),
    LayoutReady {
        stamp: LayoutStamp,
        height: f32,
    },
    HeightChanged {
        stamp: LayoutStamp,
        height: f32,
    },
    Geometry {
        stamp: LayoutStamp,
        viewport: GeometryRect,
        anchor_byte: u64,
        anchor_y: f32,
        caret: Option<GeometryRect>,
        selection: Vec<GeometryRect>,
    },
    CompositionChanged {
        identity: WindowIdentity,
        active: bool,
    },
    NeedsContext(String),
    Failure(String),
}

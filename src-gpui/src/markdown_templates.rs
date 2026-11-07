//! 空格式模板的来源信息。只由显式格式命令建立；手写符号不推断为模板。
//! 此元数据跟随原版事务、历史和窗口，不包含正文或视图回调。
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InlineFormat {
    Strong,
    Emphasis,
    Code,
    Strike,
    Underline,
    Highlight,
    Superscript,
    Subscript,
    Link,
}
impl InlineFormat {
    pub fn markers(self) -> (&'static str, &'static str) {
        match self {
            Self::Strong => ("**", "**"),
            Self::Emphasis => ("*", "*"),
            Self::Code => ("`", "`"),
            Self::Strike => ("~~", "~~"),
            Self::Underline => ("<u>", "</u>"),
            Self::Highlight => ("<mark>", "</mark>"),
            Self::Superscript => ("^", "^"),
            Self::Subscript => ("~", "~"),
            Self::Link => ("[", "](https://)"),
        }
    }
    pub fn html_markers(self) -> (&'static str, &'static str) {
        match self {
            Self::Strong => ("<strong>", "</strong>"),
            Self::Emphasis => ("<em>", "</em>"),
            Self::Code => ("<code>", "</code>"),
            Self::Strike => ("<s>", "</s>"),
            Self::Underline => ("<u>", "</u>"),
            Self::Highlight => ("<mark>", "</mark>"),
            Self::Superscript => ("<sup>", "</sup>"),
            Self::Subscript => ("<sub>", "</sub>"),
            Self::Link => ("<a>", "</a>"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineTemplate {
    pub format: InlineFormat,
    pub from_byte: u64,
    pub open_end_byte: u64,
    pub close_start_byte: u64,
    pub to_byte: u64,
    #[serde(default)]
    pub html: bool,
}
impl InlineTemplate {
    pub fn markers(&self) -> (&'static str, &'static str) {
        if self.html {
            self.format.html_markers()
        } else {
            self.format.markers()
        }
    }
    pub fn valid_shape(&self) -> bool {
        let (open, close) = self.markers();
        self.format != InlineFormat::Link
            && self.from_byte.checked_add(open.len() as u64) == Some(self.open_end_byte)
            && self.close_start_byte.checked_add(close.len() as u64) == Some(self.to_byte)
            && self.open_end_byte <= self.close_start_byte
    }
    pub fn valid_in(&self, source: &str, base: u64) -> bool {
        if !self.valid_shape() || self.from_byte < base {
            return false;
        }
        let range = |a, b| {
            usize::try_from(a - base)
                .ok()
                .zip(usize::try_from(b - base).ok())
                .map(|(a, b)| a..b)
        };
        let (open, close) = self.markers();
        range(self.from_byte, self.open_end_byte).and_then(|r| source.get(r)) == Some(open)
            && range(self.close_start_byte, self.to_byte).and_then(|r| source.get(r)) == Some(close)
    }
}

/// Some(update) 表示显式模板命令：允许修改内层模板而保留外层来源。
/// 普通输入不携带此字段，触及模板内部即结束该空模板的来源状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateUpdate {
    #[serde(default)]
    pub add: Vec<InlineTemplate>,
}

/// edits 必须按实际执行顺序给出；同一原始坐标系中的补丁应从右往左传入。
pub fn map_templates(
    templates: &mut Vec<InlineTemplate>,
    edits: impl IntoIterator<Item = (u64, u64, u64)>,
    preserve_content: bool,
) {
    for (from, to, inserted) in edits {
        if from == to && inserted == 0 {
            continue;
        }
        let touches = |a, b| {
            if from == to {
                a < from && from < b
            } else {
                from < b && to > a
            }
        };
        let map = |at: u64, right: bool| -> Option<u64> {
            let next = if at < from || (at == from && !right) {
                at as i128
            } else if at > to || (at == to && (to != from || right)) {
                at as i128 + inserted as i128 - (to - from) as i128
            } else {
                from as i128 + if right { inserted as i128 } else { 0 }
            };
            u64::try_from(next).ok()
        };
        templates.retain_mut(|template| {
            if touches(template.from_byte, template.open_end_byte)
                || touches(template.close_start_byte, template.to_byte)
                || (!preserve_content && touches(template.from_byte, template.to_byte))
            {
                return false;
            }
            let positions = [
                map(template.from_byte, true),
                map(template.open_end_byte, false),
                map(template.close_start_byte, true),
                map(template.to_byte, false),
            ];
            let [Some(from), Some(open), Some(close), Some(to)] = positions else {
                return false;
            };
            template.from_byte = from;
            template.open_end_byte = open;
            template.close_start_byte = close;
            template.to_byte = to;
            template.valid_shape()
        });
    }
}

pub fn canonicalize(templates: &mut Vec<InlineTemplate>) {
    templates.sort_by_key(|t| (t.from_byte, std::cmp::Reverse(t.to_byte)));
    templates.dedup();
}

/// 模板内容只能为空或由其它显式空模板组成；禁止把普通正文登记为“空模板”。
pub fn valid_set(templates: &[InlineTemplate]) -> bool {
    let mut markers = Vec::with_capacity(templates.len() * 2);
    let mut blocks = std::collections::BTreeMap::new();
    for t in templates {
        if !t.valid_shape() {
            return false;
        }
        markers.extend([
            (t.from_byte, t.open_end_byte),
            (t.close_start_byte, t.to_byte),
        ]);
        if blocks.insert(t.from_byte, t.to_byte).is_some() {
            return false;
        }
    }
    markers.sort_unstable();
    if markers.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return false;
    }
    templates.iter().all(|t| {
        let mut at = t.open_end_byte;
        while at < t.close_start_byte {
            let Some(&end) = blocks.get(&at) else {
                return false;
            };
            if end > t.close_start_byte || end <= at {
                return false;
            }
            at = end;
        }
        true
    })
}

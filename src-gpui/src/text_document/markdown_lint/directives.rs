//! markdownlint 0.41.1 注释作用域；只识别 Markdown 语法中的真实 HTML 注释。
//! 文件级配置先合并，文件级开关次之，局部作用域最后按行覆盖；结果不持有正文。
//! 语义依据：https://github.com/DavidAnson/markdownlint/blob/v0.41.1/lib/markdownlint.mjs
use super::{
    scanner::{Syntax, checkpoint},
    *,
};
use pulldown_cmark::{Event, Tag, TagEnd};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt, time::Instant};

const MAX_DIRECTIVES: usize = 4096;
const MAX_CONFIGURATION_BYTES: usize = 64 * 1024;

// 编号、别名和标签均大小写无关；过时或未知名称与原版一样不匹配任何规则。
const NAMES: &[(u16, &[&str])] = &[
    (1, &["MD001", "heading-increment", "headings"]),
    (3, &["MD003", "heading-style", "headings"]),
    (4, &["MD004", "ul-style", "bullet", "ul"]),
    (5, &["MD005", "list-indent", "bullet", "indentation", "ul"]),
    (7, &["MD007", "ul-indent", "bullet", "indentation", "ul"]),
    (9, &["MD009", "no-trailing-spaces", "whitespace"]),
    (10, &["MD010", "no-hard-tabs", "hard_tab", "whitespace"]),
    (11, &["MD011", "no-reversed-links", "links"]),
    (
        12,
        &["MD012", "no-multiple-blanks", "blank_lines", "whitespace"],
    ),
    (13, &["MD013", "line-length", "line_length"]),
    (14, &["MD014", "commands-show-output", "code"]),
    (
        18,
        &["MD018", "no-missing-space-atx", "atx", "headings", "spaces"],
    ),
    (
        19,
        &[
            "MD019",
            "no-multiple-space-atx",
            "atx",
            "headings",
            "spaces",
        ],
    ),
    (
        20,
        &[
            "MD020",
            "no-missing-space-closed-atx",
            "atx_closed",
            "headings",
            "spaces",
        ],
    ),
    (
        21,
        &[
            "MD021",
            "no-multiple-space-closed-atx",
            "atx_closed",
            "headings",
            "spaces",
        ],
    ),
    (
        22,
        &["MD022", "blanks-around-headings", "blank_lines", "headings"],
    ),
    (23, &["MD023", "heading-start-left", "headings", "spaces"]),
    (24, &["MD024", "no-duplicate-heading", "headings"]),
    (25, &["MD025", "single-h1", "single-title", "headings"]),
    (26, &["MD026", "no-trailing-punctuation", "headings"]),
    (
        27,
        &[
            "MD027",
            "no-multiple-space-blockquote",
            "blockquote",
            "indentation",
            "whitespace",
        ],
    ),
    (
        28,
        &["MD028", "no-blanks-blockquote", "blockquote", "whitespace"],
    ),
    (29, &["MD029", "ol-prefix", "ol"]),
    (
        30,
        &["MD030", "list-marker-space", "ol", "ul", "whitespace"],
    ),
    (
        31,
        &["MD031", "blanks-around-fences", "blank_lines", "code"],
    ),
    (
        32,
        &[
            "MD032",
            "blanks-around-lists",
            "blank_lines",
            "bullet",
            "ol",
            "ul",
        ],
    ),
    (33, &["MD033", "no-inline-html", "html"]),
    (34, &["MD034", "no-bare-urls", "links", "url"]),
    (35, &["MD035", "hr-style", "hr"]),
    (
        36,
        &["MD036", "no-emphasis-as-heading", "emphasis", "headings"],
    ),
    (
        37,
        &["MD037", "no-space-in-emphasis", "emphasis", "whitespace"],
    ),
    (38, &["MD038", "no-space-in-code", "code", "whitespace"]),
    (39, &["MD039", "no-space-in-links", "links", "whitespace"]),
    (40, &["MD040", "fenced-code-language", "code", "language"]),
    (
        41,
        &["MD041", "first-line-h1", "first-line-heading", "headings"],
    ),
    (42, &["MD042", "no-empty-links", "links"]),
    (43, &["MD043", "required-headings", "headings"]),
    (44, &["MD044", "proper-names", "spelling"]),
    (45, &["MD045", "no-alt-text", "accessibility", "images"]),
    (46, &["MD046", "code-block-style", "code"]),
    (47, &["MD047", "single-trailing-newline", "blank_lines"]),
    (48, &["MD048", "code-fence-style", "code"]),
    (49, &["MD049", "emphasis-style", "emphasis"]),
    (50, &["MD050", "strong-style", "emphasis"]),
    (51, &["MD051", "link-fragments", "links"]),
    (52, &["MD052", "reference-links-images", "images", "links"]),
    (
        53,
        &[
            "MD053",
            "link-image-reference-definitions",
            "images",
            "links",
        ],
    ),
    (54, &["MD054", "link-image-style", "images", "links"]),
    (55, &["MD055", "table-pipe-style", "table"]),
    (56, &["MD056", "table-column-count", "table"]),
    (58, &["MD058", "blanks-around-tables", "table"]),
    (
        59,
        &["MD059", "descriptive-link-text", "accessibility", "links"],
    ),
    (60, &["MD060", "table-column-style", "table"]),
];
fn all_rules() -> u64 {
    RULES.iter().fold(0, |mask, rule| mask | (1_u64 << rule))
}
fn resolve(name: &str) -> u64 {
    NAMES
        .iter()
        .filter(|(_, aliases)| aliases.iter().any(|alias| alias.eq_ignore_ascii_case(name)))
        .fold(0, |mask, (rule, _)| mask | (1_u64 << rule))
}
fn selected(parameter: &str) -> u64 {
    if parameter.trim().is_empty() {
        all_rules()
    } else {
        parameter
            .split_whitespace()
            .fold(0, |mask, name| mask | resolve(name))
    }
}

#[derive(Default)]
pub(super) struct Configuration {
    pub lines: Vec<u64>,
    options: BTreeMap<u16, Map<String, Value>>,
    comments: Vec<Range<usize>>,
}
impl Configuration {
    pub fn value(&self, rule: u16, key: &str) -> Option<&Value> {
        self.options.get(&rule)?.get(key)
    }
    pub fn boolean(&self, rule: u16, key: &str, fallback: bool) -> bool {
        self.value(rule, key)
            .and_then(Value::as_bool)
            .unwrap_or(fallback)
    }
    pub fn number(&self, rule: u16, key: &str, fallback: usize) -> usize {
        self.value(rule, key)
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(fallback)
    }
    pub fn string<'a>(&'a self, rule: u16, key: &str, fallback: &'a str) -> &'a str {
        self.value(rule, key)
            .and_then(Value::as_str)
            .unwrap_or(fallback)
    }
    pub fn strings(&self, rule: u16, key: &str) -> Option<Vec<&str>> {
        self.value(rule, key)?
            .as_array()
            .map(|v| v.iter().filter_map(Value::as_str).collect())
    }
    pub fn in_comment(&self, byte: usize) -> bool {
        let index = self.comments.partition_point(|range| range.start <= byte);
        index > 0 && byte < self.comments[index - 1].end
    }
}

struct Directive<'a> {
    action: &'a str,
    parameter: &'a str,
    line: usize,
}
// 顶层对象顺序影响别名/标签覆盖；不能使用 serde_json 默认按键排序的 Map。
struct Entries(Vec<(String, Value)>);
impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Ordered;
        impl<'de> Visitor<'de> for Ordered {
            type Value = Entries;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON configuration object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Entries, M::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    merge(&mut entries, key, value);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(Ordered)
    }
}
fn merge(entries: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some((_, old)) = entries.iter_mut().find(|(old, _)| *old == key) {
        *old = value;
    } else {
        entries.push((key, value));
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64() != Some(0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub(super) fn read(
    s: &Syntax<'_>,
    profile: RuleSet,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<Configuration> {
    let mut comments = Vec::new();
    let mut in_block = 0;
    let mut literal = None;
    for (event, range) in &s.events {
        checkpoint(request, started)?;
        match event {
            Event::Start(Tag::HtmlBlock) => {
                html_comments(s.source, range.clone(), &mut literal, &mut comments);
                in_block += 1;
            }
            Event::End(TagEnd::HtmlBlock) => in_block -= 1,
            Event::InlineHtml(_) => {
                html_comments(s.source, range.clone(), &mut literal, &mut comments)
            }
            Event::Html(_) if in_block == 0 => {
                html_comments(s.source, range.clone(), &mut literal, &mut comments)
            }
            _ => {}
        }
    }
    let mut directives = Vec::new();
    let mut entries = vec![("default".to_owned(), Value::Bool(true))];
    if profile == RuleSet::Relaxed {
        for rule in RULES.iter().filter(|rule| !profile.enabled(**rule)) {
            entries.push((format!("MD{rule:03}"), Value::Bool(false)));
        }
    }
    let mut configuration_bytes = 0;
    comments.sort_by_key(|range| range.start);
    comments.dedup();
    for comment in &comments {
        checkpoint(request, started)?;
        let content = &s.source[comment.start + 4..comment.end - 3];
        let text = content.trim_start();
        let action_end = text.find(char::is_whitespace).unwrap_or(text.len());
        let word = &text[..action_end];
        let Some(prefix) = word.get(..13) else {
            continue;
        };
        if !prefix.eq_ignore_ascii_case("markdownlint-") {
            continue;
        }
        let action = &word[13..];
        let parameter = text[action_end..].trim();
        if action.eq_ignore_ascii_case("configure-file") {
            configuration_bytes += parameter.len();
            if configuration_bytes > MAX_CONFIGURATION_BYTES {
                return Err(TextDocumentError::new(
                    "lint-config-budget",
                    "文档内格式配置超过 64 KiB，已停止检查",
                ));
            }
            let parsed = serde_json::from_str::<Entries>(parameter).map_err(|error| {
                TextDocumentError::new(
                    "lint-config-invalid",
                    format!(
                        "第 {} 行的 markdownlint-configure-file 不是有效 JSON 对象：{error}",
                        s.line_at(comment.start) + 1
                    ),
                )
            })?;
            for (key, value) in parsed.0 {
                merge(&mut entries, key, value);
            }
        } else if s.line_at(comment.start) == s.line_at(comment.end - 1) {
            if [
                "disable",
                "enable",
                "capture",
                "restore",
                "disable-file",
                "enable-file",
                "disable-line",
                "disable-next-line",
            ]
            .iter()
            .any(|known| action.eq_ignore_ascii_case(known))
            {
                if directives.len() == MAX_DIRECTIVES {
                    return Err(TextDocumentError::new(
                        "lint-directive-budget",
                        "文档内格式控制指令超过 4096 条，已停止检查",
                    ));
                }
                directives.push(Directive {
                    action,
                    parameter,
                    line: s.line_at(comment.start),
                });
            }
        }
    }
    let default = entries
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("default"))
        .is_none_or(|(_, value)| truthy(value));
    let mut base = if default { all_rules() } else { 0 };
    let mut result = Configuration::default();
    result.comments = comments;
    for (key, value) in entries {
        checkpoint(request, started)?;
        let mask = resolve(&key);
        let enabled = value.get("enabled").map_or_else(|| truthy(&value), truthy);
        if enabled {
            base |= mask;
        } else {
            base &= !mask;
        }
        for &rule in RULES.iter().filter(|rule| mask & (1_u64 << **rule) != 0) {
            let mut options = value.as_object().cloned().unwrap_or_default();
            options.remove("enabled");
            options.remove("severity");
            result.options.insert(rule, options);
        }
    }
    let mut state = base;
    let mut captured = base;
    for directive in &directives {
        if directive.action.eq_ignore_ascii_case("enable-file") {
            state |= selected(directive.parameter);
        } else if directive.action.eq_ignore_ascii_case("disable-file") {
            state &= !selected(directive.parameter);
        }
    }
    result.lines = vec![state; s.lines.len()];
    let mut cursor = 0;
    for line in s.body_line..s.lines.len() {
        checkpoint(request, started)?;
        while cursor < directives.len() && directives[cursor].line == line {
            let directive = &directives[cursor];
            if directive.action.eq_ignore_ascii_case("capture") {
                captured = state;
            } else if directive.action.eq_ignore_ascii_case("restore") {
                state = captured;
            } else if directive.action.eq_ignore_ascii_case("enable") {
                state |= selected(directive.parameter);
            } else if directive.action.eq_ignore_ascii_case("disable") {
                state &= !selected(directive.parameter);
            }
            cursor += 1;
        }
        result.lines[line] = state;
    }
    for directive in directives {
        let next = usize::from(directive.action.eq_ignore_ascii_case("disable-next-line"));
        if next == 1 || directive.action.eq_ignore_ascii_case("disable-line") {
            if let Some(state) = result.lines.get_mut(directive.line + next) {
                *state &= !selected(directive.parameter);
            }
        }
    }
    let active = result.lines.iter().fold(0, |all, mask| all | mask);
    for (rule, options) in &result.options {
        if active & (1_u64 << rule) == 0 {
            continue;
        }
        for (key, value) in options {
            validate(*rule, key, value)?;
        }
    }
    Ok(result)
}

// HTML 范围由 Markdown parser 提供；标签属性、代码块、行内代码和 raw-text 元素内的伪注释不执行。
fn html_comments(
    source: &str,
    range: Range<usize>,
    literal: &mut Option<String>,
    comments: &mut Vec<Range<usize>>,
) {
    let mut at = range.start;
    while at < range.end {
        let Some(relative) = source[at..range.end].find('<') else {
            break;
        };
        at += relative;
        let tail = &source[at..range.end];
        if literal.is_none() && tail.starts_with("<!--") {
            let Some(end) = tail[4..].find("-->") else {
                break;
            };
            let end = at + 4 + end + 3;
            comments.push(at..end);
            at = end;
            continue;
        }
        let mut end = at + 1;
        let mut quoted = None;
        while end < range.end {
            let byte = source.as_bytes()[end];
            if quoted == Some(byte) {
                quoted = None;
            } else if quoted.is_none() {
                if matches!(byte, b'\'' | b'"') {
                    quoted = Some(byte);
                } else if byte == b'>' {
                    break;
                }
            }
            end += 1;
        }
        let text = &source[at + 1..end];
        let closing = text.starts_with('/');
        let name = text
            .trim_start_matches('/')
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .next()
            .unwrap_or("");
        if closing
            && literal
                .as_ref()
                .is_some_and(|old| old.eq_ignore_ascii_case(name))
        {
            *literal = None;
        } else if literal.is_none() && !closing && !text.trim_end().ends_with('/') {
            if [
                "script",
                "style",
                "pre",
                "code",
                "textarea",
                "title",
                "xmp",
                "iframe",
                "noembed",
                "noframes",
                "plaintext",
            ]
            .iter()
            .any(|tag| name.eq_ignore_ascii_case(tag))
            {
                *literal = Some(name.to_ascii_lowercase());
            }
        }
        at = end.saturating_add(1);
    }
}

// 配置开关/别名/标签完整支持。已实现的参数在此校验；其余非默认参数明确失败，避免错误的“检查通过”。
fn validate(rule: u16, key: &str, value: &Value) -> TextDocumentResult<()> {
    let supported = match (rule, key) {
        (3, "style") => choice(
            value,
            &[
                "consistent",
                "atx",
                "atx_closed",
                "setext",
                "setext_with_atx",
                "setext_with_atx_closed",
            ],
        ),
        (4, "style") => choice(
            value,
            &["consistent", "asterisk", "dash", "plus", "sublist"],
        ),
        (7, "indent" | "start_indent")
        | (9, "br_spaces")
        | (12, "maximum")
        | (13, "line_length" | "heading_line_length" | "code_block_line_length") => {
            value.as_u64().is_some()
        }
        (7, "start_indented")
        | (9 | 10 | 13, "code_blocks")
        | (13, "headings" | "strict" | "stern" | "tables")
        | (31, "list_items")
        | (40, "language_only")
        | (51, "ignore_case")
        | (54, "autolink" | "collapsed" | "full" | "inline" | "shortcut" | "url_inline") => {
            value.is_boolean()
        }
        (26 | 36, "punctuation") | (35, "style") => value.is_string(),
        (10, "ignore_code_languages")
        | (33, "allowed_elements" | "table_allowed_elements")
        | (40, "allowed_languages")
        | (53, "ignored_definitions")
        | (59, "prohibited_texts") => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        (29, "style") => choice(value, &["one", "one_or_ordered", "ordered", "zero"]),
        (46, "style") => choice(value, &["consistent", "fenced", "indented"]),
        (48, "style") => choice(value, &["consistent", "backtick", "tilde"]),
        (49 | 50, "style") => choice(value, &["consistent", "asterisk", "underscore"]),
        (55, "style") => choice(
            value,
            &[
                "consistent",
                "leading_and_trailing",
                "leading_only",
                "no_leading_or_trailing",
                "trailing_only",
            ],
        ),
        (60, "style") => choice(value, &["any", "aligned", "compact", "tight"]),
        // 这些参数当前只支持原版默认值。关闭相应规则时不阻止其余检查。
        (1 | 25 | 41, "front_matter_title") => value.as_str() == Some(r"^\s*title\s*[:=]"),
        (9, "list_item_empty_lines" | "strict")
        | (22, "include_front_matter")
        | (24, "siblings_only")
        | (41, "allow_preamble")
        | (43, "match_case")
        | (52, "shortcut_syntax")
        | (60, "aligned_delimiter") => value.as_bool() == Some(false),
        (10, "spaces_per_tab")
        | (22, "lines_above" | "lines_below")
        | (25 | 41, "level")
        | (30, "ol_multi" | "ol_single" | "ul_multi" | "ul_single") => value.as_u64() == Some(1),
        (27, "list_items") | (44, "code_blocks" | "html_elements") => value.as_bool() == Some(true),
        (43, "headings") | (44, "names") => value.as_array().is_some_and(Vec::is_empty),
        (51, "ignored_pattern") => value.as_str() == Some(""),
        (52, "ignored_labels") => value == &serde_json::json!(["x"]),
        // 原版忽略未知参数。
        _ => true,
    };
    if supported {
        Ok(())
    } else {
        Err(TextDocumentError::new(
            "lint-config-unsupported",
            format!(
                "MD{rule:03} 的配置参数 {key} 当前不受支持或值无效；请调整文档内 configure-file 配置后重查"
            ),
        ))
    }
}
fn choice(value: &Value, choices: &[&str]) -> bool {
    value.as_str().is_some_and(|value| choices.contains(&value))
}

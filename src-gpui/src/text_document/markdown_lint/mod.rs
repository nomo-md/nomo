//! 只读 Markdown 检查。仅处理一次固定修订，结果与临时正文均有预算。
//! 规则集合沿 markdownlint 0.41.1 默认参数；超限或不支持的文档配置明确跳过。
mod advanced;
mod directives;
mod rules;
mod scanner;
mod width;

use super::{TextDocumentError, TextDocumentResult};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

// 原版默认大文件阈值为 500,000。原生按 UTF-8 字节限制，中文文档会更早跳过。
// 只在后台任务内暂存该范围；全局最多一个扫描，没有常驻正文或解析树缓存。
pub(crate) const MAX_SOURCE_BYTES: usize = 500_000;
pub(super) const MAX_LINES: usize = 16_384;
pub(super) const MAX_EVENTS: usize = 32_768;
const MAX_ISSUES: usize = 200;
const MAX_RESULT_BYTES: usize = 256 * 1024;
const MAX_DETAIL_BYTES: usize = 512;

pub(crate) const RULES: &[u16] = &[
    1, 3, 4, 5, 7, 9, 10, 11, 12, 13, 14, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55,
    56, 58, 59, 60,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RuleSet {
    #[default]
    Relaxed,
    Default,
}
impl RuleSet {
    pub(crate) fn from_setting(value: &str) -> Self {
        if value == "default" {
            Self::Default
        } else {
            Self::Relaxed
        }
    }
    pub(crate) fn enabled(self, rule: u16) -> bool {
        self == Self::Default
            || !matches!(
                rule,
                7 | 9 | 10 | 12 | 13 | 27 | 28 | 30 | 33 | 34 | 37 | 38 | 39 | 40 | 41
            )
    }
    pub(crate) fn supported_count(self) -> usize {
        RULES.iter().filter(|rule| self.enabled(**rule)).count()
    }
}

#[derive(Clone, Default)]
pub(crate) struct Request(Arc<AtomicBool>);
impl Request {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn check(&self) -> TextDocumentResult<()> {
        if self.0.load(Ordering::Acquire) {
            Err(TextDocumentError::new("lint-cancelled", "格式检查已取消"))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Issue {
    pub(crate) rule: u16,
    pub(crate) line: u64,
    // 对齐原版显示列：1-based UTF-16；实际导航始终使用 UTF-8 字节范围。
    pub(crate) column: u64,
    pub(crate) range: Range<u64>,
    pub(crate) detail: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SkipReason {
    SourceBudget,
    LineBudget,
    SyntaxBudget,
    // 保留既有 UI 匹配分支；常用行内配置由 directives 执行，异常配置通过 error 返回。
    #[allow(dead_code)]
    InlineConfiguration,
}
#[derive(Clone, Debug)]
pub(crate) struct Report {
    pub(crate) document_id: String,
    pub(crate) revision: u64,
    pub(crate) rule_set: RuleSet,
    pub(crate) issues: Vec<Issue>,
    pub(crate) total: u64,
    pub(crate) skipped: Option<SkipReason>,
    pub(crate) checked_at: SystemTime,
}
impl Report {
    pub(super) fn new(document_id: String, revision: u64, rule_set: RuleSet) -> Self {
        Self {
            document_id,
            revision,
            rule_set,
            issues: Vec::new(),
            total: 0,
            skipped: None,
            checked_at: SystemTime::now(),
        }
    }
    pub(super) fn skip(mut self, reason: SkipReason) -> Self {
        self.skipped = Some(reason);
        self.issues.clear();
        self.total = 0;
        self
    }
}

pub(super) struct Sink<'a> {
    source: &'a str,
    lines: &'a [Range<usize>],
    enabled: &'a [u64],
    report: Report,
    bytes: usize,
    column_line: usize,
    column_byte: usize,
    column_units: u64,
}
impl<'a> Sink<'a> {
    fn new(source: &'a str, lines: &'a [Range<usize>], enabled: &'a [u64], report: Report) -> Self {
        Self {
            source,
            lines,
            enabled,
            report,
            bytes: 0,
            column_line: 0,
            column_byte: 0,
            column_units: 0,
        }
    }
    fn line_at(&self, byte: usize) -> usize {
        self.lines
            .partition_point(|line| line.start <= byte)
            .saturating_sub(1)
    }
    fn add(&mut self, rule: u16, range: Range<usize>, detail: impl AsRef<str>) {
        let mut from = range.start.min(self.source.len());
        let mut to = range.end.max(from).min(self.source.len());
        while !self.source.is_char_boundary(from) {
            from -= 1;
        }
        while !self.source.is_char_boundary(to) {
            to -= 1;
        }
        let index = self.line_at(from);
        if self.enabled[index] & (1_u64 << rule) == 0 {
            return;
        }
        // 无 errorRange 的块规则只定位首行；超长行只放置光标，避免为了诊断挂载巨型选区。
        to = to.min(self.lines[index].end.max(from));
        if to.saturating_sub(from) > 4096 {
            to = from;
        }
        // 最多保留 200 条，计数不因此停止。
        self.report.total += 1;
        if self.column_line != index || self.column_byte > from {
            self.column_line = index;
            self.column_byte = self.lines[index].start;
            self.column_units = 0;
        }
        self.column_units += self.source[self.column_byte..from].encode_utf16().count() as u64;
        self.column_byte = from;
        let column = self.column_units + 1;
        let key = (index as u64 + 1, rule, column);
        // 分组执行规则仍保留文档顺序最前的 200 条；不能让较晚扫描的标题问题被挤掉。
        if self.report.issues.len() == MAX_ISSUES {
            let latest = self
                .report
                .issues
                .iter()
                .enumerate()
                .max_by_key(|(_, issue)| (issue.line, issue.rule, issue.column))
                .map(|(index, issue)| (index, (issue.line, issue.rule, issue.column)))
                .unwrap();
            if key >= latest.1 {
                return;
            }
            let removed = self.report.issues.swap_remove(latest.0);
            self.bytes -= std::mem::size_of::<Issue>() + removed.detail.len();
        }
        let detail = truncate(detail.as_ref(), MAX_DETAIL_BYTES);
        let size = std::mem::size_of::<Issue>() + detail.len();
        if self.bytes + size > MAX_RESULT_BYTES {
            return;
        }
        self.bytes += size;
        self.report.issues.push(Issue {
            rule,
            line: index as u64 + 1,
            column,
            range: from as u64..to as u64,
            detail,
        });
    }
    fn finish(mut self) -> Report {
        self.report
            .issues
            .sort_by_key(|issue| (issue.line, issue.rule, issue.column));
        self.report.checked_at = SystemTime::now();
        self.report
    }
}
fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit.saturating_sub(3);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

pub(super) use scanner::scan;

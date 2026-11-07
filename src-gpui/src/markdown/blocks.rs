//! 顶层块索引：按源码行把文档切成可独立解析与渲染的块，是语义视图虚拟化和大纲的基础。
//!
//! 切分规则是 CommonMark 的保守子集：空行、ATX 标题、围栏代码、`$$` 公式和 front matter 形成边界，
//! 其余连续非空行（段落、列表、引用、HTML）合为一块交给 pulldown-cmark 解析。跨块语义
//! （引用式链接定义、被空行隔开的同一列表）不在块间传递，这是原型的已知差异。
//!
//! 增量更新：从编辑位置前一块开始重扫，直到新块起点越过编辑区、且与平移后的旧块起点重合时停止，
//! 拼接新块并平移其后的旧块。开闭围栏这类改变后文结构的编辑会自然扫到真正对齐处。

use crate::document::Document;
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    Fence,
    Math,
    FrontMatter,
    Table,
    Rule,
    Html,
}

/// 一个顶层块：起始行与行数（不含块前后的空行）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Block {
    pub start: usize,
    pub lines: usize,
    pub kind: BlockKind,
}

impl Block {
    pub fn end(&self) -> usize {
        self.start + self.lines
    }
}

pub trait Lines {
    fn count(&self) -> usize;
    fn line(&self, index: usize) -> Cow<'_, str>;
}

impl Lines for Document {
    fn count(&self) -> usize {
        self.line_count()
    }

    fn line(&self, index: usize) -> Cow<'_, str> {
        Cow::Owned(self.line_text(index))
    }
}

/// 全文快照上的行视图，用于首次全量扫描，避免逐行从 PieceTree 拷贝。
pub struct SnapshotLines {
    pub text: String,
    pub starts: Vec<usize>,
}

impl SnapshotLines {
    pub fn new(text: String) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|(_, byte)| *byte == b'\n').map(|(index, _)| index + 1));
        Self { text, starts }
    }
}

impl Lines for SnapshotLines {
    fn count(&self) -> usize {
        self.starts.len()
    }

    fn line(&self, index: usize) -> Cow<'_, str> {
        let start = self.starts[index];
        let mut end = self.starts.get(index + 1).map_or(self.text.len(), |next| next - 1);
        if end > start && self.text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        Cow::Borrowed(&self.text[start..end])
    }
}

#[derive(Default)]
pub struct BlockIndex {
    pub blocks: Vec<Block>,
}

impl BlockIndex {
    pub fn build(lines: &impl Lines) -> Self {
        let mut blocks = Vec::new();
        scan(lines, 0, &mut |block| {
            blocks.push(block);
            true
        });
        Self { blocks }
    }

    /// 只包含真正位于块内的行；块间空行返回 None。
    pub fn block_containing(&self, line: usize) -> Option<usize> {
        let after = self.blocks.partition_point(|block| block.start <= line);
        (after > 0 && self.blocks[after - 1].end() > line).then(|| after - 1)
    }

    /// 编辑后增量更新。`first_line..=old_last` 是旧文档中被替换的行，`new_last` 是新文档中对应的末行。
    pub fn apply_edit(&mut self, lines: &impl Lines, first_line: usize, old_last: usize, new_last: usize) {
        let delta = new_last as isize - old_last as isize;
        let anchor = self.blocks.partition_point(|block| block.start <= first_line).saturating_sub(1);
        let restart = anchor.saturating_sub(1);
        let scan_from = self.blocks.get(restart).map_or(0, |block| block.start.min(first_line));
        let mut fresh = Vec::new();
        let mut resume: Option<usize> = None;
        let old = &self.blocks;
        scan(lines, scan_from, &mut |block| {
            if block.start > new_last {
                let old_start = block.start as isize - delta;
                if old_start >= 0 {
                    let old_start = old_start as usize;
                    let at = old.partition_point(|candidate| candidate.start < old_start);
                    if at >= restart && old.get(at).is_some_and(|candidate| candidate.start == old_start && candidate.lines == block.lines && candidate.kind == block.kind) {
                        resume = Some(at);
                        return false;
                    }
                }
            }
            fresh.push(block);
            true
        });
        let tail_from = resume.unwrap_or(self.blocks.len());
        let mut tail: Vec<Block> = self.blocks[tail_from..]
            .iter()
            .map(|block| Block { start: (block.start as isize + delta) as usize, ..*block })
            .collect();
        self.blocks.truncate(restart.min(self.blocks.len()));
        // restart 之前的块可能与新扫描结果重叠（scan_from 取了 min），按起点去重。
        while self.blocks.last().is_some_and(|block| fresh.first().is_some_and(|first| block.start >= first.start)) {
            self.blocks.pop();
        }
        self.blocks.extend(fresh);
        self.blocks.append(&mut tail);
    }
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// 去掉最多 3 个前导空格；4 个及以上属于缩进代码或续行，返回 None。
fn strip_indent(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

pub fn atx_level(line: &str) -> Option<u8> {
    let rest = strip_indent(line)?;
    let hashes = rest.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &rest[hashes..];
    (after.is_empty() || after.starts_with(' ') || after.starts_with('\t')).then_some(hashes as u8)
}

fn fence_open(line: &str) -> Option<(u8, usize)> {
    let rest = strip_indent(line)?;
    let marker = *rest.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let count = rest.bytes().take_while(|byte| *byte == marker).count();
    if count < 3 {
        return None;
    }
    // 反引号围栏的信息串不能再含反引号。
    if marker == b'`' && rest[count..].contains('`') {
        return None;
    }
    Some((marker, count))
}

fn fence_close(line: &str, marker: u8, count: usize) -> bool {
    let Some(rest) = strip_indent(line) else { return false };
    let run = rest.bytes().take_while(|byte| *byte == marker).count();
    run >= count && rest[run..].trim().is_empty()
}

fn is_rule(line: &str) -> bool {
    let Some(rest) = strip_indent(line) else { return false };
    let compact: Vec<u8> = rest.bytes().filter(|byte| *byte != b' ' && *byte != b'\t').collect();
    compact.len() >= 3 && matches!(compact[0], b'-' | b'*' | b'_') && compact.iter().all(|byte| *byte == compact[0])
}

pub fn is_table_delimiter(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('-') || !(trimmed.contains('|') || trimmed.starts_with(':')) {
        return false;
    }
    trimmed
        .trim_matches('|')
        .split('|')
        .all(|cell| {
            let cell = cell.trim();
            !cell.is_empty() && cell.trim_matches(':').chars().all(|ch| ch == '-') && cell.contains('-')
        })
}

/// 从 `start` 行开始切块，每产生一块回调一次；回调返回 false 时停止。
fn scan(lines: &impl Lines, start: usize, emit: &mut impl FnMut(Block) -> bool) {
    let count = lines.count();
    let mut index = start;
    while index < count {
        let line = lines.line(index);
        if is_blank(&line) {
            index += 1;
            continue;
        }
        let begin = index;
        let kind;
        if begin == 0 && line.trim_end() == "---" {
            // front matter：首行 --- 到下一个 --- 或 ...；找不到闭合时退化为分隔线。
            let close = (1..count).find(|at| matches!(lines.line(*at).trim_end(), "---" | "..."));
            match close {
                Some(close) => {
                    index = close + 1;
                    kind = BlockKind::FrontMatter;
                }
                None => {
                    index += 1;
                    kind = BlockKind::Rule;
                }
            }
        } else if let Some((marker, fence)) = fence_open(&line) {
            index += 1;
            while index < count && !fence_close(&lines.line(index), marker, fence) {
                index += 1;
            }
            index = (index + 1).min(count);
            kind = BlockKind::Fence;
        } else if strip_indent(&line).is_some_and(|rest| rest.starts_with("$$")) {
            let rest = line.trim();
            index += 1;
            if !(rest.len() > 2 && rest[2..].trim_end().ends_with("$$")) {
                while index < count && !lines.line(index).contains("$$") {
                    index += 1;
                }
                index = (index + 1).min(count);
            }
            kind = BlockKind::Math;
        } else if let Some(level) = atx_level(&line) {
            index += 1;
            kind = BlockKind::Heading(level);
        } else if is_rule(&line) {
            index += 1;
            kind = BlockKind::Rule;
        } else {
            let html = strip_indent(&line).is_some_and(|rest| rest.starts_with('<'));
            index += 1;
            while index < count {
                let next = lines.line(index);
                if is_blank(&next) || atx_level(&next).is_some() || fence_open(&next).is_some() {
                    break;
                }
                index += 1;
            }
            let second = (index - begin >= 2).then(|| lines.line(begin + 1));
            kind = match second.as_deref() {
                Some(second) if is_table_delimiter(second) && line.contains('|') => BlockKind::Table,
                Some(second) if index - begin == 2 && !second.trim().is_empty() && second.trim().chars().all(|ch| ch == '=') => BlockKind::Heading(1),
                Some(second) if index - begin == 2 && second.trim().len() >= 2 && second.trim().chars().all(|ch| ch == '-') => BlockKind::Heading(2),
                _ if html => BlockKind::Html,
                _ => BlockKind::Paragraph,
            };
        }
        if !emit(Block { start: begin, lines: index - begin, kind }) {
            return;
        }
    }
}

/// 大纲条目：标题级别、所在行与纯文本标题。
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub line: usize,
    pub text: String,
}

pub fn heading_text(line: &str) -> String {
    let rest = line.trim_start().trim_start_matches('#').trim();
    let rest = rest.trim_end_matches('#').trim_end();
    strip_inline_markers(rest)
}

/// 去掉常见行内标记，供大纲和标签标题显示。
pub fn strip_inline_markers(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' | '_' | '`' | '~' => {}
            '[' => {}
            ']' => {
                // 跳过链接目标 (url)。
                if chars.peek() == Some(&'(') {
                    for next in chars.by_ref() {
                        if next == ')' {
                            break;
                        }
                    }
                }
            }
            _ => output.push(ch),
        }
    }
    output
}

impl BlockIndex {
    pub fn headings(&self, lines: &impl Lines) -> Vec<Heading> {
        self.blocks
            .iter()
            .filter_map(|block| match block.kind {
                BlockKind::Heading(level) => {
                    let text = if block.lines == 2 { strip_inline_markers(lines.line(block.start).trim()) } else { heading_text(&lines.line(block.start)) };
                    Some(Heading { level, line: block.start, text })
                }
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(usize, usize, BlockKind)> {
        BlockIndex::build(&SnapshotLines::new(text.to_string()))
            .blocks
            .iter()
            .map(|block| (block.start, block.lines, block.kind))
            .collect()
    }

    #[test]
    fn splits_common_structures() {
        let text = "---\ntitle: x\n---\n# 标题\n正文一\n正文二\n\n```rust\nfn a() {}\n\n```\n| a | b |\n| - | - |\n| 1 | 2 |\n\n$$\nx\n$$\n\n***\n\n- 项\n- 项";
        assert_eq!(
            kinds(text),
            vec![
                (0, 3, BlockKind::FrontMatter),
                (3, 1, BlockKind::Heading(1)),
                (4, 2, BlockKind::Paragraph),
                (7, 4, BlockKind::Fence),
                (11, 3, BlockKind::Table),
                (15, 3, BlockKind::Math),
                (19, 1, BlockKind::Rule),
                (21, 2, BlockKind::Paragraph),
            ]
        );
    }

    /// 在真实 Document 上做随机结构性编辑，每一步增量结果都必须等于全量重建。
    #[test]
    fn random_edits_on_document_match_full_rebuild() {
        use crate::document::Document;
        let fragments = ["\n", "\n\n", "```\n", "~~~", "# 标题\n", "$$\n", "| a | b |\n| - | - |\n", "- 项\n", "正文", "---\n", "> 引用\n", "", "x"];
        let mut seed: u64 = 0x5eed_1234;
        let mut next = move |bound: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % bound.max(1)
        };
        let initial = "---\ntitle: t\n---\n# 一\n段落\n\n```rust\nfn a() {}\n```\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n- 列表\n- 列表\n\n$$\nx\n$$\n\n尾段";
        let mut document = Document::from_bytes(initial.as_bytes().to_vec());
        let mut index = BlockIndex::build(&document);
        for step in 0..400 {
            let len = document.len() as usize;
            let text = document.read(0, document.len());
            let mut from = next(len + 1);
            while !text.is_char_boundary(from) {
                from -= 1;
            }
            let mut to = (from + next(12)).min(len);
            while !text.is_char_boundary(to) {
                to += 1;
            }
            let fragment = fragments[next(fragments.len())];
            let first_line = document.line_of_offset(from as u64);
            let old_last = document.line_of_offset(to as u64);
            document.replace(from as u64, to as u64, fragment).expect("replace");
            let new_last = document.line_of_offset((from + fragment.len()) as u64);
            index.apply_edit(&document, first_line, old_last, new_last);
            assert_eq!(index.blocks, BlockIndex::build(&document).blocks, "step {step}: replace {from}..{to} with {fragment:?}");
        }
    }

    #[test]
    fn incremental_update_matches_full_rebuild() {
        let cases = [
            ("a\n\nb\n\nc\n\nd", 2, 2, "```\nb"),
            ("a\n\nb\n\nc\n\nd", 1, 1, "x"),
            ("# h\npara\n\n```\ncode\n```\n\ntail", 3, 3, "text"),
            ("one\n\ntwo\n\nthree", 2, 2, "two\nmore\n\nnew"),
        ];
        for (original, first, last, replacement) in cases {
            let mut lines: Vec<String> = original.split('\n').map(str::to_string).collect();
            let mut index = BlockIndex::build(&SnapshotLines::new(original.to_string()));
            let inserted: Vec<String> = replacement.split('\n').map(str::to_string).collect();
            let new_last = first + inserted.len() - 1;
            lines.splice(first..=last, inserted);
            let edited = SnapshotLines::new(lines.join("\n"));
            index.apply_edit(&edited, first, last, new_last);
            assert_eq!(index.blocks, BlockIndex::build(&edited).blocks, "case {original:?} -> {replacement:?}");
        }
    }
}

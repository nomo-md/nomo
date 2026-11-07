//! 读取页不是 Markdown 块；保留未确认后缀，在有界语义片段之间推进。
use super::parser::{self, Batch, Block, Enhancements, MAX_SEMANTIC_BYTES};
use std::collections::BTreeMap;

#[derive(Default)]
struct Raw {
    kind: String,
    fence: Option<(u8, usize)>,
    fence_opened: bool,
    blank: bool,
    continued_line: bool,
    opaque: bool,
    html_end: Option<String>,
    block: Option<Block>,
}

pub struct Scanner {
    revision: u64,
    pending: String,
    from: u64,
    line: u64,
    raw: Option<Raw>,
    finished: bool,
    limit: usize,
    examined: usize,
}

fn prefix(source: &str, limit: usize) -> usize {
    let mut end = source.len().min(limit);
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    if source.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r')
        && source.as_bytes().get(end) == Some(&b'\n')
    {
        end -= 1;
    }
    end
}
fn line_end(source: &str, eof: bool) -> Option<usize> {
    let at = source.find(['\r', '\n'])?;
    if source.as_bytes()[at] == b'\r' {
        if at + 1 == source.len() && !eof {
            return None;
        }
        Some(at + 1 + usize::from(source.as_bytes().get(at + 1) == Some(&b'\n')))
    } else {
        Some(at + 1)
    }
}
fn fence(line: &str) -> Option<(u8, usize, bool)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = *trimmed.as_bytes().first()?;
    if !b"`~".contains(&marker) {
        return None;
    }
    let run = trimmed.bytes().take_while(|ch| *ch == marker).count();
    (run >= 3).then(|| (marker, run, trimmed[run..].trim().is_empty()))
}

impl Scanner {
    pub fn new(revision: u64, from: u64, line: u64) -> Self {
        Self {
            revision,
            pending: String::new(),
            from,
            line,
            raw: None,
            finished: false,
            limit: MAX_SEMANTIC_BYTES,
            examined: 0,
        }
    }
    pub fn consumed(&self) -> u64 {
        self.from
    }
    pub fn pending_bytes(&self) -> usize {
        self.pending.len()
    }
    fn consume(&mut self, end: usize) {
        self.from += end as u64;
        self.line += parser::line_count(&self.pending[..end]);
        self.pending.drain(..end);
        self.examined = 0;
    }
    /// 每次最多读入 16 KiB；返回定义增量，由索引服务维护全文首次有效定义。
    /// 调用方在每次 push 前后检查取消，单次同步解析不超过 32 KiB。
    pub fn push(&mut self, text: &str, eof: bool) -> Result<Batch, String> {
        if self.finished {
            return Err("结构扫描已结束".into());
        }
        if text.len() > 16 * 1024 {
            return Err("结构读取页超过 16 KiB".into());
        }
        self.pending.push_str(text);
        let mut blocks = vec![];
        let mut references = BTreeMap::new();
        while !self.pending.is_empty() {
            if self.raw.is_some() {
                if !self.consume_raw(&mut blocks, eof) {
                    break;
                }
                continue;
            }
            let oversized = self.pending.len() > self.limit;
            let mut end = prefix(&self.pending, self.limit);
            if !eof || oversized {
                let candidate = &self.pending[..end];
                end = candidate.rfind(['\r', '\n']).map(|at| at + 1).unwrap_or(0);
                if end > 0 && self.pending.as_bytes()[end - 1] == b'\r' {
                    end -= 1;
                }
            }
            if !eof && !oversized && end == self.examined {
                break;
            }
            self.examined = end;
            let candidate = &self.pending[..end];
            let mut cutoff = if eof && !oversized {
                end
            } else {
                parser::stable_cutoff(candidate, self.from == 0)
            };
            if cutoff == 0 && oversized {
                cutoff = parser::bullet_list_cutoff(candidate);
            }
            if cutoff > 0 {
                for mut block in
                    parser::parse_fragment(&candidate[..cutoff], self.revision, self.from == 0)?
                {
                    block.from_byte += self.from;
                    block.to_byte += self.from;
                    block.from_line += self.line - 1;
                    block.to_line += self.line - 1;
                    for h in &mut block.headings {
                        h.byte += self.from;
                        h.line += self.line - 1;
                    }
                    for note in &mut block.footnotes {
                        note.from_byte += self.from;
                        note.to_byte += self.from;
                    }
                    for (key, value) in &block.definitions {
                        references
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                    blocks.push(block);
                }
                self.consume(cutoff);
                continue;
            }
            if !oversized {
                break;
            }
            let kind = parser::parse_fragment(candidate, self.revision, self.from == 0)?
                .first()
                .map(|block| block.kind.clone())
                .unwrap_or_else(|| "unknown".into());
            let lower = candidate.trim_start().to_ascii_lowercase();
            let front = self.from == 0
                && (candidate.starts_with("---\n") || candidate.starts_with("---\r\n"));
            self.raw = Some(Raw {
                fence: if kind == "fence" {
                    fence(candidate.lines().next().unwrap_or(""))
                        .map(|(marker, length, _)| (marker, length))
                } else {
                    None
                },
                opaque: lower.starts_with("<details") || front,
                html_end: ["script", "pre", "style", "textarea"]
                    .iter()
                    .find(|tag| lower.starts_with(&format!("<{tag}")))
                    .map(|tag| format!("</{tag}"))
                    .or_else(|| {
                        lower
                            .starts_with("<!-- toc -->")
                            .then(|| "<!-- /toc -->".into())
                    }),
                kind,
                ..Raw::default()
            });
        }
        self.finished = eof && self.pending.is_empty();
        Ok(Batch {
            blocks,
            references,
            consumed_bytes: self.from,
            complete: self.finished,
        })
    }
    fn consume_raw(&mut self, output: &mut Vec<Block>, eof: bool) -> bool {
        let newline = line_end(&self.pending, eof);
        let length = newline.unwrap_or_else(|| {
            if eof {
                self.pending.len()
            } else if self.pending.len() > self.limit {
                prefix(&self.pending, self.limit)
            } else {
                0
            }
        });
        if length == 0 {
            return false;
        }
        let raw = self.raw.as_mut().unwrap();
        let line = &self.pending[..length];
        let whole_line = !raw.continued_line && (newline.is_some() || eof);
        let mut ends = false;
        if !raw.opaque {
            if let Some((marker, length)) = raw.fence {
                if whole_line {
                    if let Some((next, run, closing)) = fence(line) {
                        ends = raw.fence_opened && next == marker && run >= length && closing;
                        raw.fence_opened = true;
                    }
                }
            } else if let Some(end) = &raw.html_end {
                ends = line.to_ascii_lowercase().contains(end);
            } else if raw.kind != "unknown"
                && raw.block.is_some()
                && whole_line
                && raw.blank
                && !line.starts_with(char::is_whitespace)
            {
                let trimmed = line.trim();
                let list = trimmed.starts_with(['-', '+', '*', '>'])
                    || trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit());
                if !list {
                    self.raw = None;
                    return true;
                }
            }
        }
        let block = raw.block.get_or_insert_with(|| Block {
            id: parser::new_block_id(),
            version: self.revision,
            content_version: self.revision,
            from_byte: self.from,
            to_byte: self.from,
            from_line: self.line,
            to_line: self.line,
            kind: raw.kind.clone(),
            mode: "raw".into(),
            layout_units: 0,
            enhancement_objects: 0,
            enhancements: Enhancements::default(),
            metrics_complete: false,
            semantic_complete: false,
            headings: vec![],
            references: vec![],
            definitions: BTreeMap::new(),
            source_hash: None,
            footnotes: vec![],
        });
        block.to_byte = self.from + length as u64;
        block.to_line = self.line + parser::line_count(line);
        if output.last().is_some_and(|last| last.id == block.id) {
            *output.last_mut().unwrap() = block.clone();
        } else {
            output.push(block.clone());
        }
        raw.blank = whole_line && line.trim().is_empty();
        raw.continued_line = newline.is_none() && !eof;
        self.consume(length);
        if ends {
            self.raw = None;
        }
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scan(source: &str, page: usize, limit: usize) -> Vec<Block> {
        let mut scanner = Scanner::new(2, 0, 1);
        scanner.limit = limit;
        let mut at = 0;
        let mut result: Vec<Block> = vec![];
        while at < source.len() {
            let mut to = (at + page).min(source.len());
            while !source.is_char_boundary(to) {
                to += 1;
            }
            for block in scanner
                .push(&source[at..to], to == source.len())
                .unwrap()
                .blocks
            {
                if result.last().is_some_and(|last| last.id == block.id) {
                    *result.last_mut().unwrap() = block;
                } else {
                    result.push(block);
                }
            }
            assert!(scanner.pending_bytes() <= limit + 16384);
            at = to;
        }
        assert_eq!(scanner.consumed(), source.len() as u64);
        assert_eq!(result.first().unwrap().from_byte, 0);
        assert_eq!(result.last().unwrap().to_byte, source.len() as u64);
        for pair in result.windows(2) {
            assert_eq!(pair[0].to_byte, pair[1].from_byte);
        }
        result
    }
    #[test]
    fn giant_fence_is_bounded_and_resumes_after_real_closing_fence() {
        let source = format!("```\r\n{}\r\n```\r\n\r\n# 后续🙂\r\n", "a".repeat(200000));
        for page in [4096, 16384] {
            let blocks = scan(&source, page, 32768);
            assert_eq!(blocks[0].mode, "raw");
            assert_eq!(blocks.last().unwrap().headings[0].title, "后续🙂");
        }
    }
    #[test]
    fn oversized_details_does_not_invent_headings_from_ambiguous_suffix() {
        let source = format!(
            "<details>\n<summary>x</summary>\n{}\n\n# inside\n</details>\n",
            "a".repeat(40000)
        );
        let blocks = scan(&source, 16384, 32768);
        assert_eq!(blocks.len(), 1);
        assert!(!blocks[0].metrics_complete);
        assert!(blocks[0].headings.is_empty());
    }
    #[test]
    fn crlf_utf8_and_nested_containers_do_not_depend_on_read_pages() {
        let source = "# 中文🙂\r\n\r\n> 引用\r\n>\r\n> ## 子标题\r\n\r\n- list\r\n  continuation\r\n\r\n# End\r\n";
        let expected = scan(source, 16384, 32768);
        for page in [1, 3, 17] {
            let actual = scan(source, page, 32768);
            let facts = |blocks: Vec<Block>| {
                blocks
                    .into_iter()
                    .map(|b| {
                        (
                            b.from_byte,
                            b.to_byte,
                            b.from_line,
                            b.to_line,
                            b.kind,
                            b.headings,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(facts(actual), facts(expected.clone()));
        }
    }
}

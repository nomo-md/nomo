//! 阶段 0 的共同结构事实接口。没有应用会话依赖，不会改变生产索引。
//! 规范化的是两种内核的输出形状，不用正则补出内核没有识别的语义。
use super::{markdown_it, options, Engine};
use markdown_it::parser::core::Root;
use markdown_it::plugins::cmark::block::{heading::ATXHeading, lheading::SetextHeader,
    reference::{DefaultReferenceMap, ReferenceMap}};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Heading { level: u8, text: String, byte: usize, line: usize }
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Block { from_byte: usize, to_byte: usize, kind: String }
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
struct Definition { href: String, title: String }
#[derive(Debug, Serialize, Clone, Default, PartialEq, Eq)]
struct Facts {
    blocks: Vec<Block>,
    headings: Vec<Heading>,
    references: BTreeMap<String, Definition>,
}

fn normalize_label(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase()
}
fn line_at(source: &str, byte: usize) -> usize {
    let mut count = 1;
    let mut previous = 0;
    for ch in source[..byte].bytes() {
        if ch == b'\r' || (ch == b'\n' && previous != b'\r') { count += 1; }
        previous = ch;
    }
    count
}
fn line_start(source: &str, byte: usize) -> usize {
    source[..byte].rfind(['\r', '\n']).map(|at| at + 1).unwrap_or(0)
}
fn block_kind(tag: &Tag<'_>) -> &'static str {
    match tag {
        Tag::Heading { .. } => "heading_open", Tag::Paragraph => "paragraph_open",
        Tag::BlockQuote(_) => "blockquote_open", Tag::List(Some(_)) => "ordered_list_open",
        Tag::List(None) => "bullet_list_open", Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(_)) => "fence",
        Tag::CodeBlock(_) => "code_block", Tag::HtmlBlock => "html_block", Tag::Table(_) => "table_open",
        Tag::FootnoteDefinition(_) => "footnote_def_open", Tag::MetadataBlock(_) => "front_matter", _ => "fragment",
    }
}
fn push_heading(result: &mut Facts, source: &str, level: u8, text: String, byte: usize) {
    let byte = line_start(source, byte);
    result.headings.push(Heading { level, text: text.trim().replace(['\r', '\n'], " "), byte, line: line_at(source, byte) });
}
fn normalize_blocks(source: &str, starts: Vec<(usize, String)>) -> Vec<Block> {
    if source.is_empty() { return vec![]; }
    let mut starts = starts;
    starts.sort_by_key(|(byte, _)| *byte);
    starts.dedup_by_key(|(byte, _)| *byte);
    if starts.is_empty() { starts.push((0, "fragment".into())); }
    starts[0].0 = 0;
    starts.iter().enumerate().map(|(i, (from, kind))| Block {
        from_byte: *from, to_byte: starts.get(i + 1).map(|(byte, _)| *byte).unwrap_or(source.len()), kind: kind.clone(),
    }).collect()
}

fn analyze(engine: Engine, parser: &markdown_it::MarkdownIt, source: &str, extensions: bool) -> Facts {
    analyze_at(engine, parser, source, extensions, true)
}
fn analyze_at(engine: Engine, parser: &markdown_it::MarkdownIt, source: &str, extensions: bool, document_start: bool) -> Facts {
    if matches!(engine, Engine::PulldownCmark) && extensions {
        let blocks = super::nomo::parse_fragment(source, 1, document_start).expect("bounded experiment fragment");
        let mut facts = Facts::default();
        for block in blocks {
            facts.blocks.push(Block { from_byte: block.from_byte as usize, to_byte: block.to_byte as usize, kind: block.kind });
            facts.headings.extend(block.headings.into_iter().map(|h| Heading { level: h.level, text: h.title, byte: h.byte as usize, line: h.line as usize }));
            for (key, definition) in block.definitions { facts.references.entry(key).or_insert(Definition { href: definition.href, title: definition.title }); }
        }
        return facts;
    }
    let mut result = Facts::default();
    let mut starts = vec![];
    match engine {
        Engine::PulldownCmark => {
            let extra = if extensions { Options::ENABLE_FOOTNOTES | Options::ENABLE_MATH
                | Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS }
                else { Options::empty() };
            let parser = Parser::new_ext(source, options(true) | extra);
            for (label, definition) in parser.reference_definitions().iter() {
                result.references.insert(normalize_label(label), Definition {
                    href: definition.dest.to_string(), title: definition.title.as_deref().unwrap_or("").into() });
            }
            let mut depth = 0usize;
            let mut heading: Option<(u8, usize, String)> = None;
            for (event, range) in parser.into_offset_iter() {
                match event {
                    Event::Start(tag) => {
                        if depth == 0 { starts.push((line_start(source, range.start), block_kind(&tag).into())); }
                        depth += 1;
                        if let Tag::Heading { level, .. } = tag { heading = Some((level as u8, range.start, String::new())); }
                    }
                    Event::End(tag) => {
                        depth = depth.saturating_sub(1);
                        if matches!(tag, TagEnd::Heading(_)) {
                            if let Some((level, byte, text)) = heading.take() { push_heading(&mut result, source, level, text, byte); }
                        }
                    }
                    Event::Rule if depth == 0 => starts.push((range.start, "hr".into())),
                    Event::Text(text) | Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                        if let Some((_, _, title)) = &mut heading { title.push_str(&text); }
                    }
                    Event::SoftBreak | Event::HardBreak => if let Some((_, _, text)) = &mut heading { text.push(' '); },
                    _ => {},
                }
            }
        }
        Engine::MarkdownIt => {
            let root = parser.parse(source);
            if let Some(map) = root.cast::<Root>().and_then(|r| r.ext.get::<ReferenceMap>())
                .and_then(|r| r.downcast_ref::<DefaultReferenceMap>()) {
                for (label, href, title) in map.iter() {
                    result.references.insert(normalize_label(label), Definition { href: href.into(), title: title.unwrap_or("").into() });
                }
            }
            for child in &root.children {
                let name = child.name().rsplit("::").next().unwrap();
                if name == "Definition" { continue; }
                let kind = match name {
                    "ATXHeading" | "SetextHeader" => "heading_open", "Paragraph" => "paragraph_open",
                    "Blockquote" => "blockquote_open", "BulletList" => "bullet_list_open", "OrderedList" => "ordered_list_open",
                    "CodeFence" => "fence", "CodeBlock" => "code_block", "HtmlBlock" => "html_block", "Table" => "table_open",
                    "ThematicBreak" => "hr", _ => name,
                };
                if let Some(map) = child.srcmap { starts.push((line_start(source, map.get_byte_offsets().0), kind.into())); }
            }
            root.walk(|node, _| {
                let level = node.cast::<ATXHeading>().map(|h| h.level).or_else(|| node.cast::<SetextHeader>().map(|h| h.level));
                if let (Some(level), Some(map)) = (level, node.srcmap) {
                    push_heading(&mut result, source, level, node.collect_text(), map.get_byte_offsets().0);
                }
            });
        }
    }
    result.blocks = normalize_blocks(source, starts);
    result
}

/// 模拟当前有界扫描策略：只提交最后一个顶层块之前的稳定前缀。
/// 这是边界可行性探针；没有支持 Nomo 容器时必须显式报告差异，不能标为迁移通过。
fn scan(engine: Engine, parser: &markdown_it::MarkdownIt, source: &str, read_bytes: usize, extensions: bool) -> Facts {
    if matches!(engine, Engine::PulldownCmark) && extensions {
        let mut scanner = super::scanner::Scanner::new(1, 0, 1);
        let mut from = 0;
        let mut result = Facts::default();
        let mut previous_id = String::new();
        while from < source.len() {
            let mut end = (from + read_bytes.min(16380)).min(source.len());
            while !source.is_char_boundary(end) { end += 1; }
            let batch = scanner.push(&source[from..end], end == source.len()).unwrap();
            for block in batch.blocks {
                let item = Block { from_byte: block.from_byte as usize, to_byte: block.to_byte as usize, kind: block.kind };
                if block.id == previous_id { *result.blocks.last_mut().unwrap() = item; } else { result.blocks.push(item); }
                previous_id = block.id;
                result.headings.extend(block.headings.into_iter().map(|h| Heading { level: h.level, text: h.title, byte: h.byte as usize, line: h.line as usize }));
            }
            for (key, value) in batch.references { result.references.entry(key).or_insert(Definition { href: value.href, title: value.title }); }
            from = end;
        }
        return result;
    }
    let mut from = 0;
    let mut end = 0;
    let mut result = Facts::default();
    while from < source.len() {
        end = (end + read_bytes).min(source.len());
        while end < source.len() && !source.is_char_boundary(end) { end += 1; }
        let safe_end = if end == source.len() { end } else {
            let last = if source.as_bytes()[end - 1] == b'\r' { end - 1 } else { end };
            source[from..last].rfind(['\r', '\n']).map(|at| from + at + 1).unwrap_or(from)
        };
        if safe_end == from { continue; }
        let pending = &source[from..safe_end];
        let current = analyze_at(engine, parser, pending, extensions, from == 0);
        let cut = if end == source.len() { pending.len() } else {
            if matches!(engine, Engine::PulldownCmark) && extensions { super::nomo::stable_cutoff(pending, from == 0) }
            else { current.blocks.last().map(|block| block.from_byte).unwrap_or(0) }
        };
        if cut == 0 { continue; }
        let mut committed = analyze_at(engine, parser, &pending[..cut], extensions, from == 0);
        for block in &mut committed.blocks { block.from_byte += from; block.to_byte += from; }
        for heading in &mut committed.headings { heading.line = line_at(source, from + heading.byte); heading.byte += from; }
        result.blocks.extend(committed.blocks);
        result.headings.extend(committed.headings);
        for (key, value) in committed.references { result.references.entry(key).or_insert(value); }
        from += cut;
    }
    result
}

fn incremental_probe(engine: Engine, parser: &markdown_it::MarkdownIt) -> Value {
    let before = format!("# 标题\n\n{}[id]: /tail\n", "普通正文\n\n".repeat(3000));
    let after = before.replacen("标题", "新的标题", 1);
    let previous = scan(engine, parser, &before, 1024, true);
    let full = scan(engine, parser, &after, 1024, true);
    let delta = after.len() - before.len();
    let mut read = 1024.min(after.len());
    while !after.is_char_boundary(read) { read -= 1; }
    let first = analyze(engine, parser, &after[..read], true);
    let convergence = first.blocks.iter().skip(1).find_map(|block| {
        let old_from = block.from_byte.checked_sub(delta)?;
        let old_to = block.to_byte.checked_sub(delta)?;
        previous.blocks.iter().find(|old| old.from_byte == old_from && old.to_byte == old_to && old.kind == block.kind
            && before[old_from..old_to] == after[block.from_byte..block.to_byte]).map(|old| (block, old))
    });
    if let Some((block, old)) = convergence {
        let mut reused = first.blocks.iter().take_while(|b| b.from_byte < block.to_byte).cloned().collect::<Vec<_>>();
        reused.extend(previous.blocks.iter().filter(|b| b.from_byte >= old.to_byte).map(|b| Block {
            from_byte: b.from_byte + delta, to_byte: b.to_byte + delta, kind: b.kind.clone() }));
        json!({ "engine": engine, "inputBytes": before.len(), "rescannedBytes": read,
            "blockRangesMatchFull": reused == full.blocks, "reusedTailBlocks": previous.blocks.iter().filter(|b| b.from_byte >= old.to_byte).count(),
            "limitations": "range convergence only; no production IDs, reference invalidation, or disk-page implementation" })
    } else { json!({ "engine": engine, "converged": false }) }
}

fn cancellation_probe(engine: Engine, source: &str) -> Value {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let thread = scope.spawn(move || {
            let parser = markdown_it(true);
            started_tx.send(()).unwrap();
            // 故意在解析片段内请求取消，证明只能在返回后丢弃，不冒充可抢占解析器。
            let facts = analyze(engine, &parser, source, true);
            if flag.load(Ordering::Acquire) { None } else { Some(facts) }
        });
        started_rx.recv().unwrap();
        let requested = Instant::now();
        cancelled.store(true, Ordering::Release);
        let published = thread.join().unwrap();
        json!({ "engine": engine, "inputBytes": source.len(), "cancelToJoinMs": requested.elapsed().as_secs_f64()*1000.0,
            "publishedAfterCancel": published.is_some(), "boundary": "after parser returns; not internal preemption" })
    })
}

#[derive(Deserialize)]
struct Fixture { name: String, markdown: String, family: String }
pub fn evaluate() -> Value {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!("../nomo-fixtures.json")).unwrap();
    let parser = markdown_it(true);
    let mut runs = vec![];
    for fixture in &fixtures {
        for engine in [Engine::PulldownCmark, Engine::MarkdownIt] {
            let full = analyze(engine, &parser, &fixture.markdown, true);
            let chunks: Vec<_> = [1, 3, 17, 16384].iter().map(|size| {
                let chunked = scan(engine, &parser, &fixture.markdown, *size, true);
                json!({ "readBytes": size, "matchesFull": chunked == full, "facts": chunked })
            }).collect();
            runs.push(json!({ "name": fixture.name, "family": fixture.family, "engine": engine, "facts": full, "chunkRuns": chunks }));
        }
    }
    let source = "[".repeat(32 * 1024);
    let cancellation = [Engine::PulldownCmark, Engine::MarkdownIt].map(|engine| {
        json!({ "engine": engine, "rounds": (0..3).map(|_| cancellation_probe(engine, &source)).collect::<Vec<_>>() })
    });
    let incremental = [Engine::PulldownCmark, Engine::MarkdownIt].map(|engine| incremental_probe(engine, &parser));
    json!({ "status": "prototype; built-in extensions enabled where available; not a completed Nomo adapter",
        "fixtures": runs, "cancellation": cancellation, "incrementalRangeConvergence": incremental })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_ranges_cover_the_source_across_read_boundaries() {
        let parser = markdown_it(true);
        let source = "# 中文😀\r\n\r\n正文\r\n\r\n[id]: /target\r\n";
        for engine in [Engine::PulldownCmark, Engine::MarkdownIt] {
            let full = analyze(engine, &parser, source, true);
            for size in [1, 3, 17, 16384] {
                let chunked = scan(engine, &parser, source, size, true);
                assert_eq!(chunked, full, "{engine:?}, read {size}");
                let mut byte = 0;
                for block in chunked.blocks {
                    assert_eq!(block.from_byte, byte);
                    assert!(source.get(block.from_byte..block.to_byte).is_some());
                    byte = block.to_byte;
                }
                assert_eq!(byte, source.len());
                assert_eq!(chunked.headings[0].text, "中文😀");
                assert_eq!(chunked.headings[0].line, 1);
            }
        }
    }

    #[test]
    fn bounded_rescan_converges_to_full_block_ranges_after_prefix_edit() {
        let parser = markdown_it(true);
        for engine in [Engine::PulldownCmark, Engine::MarkdownIt] {
            let result = incremental_probe(engine, &parser);
            assert_eq!(result["blockRangesMatchFull"], true, "{engine:?}");
            assert!(result["rescannedBytes"].as_u64().unwrap() < 4096);
            assert!(result["reusedTailBlocks"].as_u64().unwrap() > 2000);
        }
    }
}

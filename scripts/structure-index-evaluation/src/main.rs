mod allocator;
mod probes;
mod structure;
mod reference_probes;
#[allow(dead_code)]
#[path = "../../../src-tauri/src/text_document/structure_index/parser.rs"]
mod nomo;
use nomo as parser;
#[allow(dead_code)]
#[path = "../../../src-tauri/src/text_document/structure_index/scanner.rs"]
mod scanner;
#[path = "../../../src-tauri/src/text_document/structure_index/gfm.rs"]
mod gfm;

use markdown_it::MarkdownIt;
use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

#[global_allocator]
static ALLOCATOR: allocator::Meter = allocator::Meter;

#[derive(Deserialize)]
struct Example {
    markdown: String,
    html: String,
    example: u32,
    section: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Engine { PulldownCmark, MarkdownIt }

fn options(gfm: bool) -> Options {
    if gfm { Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS }
    else { Options::empty() }
}

fn markdown_it(gfm: bool) -> MarkdownIt {
    let mut parser = MarkdownIt::new();
    markdown_it::plugins::cmark::add(&mut parser);
    markdown_it::plugins::html::add(&mut parser);
    if gfm {
        // 不调用 extra::add，避免不相关的 typographer/smartquotes 改变用户文本。
        markdown_it::plugins::extra::tables::add(&mut parser);
        markdown_it::plugins::extra::strikethrough::add(&mut parser);
        markdown_it::plugins::extra::linkify::add(&mut parser);
    }
    parser
}

fn render(engine: Engine, parser: &MarkdownIt, source: &str, gfm: bool) -> String {
    match engine {
        Engine::PulldownCmark => {
            let mut result = String::new();
            if gfm { html::push_html(&mut result, self::gfm::events(Parser::new_ext(source, options(true))).into_iter()); }
            else { html::push_html(&mut result, Parser::new_ext(source, options(false))); }
            result
        }
        Engine::MarkdownIt => parser.parse(source).xrender(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Difference {
    example: u32,
    section: String,
    markdown: String,
    expected: String,
    actual: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Suite {
    engine: Engine,
    corpus: String,
    total: usize,
    passed: usize,
    elapsed_ms: f64,
    max_parse_allocation_bytes: usize,
    differences: Vec<Difference>,
}

fn run_suite(engine: Engine, path: &Path, gfm: bool) -> Result<Suite, Box<dyn std::error::Error>> {
    let examples: Vec<Example> = serde_json::from_slice(&std::fs::read(path)?)?;
    let parser = markdown_it(gfm);
    let mut result = Suite { engine, corpus: path.file_name().unwrap().to_string_lossy().into(),
        total: 0, passed: 0, elapsed_ms: 0.0, max_parse_allocation_bytes: 0, differences: vec![] };
    for example in examples {
        // GFM 基础版本较旧，只测试显式选择的扩展；基础契约单独用 0.31.2。
        if gfm && !matches!(example.section.as_str(), "Tables (extension)" | "Task list items (extension)"
            | "Strikethrough (extension)" | "Autolinks (extension)" | "Disallowed Raw HTML (extension)") { continue; }
        let memory = allocator::start();
        let started = Instant::now();
        let actual = render(engine, &parser, &example.markdown, gfm);
        result.elapsed_ms += started.elapsed().as_secs_f64() * 1000.0;
        let (peak, _) = allocator::finish(memory);
        result.max_parse_allocation_bytes = result.max_parse_allocation_bytes.max(peak);
        result.total += 1;
        if actual == example.html { result.passed += 1; }
        else { result.differences.push(Difference { example: example.example, section: example.section,
            markdown: example.markdown, expected: example.html, actual }); }
    }
    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 { return Err("usage: evaluation <commonmark.json> <gfm.json> <report.json>".into()); }
    let mut suites = vec![];
    for engine in [Engine::PulldownCmark, Engine::MarkdownIt] {
        suites.push(run_suite(engine, Path::new(&args[1]), false)?);
        suites.push(run_suite(engine, Path::new(&args[2]), true)?);
    }
    let report = serde_json::json!({ "schemaVersion": 1,
        "purpose": "parser capability experiment; not production structure-index acceptance",
        "versions": { "pulldown-cmark": "0.13.5", "markdown-it": "0.6.1" },
        "suites": suites, "probes": probes::run(), "structureAdapters": structure::evaluate(),
        "globalReferenceProbes": reference_probes::run() });
    std::fs::write(&args[3], serde_json::to_vec_pretty(&report)?)?;
    for suite in &suites { println!("{:?} {}: {}/{} exact HTML matches, {:.2}ms", suite.engine, suite.corpus, suite.passed, suite.total, suite.elapsed_ms); }
    Ok(())
}

//! 同样的输入与事实结构用于两个内核；不把裸库输出伪装为 Nomo 扩展适配。
use super::*;
use pulldown_cmark::{Event, Tag};
use serde_json::{json, Value};

fn facts(engine: Engine, parser: &MarkdownIt, source: &str) -> Value {
    let mut ranges = vec![];
    match engine {
        Engine::PulldownCmark => {
            for (event, range) in Parser::new_ext(source, options(true)).into_offset_iter() {
                if let Event::Start(tag) = event {
                    let kind = match tag { Tag::Heading { .. } => "heading", Tag::Paragraph => "paragraph",
                        Tag::Link { .. } => "link", Tag::BlockQuote(_) => "quote", Tag::Table(_) => "table",
                        Tag::List(_) => "list", Tag::CodeBlock(_) => "code", Tag::HtmlBlock => "html", _ => continue };
                    ranges.push(json!({ "kind": kind, "from": range.start, "to": range.end,
                        "validUtf8Range": source.get(range.clone()).is_some(), "source": &source[range] }));
                }
            }
        }
        Engine::MarkdownIt => {
            let root = parser.parse(source);
            root.walk(|node, _| {
                if let Some(map) = node.srcmap {
                    let (from, to) = map.get_byte_offsets();
                    ranges.push(json!({ "kind": node.name(), "from": from, "to": to,
                        "validUtf8Range": source.get(from..to).is_some(), "source": source.get(from..to) }));
                }
            });
        }
    }
    json!({ "ranges": ranges, "html": render(engine, parser, source, true) })
}

pub fn run() -> Value {
    let fixtures = [
        ("unicode-crlf", "# 中文😀\r\n\r\n[文字][id]\r\n\r\n[id]: /first \"题\"\r\n"),
        ("definitions-first-wins", "[id]: /first\n\n[id]: /second\n\n[id]\n"),
        ("definitions-after-use", "[标签][id]\n\n[id]: /tail\n"),
        ("duplicate-headings", "# A\n\n# A\n\n# A-2\n\n# 😀\n"),
        ("details", "<details>\n<summary>标题</summary>\n\n# 内部\n\n</details>\n\n# 外部\n"),
        ("callout", "> [!NOTE]\n> # 内部标题\n> 正文\n"),
        ("footnote", "正文[^n]\n\n[^n]: 定义\n\n    后续段落\n"),
        ("front-matter", "---\ntitle: 示例\n---\n\n# 正文\n"),
        ("toc", "<!-- toc -->\n- [旧目录](#old)\n<!-- /toc -->\n\n# 新标题\n"),
        ("inline-extensions", "# H~2~O x^2^ $x$ ![alt](a.png){width=120}\n\n$$\nx\n$$\n"),
        ("literal-html", "<script>\n# 不应当作标题\n</script>\n\n# 正文\n"),
        ("setext", "中文😀\r\n====\r\n\r\n正文\r\n"),
    ];
    let parser = markdown_it(true);
    let mut cases = vec![];
    for (name, source) in fixtures {
        cases.push(json!({ "name": name, "source": source,
            "pulldownCmark": facts(Engine::PulldownCmark, &parser, source),
            "markdownIt": facts(Engine::MarkdownIt, &parser, source) }));
    }
    let mut benchmarks = vec![];
    for (name, unit) in [("headings", "# 中文😀\n\n"), ("brackets", "[[[[[[[[[[[[[[[["),
        ("nested-containers", "> > > > - **正文**\n"), ("long-inline", "正文***a*** ")] {
        let mut source = unit.repeat(32 * 1024 / unit.len());
        while source.len() > 32 * 1024 { source.pop(); }
        for engine in [Engine::PulldownCmark, Engine::MarkdownIt] {
            let _ = render(engine, &parser, &source, true);
            let mut rounds = vec![];
            for _ in 0..3 {
                let memory = allocator::start();
                let start = Instant::now();
                let result = render(engine, &parser, &source, true);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                let (peak, allocated) = allocator::finish(memory);
                std::hint::black_box(&result);
                rounds.push(json!({ "parseAndRenderMs": elapsed, "peakAdditionalAllocationBytes": peak,
                    "totalAllocatedBytes": allocated }));
            }
            benchmarks.push(json!({ "name": name, "engine": engine, "inputBytes": source.len(), "rounds": rounds }));
        }
    }
    json!({ "fixtures": cases, "boundedFragmentBenchmarks": benchmarks })
}

//! 当前代码块的有状态词法着色。语法库按需创建，结果缓存不持有视图、回调或文档会话。
use super::{Spans, style};
use crate::layout::{ColorRole, InlineStyle};
use std::{cell::RefCell, collections::VecDeque, hash::{Hash, Hasher}, sync::{Arc, OnceLock}};
use two_face::re_exports::syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 120;
const MAX_BLOCK_BYTES: usize = 256 * 1024;
const MAX_LINE_BYTES: usize = 8 * 1024;
const MAX_SPANS: usize = 32 * 1024;

struct Entry { key: u64, language: Box<str>, source: Box<str>, lines: Arc<Vec<Spans>>, bytes: usize }
#[derive(Default)]
struct Cache { entries: VecDeque<Entry>, bytes: usize }
thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_no_newlines)
}

fn language(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "plaintext" | "txt" => "text",
        "js" => "javascript", "ts" => "typescript", "py" => "python", "md" => "markdown",
        "rs" => "rust", "rb" => "ruby", "yml" => "yaml", "golang" => "go",
        "sh" | "shell" => "bash", "c++" => "cpp", "csharp" | "c#" => "cs",
        "powershell" | "ps" => "ps1", "docker" => "dockerfile", "gql" => "graphql",
        _ => return value,
    }.to_string()
}

/// 返回逐行、UTF-8 字节坐标的互不重叠样式；未知语言与超预算内容保持可读纯文本。
pub fn code_block(text: &str, lang: &str) -> Arc<Vec<Spans>> {
    let lang = language(lang);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut hash); text.hash(&mut hash);
    let key = hash.finish();
    if let Some(hit) = CACHE.with_borrow_mut(|cache| {
        let index = cache.entries.iter().position(|entry| entry.key == key && &*entry.language == lang && &*entry.source == text)?;
        let entry = cache.entries.remove(index)?;
        let lines = entry.lines.clone(); cache.entries.push_back(entry); Some(lines)
    }) { return hit; }
    let lines = Arc::new(tokenize(text, &lang));
    let bytes = std::mem::size_of::<Entry>() + text.len() + lang.len()
        + lines.capacity() * std::mem::size_of::<Spans>()
        + lines.iter().map(|line| line.capacity() * std::mem::size_of::<(std::ops::Range<usize>, InlineStyle)>()).sum::<usize>() + 64;
    if bytes <= MAX_CACHE_BYTES && text.len() <= MAX_BLOCK_BYTES {
        CACHE.with_borrow_mut(|cache| {
            while cache.bytes + bytes > MAX_CACHE_BYTES || cache.entries.len() >= MAX_CACHE_ENTRIES {
                let Some(old) = cache.entries.pop_front() else { break; }; cache.bytes -= old.bytes;
            }
            cache.bytes += bytes;
            cache.entries.push_back(Entry { key, language: lang.into(), source: text.into(), lines: lines.clone(), bytes });
        });
    }
    lines
}

/// 一次性导出复用词法规则，不让正文进入后台线程的常驻 LRU。
pub(crate) fn code_block_uncached(text: &str, lang: &str) -> Vec<Spans> { tokenize(text, &language(lang)) }

fn tokenize(text: &str, lang: &str) -> Vec<Spans> {
    let plain = || text.split('\n').map(|_| Vec::new()).collect();
    if lang == "text" || text.len() > MAX_BLOCK_BYTES { return plain(); }
    let syntaxes = syntaxes();
    let Some(syntax) = syntaxes.find_syntax_by_token(lang) else { return plain(); };
    let mut parser = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut lines = Vec::new();
    let mut count = 0usize;
    let mut stopped = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if stopped || line.len() > MAX_LINE_BYTES || count >= MAX_SPANS {
            stopped = true; lines.push(Vec::new()); continue;
        }
        let mut spans = Vec::new();
        match parser.parse_line(line, syntaxes) {
            Ok(changes) => {
                let mut at = 0;
                for (end, op) in changes {
                    if end < at || end > line.len() || !line.is_char_boundary(end) { stopped = true; break; }
                    push(&mut spans, at..end, token_style(&stack));
                    if stack.apply(&op).is_err() || stack.scopes.len() > 128 { stopped = true; break; }
                    at = end;
                }
                if !stopped { push(&mut spans, at..line.len(), token_style(&stack)); }
            }
            Err(error) => { eprintln!("代码高亮失败，保留纯文本：{error}"); stopped = true; }
        }
        if stopped || count.saturating_add(spans.len()) > MAX_SPANS { stopped = true; spans.clear(); }
        count += spans.len(); lines.push(spans);
    }
    lines
}

fn push(spans: &mut Spans, range: std::ops::Range<usize>, value: InlineStyle) {
    if range.is_empty() { return; }
    if let Some((previous, previous_style)) = spans.last_mut() {
        if *previous_style == value && previous.end == range.start { previous.end = range.end; return; }
    }
    spans.push((range, value));
}

fn token_style(stack: &ScopeStack) -> InlineStyle {
    static RULES: OnceLock<Vec<(Scope, InlineStyle)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| [
        ("punctuation.definition.comment", ColorRole::Comment), ("punctuation.definition.string", ColorRole::Str),
        ("comment", ColorRole::Comment), ("string", ColorRole::Str),
        ("constant.numeric", ColorRole::Number), ("constant.language", ColorRole::Number),
        ("constant.character", ColorRole::Number), ("constant.other", ColorRole::Number),
        ("keyword", ColorRole::Keyword), ("storage", ColorRole::Keyword),
        ("entity.name.function", ColorRole::CodeFunction), ("support.function", ColorRole::CodeFunction),
        ("entity.name.type", ColorRole::CodeFunction), ("entity.name.class", ColorRole::CodeFunction),
        ("entity.name.tag", ColorRole::CodeTag), ("entity.other.attribute-name", ColorRole::Number),
        ("support.type", ColorRole::Number), ("support.class", ColorRole::Number),
        ("variable.language", ColorRole::Number), ("variable.parameter.function", ColorRole::CodeFg),
        ("variable.parameter", ColorRole::CodeParameter), ("meta.property-name", ColorRole::Number),
        ("markup.inserted", ColorRole::CodeInserted), ("markup.deleted", ColorRole::CodeDeleted),
        ("punctuation", ColorRole::Punct),
    ].into_iter().map(|(scope, color)| (Scope::new(scope).expect("static scope"), style(color))).collect());
    for scope in stack.scopes.iter().rev() {
        if let Some((_, style)) = rules.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope)) { return *style; }
    }
    style(ColorRole::CodeFg)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn color_at(lines: &[Spans], line: usize, byte: usize) -> ColorRole {
        lines[line].iter().find(|(range, _)| range.contains(&byte)).map_or(ColorRole::CodeFg, |(_, style)| style.color)
    }

    #[test]
    fn keeps_multiline_comment_raw_string_and_python_string_state() {
        let rust = "/* 中文注释\nfn ignored\n*/ let s = r#\"第一行\n第二行\"#;";
        let lines = code_block(rust, "rust");
        assert_eq!(color_at(&lines, 1, 0), ColorRole::Comment);
        assert_eq!(color_at(&lines, 2, 3), ColorRole::Keyword);
        assert_eq!(color_at(&lines, 3, 0), ColorRole::Str);
        let python = code_block("s = \"\"\"first\nsecond ' still\n\"\"\"\nx = 1", "py");
        assert_eq!(color_at(&python, 1, 0), ColorRole::Str);
        assert_eq!(color_at(&python, 3, 4), ColorRole::Number);
    }

    #[test]
    fn grammar_language_aliases_and_plain_text_are_isolated() {
        let text = "const value: string = `你好\n${1 + 2}`;";
        let typed = code_block(text, "ts");
        assert!(Arc::ptr_eq(&typed, &code_block(text, "typescript")));
        assert_eq!(color_at(&typed, 0, 0), ColorRole::Keyword);
        assert!(code_block(text, "text").iter().all(Vec::is_empty));
        assert!(code_block(text, "nomo-unknown-language").iter().all(Vec::is_empty));
        for (line, spans) in text.split('\n').zip(typed.iter()) {
            assert!(spans.windows(2).all(|pair| pair[0].0.end <= pair[1].0.start));
            assert!(spans.iter().all(|(range, _)| line.is_char_boundary(range.start) && line.is_char_boundary(range.end)));
        }
    }

    #[test]
    fn large_lines_stop_coloring_and_cache_evicts_old_sources() {
        CACHE.with_borrow_mut(|cache| *cache = Cache::default());
        let first = format!("// first\n{}", "x".repeat(64 * 1024));
        let lines = code_block(&first, "rust");
        assert!(lines[1].is_empty());
        for i in 0..140 { code_block(&format!("// {i}\n{}", "x".repeat(64 * 1024)), "rust"); }
        CACHE.with_borrow(|cache| {
            assert!(cache.bytes <= MAX_CACHE_BYTES);
            assert!(cache.entries.len() <= MAX_CACHE_ENTRIES);
            assert!(cache.entries.iter().all(|entry| &*entry.source != first));
        });
        assert!(code_block(&"x".repeat(MAX_BLOCK_BYTES + 1), "rust").iter().all(Vec::is_empty));
    }
}

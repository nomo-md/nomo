//! 显式 GFM 扩展。链接识别只作用于普通文本，代码及已有链接保持原义。
use pulldown_cmark::{CowStr, Event, LinkType, Tag, TagEnd};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct AutoLink { pub from: usize, pub to: usize, pub href: String }
pub fn autolinks(text: &str) -> Vec<AutoLink> {
    static CANDIDATES: OnceLock<Regex> = OnceLock::new();
    let regex = CANDIDATES.get_or_init(|| Regex::new(r"(?:https?://|ftp://|www\.)[^\s<]+|[\p{L}\p{N}.\-+_]+@[\p{L}\p{N}_-]+(?:\.[\p{L}\p{N}_-]+)+\.?").unwrap());
    let mut result = vec![];
    for found in regex.find_iter(text) {
        let from = found.start();
        let candidate = found.as_str();
        let email = !["http://", "https://", "ftp://", "www."].iter().any(|prefix| candidate.starts_with(prefix));
        if !email && from > 0 && !text[..from].chars().next_back().is_some_and(|ch| ch.is_whitespace() || "*_~(".contains(ch)) { continue; }
        let mut end = candidate.len();
        if email {
            if candidate.ends_with(['-', '_']) { continue; }
            if candidate.ends_with('.') { end -= 1; }
        } else {
            end = candidate.trim_end_matches(['?', '!', '.', ',', ':', '*', '_', '~']).len();
            if candidate[..end].ends_with(')') {
                let opens = candidate[..end].bytes().filter(|ch| *ch == b'(').count();
                let closes = candidate[..end].bytes().filter(|ch| *ch == b')').count();
                let mut unmatched = closes.saturating_sub(opens);
                while unmatched > 0 && candidate[..end].ends_with(')') { end -= 1; unmatched -= 1; }
            }
            if candidate[..end].ends_with(';') {
                if let Some(amp) = candidate[..end].rfind('&') {
                    if end > amp + 2 && candidate[amp + 1..end - 1].chars().all(char::is_alphanumeric) { end = amp; }
                }
            }
            let scheme_end = candidate.find("://").map(|at| at + 3).unwrap_or(0);
            let domain = candidate[scheme_end..end].split(['/', '?', '#', ':']).next().unwrap_or("");
            let segments: Vec<_> = domain.split('.').collect();
            if segments.len() < 2 || segments.iter().any(|segment| segment.is_empty() || !segment.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_'))
                || segments.iter().rev().take(2).any(|segment| segment.contains('_')) { continue; }
        }
        if end == 0 { continue; }
        let visible = &candidate[..end];
        let href = if email { format!("mailto:{visible}") } else if visible.starts_with("www.") { format!("http://{visible}") } else { visible.to_owned() };
        result.push(AutoLink { from, to: from + end, href });
    }
    result
}

pub fn filter_html(source: &str) -> String {
    static FORBIDDEN: OnceLock<Regex> = OnceLock::new();
    let regex = FORBIDDEN.get_or_init(|| Regex::new(r"(?i)</?(?:title|textarea|style|xmp|iframe|noembed|noframes|script|plaintext)(?:[\s/>])").unwrap());
    let mut result = String::new();
    let mut from = 0;
    for found in regex.find_iter(source) {
        result.push_str(&source[from..found.start()]); result.push_str("&lt;"); result.push_str(&source[found.start() + 1..found.end()]); from = found.end();
    }
    result.push_str(&source[from..]); result
}

pub fn events<'a>(input: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    fn flush<'a>(pending: &mut String, output: &mut Vec<Event<'a>>) {
        if pending.is_empty() { return; }
        let mut from = 0;
        for link in autolinks(pending) {
            if from < link.from { output.push(Event::Text(pending[from..link.from].to_owned().into())); }
            output.push(Event::Start(Tag::Link { link_type: LinkType::Autolink, dest_url: link.href.into(), title: CowStr::Borrowed(""), id: CowStr::Borrowed("") }));
            output.push(Event::Text(pending[link.from..link.to].to_owned().into()));
            output.push(Event::End(TagEnd::Link)); from = link.to;
        }
        if from < pending.len() { output.push(Event::Text(pending[from..].to_owned().into())); }
        pending.clear();
    }
    let mut result = vec![];
    let mut pending = String::new();
    let mut protected = 0usize;
    for event in input {
        if let Event::Text(ref text) = event {
            if protected == 0 { pending.push_str(text); continue; }
        }
        flush(&mut pending, &mut result);
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)) => protected += 1,
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => protected = protected.saturating_sub(1),
            _ => {},
        }
        result.push(match event {
            Event::Html(text) => Event::Html(filter_html(&text).into()),
            Event::InlineHtml(text) => Event::InlineHtml(filter_html(&text).into()),
            event => event,
        });
    }
    flush(&mut pending, &mut result); result
}

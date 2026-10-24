//! 全局定义优先级是分片解析的硬契约；不能只验证片段内部 first-wins。
use markdown_it::{MarkdownIt, Node};
use markdown_it::parser::{block::builtin::BlockParserRule, core::{CoreRule, Root}};
use markdown_it::plugins::cmark::block::reference::{CustomReferenceMap, DefaultReferenceMap, ReferenceMap};
use pulldown_cmark::{html, BrokenLink, CowStr, Event, LinkType, Parser, Tag};
use serde_json::{json, Value};

#[derive(Debug, Default)]
struct GlobalFirst(DefaultReferenceMap);
impl CustomReferenceMap for GlobalFirst {
    fn get(&self, label: &str) -> Option<(&str, Option<&str>)> {
        if label.eq_ignore_ascii_case("id") { Some(("/global-first", Some("global"))) }
        else { self.0.get(label) }
    }
    fn insert(&mut self, label: String, destination: String, title: Option<String>) -> bool {
        self.0.insert(label, destination, title)
    }
}
struct Install;
impl CoreRule for Install {
    fn run(root: &mut Node, _: &MarkdownIt) {
        root.cast_mut::<Root>().unwrap().ext.insert(ReferenceMap::new(GlobalFirst::default()));
    }
}

pub fn run() -> Value {
    let mut markdown_it = super::markdown_it(true);
    markdown_it.add_rule::<Install>().before::<BlockParserRule>();
    [
        ("external-definition", "[显示][id]\n"),
        ("local-duplicate-cannot-shadow-earlier-global", "[id]: /local \"local\"\n\n[显示][id]\n"),
        ("inline-link-not-overridden", "[显示](/explicit)\n"),
        ("escaped-first-label-leaves-shortcut-reference", "\\[显示][id]\n"),
    ].map(|(name, source)| {
        let mut callback_count = 0;
        let mut callback = |link: BrokenLink<'_>| -> Option<(CowStr<'static>, CowStr<'static>)> {
            callback_count += 1;
            if link.reference.eq_ignore_ascii_case("id") { Some(("/global-first".into(), "global".into())) } else { None }
        };
        let parser = Parser::new_with_broken_link_callback(source, super::options(true), Some(&mut callback));
        let mut native = String::new();
        html::push_html(&mut native, parser);
        // 证明适配可行：同时处理已解析 reference 事件，而非只依赖 broken-link callback。
        // 保留原事件源码位置；显式内联 URL 不进入全局定义查询。
        let mut callback = |link: BrokenLink<'_>| -> Option<(CowStr<'static>, CowStr<'static>)> {
            link.reference.eq_ignore_ascii_case("id").then(|| ("/global-first".into(), "global".into()))
        };
        let events = Parser::new_with_broken_link_callback(source, super::options(true), Some(&mut callback)).map(|event| {
            match event {
                Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                    let reference = matches!(link_type, LinkType::Reference | LinkType::ReferenceUnknown
                        | LinkType::Collapsed | LinkType::CollapsedUnknown | LinkType::Shortcut | LinkType::ShortcutUnknown);
                    if reference && id.eq_ignore_ascii_case("id") {
                        Event::Start(Tag::Link { link_type, dest_url: "/global-first".into(), title: "global".into(), id })
                    } else { Event::Start(Tag::Link { link_type, dest_url, title, id }) }
                }
                event => event,
            }
        });
        let mut adapted = String::new();
        html::push_html(&mut adapted, events);
        let other = markdown_it.parse(source).xrender();
        json!({ "name": name, "source": source, "pulldownCallbackOnly": native,
            "pulldownCallbackCalls": callback_count, "pulldownWithAuthoritativeEventAdapter": adapted,
            "markdownItCustomMap": other, "adaptedOutputsEqual": adapted == other })
    }).into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn authoritative_reference_context_beats_local_duplicates_without_rewriting_inline_links() {
        let cases = super::run();
        for case in cases.as_array().unwrap() {
            assert_eq!(case["adaptedOutputsEqual"], true, "{}", case["name"]);
        }
        let duplicate = &cases[1];
        assert_eq!(duplicate["pulldownCallbackCalls"], 0);
        assert!(duplicate["pulldownCallbackOnly"].as_str().unwrap().contains("/local"));
        assert!(duplicate["pulldownWithAuthoritativeEventAdapter"].as_str().unwrap().contains("/global-first"));
        assert!(cases[2]["pulldownWithAuthoritativeEventAdapter"].as_str().unwrap().contains("/explicit"));
    }
}

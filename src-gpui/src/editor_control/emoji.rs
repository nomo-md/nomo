//! 消费宿主共享 Emoji 词库，沿用原版查询边界和排序；这里不下载或缓存文件。
use super::parse::ParsedWindow;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmojiEntry {
    pub id: String,
    pub unicode: String,
    pub en: String,
    pub zh: String,
    pub group: u8,
    pub order: u64,
    pub shortcodes: Vec<String>,
    pub terms: Vec<String>,
}
pub struct EmojiCatalog {
    pub version: String,
    pub entries: Vec<EmojiEntry>,
}
pub struct EmojiQuery {
    pub range: Range<usize>,
    pub text: String,
}
pub fn normalize(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .trim()
        .trim_matches(':')
        .to_lowercase()
}
impl EmojiCatalog {
    /// 与原版 buildEmojiCatalog 使用相同字段、分组、别名和排序。
    /// JSON 元数据属于宿主资源，构建后即可释放，不保存第二份原始词库。
    pub fn from_payload(
        version: String,
        en: serde_json::Value,
        zh: serde_json::Value,
        aliases: serde_json::Value,
    ) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Compact {
            hexcode: String,
            unicode: String,
            label: String,
            group: Option<u8>,
            order: Option<u64>,
            #[serde(default)]
            tags: Vec<String>,
            #[serde(default)]
            skins: Vec<Compact>,
        }
        fn flatten(entries: Vec<Compact>) -> Vec<Compact> {
            entries
                .into_iter()
                .flat_map(|mut entry| {
                    let skins = std::mem::take(&mut entry.skins);
                    std::iter::once(entry).chain(skins)
                })
                .collect()
        }
        let en = serde_json::from_value::<Vec<Compact>>(en).map_err(|e| e.to_string())?;
        let zh = serde_json::from_value::<Vec<Compact>>(zh).map_err(|e| e.to_string())?;
        let zh: std::collections::HashMap<_, _> = flatten(zh)
            .into_iter()
            .map(|entry| (entry.hexcode.clone(), entry))
            .collect();
        let aliases = aliases.as_object().ok_or("无效的 Emoji 别名数据")?;
        let mut entries = Vec::new();
        for entry in flatten(en) {
            let Some(group) = entry
                .group
                .filter(|group| [0, 1, 3, 4, 6, 5, 7, 8, 9].contains(group))
            else {
                continue;
            };
            let chinese = zh.get(&entry.hexcode);
            let mut shortcodes: Vec<String> = match aliases.get(&entry.hexcode) {
                Some(serde_json::Value::String(alias)) => vec![alias.clone()],
                Some(serde_json::Value::Array(values)) => values
                    .iter()
                    .filter_map(|value| value.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
            let mut name = String::new();
            for ch in entry.label.to_lowercase().chars() {
                if ch.is_alphanumeric() {
                    name.push(ch);
                } else if !name.is_empty() && !name.ends_with('_') {
                    name.push('_');
                }
            }
            let name = name.trim_end_matches('_').to_owned();
            if !name.is_empty() && !shortcodes.contains(&name) {
                shortcodes.push(name);
            }
            let chinese_label = chinese.map_or(&entry.label, |entry| &entry.label).clone();
            let mut terms = Vec::new();
            for term in shortcodes
                .iter()
                .chain([&entry.label, &chinese_label])
                .chain(&entry.tags)
                .chain(chinese.into_iter().flat_map(|entry| &entry.tags))
            {
                let term = normalize(term);
                if !term.is_empty() && !terms.contains(&term) {
                    terms.push(term);
                }
            }
            entries.push(EmojiEntry {
                id: entry.hexcode,
                unicode: entry.unicode,
                en: entry.label,
                zh: chinese_label,
                group,
                order: entry.order.unwrap_or(9_007_199_254_740_991),
                shortcodes,
                terms,
            });
        }
        entries.sort_by_key(|entry| entry.order);
        let catalog = Self { version, entries };
        if catalog.estimated_bytes() > 16 * 1024 * 1024 {
            return Err("Emoji 索引超出宿主资源预算".into());
        }
        Ok(catalog)
    }
    pub fn search(&self, text: &str, limit: usize) -> Vec<&EmojiEntry> {
        let query = normalize(text);
        let words: Vec<_> = query.split_whitespace().collect();
        let mut matches = Vec::new();
        for entry in &self.entries {
            if !words.iter().all(|word| {
                entry.unicode == *word || entry.terms.iter().any(|term| term.contains(word))
            }) {
                continue;
            }
            let score = if query.is_empty()
                || entry.unicode == query
                || entry.shortcodes.contains(&query)
            {
                0
            } else if entry.terms.contains(&query) {
                1
            } else if entry.shortcodes.iter().any(|term| term.starts_with(&query)) {
                2
            } else if entry.terms.iter().any(|term| term.starts_with(&query)) {
                3
            } else {
                4
            };
            matches.push((score, entry));
        }
        matches.sort_by_key(|(score, entry)| (*score, entry.order));
        matches
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect()
    }
    pub fn estimated_bytes(&self) -> usize {
        self.version.len()
            + self
                .entries
                .iter()
                .map(|entry| {
                    std::mem::size_of::<EmojiEntry>()
                        + entry.id.len()
                        + entry.unicode.len()
                        + entry.en.len()
                        + entry.zh.len()
                        + entry
                            .shortcodes
                            .iter()
                            .chain(&entry.terms)
                            .map(|s| s.len() + std::mem::size_of::<String>())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}
pub fn query(parsed: &ParsedWindow, at: usize) -> Option<EmojiQuery> {
    use pulldown_cmark::{Event, Tag};
    let source = &parsed.source;
    let before = source.get(..at)?;
    let end = if before.ends_with(':') { at - 1 } else { at };
    let mut start = end;
    for (offset, ch) in source[..end].char_indices().rev().take(65) {
        if ch.is_alphanumeric() || matches!(ch, '_' | '+' | '-') {
            start = offset;
        } else {
            break;
        }
    }
    let text = &source[start..end];
    if text.is_empty()
        || text.chars().count() > 64
        || start == 0
        || source.as_bytes()[start - 1] != b':'
    {
        return None;
    }
    let colon = start - 1;
    if let Some(previous) = source[..colon].chars().last() {
        if previous.is_alphanumeric() || matches!(previous, '\\' | '/' | ':' | '`' | '_' | '$') {
            return None;
        }
    }
    if parsed.events.iter().any(|(event, range)| {
        range.start <= colon
            && colon < range.end
            && matches!(
                event,
                Event::Code(_)
                    | Event::InlineMath(_)
                    | Event::DisplayMath(_)
                    | Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. })
            )
    }) {
        return None;
    }
    Some(EmojiQuery {
        range: colon..at,
        text: text.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emoji_queries_exclude_protocols_code_and_math() {
        for source in [
            "https:smile",
            "C:smile",
            "`:smile`",
            "$:smile$",
            "[x](https://a/:smile)",
        ] {
            let parsed = super::super::parse::fixture(source);
            let at = source.find("smile").unwrap() + 5;
            assert!(query(&parsed, at).is_none(), "{source}");
        }
        let parsed = super::super::parse::fixture("中文 :笑脸:");
        assert_eq!(query(&parsed, parsed.source.len()).unwrap().text, "笑脸");
        assert_eq!(normalize("：ＳＭＩＬＥ："), "smile");
    }
    #[test]
    fn original_catalog_fields_and_multilingual_ranking() {
        use serde_json::json;
        let catalog = EmojiCatalog::from_payload("fixture".into(), json!([
            {"hexcode":"1F600","unicode":"😀","label":"grinning face","group":0,"order":1,"tags":["happy"]},
            {"hexcode":"1F44D","unicode":"👍","label":"thumbs up","group":1,"order":2,"skins":[{"hexcode":"1F44D-1F3FB","unicode":"👍🏻","label":"thumbs up: light skin tone","group":1,"order":3}]},
            {"hexcode":"excluded","unicode":"x","label":"component","group":2}
        ]), json!([{ "hexcode":"1F600","unicode":"😀","label":"笑脸","tags":["开心"] }]), json!({"1F600":["smile","grinning"],"1F44D":"+1"})).unwrap();
        assert_eq!(catalog.entries.len(), 3);
        assert_eq!(catalog.search("开心", 8)[0].unicode, "😀");
        assert_eq!(catalog.search("+1", 8)[0].unicode, "👍");
        assert_eq!(catalog.search("light_skin", 8)[0].unicode, "👍🏻");
        assert_eq!(catalog.search("ＳＭＩＬＥ", 8)[0].unicode, "😀");
    }
}

//! 属性卡片只提取可展示的标量和标签，不重写 YAML；复杂值保留原文并提示。
#[derive(Default)]
pub(crate) struct Metadata {
    pub title: String,
    pub status: String,
    pub created: String,
    pub updated: String,
    pub tags: Vec<String>,
    pub extra: usize,
    pub warning: bool,
}

fn scalar(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with('"') { return serde_json::from_str::<String>(value).ok(); }
    if let Some(value) = value.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')) { return Some(value.replace("''", "'")); }
    if value.starts_with(['[', '{', '|', '>', '&', '*', '!']) { return None; }
    Some(value.split(" #").next().unwrap_or("").trim().to_owned())
}

pub(crate) fn read(raw: &str) -> Metadata {
    let mut out = Metadata::default();
    let mut tags = false;
    for line in raw.lines().skip(1) {
        let trim = line.trim();
        if matches!(trim, "---" | "...") { break; }
        if trim.is_empty() || trim.starts_with('#') { continue; }
        if tags && trim.starts_with("- ") {
            if let Some(tag) = scalar(&trim[2..]).filter(|value| !value.is_empty()) { out.tags.push(tag); }
            else { out.warning = true; }
            continue;
        }
        if line.starts_with([' ', '\t']) { if tags { out.warning = true; } continue; }
        tags = false;
        let Some((key, value)) = line.split_once(':') else { out.warning = true; continue; };
        let key = key.trim();
        if key == "tags" {
            tags = value.trim().is_empty();
            if tags { continue; }
            let value = value.trim();
            if let Some(list) = value.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
                // 引号内的逗号属于标签内容。
                let mut quote = None; let mut escaped = false; let mut start = 0;
                for (at, ch) in list.char_indices().chain(std::iter::once((list.len(), ','))) {
                    if escaped { escaped = false; continue; }
                    if quote == Some('"') && ch == '\\' { escaped = true; continue; }
                    if quote == Some(ch) { quote = None; }
                    else if quote.is_none() && matches!(ch, '\'' | '"') { quote = Some(ch); }
                    else if ch == ',' && quote.is_none() {
                        if let Some(tag) = scalar(&list[start..at]).filter(|value| !value.is_empty()) { out.tags.push(tag); }
                        else if !list[start..at].trim().is_empty() { out.warning = true; }
                        start = at + 1;
                    }
                }
                out.warning |= quote.is_some();
            } else if let Some(tag) = scalar(value) { if !tag.is_empty() { out.tags.push(tag); } }
            else { out.warning = true; }
            continue;
        }
        let target = match key {
            "title" => &mut out.title, "status" => &mut out.status,
            "created" | "date" => &mut out.created, "updated" => &mut out.updated,
            _ => { out.extra += 1; continue; }
        };
        if let Some(value) = scalar(value) { *target = value; } else { out.warning = true; }
    }
    out
}

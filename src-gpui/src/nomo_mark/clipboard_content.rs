//! 剪贴板语义投影只处理选区所在窗口；输出按需生成，不缓存正文。
use super::{mapping::text_segments, parse::ParsedWindow, protocol::ControlError};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

pub(crate) struct Payload { pub text: String, pub html: String }

pub(super) fn from_session(port: &super::native_port::NativeWindowPort, revision: u64, range: Range<u64>, markdown: bool, request: &super::session_window::WindowRequest) -> Result<Payload, ControlError> {
    let mut payload = Payload { text: String::new(), html: String::new() };
    let mut at = range.start;
    while at < range.end {
        request.check()?;
        let loaded = port.load_group(revision, at, None, request)?;
        if loaded.input.from_byte > at || loaded.input.to_byte <= at { return Err(ControlError::InvalidRange); }
        let end = loaded.input.to_byte.min(range.end);
        let base = loaded.input.from_byte;
        let parsed = ParsedWindow::parse(loaded.input, "clipboard", 0, loaded.context)?;
        if parsed.context.source_only { return Err(ControlError::NeedsContext("选区含不完整的超大结构，请在源码模式复制以保留原文".into())); }
        let part = selected(&parsed, (at - base) as usize..(end - base) as usize, markdown)?;
        payload.text.push_str(&part.text);
        payload.html.push_str(&part.html);
        if payload.text.len().saturating_add(payload.html.len()) > 64 * 1024 * 1024 {
            return Err(ControlError::Unsupported("复制内容超过 64 MiB 剪贴板预算；原文已保留".into()));
        }
        if !markdown && end < range.end && !payload.text.ends_with('\n') { payload.text.push('\n'); }
        at = end;
    }
    Ok(payload)
}

pub(super) fn selected(parsed: &ParsedWindow, range: Range<usize>, markdown: bool) -> Result<Payload, ControlError> {
    let raw = parsed.source.get(range.clone()).ok_or(ControlError::InvalidRange)?;
    let mut before = String::new();
    let mut after = String::new();
    let mut spans: Vec<_> = parsed.inline.iter().filter(|s| s.content.start < range.end && range.start < s.content.end).collect();
    spans.sort_by_key(|s| (s.open.start, std::cmp::Reverse(s.close.end)));
    for span in spans {
        if span.open.start < range.start && range.start <= span.content.end { before.push_str(&parsed.source[span.open.clone()]); }
        if span.content.start <= range.end && range.end < span.close.end { after.insert_str(0, &parsed.source[span.close.clone()]); }
    }
    let fragment = format!("{before}{raw}{after}");
    let text = if markdown { fragment.clone() } else { plain(parsed, &range)? };
    Ok(Payload { text, html: html(&fragment) })
}

fn plain(parsed: &ParsedWindow, range: &Range<usize>) -> Result<String, ControlError> {
    let mut output = String::new();
    let mut fragments = std::collections::HashSet::new();
    for (event, span) in &parsed.events {
        if span.end <= range.start || range.end <= span.start { continue; }
        match event {
            Event::Text(text) => {
                for segment in text_segments(text, &parsed.source[span.clone()], span.start) {
                    let start = range.start.max(segment.source.start);
                    let end = range.end.min(segment.source.end);
                    if start >= end { continue; }
                    let display = if segment.exact { segment.display.start + start - segment.source.start..segment.display.start + end - segment.source.start } else { segment.display.clone() };
                    output.push_str(text.get(display).ok_or(ControlError::InvalidRange)?);
                }
            }
            Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                let content = parsed.inline.iter().find(|s| s.open.start == span.start && s.close.end == span.end).map(|s| s.content.clone());
                if let Some(content) = content {
                    let start = range.start.max(content.start); let end = range.end.min(content.end);
                    if start < end { output.push_str(&parsed.source[start..end]); }
                } else { output.push_str(text); }
            }
            Event::SoftBreak | Event::HardBreak => output.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::TableRow | TagEnd::CodeBlock) if span.end < range.end => {
                if !output.ends_with('\n') { output.push('\n'); }
            }
            Event::End(TagEnd::TableCell) if span.end < range.end => output.push('\t'),
            Event::TaskListMarker(done) => output.push_str(if *done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(id) => output.push_str(&format!("[{id}]")),
            Event::Html(_) => {
                for fragment in parsed.html_fragments.iter().filter(|fragment| fragment.source.start < span.end && span.start < fragment.source.end) {
                    if !fragments.insert(fragment.source.start) { continue; }
                    let mut first = true;
                    for cell in &fragment.cells {
                        if cell.content.end <= range.start || range.end <= cell.content.start { continue; }
                        if !first && !output.ends_with('\n') { output.push('\n'); }
                        first = false;
                        for run in &cell.runs {
                            let start = run.source.start.max(range.start); let end = run.source.end.min(range.end);
                            if start >= end { continue; }
                            if parsed.source.get(run.source.clone()) == Some(run.text.as_str()) {
                                output.push_str(&run.text[start - run.source.start..end - run.source.start]);
                            } else { output.push_str(&run.text); }
                        }
                    }
                }
                if let Some(table) = parsed.html_tables.iter().find(|t| t.source.start == span.start) {
                    let mut last_row = None;
                    for cell in &table.cells {
                        if cell.content.end <= range.start || range.end <= cell.content.start { continue; }
                        if let Some(row) = last_row { output.push(if row == cell.row { '\t' } else { '\n' }); }
                        last_row = Some(cell.row);
                        for run in &cell.runs {
                            if run.source.start < range.end && range.start < run.source.end {
                                if run.text.len() == run.source.len() {
                                    output.push_str(&run.text[range.start.saturating_sub(run.source.start)..range.end.saturating_sub(run.source.start).min(run.text.len())]);
                                } else { output.push_str(&run.text); }
                            }
                        }
                    }
                }
            }
            Event::InlineHtml(value) if value.to_ascii_lowercase().starts_with("<br") => output.push('\n'),
            _ => {}
        }
    }
    Ok(output)
}

fn safe_url(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.chars().any(|c| c.is_control()) && !trimmed.split_once(':').is_some_and(|(scheme, _)| !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "mailto" | "file"))
}

pub(super) fn html(markdown: &str) -> String {
    let mut output = String::new();
    let mut image: Option<(String, String, String)> = None;
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_MATH;
    let projected = super::nomo_syntax::project_comment_paragraphs(markdown, options);
    for (event, range) in Parser::new_ext(&projected, options).into_offset_iter() {
        let event = super::nomo_syntax::restore_inline_comment(markdown, event, &range);
        match &event {
            Event::Start(Tag::Image { dest_url, title, .. }) => { image = Some((dest_url.to_string(), title.to_string(), String::new())); continue; }
            Event::Text(text) | Event::Code(text) if image.is_some() => { image.as_mut().unwrap().2.push_str(text); continue; }
            Event::End(TagEnd::Image) => {
                if let Some((url, title, alt)) = image.take().filter(|(url, _, _)| safe_url(url)) {
                    output.push_str(&format!("<img src=\"{}\" alt=\"{}\" title=\"{}\">",
                        html_escape::encode_double_quoted_attribute(&url), html_escape::encode_double_quoted_attribute(&alt), html_escape::encode_double_quoted_attribute(&title)));
                }
                continue;
            }
            _ if image.is_some() => continue,
            _ => {}
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => output.push_str("<p>"),
                Tag::Heading { level, .. } => output.push_str(&format!("<h{}>", level as u8)),
                Tag::Strong => output.push_str("<strong>"), Tag::Emphasis => output.push_str("<em>"),
                Tag::Strikethrough => output.push_str("<del>"), Tag::BlockQuote(_) => output.push_str("<blockquote>"),
                Tag::CodeBlock(_) => output.push_str("<pre><code>"),
                Tag::List(start) => output.push_str(&start.map_or("<ul>".into(), |n| format!("<ol start=\"{n}\">"))),
                Tag::Item => output.push_str("<li>"), Tag::Table(_) => output.push_str("<table>"),
                Tag::TableHead => output.push_str("<thead><tr>"), Tag::TableRow => output.push_str("<tr>"), Tag::TableCell => output.push_str("<td>"),
                Tag::Link { dest_url, .. } => output.push_str(&format!("<a href=\"{}\">", html_escape::encode_double_quoted_attribute(if safe_url(&dest_url) { dest_url.as_ref() } else { "" }))),
                _ => {}
            },
            Event::End(tag) => output.push_str(&match tag {
                TagEnd::Paragraph => "</p>".into(), TagEnd::Heading(level) => format!("</h{}>", level as u8),
                TagEnd::Strong => "</strong>".into(), TagEnd::Emphasis => "</em>".into(), TagEnd::Strikethrough => "</del>".into(),
                TagEnd::BlockQuote(_) => "</blockquote>".into(), TagEnd::CodeBlock => "</code></pre>".into(),
                TagEnd::List(true) => "</ol>".into(), TagEnd::List(false) => "</ul>".into(), TagEnd::Item => "</li>".into(),
                TagEnd::Table => "</table>".into(), TagEnd::TableHead => "</tr></thead>".into(), TagEnd::TableRow => "</tr>".into(), TagEnd::TableCell => "</td>".into(),
                TagEnd::Link => "</a>".into(), _ => String::new()
            }),
            Event::Text(text) => output.push_str(&html_escape::encode_text(&text)),
            Event::InlineMath(text) => output.push_str(&format!("<span class=\"math-inline\" data-tex=\"{}\">{}</span>",
                html_escape::encode_double_quoted_attribute(&text), html_escape::encode_text(&text))),
            Event::DisplayMath(text) => output.push_str(&format!("<div class=\"math-block\" data-tex=\"{}\">{}</div>",
                html_escape::encode_double_quoted_attribute(&text), html_escape::encode_text(&text))),
            Event::Code(text) => output.push_str(&format!("<code>{}</code>", html_escape::encode_text(&text))),
            Event::SoftBreak | Event::HardBreak => output.push_str("<br>"),
            Event::Rule => output.push_str("<hr>"),
            Event::TaskListMarker(done) => output.push_str(if done { "☑ " } else { "☐ " }),
            Event::Html(value) | Event::InlineHtml(value) => output.push_str(&sanitize_html(&value)),
            _ => {}
        }
    }
    output
}

/// 不执行外部 HTML；仅保留编辑器支持的文本、结构与安全链接。
pub(super) fn sanitize_html(input: &str) -> String {
    let mut output = String::new(); let mut at = 0; let mut suppressed: Option<String> = None;
    while at < input.len() {
        if input[at..].starts_with("<!--") {
            at = input[at..].find("-->").map_or(input.len(), |n| at + n + 3); continue;
        }
        if let Some(tag) = super::nomo_syntax::html_tag(input, at) {
            at = tag.end;
            if let Some(name) = &suppressed { if tag.closing && &tag.name == name { suppressed = None; } continue; }
            if matches!(tag.name.as_str(), "script" | "style" | "iframe" | "object") {
                if !tag.closing { suppressed = Some(tag.name); } continue;
            }
            if !matches!(tag.name.as_str(), "p" | "div" | "span" | "br" | "strong" | "b" | "em" | "i" | "u" | "s" | "del" | "mark" | "sub" | "sup" | "pre" | "code" | "blockquote" | "ul" | "ol" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "a" | "img" | "table" | "tbody" | "thead" | "tr" | "td" | "th" | "hr") { continue; }
            if tag.closing { output.push_str(&format!("</{}>", tag.name)); continue; }
            output.push('<'); output.push_str(&tag.name);
            if tag.name == "table" { output.push_str(" data-nomo-table=\"semantic\""); }
            for (key, value) in &tag.attrs {
                if (matches!(key.as_str(), "rowspan" | "colspan" | "start") && value.parse::<u16>().is_ok())
                    || (key == "href" && tag.name == "a" && safe_url(value))
                    || (tag.name == "img" && ((key == "src" && safe_url(value)) || matches!(key.as_str(), "alt" | "title")))
                    || (key == "class" && matches!(value.as_str(), "math-inline" | "math-block"))
                    || (key == "data-tex" && matches!(tag.attrs.get("class").map(String::as_str), Some("math-inline" | "math-block")))
                    || (matches!(key.as_str(), "align" | "data-align") && matches!(value.as_str(), "left" | "center" | "right")) {
                    let key = if key == "align" { "data-align" } else { key.as_str() };
                    output.push_str(&format!(" {key}=\"{}\"", html_escape::encode_double_quoted_attribute(value)));
                }
            }
            output.push('>');
        } else {
            let step = input[at..].chars().next().unwrap().len_utf8();
            let end = input[at + step..].find('<').map_or(input.len(), |n| at + step + n);
            if suppressed.is_none() { output.push_str(&html_escape::encode_text(&html_escape::decode_html_entities(&input[at..end]))); }
            at = end;
        }
    }
    output
}

pub(super) fn literal(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() { if "\\`*_~^<>[]$#|!+-.".contains(ch) { output.push('\\'); } output.push(ch); }
    output
}

pub(super) fn literal_paragraphs(text: &str) -> String {
    let text = literal(text);
    let lines: Vec<_> = text.split('\n').collect();
    let mut output = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            if !lines[i - 1].trim_end_matches('\r').is_empty() && !line.trim_end_matches('\r').is_empty() { output.push_str("  "); }
            output.push('\n');
        }
        output.push_str(line.trim_end_matches('\r'));
    }
    output
}

pub(super) fn from_html(input: &str) -> String {
    let input = sanitize_html(input);
    let mut output = String::new(); let mut at = 0; let mut links = Vec::new();
    while at < input.len() {
        if let Some(tag) = super::nomo_syntax::html_tag(&input, at) {
            let start = at; at = tag.end;
            if !tag.closing && matches!(tag.attrs.get("class").map(String::as_str), Some("math-inline" | "math-block")) {
                if let Some(tex) = tag.attrs.get("data-tex") {
                    if let Some(length) = input[at..].find(&format!("</{}>", tag.name)) {
                        let tex = html_escape::decode_html_entities(tex);
                        if tag.attrs.get("class").is_some_and(|c| c == "math-block") { output.push_str(&format!("\n\n$$\n{tex}\n$$\n\n")); }
                        else { output.push_str(&format!("{}{tex}{}", "$", "$")); }
                        at += length + tag.name.len() + 3;
                        continue;
                    }
                }
            }
            match tag.name.as_str() {
                "table" if !tag.closing => {
                    if let Some(end) = input[at..].find("</table>") {
                        let end = at + end + 8;
                        output.push_str("\n\n"); output.push_str(&normalize_table(&input[start..end])); output.push_str("\n\n"); at = end;
                    }
                }
                "p" | "div" | "blockquote" => { if !output.ends_with("\n\n") && !output.is_empty() { output.push_str("\n\n"); } if tag.name == "blockquote" && !tag.closing { output.push_str("> "); } }
                "br" => output.push_str("<br>"),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => { if tag.closing { output.push_str("\n\n"); } else { output.push_str(&format!("\n\n{} ", "#".repeat(tag.name.as_bytes()[1] as usize - b'0' as usize))); } }
                "strong" | "b" => output.push_str("**"), "em" | "i" => output.push('*'), "s" | "del" => output.push_str("~~"),
                "u" | "mark" | "sub" | "sup" => output.push_str(&input[start..at]),
                "pre" if !tag.closing => {
                    if let Some(length) = input[at..].find("</pre>") {
                        let raw = &input[at..at + length];
                        let raw = raw.strip_prefix("<code>").and_then(|s| s.strip_suffix("</code>")).unwrap_or(raw);
                        let text = html_escape::decode_html_entities(raw);
                        let fence = "`".repeat((text.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1).max(3));
                        output.push_str(&format!("\n\n{fence}\n{text}\n{fence}\n\n"));
                        at += length + 6;
                    }
                }
                "code" if !tag.closing => {
                    if let Some(length) = input[at..].find("</code>") {
                        let text = html_escape::decode_html_entities(&input[at..at + length]);
                        let fence = "`".repeat(text.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1);
                        output.push_str(&format!("{fence} {text} {fence}"));
                        at += length + 7;
                    }
                }
                "img" if !tag.closing => output.push_str(&input[start..at]),
                "li" => output.push_str(if tag.closing { "\n" } else { "- " }),
                "a" if !tag.closing => { output.push('['); links.push(tag.attrs.get("href").cloned().unwrap_or_default()); }
                "a" => if let Some(url) = links.pop() { output.push_str(&format!("]({})", url.replace('(', "%28").replace(')', "%29"))); },
                "hr" => output.push_str("\n\n---\n\n"),
                _ => {}
            }
        } else {
            let end = input[at..].find('<').map_or(input.len(), |n| at + n);
            if end == at { at += 1; continue; }
            let text = html_escape::decode_html_entities(&input[at..end]);
            output.push_str(&literal(&text)); at = end;
        }
    }
    output.trim_matches('\n').to_string()
}

fn normalize_table(input: &str) -> String {
    let mut output = String::from("<table data-nomo-table=\"semantic\"><tbody>");
    let mut at = 0;
    while at < input.len() {
        if let Some(tag) = super::nomo_syntax::html_tag(input, at) {
            let start = at; at = tag.end;
            match tag.name.as_str() {
                "table" | "tbody" | "thead" | "tfoot" => {}
                "td" | "th" if !tag.closing => {
                    output.push_str(&input[start..at]);
                    let close = format!("</{}>", tag.name);
                    if let Some(length) = input[at..].find(&close) {
                        let content = input[at..at + length].trim();
                        if content.starts_with("<p>") { output.push_str(content); }
                        else { output.push_str("<p>"); output.push_str(content); output.push_str("</p>"); }
                        output.push_str(&close); at += length + close.len();
                    }
                }
                _ => output.push_str(&input[start..at]),
            }
        } else {
            let step = input[at..].chars().next().unwrap().len_utf8();
            let end = input[at + step..].find('<').map_or(input.len(), |n| at + step + n);
            if !input[at..end].trim().is_empty() { output.push_str(&input[at..end]); }
            at = end;
        }
    }
    output.push_str("</tbody></table>");
    output
}

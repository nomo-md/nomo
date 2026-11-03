//! 链接地址的可读投影。显示字符映射到原文，导航与保存始终使用原地址。
use super::{mapping::SourceSegment, parse::{InlineFormat, ParsedWindow}};
use pulldown_cmark::CowStr;
use std::ops::Range;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use unicode_segmentation::UnicodeSegmentation;

/// 仅投影已解析的地址；标题、属性引号和标记中的其他文字保留原样。
fn destination(marker: &str) -> Option<Range<usize>> {
    if marker.get(..2).is_some_and(|start| start.eq_ignore_ascii_case("<a")) {
        return super::nomo_syntax::html_tag(marker, 0)?.href_range;
    }
    let inner = marker.strip_prefix("](")?.strip_suffix(')')?;
    let target = inner.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let start = marker.len() - 1 - target.len();
    if let Some(angled) = target.strip_prefix('<') {
        let mut bytes = angled.bytes().enumerate();
        while let Some((index, byte)) = bytes.next() {
            match byte {
                b'>' => return Some(start + 1..start + 1 + index),
                b'\\' => { bytes.next(); }
                _ => {}
            }
        }
        return None;
    }
    let length = target.find(|ch: char| ch.is_ascii_whitespace()).unwrap_or(target.len());
    Some(start..start + length)
}

fn percent_byte(bytes: &[u8]) -> Option<u8> {
    let [b'%', high, low, ..] = bytes else { return None; };
    Some(((char::from(*high).to_digit(16)? << 4) | char::from(*low).to_digit(16)?) as u8)
}

fn readable_char(encoded: &[u8]) -> Option<char> {
    let first = percent_byte(encoded)?;
    let width = match first {
        b' ' | b'-' | b'.' | b'_' | b'~' | b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => return Some(first.into()),
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None, // 保留 %2F、%23 和 %25 等有 URL 语义的转义。
    };
    let mut bytes = [first; 4];
    for (byte, triplet) in bytes[1..width].iter_mut().zip(encoded.get(3..3 * width)?.chunks_exact(3)) {
        *byte = percent_byte(triplet)?;
    }
    let ch = std::str::from_utf8(&bytes[..width]).ok()?.chars().next()?;
    match ch.general_category() {
        GeneralCategory::Control | GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator => None,
        GeneralCategory::Format if !matches!(ch, '\u{200c}' | '\u{200d}') => None,
        _ => Some(ch),
    }
}

/// 普通文字保持连续的逐字节映射，每个解码字符独占其完整的 %XX 源码区间。
pub(super) fn parts(text: &str, target: Range<usize>) -> impl Iterator<Item = (Range<usize>, CowStr<'_>)> {
    let mut escapes = text[target.clone()].match_indices('%').filter_map(move |(offset, _)| {
        let at = target.start + offset;
        readable_char(&text.as_bytes()[at..target.end]).map(|ch| (at, ch))
    }).peekable();
    let mut cursor = 0;
    std::iter::from_fn(move || {
        let start = cursor;
        if let Some(&(at, ch)) = escapes.peek() {
            if cursor == at {
                escapes.next();
                cursor += 3 * ch.len_utf8();
                return Some((start..cursor, ch.into()));
            }
            cursor = at;
        } else {
            cursor = text.len();
        }
        (start < cursor).then(|| (start..cursor, text[start..cursor].into()))
    })
}

pub(crate) fn marker_parts(source: &str, marker: Range<usize>) -> impl Iterator<Item = (Range<usize>, CowStr<'_>)> {
    let raw = &source[marker.clone()];
    let target = if raw.contains('%') { destination(raw) } else { None };
    parts(raw, target.unwrap_or(0..0)).map(move |(part, text)| (marker.start + part.start..marker.start + part.end, text))
}

/// 语义退格／删除按屏幕上的字素执行，不能只删掉中文或 Emoji 编码中的一个十六进制位。
pub(super) fn deletion(parsed: &ParsedWindow, at: usize, backward: bool) -> Option<Range<usize>> {
    let probe = if backward { at.checked_sub(1)? } else { at };
    let marker_range = parsed.inline.iter().filter(|span| span.format == InlineFormat::Link)
        .flat_map(|span| [&span.open, &span.close]).find(|marker| marker.contains(&probe))?;
    let marker = &parsed.source[marker_range.clone()];
    // HTML 单元格的属性通过链接弹层修改，保留该路径既有的结构校验。
    if marker.starts_with('<') && parsed.html_cell_at(probe).is_some() { return None; }
    let target = destination(marker)?;
    if !target.contains(&(probe - marker_range.start)) { return None; }
    let base = marker_range.start + target.start;
    let raw = &marker[target];
    if !raw.contains('%') { return None; }

    let mut display = String::new();
    let mut segments = Vec::new();
    for (source, text) in parts(raw, 0..raw.len()) {
        let start = display.len();
        display.push_str(&text);
        segments.push(SourceSegment {
            display: start..display.len(), exact: source.len() == text.len(),
            source: base + source.start..base + source.end,
        });
    }
    let caret = segments.iter().find(|s| s.source.contains(&probe))?.to_display(at, backward);
    let probe = if backward { caret.checked_sub(1)? } else { caret };
    let (start, grapheme) = display.grapheme_indices(true).find(|(start, text)| start + text.len() > probe)?;
    let end = start + grapheme.len();
    let touched = &segments[segments.partition_point(|s| s.display.end <= start)..segments.partition_point(|s| s.display.start < end)];
    if touched.iter().all(|s| s.exact) { return None; }
    Some(touched.first()?.to_source(start, false)..touched.last()?.to_source(end, true))
}

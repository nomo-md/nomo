//! 中英混排段落的 Knuth–Plass 断行；所有位置都是原段落的 UTF-8 字节。
//! 英文断词和悬挂只附加测量信息，不向正文插入字符。超过求解预算时沿合法断点退化。

use unicode_segmentation::UnicodeSegmentation;
pub(crate) mod hyphenation;

const HAN_STRETCH_EM: f32 = 0.12;
const HAN_SHRINK_EM: f32 = 0.04;
const LINE_PENALTY: f32 = 10.0;
const FITNESS_JUMP_DEMERITS: f32 = 3000.0;
const MAX_RATIO: f32 = 3.0;
/// 精细求解的临时数据有界；更长窗口仍完整输出，只使用线性退化路径。
pub(crate) const MAX_KP_ITEMS: usize = 16_384;
const MAX_NODES: usize = 8_192;
const MAX_CANDIDATES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemKind {
    Box,
    /// 断行时被丢弃的空白；`width` 为自然宽度。
    Glue { stretch: f32, shrink: f32 },
}

/// 汉字间的零宽胶仍使用空字节范围，保持既有条目接口。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub from: usize,
    pub to: usize,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub items: std::ops::Range<usize>,
    pub ratio: f32,
}

/// 由实际字体测量得出的行边缘留白；hang 不缩放或裁剪标点字形。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Edges {
    pub trim_start: f32,
    pub trim_end: f32,
    pub hang: f32,
}

/// 只有选择该断点时才计入可见连字符宽度。at 为前一条目的 exclusive 索引。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hyphen {
    pub at: usize,
    pub width: f32,
}

fn is_cjk_char(ch: char) -> bool {
    matches!(ch as u32,
        0x3000..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
        || "“”‘’".contains(ch) || ch == '\u{FFFC}'
}
fn is_cjk(grapheme: &str) -> bool { grapheme.chars().next().is_some_and(is_cjk_char) }

pub(crate) fn no_line_start(grapheme: &str) -> bool {
    !grapheme.is_empty() && "）］｝》〉」』】〕〗〙〛”’，。、；：！？".contains(grapheme)
}
pub(crate) fn no_line_end(grapheme: &str) -> bool {
    !grapheme.is_empty() && "（［｛《〈「『【〔〖〘〚“‘".contains(grapheme)
}
pub(crate) fn can_break_between(before: &str, after: &str) -> bool {
    !no_line_end(before) && !no_line_start(after)
        && !matches!((before, after), ("—", "—") | ("…", "…"))
        && !before.contains(['\u{00a0}', '\u{202f}', '\u{2060}'])
        && !after.contains(['\u{00a0}', '\u{202f}', '\u{2060}'])
}
fn breakable_space(text: &str) -> bool {
    text.chars().all(|ch| ch.is_whitespace() && !matches!(ch, '\u{00a0}' | '\u{202f}'))
}

/// 按字素和合法禁则边界生成条目，英文候选由布局层测量后另行传入。
pub fn build_items(text: &str, font_size: f32, space_width: f32, x_at: impl Fn(usize) -> f32) -> Vec<Item> {
    let mut items = Vec::new();
    let mut word_start = None;
    let mut previous: Option<&str> = None;
    let flush_word = |items: &mut Vec<Item>, start: &mut Option<usize>, end: usize| {
        if let Some(from) = start.take() {
            items.push(Item { kind: ItemKind::Box, from, to: end, width: x_at(end) - x_at(from) });
        }
    };
    let mut graphemes = text.grapheme_indices(true).peekable();
    while let Some((index, grapheme)) = graphemes.next() {
        let end = index + grapheme.len();
        if breakable_space(grapheme) {
            flush_word(&mut items, &mut word_start, index);
            let mut end = end;
            let mut count = 1;
            while graphemes.peek().is_some_and(|(_, next)| breakable_space(next)) {
                let (at, next) = graphemes.next().unwrap();
                end = at + next.len();
                count += 1;
            }
            let following = graphemes.peek().map_or("", |(_, next)| *next);
            let allowed = can_break_between(previous.unwrap_or(""), following);
            let kind = if allowed {
                ItemKind::Glue { stretch: space_width * 0.5 * count as f32, shrink: space_width / 3.0 * count as f32 }
            } else { ItemKind::Box };
            items.push(Item { kind, from: index, to: end, width: x_at(end) - x_at(index) });
            previous = Some(&text[index..end]);
            continue;
        }
        if !is_cjk(grapheme) {
            if word_start.is_none() {
                if previous.is_some_and(|before| !breakable_space(before) && can_break_between(before, grapheme)) {
                    push_han_glue(&mut items, index, font_size);
                }
                word_start = Some(index);
            }
            previous = Some(grapheme);
            continue;
        }
        flush_word(&mut items, &mut word_start, index);
        if previous.is_some_and(|before| !breakable_space(before) && can_break_between(before, grapheme)) {
            push_han_glue(&mut items, index, font_size);
        }
        items.push(Item { kind: ItemKind::Box, from: index, to: end, width: x_at(end) - x_at(index) });
        previous = Some(grapheme);
    }
    flush_word(&mut items, &mut word_start, text.len());
    items
}

fn push_han_glue(items: &mut Vec<Item>, at: usize, font_size: f32) {
    items.push(Item {
        kind: ItemKind::Glue { stretch: font_size * HAN_STRETCH_EM, shrink: font_size * HAN_SHRINK_EM },
        from: at, to: at, width: 0.0,
    });
}

#[derive(Clone, Copy)]
struct Node {
    at: usize,
    fitness: u8,
    hyphenated: bool,
    demerits: f32,
    ratio: f32,
    previous: Option<usize>,
}

fn line_start(items: &[Item], mut at: usize) -> usize {
    while at < items.len() && matches!(items[at].kind, ItemKind::Glue { .. }) { at += 1; }
    at
}

fn candidates<'a>(items: &'a [Item], hyphens: &'a [Hyphen]) -> impl Iterator<Item = Hyphen> + Clone + 'a {
    let mut index = 0;
    (1..items.len()).filter_map(move |at| {
        while index < hyphens.len() && hyphens[index].at < at { index += 1; }
        if matches!(items[at].kind, ItemKind::Glue { .. }) && matches!(items[at - 1].kind, ItemKind::Box) {
            Some(Hyphen { at, width: 0.0 })
        } else { hyphens.get(index).filter(|point| point.at == at).copied() }
    }).chain(std::iter::once(Hyphen { at: items.len(), width: 0.0 }))
}

fn start_trim(edges: &[Edges], at: usize) -> f32 { edges.get(at).map_or(0.0, |edge| edge.trim_start) }
fn end_trim(edges: &[Edges], at: usize) -> f32 {
    at.checked_sub(1).and_then(|at| edges.get(at)).map_or(0.0, |edge| edge.trim_end + edge.hang)
}
fn max_end_trim(edges: &[Edges]) -> f32 {
    edges.iter().map(|edge| edge.trim_end + edge.hang).fold(0.0, f32::max)
}

pub fn break_lines(items: &[Item], width: f32) -> Vec<Line> {
    break_lines_with(items, width, &[], &[])
}

/// 断词候选与行边缘宽度共同参与 KP；活动状态和求解次数都有确定上限。
pub(crate) fn break_lines_with(items: &[Item], width: f32, edges: &[Edges], hyphens: &[Hyphen]) -> Vec<Line> {
    if items.is_empty() { return vec![Line { items: 0..0, ratio: 0.0 }]; }
    if !width.is_finite() || width <= 0.0 || items.len() > MAX_KP_ITEMS {
        return greedy_lines_with(items, width, edges, hyphens);
    }
    let mut sum_w = vec![0.0f32; items.len() + 1];
    let mut sum_y = vec![0.0f32; items.len() + 1];
    let mut sum_z = vec![0.0f32; items.len() + 1];
    for (index, item) in items.iter().enumerate() {
        let (stretch, shrink) = match item.kind {
            ItemKind::Glue { stretch, shrink } => (stretch, shrink),
            ItemKind::Box => (0.0, 0.0),
        };
        sum_w[index + 1] = sum_w[index] + item.width;
        sum_y[index + 1] = sum_y[index] + stretch;
        sum_z[index + 1] = sum_z[index] + shrink;
    }
    // 后缀最小值保守涵盖零宽胶的负收缩，以及后续断点可能得到的悬挂宽度。
    // 不能仅凭当前连字符太宽或当前行无悬挂就提前删除前驱节点。
    let mut minimum: Vec<_> = sum_w.iter().zip(&sum_z).map(|(w, z)| w - z).collect();
    for index in (0..items.len()).rev() { minimum[index] = minimum[index].min(minimum[index + 1]); }
    let edge_allowance = max_end_trim(edges);
    let points = candidates(items, hyphens);
    let mut nodes = vec![Node { at: 0, fitness: 1, hyphenated: false, demerits: 0.0, ratio: 0.0, previous: None }];
    let mut active = vec![0];
    let mut examined = 0;

    for point in points {
        let last = point.at == items.len();
        let mut best: [Option<(f32, usize, f32)>; 4] = [None; 4];
        for &node_index in &active {
            examined += 1;
            if examined > MAX_CANDIDATES { return greedy_lines_with(items, width, edges, hyphens); }
            let node = nodes[node_index];
            let start = line_start(items, node.at);
            if start >= point.at { continue; }
            let natural = sum_w[point.at] - sum_w[start] + point.width
                - start_trim(edges, start) - end_trim(edges, point.at);
            let delta = width - natural;
            let ratio = if delta.abs() <= 0.001 || (last && delta >= 0.0) { 0.0 }
                else if delta > 0.0 {
                    let stretch = sum_y[point.at] - sum_y[start];
                    if stretch > 0.0 { delta / stretch } else { f32::INFINITY }
                } else {
                    let shrink = sum_z[point.at] - sum_z[start];
                    if shrink > 0.0 { delta / shrink } else { f32::NEG_INFINITY }
                };
            if !(-1.0..=MAX_RATIO).contains(&ratio) { continue; }
            let badness = 100.0 * ratio.abs().powi(3);
            let fitness = if ratio < -0.5 { 0 } else if ratio <= 0.5 { 1 } else if ratio <= 1.0 { 2 } else { 3 };
            let mut demerits = (LINE_PENALTY + badness).powi(2);
            if point.width > 0.0 { demerits += 50.0f32.powi(2); }
            if node.hyphenated && point.width > 0.0 { demerits += 3000.0; }
            if last && node.hyphenated { demerits += 1500.0; }
            if (fitness as i8 - node.fitness as i8).abs() > 1 { demerits += FITNESS_JUMP_DEMERITS; }
            let total = node.demerits + demerits;
            let slot = &mut best[fitness as usize];
            if slot.is_none_or(|(current, _, _)| total < current) { *slot = Some((total, node_index, ratio)); }
        }
        active.retain(|&node_index| {
            let start = line_start(items, nodes[node_index].at);
            start >= point.at || minimum[point.at] - (sum_w[start] - sum_z[start])
                - start_trim(edges, start) - edge_allowance <= width + 0.001
        });
        for (fitness, entry) in best.into_iter().enumerate() {
            if let Some((demerits, previous, ratio)) = entry {
                if nodes.len() >= MAX_NODES { return greedy_lines_with(items, width, edges, hyphens); }
                nodes.push(Node { at: point.at, fitness: fitness as u8, hyphenated: point.width > 0.0,
                    demerits, ratio, previous: Some(previous) });
                active.push(nodes.len() - 1);
            }
        }
        if active.is_empty() { return greedy_lines_with(items, width, edges, hyphens); }
    }
    let Some(mut at) = nodes.iter().enumerate().filter(|(_, node)| node.at == items.len())
        .min_by(|a, b| a.1.demerits.total_cmp(&b.1.demerits)).map(|(index, _)| index)
    else { return greedy_lines_with(items, width, edges, hyphens); };
    let mut lines = Vec::new();
    while let Some(previous) = nodes[at].previous {
        lines.push(Line { items: line_start(items, nodes[previous].at)..nodes[at].at, ratio: nodes[at].ratio });
        at = previous;
    }
    lines.reverse();
    lines
}

pub fn greedy_lines(items: &[Item], width: f32) -> Vec<Line> {
    greedy_lines_with(items, width, &[], &[])
}

/// 只沿合法断点退化；没有可容纳的断点时让不可断组溢出，不拆散避头尾标点。
fn greedy_lines_with(items: &[Item], width: f32, edges: &[Edges], hyphens: &[Hyphen]) -> Vec<Line> {
    if items.is_empty() { return vec![Line { items: 0..0, ratio: 0.0 }]; }
    let width = if width.is_finite() { width.max(0.0) } else { f32::MAX };
    let mut sums = vec![0.0; items.len() + 1];
    for (index, item) in items.iter().enumerate() { sums[index + 1] = sums[index] + item.width; }
    let mut points = candidates(items, hyphens).peekable();
    let allowance = max_end_trim(edges);
    let mut lines = Vec::new();
    let mut start = 0;
    while start < items.len() {
        while points.peek().is_some_and(|point| point.at <= start) { points.next(); }
        let first = points.clone();
        let mut best = None;
        while let Some(point) = points.next() {
            let raw = sums[point.at] - sums[start];
            let natural = raw + point.width - start_trim(edges, start) - end_trim(edges, point.at);
            if natural <= width + 0.001 { best = Some((point, points.clone())); }
            if raw > width + start_trim(edges, start) + allowance { break; }
        }
        let (chosen, remaining) = best.unwrap_or_else(|| {
            let mut remaining = first;
            (remaining.next().expect("paragraph end is a legal break"), remaining)
        });
        let end = chosen.at;
        lines.push(Line { items: start..end, ratio: 0.0 });
        start = line_start(items, end);
        points = remaining;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(text: &str) -> Vec<Item> {
        // 每个字素宽 16，空格宽 4。
        let x_at = |byte: usize| {
            let mut x = 0.0;
            for (index, grapheme) in text.grapheme_indices(true) {
                if index >= byte { break; }
                x += if grapheme == " " { 4.0 } else { 16.0 };
            }
            x
        };
        build_items(text, 16.0, 4.0, x_at)
    }

    #[test]
    fn justified_han_lines_fill_width_and_respect_kinsoku() {
        let text = "中文排版需要两端对齐，同时句号和逗号不能出现在行首。这是一段用于测试的文字。";
        let items = uniform(text);
        let lines = break_lines(&items, 16.0 * 9.5);
        assert!(lines.len() > 2);
        for line in &lines {
            let first = items[line.items.start];
            let glyph = &text[first.from..first.to];
            assert!(!no_line_start(glyph), "行首出现避头标点：{glyph}");
            assert!(line.ratio >= -1.0 && line.ratio <= MAX_RATIO);
        }
        let covered: usize = lines.iter().map(|line| line.items.len()).sum();
        assert!(covered <= items.len());
    }

    #[test]
    fn unbreakable_run_falls_back_without_losing_items() {
        let text = "a".repeat(200);
        let items = uniform(&text);
        let lines = break_lines(&items, 100.0);
        assert_eq!(lines.iter().map(|line| line.items.len()).sum::<usize>(), items.len());
    }
}

//! 中英混排段落的 Knuth–Plass 断行，输入为已测量宽度的条目，输出断点与每行伸缩比。
//!
//! 规则是 `src/lib/typography` 的子集：汉字间零宽可伸缩胶、拉丁空格胶、中文避头尾；
//! 未实现英文断词、悬挂标点与 KP 质量分档的提前剪枝，原型只验证断行和逐字形定位可控。

use unicode_segmentation::UnicodeSegmentation;

/// 汉字间胶的伸缩量，按字号的比例给出；取值沿用常见中文两端对齐的经验值。
const HAN_STRETCH_EM: f32 = 0.12;
const HAN_SHRINK_EM: f32 = 0.04;
/// 行惩罚 l 与相邻行松紧档跳变惩罚，来自 Knuth–Plass 原文的常用取值。
const LINE_PENALTY: f32 = 10.0;
const FITNESS_JUMP_DEMERITS: f32 = 3000.0;
/// 允许的最大伸展比；超过后改用退化方案，保证超长不可断串仍能输出。
const MAX_RATIO: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemKind {
    Box,
    /// 断行时被丢弃的空白；`width` 为自然宽度。
    Glue { stretch: f32, shrink: f32 },
}

/// 一个排版条目，`from..to` 是段内 UTF-8 字节范围；汉字间零宽胶的范围为空。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub from: usize,
    pub to: usize,
    pub width: f32,
}

/// 一行的排版结果：`items` 为段内条目区间（不含行首被丢弃的胶），
/// `ratio` 为胶的调整比，正值按 stretch、负值按 shrink 缩放。
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub items: std::ops::Range<usize>,
    pub ratio: f32,
}

fn is_cjk(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(|ch| {
        matches!(ch as u32,
            0x3000..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
            || "“”‘’".contains(ch)
            // 行内对象占位符独立成盒，前后都可断行。
            || ch == '\u{FFFC}'
    })
}

/// 不能出现在行首的标点（避头）。
fn no_line_start(grapheme: &str) -> bool {
    "）］｝》〉」』】〕〗〙〛”’，。、；：！？".contains(grapheme)
}

/// 不能出现在行尾的标点（避尾）。
fn no_line_end(grapheme: &str) -> bool {
    "（［｛《〈「『【〔〖〘〚“‘".contains(grapheme)
}

/// 按字素切分段落并生成条目。`x_at(byte)` 返回整段一次塑形后该字节处的横坐标，
/// 条目宽度取差值，因而包含字距调整但忽略跨条目的连字。
pub fn build_items(text: &str, font_size: f32, space_width: f32, x_at: impl Fn(usize) -> f32) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut word_start: Option<usize> = None;
    let mut previous: Option<(&str, bool)> = None;
    let flush_word = |items: &mut Vec<Item>, start: &mut Option<usize>, end: usize| {
        if let Some(from) = start.take() {
            items.push(Item { kind: ItemKind::Box, from, to: end, width: x_at(end) - x_at(from) });
        }
    };
    for (index, grapheme) in text.grapheme_indices(true) {
        let end = index + grapheme.len();
        if grapheme.chars().all(char::is_whitespace) {
            flush_word(&mut items, &mut word_start, index);
            items.push(Item {
                kind: ItemKind::Glue { stretch: space_width * 0.5, shrink: space_width / 3.0 },
                from: index,
                to: end,
                width: x_at(end) - x_at(index),
            });
            previous = Some((grapheme, false));
            continue;
        }
        let cjk = is_cjk(grapheme);
        if !cjk {
            // 拉丁字母与数字连续组成一个不可断的词盒；与汉字相邻时在边界插入零宽胶。
            if word_start.is_none() {
                if let Some((before, _)) = previous {
                    if !before.chars().all(char::is_whitespace) && !no_line_end(before) {
                        push_han_glue(&mut items, index, font_size);
                    }
                }
                word_start = Some(index);
            }
            previous = Some((grapheme, false));
            continue;
        }
        flush_word(&mut items, &mut word_start, index);
        if let Some((before, _)) = previous {
            let breakable = !before.chars().all(char::is_whitespace)
                && !no_line_end(before)
                && !no_line_start(grapheme);
            if breakable {
                push_han_glue(&mut items, index, font_size);
            }
        }
        items.push(Item { kind: ItemKind::Box, from: index, to: end, width: x_at(end) - x_at(index) });
        previous = Some((grapheme, true));
    }
    flush_word(&mut items, &mut word_start, text.len());
    items
}

fn push_han_glue(items: &mut Vec<Item>, at: usize, font_size: f32) {
    items.push(Item {
        kind: ItemKind::Glue { stretch: font_size * HAN_STRETCH_EM, shrink: font_size * HAN_SHRINK_EM },
        from: at,
        to: at,
        width: 0.0,
    });
}

#[derive(Clone, Copy)]
struct Node {
    /// 断点位置：断在第 `at` 个条目（必须是胶）之前；0 表示段首。
    at: usize,
    line: usize,
    fitness: u8,
    demerits: f32,
    previous: Option<usize>,
}

/// 计算断行；`width` 为行宽。末行左对齐（ratio 不大于 0 时保持实际值，便于超长行压缩）。
///
/// 采用活动节点表的标准 KP：每个胶都是候选断点，前缀和求行宽，O(n·活动节点数)。
/// 没有可行解时（例如超长不可断串）退回贪心断行，保证输出不丢字。
pub fn break_lines(items: &[Item], width: f32) -> Vec<Line> {
    if items.is_empty() {
        return vec![Line { items: 0..0, ratio: 0.0 }];
    }
    let mut sum_w = vec![0.0f32; items.len() + 1];
    let mut sum_y = vec![0.0f32; items.len() + 1];
    let mut sum_z = vec![0.0f32; items.len() + 1];
    for (index, item) in items.iter().enumerate() {
        let (y, z) = match item.kind {
            ItemKind::Glue { stretch, shrink } => (stretch, shrink),
            ItemKind::Box => (0.0, 0.0),
        };
        sum_w[index + 1] = sum_w[index] + item.width;
        sum_y[index + 1] = sum_y[index] + y;
        sum_z[index + 1] = sum_z[index] + z;
    }
    // 行从断点之后第一个非胶条目开始。
    let line_start = |at: usize| {
        let mut start = at;
        while start < items.len() && matches!(items[start].kind, ItemKind::Glue { .. }) {
            start += 1;
        }
        start
    };
    let mut nodes = vec![Node { at: 0, line: 0, fitness: 1, demerits: 0.0, previous: None }];
    let mut active: Vec<usize> = vec![0];
    let candidates = items
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            matches!(item.kind, ItemKind::Glue { .. })
                && *index > 0
                && matches!(items[index - 1].kind, ItemKind::Box)
        })
        .map(|(index, _)| index)
        .chain(std::iter::once(items.len()));

    for at in candidates {
        let last = at == items.len();
        let mut best: [Option<(f32, usize, f32)>; 4] = [None; 4];
        active.retain(|&node_index| {
            let node = nodes[node_index];
            let start = line_start(node.at);
            if start >= at {
                return true;
            }
            let natural = sum_w[at] - sum_w[start];
            let (ratio, keep) = if natural < width {
                let stretch = sum_y[at] - sum_y[start];
                if last {
                    (0.0, true)
                } else if stretch > 0.0 {
                    ((width - natural) / stretch, true)
                } else {
                    (f32::INFINITY, true)
                }
            } else {
                let shrink = sum_z[at] - sum_z[start];
                let ratio = if shrink > 0.0 { (width - natural) / shrink } else { f32::NEG_INFINITY };
                // 已经比行宽长且无法再压缩：此后更远的断点只会更长，移出活动表。
                (ratio, ratio >= -1.0)
            };
            if (-1.0..=MAX_RATIO).contains(&ratio) {
                let badness = 100.0 * ratio.abs().powi(3);
                let fitness = if ratio < -0.5 { 0 } else if ratio <= 0.5 { 1 } else if ratio <= 1.0 { 2 } else { 3 };
                let mut demerits = (LINE_PENALTY + badness).powi(2);
                if (fitness as i8 - node.fitness as i8).abs() > 1 {
                    demerits += FITNESS_JUMP_DEMERITS;
                }
                let total = node.demerits + demerits;
                let slot = &mut best[fitness as usize];
                if slot.is_none_or(|(current, _, _)| total < current) {
                    *slot = Some((total, node_index, ratio));
                }
            }
            keep
        });
        for (fitness, entry) in best.iter().enumerate() {
            if let Some((demerits, previous, _)) = entry {
                nodes.push(Node {
                    at,
                    line: nodes[*previous].line + 1,
                    fitness: fitness as u8,
                    demerits: *demerits,
                    previous: Some(*previous),
                });
                active.push(nodes.len() - 1);
            }
        }
        if active.is_empty() {
            return greedy(items, width, &sum_w);
        }
    }

    let Some(end) = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.at == items.len())
        .min_by(|a, b| a.1.demerits.total_cmp(&b.1.demerits))
        .map(|(index, _)| index)
    else {
        return greedy(items, width, &sum_w);
    };
    let mut breaks = Vec::new();
    let mut cursor = Some(end);
    while let Some(index) = cursor {
        breaks.push(nodes[index].at);
        cursor = nodes[index].previous;
    }
    breaks.reverse();
    breaks
        .windows(2)
        .map(|pair| {
            let start = line_start(pair[0]);
            let at = pair[1];
            Line { items: start..at, ratio: line_ratio(start..at, width, at == items.len(), &sum_w, &sum_y, &sum_z) }
        })
        .collect()
}

fn line_ratio(
    range: std::ops::Range<usize>,
    width: f32,
    last: bool,
    sum_w: &[f32],
    sum_y: &[f32],
    sum_z: &[f32],
) -> f32 {
    let natural = sum_w[range.end] - sum_w[range.start];
    if natural < width {
        let stretch = sum_y[range.end] - sum_y[range.start];
        if last || stretch <= 0.0 { 0.0 } else { ((width - natural) / stretch).min(MAX_RATIO) }
    } else {
        let shrink = sum_z[range.end] - sum_z[range.start];
        if shrink <= 0.0 { 0.0 } else { ((width - natural) / shrink).max(-1.0) }
    }
}

/// 贪心断行，用作 KP 的对照组（等价于浏览器默认的首次适配换行，但不做两端对齐）。
pub fn greedy_lines(items: &[Item], width: f32) -> Vec<Line> {
    if items.is_empty() {
        return vec![Line { items: 0..0, ratio: 0.0 }];
    }
    let mut sum_w = vec![0.0f32; items.len() + 1];
    for (index, item) in items.iter().enumerate() {
        sum_w[index + 1] = sum_w[index] + item.width;
    }
    greedy(items, width, &sum_w)
}

/// 退化方案：逐条目装填，单个条目超过行宽时独占一行（由绘制层裁剪）。
fn greedy(items: &[Item], width: f32, sum_w: &[f32]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut last_break: Option<usize> = None;
    let mut index = 0;
    while index < items.len() {
        if matches!(items[index].kind, ItemKind::Glue { .. }) && index > start {
            last_break = Some(index);
        }
        let natural = sum_w[index + 1] - sum_w[start];
        if natural > width && index > start {
            let at = last_break.filter(|at| *at > start).unwrap_or(index);
            lines.push(Line { items: start..at, ratio: 0.0 });
            start = at;
            while start < items.len() && matches!(items[start].kind, ItemKind::Glue { .. }) {
                start += 1;
            }
            last_break = None;
            index = start;
            continue;
        }
        index += 1;
    }
    lines.push(Line { items: start..items.len(), ratio: 0.0 });
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

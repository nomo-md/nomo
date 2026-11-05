//! 固定 en-US Liang 模式；只识别保守的英文正文词，不产生正文字符。
//! 模式来自 hyph-en-us.tex，保留原版 4,938 条模式及 left=2/right=3，不启用额外例外词表。
use std::{ops::Range, sync::OnceLock};

const PATTERNS: &str = include_str!("../../resources/hyph-en-us.tex");
const NODES: usize = 8_399;
const WEIGHTS: usize = 25_798;
const MAX_PATTERN: usize = 9;
pub(crate) const MAX_WORD: usize = 80;

/// 紧凑兄弟链 trie：每节点 8 字节，权重另用 u8，共 92,990 字节。
/// 只在首个合格词需要匹配时创建一次；不缓存用户词语或段落。
#[derive(Default, Clone, Copy)]
struct Node {
    child: u16,
    sibling: u16,
    weights: u16,
    count: u8,
    letter: u8,
}

struct Dictionary {
    nodes: Box<[Node]>,
    weights: Box<[u8]>,
}

impl Dictionary {
    fn new() -> Self {
        let mut nodes = Vec::with_capacity(NODES);
        nodes.push(Node::default());
        let mut weights = Vec::with_capacity(WEIGHTS);
        let body = PATTERNS.split_once("\\patterns{").expect("fixed en-US patterns").1
            .split_once('}').expect("complete en-US patterns").0;
        for line in body.lines() {
            for pattern in line.split('%').next().unwrap_or("").split_whitespace() {
                let mut node = 0;
                let mut points = [0u8; MAX_PATTERN + 1];
                let mut count = 0;
                for byte in pattern.bytes() {
                    if byte.is_ascii_digit() {
                        points[count] = byte - b'0';
                        continue;
                    }
                    assert!(byte.is_ascii_lowercase() || byte == b'.');
                    assert!(count < MAX_PATTERN);
                    let mut child = nodes[node].child;
                    while child != 0 && nodes[child as usize].letter != byte {
                        child = nodes[child as usize].sibling;
                    }
                    if child == 0 {
                        assert!(nodes.len() < NODES);
                        child = nodes.len() as u16;
                        nodes.push(Node { letter: byte, sibling: nodes[node].child, ..Node::default() });
                        nodes[node].child = child;
                    }
                    node = child as usize;
                    count += 1;
                }
                assert!(weights.len() + count < WEIGHTS);
                nodes[node].weights = weights.len() as u16;
                nodes[node].count = (count + 1) as u8;
                weights.extend_from_slice(&points[..=count]);
            }
        }
        assert_eq!(nodes.len(), NODES);
        assert_eq!(weights.len(), WEIGHTS);
        Self { nodes: nodes.into_boxed_slice(), weights: weights.into_boxed_slice() }
    }

    fn points(&self, word: &str) -> Vec<usize> {
        let mut letters = [b'.'; MAX_WORD + 2];
        for (to, from) in letters[1..].iter_mut().zip(word.bytes()) {
            *to = from.to_ascii_lowercase();
        }
        let length = word.len() + 2;
        let mut points = [0u8; MAX_WORD + 3];
        for start in 0..length {
            let mut node = 0;
            for &letter in letters[start..length.min(start + MAX_PATTERN)].iter() {
                let mut child = self.nodes[node].child;
                while child != 0 && self.nodes[child as usize].letter != letter {
                    child = self.nodes[child as usize].sibling;
                }
                if child == 0 { break; }
                node = child as usize;
                let entry = self.nodes[node];
                for (offset, &weight) in self.weights[entry.weights as usize..][..entry.count as usize].iter().enumerate() {
                    points[start + offset] = points[start + offset].max(weight);
                }
            }
        }
        (2..=word.len() - 3).filter(|&at| points[at + 1] % 2 == 1).collect()
    }
}

/// 调用者先排除链接、代码、非英文语言和不可拆塑形簇，才触发字典初始化。
pub(crate) fn points(word: &str) -> Vec<usize> {
    if !(8..=MAX_WORD).contains(&word.len()) || !word.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Vec::new();
    }
    static ENGLISH: OnceLock<Dictionary> = OnceLock::new();
    ENGLISH.get_or_init(Dictionary::new).points(word)
}

fn delimiter(ch: char) -> bool {
    ch.is_whitespace() || "，。、；：！？（）《》“”‘’".contains(ch)
}

fn word_neighbor(ch: char) -> bool {
    // 原版拒绝 Latin / Number / Mark / _ / - 邻接；其它非中日韩字母同样保守排除。
    ch == '_' || ch == '-' || ch.is_numeric()
        || (ch.is_alphabetic() && (!super::is_cjk_char(ch) || (0xFF00..=0xFFEF).contains(&(ch as u32))))
        || unicode_normalization::char::is_combining_mark(ch)
}

/// 按 token 单次扫描，避免一个长 URL 内每个英文片段反复向两端搜索。
pub(crate) fn for_each_word(text: &str, mut visit: impl FnMut(Range<usize>)) {
    let mut start = 0;
    for end in text.char_indices().filter_map(|(at, ch)| delimiter(ch).then_some(at))
        .chain(std::iter::once(text.len())) {
        let token = &text[start..end];
        let bytes = token.as_bytes();
        let address = bytes.iter().any(|byte| matches!(byte, b'@' | b'/' | b'\\'))
            || bytes.windows(3).any(|part| part[0].is_ascii_alphanumeric() && part[1] == b'.' && part[2].is_ascii_alphanumeric())
            || bytes.windows(5).any(|part| part[..4].eq_ignore_ascii_case(b"http") && matches!(part[4], b'.' | b':'))
            || bytes.windows(6).any(|part| part[..5].eq_ignore_ascii_case(b"https") && matches!(part[5], b'.' | b':'))
            || bytes.windows(4).any(|part| part[..3].eq_ignore_ascii_case(b"www") && matches!(part[3], b'.' | b':'));
        if !address {
            let mut at = 0;
            while at < bytes.len() {
                if !bytes[at].is_ascii_alphabetic() { at += 1; continue; }
                let from = at;
                while at < bytes.len() && bytes[at].is_ascii_alphabetic() { at += 1; }
                let word = &token[from..at];
                if !(8..=MAX_WORD).contains(&word.len())
                    || !word.as_bytes()[1..].iter().all(|byte| byte.is_ascii_lowercase())
                    || token[..from].chars().next_back().is_some_and(word_neighbor)
                    || token[at..].chars().next().is_some_and(word_neighbor) {
                    continue;
                }
                visit(start + from..start + at);
            }
        }
        start = end + text[end..].chars().next().map_or(0, char::len_utf8);
    }
}

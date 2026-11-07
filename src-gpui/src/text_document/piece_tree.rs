use super::{TextDocumentError, TextDocumentResult};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

static NEXT_NODE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum PieceSource {
    Original { offset: u64, length: u64 },
    Added { offset: u64, length: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Piece {
    pub(super) source: PieceSource,
    // 原文件首窗返回时尚未完成全量扫描，因此换行统计允许暂时未知。
    pub(super) line_breaks: Option<u64>,
}

impl Piece {
    pub(super) fn original(offset: u64, length: u64) -> Self {
        Self {
            source: PieceSource::Original { offset, length },
            line_breaks: None,
        }
    }

    pub(super) fn added(offset: u64, length: u64, line_breaks: u64) -> Self {
        Self {
            source: PieceSource::Added { offset, length },
            line_breaks: Some(line_breaks),
        }
    }

    pub(super) fn len(&self) -> u64 {
        match self.source {
            PieceSource::Original { length, .. } | PieceSource::Added { length, .. } => length,
        }
    }

    fn slice(&self, start: u64, length: u64) -> Self {
        let source = match self.source {
            PieceSource::Original { offset, .. } => PieceSource::Original {
                offset: offset + start,
                length,
            },
            PieceSource::Added { offset, .. } => PieceSource::Added {
                offset: offset + start,
                length,
            },
        };
        Self {
            source,
            // 未读取被切分区间时不能猜测换行数；行索引负责精确映射。
            line_breaks: if start == 0 && length == self.len() {
                self.line_breaks
            } else {
                None
            },
        }
    }
}

type Link = Option<Arc<Node>>;

#[derive(Debug)]
struct Node {
    piece: Piece,
    left: Link,
    right: Link,
    priority: u64,
    logical_len: u64,
    line_breaks: Option<u64>,
}

impl Node {
    fn new(piece: Piece) -> Arc<Self> {
        make_node(piece, None, None, next_priority())
    }
}

/// 持久化隐式 Treap；clone 只增加根节点引用，编辑和按字节定位均为期望 O(log n)。
#[derive(Debug, Clone)]
pub(super) struct PieceTree {
    root: Link,
}

impl PieceTree {
    pub(super) fn from_original(length: u64) -> Self {
        Self {
            root: (length > 0).then(|| Node::new(Piece::original(0, length))),
        }
    }

    pub(super) fn len(&self) -> u64 {
        link_len(&self.root)
    }

    /// 定位源片段，不分配全文片段列表；保存点比较可跳过引用完全相同的长区间。
    pub(super) fn piece_at(&self, offset: u64) -> Option<(&Piece, u64)> {
        let mut link = self.root.as_ref();
        let mut offset = offset;
        while let Some(node) = link {
            let left = link_len(&node.left);
            if offset < left { link = node.left.as_ref(); }
            else if offset < left + node.piece.len() { return Some((&node.piece, offset - left)); }
            else { offset -= left + node.piece.len(); link = node.right.as_ref(); }
        }
        None
    }

    pub(super) fn replace_range(
        &self,
        from: u64,
        to: u64,
        inserted: Option<Piece>,
    ) -> TextDocumentResult<Self> {
        if from > to || to > self.len() {
            return Err(TextDocumentError::new(
                "invalid-edit-range",
                format!("编辑范围越界：{from}..{to}, len={}", self.len()),
            ));
        }
        let (left, remainder) = split(self.root.clone(), from)?;
        let (_, right) = split(remainder, to - from)?;
        let middle = inserted.filter(|piece| piece.len() > 0).map(Node::new);
        Ok(Self {
            root: merge(merge(left, middle), right),
        })
    }

    pub(super) fn replace_all_with_added(
        &self,
        offset: u64,
        length: u64,
        line_breaks: u64,
    ) -> Self {
        Self {
            root: (length > 0).then(|| Node::new(Piece::added(offset, length, line_breaks))),
        }
    }

    /// 只展开与目标字节范围相交的叶片，避免每次窗口读取线性扫描全部 Piece。
    pub(super) fn segments(
        &self,
        start: u64,
        end: u64,
    ) -> TextDocumentResult<Vec<(Piece, u64, u64)>> {
        if start > end || end > self.len() {
            return Err(TextDocumentError::new(
                "invalid-read-range",
                format!("Piece 读取范围越界：{start}..{end}, len={}", self.len()),
            ));
        }
        let mut output = Vec::new();
        collect_segments(&self.root, 0, start, end, &mut output);
        Ok(output)
    }

    #[cfg(test)]
    pub(super) fn aggregate_line_breaks(&self) -> Option<u64> {
        self.root.as_ref().and_then(|root| root.line_breaks)
    }
}

fn split(root: Link, offset: u64) -> TextDocumentResult<(Link, Link)> {
    let Some(node) = root else {
        return if offset == 0 {
            Ok((None, None))
        } else {
            Err(TextDocumentError::new(
                "piece-tree-split-invalid",
                "Piece Tree 分割位置越界",
            ))
        };
    };
    if offset > node.logical_len {
        return Err(TextDocumentError::new(
            "piece-tree-split-invalid",
            "Piece Tree 分割位置越界",
        ));
    }
    let left_len = link_len(&node.left);
    let piece_end = left_len + node.piece.len();
    if offset < left_len {
        let (left, remainder) = split(node.left.clone(), offset)?;
        let right = Some(make_node(
            node.piece.clone(),
            remainder,
            node.right.clone(),
            node.priority,
        ));
        return Ok((left, right));
    }
    if offset > piece_end {
        let (remainder, right) = split(node.right.clone(), offset - piece_end)?;
        let left = Some(make_node(
            node.piece.clone(),
            node.left.clone(),
            remainder,
            node.priority,
        ));
        return Ok((left, right));
    }
    if offset == left_len {
        let right = Some(make_node(
            node.piece.clone(),
            None,
            node.right.clone(),
            node.priority,
        ));
        return Ok((node.left.clone(), right));
    }
    if offset == piece_end {
        let left = Some(make_node(
            node.piece.clone(),
            node.left.clone(),
            None,
            node.priority,
        ));
        return Ok((left, node.right.clone()));
    }

    let within = offset - left_len;
    let left_piece = Node::new(node.piece.slice(0, within));
    let right_piece = Node::new(node.piece.slice(within, node.piece.len() - within));
    Ok((
        merge(node.left.clone(), Some(left_piece)),
        merge(Some(right_piece), node.right.clone()),
    ))
}

fn merge(left: Link, right: Link) -> Link {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) if left.priority <= right.priority => {
            let merged_right = merge(left.right.clone(), Some(right));
            Some(make_node(
                left.piece.clone(),
                left.left.clone(),
                merged_right,
                left.priority,
            ))
        }
        (Some(left), Some(right)) => {
            let merged_left = merge(Some(left), right.left.clone());
            Some(make_node(
                right.piece.clone(),
                merged_left,
                right.right.clone(),
                right.priority,
            ))
        }
    }
}

fn make_node(piece: Piece, left: Link, right: Link, priority: u64) -> Arc<Node> {
    let logical_len = link_len(&left) + piece.len() + link_len(&right);
    let line_breaks = combine_line_breaks(
        left.as_ref().and_then(|node| node.line_breaks),
        piece.line_breaks,
        right.as_ref().and_then(|node| node.line_breaks),
        left.is_none(),
        right.is_none(),
    );
    Arc::new(Node {
        piece,
        left,
        right,
        priority,
        logical_len,
        line_breaks,
    })
}

fn combine_line_breaks(
    left: Option<u64>,
    current: Option<u64>,
    right: Option<u64>,
    left_empty: bool,
    right_empty: bool,
) -> Option<u64> {
    let left = if left_empty { Some(0) } else { left }?;
    let right = if right_empty { Some(0) } else { right }?;
    Some(left + current? + right)
}

fn collect_segments(
    root: &Link,
    subtree_start: u64,
    wanted_start: u64,
    wanted_end: u64,
    output: &mut Vec<(Piece, u64, u64)>,
) {
    let Some(node) = root else {
        return;
    };
    let subtree_end = subtree_start + node.logical_len;
    if subtree_end <= wanted_start || subtree_start >= wanted_end {
        return;
    }
    let left_len = link_len(&node.left);
    collect_segments(&node.left, subtree_start, wanted_start, wanted_end, output);
    let piece_start = subtree_start + left_len;
    let piece_end = piece_start + node.piece.len();
    if piece_end > wanted_start && piece_start < wanted_end {
        let overlap_start = piece_start.max(wanted_start);
        let overlap_end = piece_end.min(wanted_end);
        output.push((
            node.piece.clone(),
            overlap_start - piece_start,
            overlap_end - overlap_start,
        ));
    }
    collect_segments(&node.right, piece_end, wanted_start, wanted_end, output);
}

fn link_len(link: &Link) -> u64 {
    link.as_ref().map_or(0, |node| node.logical_len)
}

fn next_priority() -> u64 {
    // SplitMix64 仅用于平衡树形，不参与安全随机；序列确定且不需要外部 RNG 依赖。
    let mut value = NEXT_NODE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_tree_splits_and_reports_only_overlapping_segments() {
        let original = PieceTree::from_original(10);
        let edited = original
            .replace_range(3, 7, Some(Piece::added(0, 2, 1)))
            .expect("replace");

        assert_eq!(original.len(), 10, "旧 revision 必须保持不可变");
        assert_eq!(edited.len(), 8);
        let segments = edited.segments(2, 7).expect("segments");
        assert_eq!(segments.iter().map(|entry| entry.2).sum::<u64>(), 5);
        assert_eq!(segments.len(), 3);
        assert_eq!(edited.aggregate_line_breaks(), None);
    }
}

//! 派生历史恢复会替换整个逻辑范围；用共享源片段证明块边界仍属于同一块。
use crate::text_document::piece_tree::{PieceSource, PieceTree};

// 只在跨块失效时临时建立，最多 128 KiB；无法证明的身份仍按删除/新增处理。
const LIMIT: usize = 4096;
pub(super) struct SourceAnchors(Vec<(u8, u64, u64, u64)>);
impl SourceAnchors {
    pub fn new(tree: &PieceTree) -> Option<Self> {
        let mut ranges = Vec::new();
        let mut position = 0;
        while position < tree.len() {
            if ranges.len() == LIMIT { return None; }
            let (piece, _) = tree.piece_at(position)?;
            let (kind, offset) = match piece.source {
                PieceSource::Original { offset, .. } => (0, offset),
                PieceSource::Added { offset, .. } => (1, offset),
            };
            ranges.push((kind, offset, piece.len(), position));
            position += piece.len();
        }
        ranges.sort_unstable_by_key(|range| (range.0, range.1));
        // 复制/重排可能重复引用同一片段；重叠来源不能用作唯一身份。
        if ranges.windows(2).any(|pair| pair[0].0 == pair[1].0 && pair[0].1 + pair[0].2 > pair[1].1) {
            return None;
        }
        Some(Self(ranges))
    }
    pub fn original_position(&self, tree: &PieceTree, position: u64) -> Option<u64> {
        let (piece, within) = tree.piece_at(position)?;
        let (kind, offset) = match piece.source {
            PieceSource::Original { offset, .. } => (0, offset + within),
            PieceSource::Added { offset, .. } => (1, offset + within),
        };
        let index = self.0.partition_point(|range| (range.0, range.1) <= (kind, offset));
        let range = self.0.get(index.checked_sub(1)?)?;
        (range.0 == kind && offset < range.1 + range.2).then(|| range.3 + offset - range.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_document::piece_tree::Piece;

    #[test]
    fn shared_sources_map_across_edits_without_matching_new_lookalike_bytes() {
        let original = PieceTree::from_original(20);
        let edited = original.replace_range(4, 6, Some(Piece::added(0, 3, 0))).unwrap();
        let anchors = SourceAnchors::new(&edited).unwrap();
        assert_eq!(anchors.original_position(&original, 3), Some(3));
        assert_eq!(anchors.original_position(&original, 4), None);
        assert_eq!(anchors.original_position(&original, 6), Some(7));
        let unrelated = original.replace_range(0, 20, Some(Piece::added(3, 20, 0))).unwrap();
        assert_eq!(anchors.original_position(&unrelated, 0), None);
        let duplicated = original.replace_range(4, 4, Some(Piece::original(0, 2))).unwrap();
        assert!(SourceAnchors::new(&duplicated).is_none());
    }
}

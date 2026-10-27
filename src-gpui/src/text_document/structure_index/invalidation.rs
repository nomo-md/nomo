//! 多次编辑合并为上一已发布修订中的保守范围，不保存插入正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Invalidation {
    pub from: u64,
    pub to: u64,
    pub delta: i64,
}
impl Invalidation {
    pub fn merge(previous: Option<Self>, edits: &[(u64, u64, u64)]) -> Option<Self> {
        if edits.is_empty() {
            return previous;
        }
        let original = |position: u64, end: bool| {
            let Some(old) = previous else {
                return position;
            };
            if position <= old.from {
                position
            } else if position >= old.to.saturating_add_signed(old.delta) {
                position.saturating_add_signed(-old.delta)
            } else if end {
                old.to
            } else {
                old.from
            }
        };
        Some(Self {
            from: edits.iter().fold(
                previous.map(|old| old.from).unwrap_or(u64::MAX),
                |from, (start, _, _)| from.min(original(*start, false)),
            ),
            to: edits.iter().fold(
                previous.map(|old| old.to).unwrap_or(0),
                |to, (_, end, _)| to.max(original(*end, true)),
            ),
            delta: previous.map(|old| old.delta).unwrap_or(0)
                + edits
                    .iter()
                    .map(|(from, to, inserted)| *inserted as i64 - (*to - *from) as i64)
                    .sum::<i64>(),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successive_edits_use_original_coordinates() {
        let first = Invalidation::merge(None, &[(3, 5, 7)]).unwrap();
        assert_eq!(
            Invalidation::merge(Some(first), &[(6, 9, 1)]),
            Some(Invalidation {
                from: 3,
                to: 5,
                delta: 3
            })
        );
        assert_eq!(
            Invalidation::merge(Some(first), &[(20, 22, 0)]),
            Some(Invalidation {
                from: 3,
                to: 17,
                delta: 3
            })
        );
    }
}

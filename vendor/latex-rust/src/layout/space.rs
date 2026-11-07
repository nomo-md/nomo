//! TeXbook Table 18 inter-atom spacing (mu). Negative table entries vanish in script style.

use crate::dim::Dim;
use crate::layout::metrics::MathParams;
use crate::layout::style::MathStyle;
use crate::parser::AtomKind;

/// Space in mu: 0 none, 3 thin, 4 medium, 5 thick.
pub fn atom_space_mu(left: AtomKind, right: AtomKind, style: MathStyle) -> i64 {
    let l = idx(left);
    let r = idx(right);
    let raw = TABLE[l][r];
    if raw < 0 {
        if style.script_level() > 0 {
            0
        } else {
            i64::from(-raw)
        }
    } else {
        i64::from(raw)
    }
}

/// Convert a TeX Bin that cannot stay binary (start of list, or next to Rel/Close/…).
#[must_use]
pub fn convert_bin(prev: Option<AtomKind>, this: AtomKind, next: Option<AtomKind>) -> AtomKind {
    if this != AtomKind::Bin {
        return this;
    }
    let prev_bad = matches!(
        prev,
        None | Some(
            AtomKind::Bin | AtomKind::Op | AtomKind::Rel | AtomKind::Open | AtomKind::Punct
        )
    );
    let next_bad = matches!(
        next,
        None | Some(AtomKind::Rel | AtomKind::Close | AtomKind::Punct)
    );
    if prev_bad || next_bad {
        AtomKind::Ord
    } else {
        AtomKind::Bin
    }
}

/// Kern width for a Table 18 space at `style`.
pub fn space_width(mu: i64, params: &MathParams, style: MathStyle) -> Dim {
    if mu == 0 {
        return Dim::zero();
    }
    params.mu(style) * Dim::from_i64(mu)
}

fn idx(k: AtomKind) -> usize {
    match k {
        AtomKind::Ord => 0,
        AtomKind::Op => 1,
        AtomKind::Bin => 2,
        AtomKind::Rel => 3,
        AtomKind::Open => 4,
        AtomKind::Close => 5,
        AtomKind::Punct => 6,
        AtomKind::Inner => 7,
    }
}

// TeXbook p. 170. Values 0, 3, 4, 5 mu. Negative = not in script style.
const TABLE: [[i8; 8]; 8] = [
    //        Ord Op Bin Rel Open Close Punct Inner
    /* Ord */
    [0, 3, -4, -5, 0, 0, 0, -3],
    /* Op */ [3, 3, 0, -5, 0, 0, 0, -3],
    /* Bin */ [-4, -4, 0, 0, -4, 0, 0, -4],
    /* Rel */ [-5, -5, 0, 0, 5, 0, 0, -5],
    /* Open */ [0, 0, 0, 0, 0, 0, 0, 0],
    /* Close */ [0, 3, -4, -5, 0, 0, 0, -3],
    /* Punct */ [-3, -3, 0, -3, -3, -3, -3, -3],
    /* Inner */ [-3, 3, -4, -5, -3, 0, -3, -3],
];

#[cfg(test)]
mod tests {
    use super::{atom_space_mu, convert_bin, AtomKind, MathStyle};

    const KINDS: [AtomKind; 8] = [
        AtomKind::Ord,
        AtomKind::Op,
        AtomKind::Bin,
        AtomKind::Rel,
        AtomKind::Open,
        AtomKind::Close,
        AtomKind::Punct,
        AtomKind::Inner,
    ];

    #[test]
    fn table_18_nonzero_text() {
        let want: [[i64; 8]; 8] = [
            [0, 3, 4, 5, 0, 0, 0, 3],
            [3, 3, 0, 5, 0, 0, 0, 3],
            [4, 4, 0, 0, 4, 0, 0, 4],
            [5, 5, 0, 0, 5, 0, 0, 5],
            [0, 0, 0, 0, 0, 0, 0, 0],
            [0, 3, 4, 5, 0, 0, 0, 3],
            [3, 3, 0, 3, 3, 3, 3, 3],
            [3, 3, 4, 5, 3, 0, 3, 3],
        ];
        for (i, l) in KINDS.iter().enumerate() {
            for (j, r) in KINDS.iter().enumerate() {
                assert_eq!(
                    atom_space_mu(*l, *r, MathStyle::Text),
                    want[i][j],
                    "{l:?} {r:?}"
                );
            }
        }
    }

    #[test]
    fn table_18_script_drops_negative() {
        assert_eq!(
            atom_space_mu(AtomKind::Ord, AtomKind::Bin, MathStyle::Script),
            0
        );
        assert_eq!(
            atom_space_mu(AtomKind::Ord, AtomKind::Rel, MathStyle::Script),
            0
        );
        assert_eq!(
            atom_space_mu(AtomKind::Op, AtomKind::Ord, MathStyle::Script),
            3
        );
        assert_eq!(
            atom_space_mu(AtomKind::Open, AtomKind::Bin, MathStyle::Text),
            0
        );
    }

    #[test]
    fn convert_bin_after_open() {
        assert_eq!(
            convert_bin(Some(AtomKind::Open), AtomKind::Bin, Some(AtomKind::Ord)),
            AtomKind::Ord
        );
        assert_eq!(
            convert_bin(Some(AtomKind::Ord), AtomKind::Bin, Some(AtomKind::Ord)),
            AtomKind::Bin
        );
        assert_eq!(
            convert_bin(None, AtomKind::Bin, Some(AtomKind::Ord)),
            AtomKind::Ord
        );
    }
}

//! TeX atom classes for named symbols (TeXbook Table 18).

use crate::parser::AtomKind;

/// Atom class for a control-sequence name (`times`, `leq`, `langle`) or a
/// single-character operator (`+`, `=`).
///
/// # Examples
///
/// ```
/// use latex_rust::{symbol_atom_kind, AtomKind};
///
/// assert_eq!(symbol_atom_kind("times"), AtomKind::Bin);
/// assert_eq!(symbol_atom_kind("sum"), AtomKind::Op);
/// ```
#[must_use]
pub fn symbol_atom_kind(name: &str) -> AtomKind {
    match name {
        "sum" | "prod" | "coprod" | "int" | "iint" | "iiint" | "oint" | "oiint" | "bigwedge"
        | "bigvee" | "bigcup" | "bigcap" | "bigsqcup" | "bigoplus" | "bigotimes" | "biguplus"
        | "bigodot" | "lim" => AtomKind::Op,

        "+" | "-" | "*" | "times" | "div" | "cdot" | "pm" | "mp" | "ast" | "star" | "circ"
        | "bullet" | "cap" | "cup" | "sqcap" | "sqcup" | "vee" | "wedge" | "land" | "lor"
        | "oplus" | "ominus" | "otimes" | "oslash" | "odot" | "bigcirc" | "dagger" | "ddagger"
        | "amalg" | "setminus" | "wr" | "triangleleft" | "triangleright" | "dotplus" | "ltimes"
        | "rtimes" | "leftthreetimes" | "rightthreetimes" | "circledcirc" | "circledast"
        | "circleddash" | "boxplus" | "boxminus" | "boxtimes" | "boxdot" | "intercal"
        | "divideontimes" | "curlyvee" | "curlywedge" | "barwedge" | "veebar"
        | "doublebarwedge" | "Cap" | "Cup" | "centerdot" | "diamond" | "dag" | "ddag" => {
            AtomKind::Bin
        }

        "="
        | "<"
        | ">"
        | "leq"
        | "geq"
        | "neq"
        | "ll"
        | "gg"
        | "doteq"
        | "sim"
        | "simeq"
        | "approx"
        | "cong"
        | "equiv"
        | "prec"
        | "succ"
        | "preceq"
        | "succeq"
        | "subset"
        | "supset"
        | "subseteq"
        | "supseteq"
        | "sqsubset"
        | "sqsupset"
        | "sqsubseteq"
        | "sqsupseteq"
        | "in"
        | "ni"
        | "notin"
        | "vdash"
        | "dashv"
        | "models"
        | "perp"
        | "mid"
        | "parallel"
        | "bowtie"
        | "Join"
        | "smile"
        | "frown"
        | "asymp"
        | "propto"
        | "between"
        | "to"
        | "gets"
        | "leftarrow"
        | "rightarrow"
        | "leftrightarrow"
        | "Leftarrow"
        | "Rightarrow"
        | "Leftrightarrow"
        | "longleftarrow"
        | "longrightarrow"
        | "longleftrightarrow"
        | "Longleftarrow"
        | "Longrightarrow"
        | "Longleftrightarrow"
        | "uparrow"
        | "downarrow"
        | "updownarrow"
        | "Uparrow"
        | "Downarrow"
        | "Updownarrow"
        | "nearrow"
        | "searrow"
        | "swarrow"
        | "nwarrow"
        | "hookleftarrow"
        | "hookrightarrow"
        | "leftharpoonup"
        | "leftharpoondown"
        | "rightharpoonup"
        | "rightharpoondown"
        | "rightleftharpoons"
        | "leftrightharpoons"
        | "mapsto"
        | "longmapsto"
        | "iff"
        | "implies"
        | "impliedby"
        | "colon"
        | "nless"
        | "ngtr"
        | "nleq"
        | "ngeq"
        | "nsim"
        | "ncong"
        | "nprec"
        | "nsucc"
        | "nvdash"
        | "nvDash"
        | "nVdash"
        | "nVDash"
        | "leqq"
        | "geqq"
        | "leqslant"
        | "geqslant"
        | "eqslantless"
        | "eqslantgtr"
        | "lesssim"
        | "gtrsim"
        | "lessapprox"
        | "gtrapprox"
        | "approxeq"
        | "lessdot"
        | "gtrdot"
        | "lll"
        | "ggg"
        | "lessgtr"
        | "gtrless"
        | "lesseqgtr"
        | "gtreqless"
        | "lesseqqgtr"
        | "gtreqqless"
        | "doteqdot"
        | "eqcirc"
        | "circeq"
        | "triangleq"
        | "risingdotseq"
        | "fallingdotseq"
        | "backsim"
        | "backsimeq"
        | "subseteqq"
        | "supseteqq"
        | "Subset"
        | "Supset"
        | "preccurlyeq"
        | "succcurlyeq"
        | "curlyeqprec"
        | "curlyeqsucc"
        | "precsim"
        | "succsim"
        | "precapprox"
        | "succapprox"
        | "varsubsetneq"
        | "varsupsetneq"
        | "subsetneq"
        | "supsetneq"
        | "subsetneqq"
        | "supsetneqq"
        | "vDash"
        | "Vdash"
        | "Vvdash"
        | "smallsmile"
        | "smallfrown"
        | "bumpeq"
        | "Bumpeq"
        | "therefore"
        | "because"
        | "eqsim"
        | "nsubseteq"
        | "nsupseteq"
        | "nmid"
        | "nparallel"
        | "nleftarrow"
        | "nrightarrow"
        | "nLeftarrow"
        | "nRightarrow"
        | "nleftrightarrow"
        | "nLeftrightarrow"
        | "ntriangleleft"
        | "ntriangleright"
        | "ntrianglelefteq"
        | "ntrianglerighteq"
        | "vartriangleleft"
        | "vartriangleright"
        | "trianglelefteq"
        | "trianglerighteq"
        | "pitchfork"
        | "backepsilon"
        | "blacktriangleleft"
        | "blacktriangleright"
        | "lhd"
        | "rhd"
        | "unlhd"
        | "unrhd"
        | "dashleftarrow"
        | "dashrightarrow"
        | "leftleftarrows"
        | "rightrightarrows"
        | "leftrightarrows"
        | "rightleftarrows"
        | "Lleftarrow"
        | "Rrightarrow"
        | "twoheadleftarrow"
        | "twoheadrightarrow"
        | "leftarrowtail"
        | "rightarrowtail"
        | "looparrowleft"
        | "looparrowright"
        | "curvearrowleft"
        | "curvearrowright"
        | "circlearrowleft"
        | "circlearrowright"
        | "Lsh"
        | "Rsh"
        | "upuparrows"
        | "downdownarrows"
        | "upharpoonleft"
        | "upharpoonright"
        | "downharpoonleft"
        | "downharpoonright"
        | "multimap"
        | "rightsquigarrow"
        | "leftrightsquigarrow"
        | "leadsto"
        | "restriction"
        | "owns"
        | "le"
        | "ge"
        | "ne"
        | "notequiv" => AtomKind::Rel,

        "(" | "[" | "{" | "langle" | "lfloor" | "lceil" | "lvert" | "lVert" | "ulcorner"
        | "llcorner" => AtomKind::Open,

        ")" | "]" | "}" | "rangle" | "rfloor" | "rceil" | "rvert" | "rVert" | "urcorner"
        | "lrcorner" => AtomKind::Close,

        "," | ";" | "ldotp" | "cdotp" | "ldots" | "dotsc" | "dotso" => AtomKind::Punct,

        "cdots" | "vdots" | "ddots" | "iddots" | "dotsb" | "dotsm" | "dotsi" | "dots" => {
            AtomKind::Inner
        }

        "vert" | "Vert" | "|" => AtomKind::Ord,

        _ => AtomKind::Ord,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_examples() {
        assert_eq!(symbol_atom_kind("alpha"), AtomKind::Ord);
        assert_eq!(symbol_atom_kind("infty"), AtomKind::Ord);
        assert_eq!(symbol_atom_kind("sum"), AtomKind::Op);
        assert_eq!(symbol_atom_kind("times"), AtomKind::Bin);
        assert_eq!(symbol_atom_kind("+"), AtomKind::Bin);
        assert_eq!(symbol_atom_kind("leq"), AtomKind::Rel);
        assert_eq!(symbol_atom_kind("="), AtomKind::Rel);
        assert_eq!(symbol_atom_kind("langle"), AtomKind::Open);
        assert_eq!(symbol_atom_kind("rangle"), AtomKind::Close);
        assert_eq!(symbol_atom_kind("ldots"), AtomKind::Punct);
        assert_eq!(symbol_atom_kind("rightarrow"), AtomKind::Rel);
    }
}

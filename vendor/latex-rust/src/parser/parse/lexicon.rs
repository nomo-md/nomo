//! Unicode atoms that are always on, and the optional lexicons.
//!
//! A bare word is one maximal run of ASCII letters. The parser accepts the
//! run only when the whole word is in this table, so `sinh` is not `sin`
//! plus `h` and `xsin` stays letters. The rule is documented on
//! [`ParseOptions::with_bare_operators`](super::ParseOptions::with_bare_operators).

use crate::parser::ast::{MathNode, TextRun, TextStyle};

pub(crate) fn is_superscript_char(c: char) -> bool {
    superscript_base(c).is_some()
}

/// Map a superscript digit, letter, or minus back to the ASCII character
/// [`super::atom_kind`] classifies.
pub(super) fn superscript_base(c: char) -> Option<char> {
    Some(match c {
        '\u{2070}' => '0',
        '\u{00B9}' => '1',
        '\u{00B2}' => '2',
        '\u{00B3}' => '3',
        '\u{2074}' => '4',
        '\u{2075}' => '5',
        '\u{2076}' => '6',
        '\u{2077}' => '7',
        '\u{2078}' => '8',
        '\u{2079}' => '9',
        '\u{207B}' => '-',
        '\u{1D43}' => 'a',
        '\u{1D47}' => 'b',
        '\u{1D9C}' => 'c',
        '\u{1D48}' => 'd',
        '\u{1D49}' => 'e',
        '\u{1DA0}' => 'f',
        '\u{1D4D}' => 'g',
        '\u{02B0}' => 'h',
        '\u{2071}' => 'i',
        '\u{02B2}' => 'j',
        '\u{1D4F}' => 'k',
        '\u{02E1}' => 'l',
        '\u{1D50}' => 'm',
        '\u{207F}' => 'n',
        '\u{1D52}' => 'o',
        '\u{1D56}' => 'p',
        '\u{107A5}' => 'q',
        '\u{02B3}' => 'r',
        '\u{02E2}' => 's',
        '\u{1D57}' => 't',
        '\u{1D58}' => 'u',
        '\u{1D5B}' => 'v',
        '\u{02B7}' => 'w',
        '\u{02E3}' => 'x',
        '\u{02B8}' => 'y',
        '\u{1DBB}' => 'z',
        _ => return None,
    })
}

pub(super) fn unicode_symbol(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "alpha",
        'Γ' => "Gamma",
        'π' => "pi",
        '≤' => "leq",
        '≥' => "geq",
        '≠' => "neq",
        '∞' => "infty",
        '×' => "times",
        '·' | '⋅' => "cdot",
        _ => return None,
    })
}

/// `in_cases` makes the identifier `otherwise` a text node. Every other word
/// is the operator table or `\zeta`.
pub(super) fn bare_identifier(word: &str, in_cases: bool) -> Option<MathNode> {
    if in_cases && word == "otherwise" {
        return Some(MathNode::Text(TextRun::plain("otherwise"), TextStyle::Text));
    }
    if word == "zeta" {
        return Some(MathNode::Symbol("zeta".into()));
    }
    let limits = operator_limits(word)?;
    Some(MathNode::Operator(word.to_string(), limits))
}

fn operator_limits(word: &str) -> Option<bool> {
    Some(match word {
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh"
        | "cosh" | "tanh" | "coth" | "log" | "ln" | "lg" | "exp" | "limsup" | "liminf" | "sup"
        | "inf" | "max" | "min" | "det" | "dim" | "ker" | "deg" | "gcd" | "lcm" | "Pr" | "arg" => {
            false
        }
        "coprod" | "bigcup" | "bigcap" | "bigsqcup" | "bigvee" | "bigwedge" | "bigoplus"
        | "bigotimes" | "biguplus" => true,
        _ => return None,
    })
}

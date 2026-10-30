//! Input trim run before tokenization.
//!
//! Before 3.0.0 this module also rewrote the input string (`{a \over b}` →
//! `\frac`, `{\rm x}` → `\mathrm`, `{ (}` → `(`, `\mbox` → `\text`). Those
//! rewrites could match bytes inside text bodies and escapes, so they are now
//! token rules in `group_rules` and the command table (spec 3.1).

/// Normalize raw math input for the tokenizer: trim the surrounding
/// whitespace, keeping the space of a trailing control space `\ `.
///
/// # Examples
///
/// ```
/// use latex_rust::preprocess;
///
/// assert_eq!(preprocess("  {a \\over b} "), r"{a \over b}");
/// assert_eq!(preprocess(r" x\ "), r"x\ ");
/// ```
#[must_use]
pub fn preprocess(raw_input: &str) -> String {
    // `str::trim` would delete the space in a trailing `\ `, and the tokenizer
    // would then see a stray backslash. `trim_math` keeps that one space.
    trim_math(raw_input).to_string()
}

/// Drop surrounding whitespace, but keep the ASCII space of a trailing
/// control space.
///
/// A control space is an odd-length run of backslashes followed by one
/// space (`\ `, `\\\ `, …). [`str::trim`] deletes that space and leaves a
/// stray `\`, which the tokenizer reports as a trailing backslash. Extra
/// whitespace after that space is still dropped. An even run (`\\ `) is a
/// control symbol plus an ordinary space, so the space goes.
fn trim_math(input: &str) -> &str {
    let start = input.len() - input.trim_start().len();
    let body = &input[start..];
    let kept = body.trim_end();
    if kept.len() == body.len() || !ends_with_odd_backslash(kept) {
        return &input[start..start + kept.len()];
    }
    // The first discarded character is the control space only when it is an
    // ASCII space. A trailing newline after `\` is not a control space.
    if body.as_bytes().get(kept.len()) == Some(&b' ') {
        &input[start..start + kept.len() + 1]
    } else {
        &input[start..start + kept.len()]
    }
}

fn ends_with_odd_backslash(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut n = 0usize;
    while n < bytes.len() && bytes[bytes.len() - 1 - n] == b'\\' {
        n += 1;
    }
    n % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_period_kept() {
        assert_eq!(preprocess("x^2."), "x^2.");
    }

    #[test]
    fn rewrites_are_gone() {
        for s in [
            r"{a \over b}",
            r"{n \choose k}",
            r"{\rm x}",
            r"{ (}",
            r"{[}",
            r"\mbox{x}",
            r"\left(\frac{1}{2}\right)",
            r"\tfrac{a}{b}",
            r"a\,b",
        ] {
            assert_eq!(preprocess(s), s);
        }
    }

    #[test]
    fn control_space_survives_trim() {
        assert_eq!(preprocess(r"\ "), r"\ ");
        assert_eq!(preprocess(r"  \   "), r"\ ");
        assert_eq!(preprocess("\\ %@QJXi"), "\\ %@QJXi");
        assert_eq!(preprocess(r"\\ "), r"\\");
        assert_eq!(preprocess(r"\\\ "), r"\\\ ");
    }

    #[test]
    fn empty() {
        assert_eq!(preprocess("   "), "");
        assert!(matches!(
            crate::parse(""),
            Err(crate::ParseError::Malformed(_))
        ));
        assert!(matches!(
            crate::parse("   "),
            Err(crate::ParseError::Malformed(_))
        ));
    }
}

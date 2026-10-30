//! Unicode Mathematical Alphanumeric Symbols for `\mathrm`, `\mathbb`, …

use crate::parser::TextStyle;

/// Map `ch` under a math font style. Unmapped characters are returned unchanged.
///
/// # Examples
///
/// ```
/// use latex_rust::{styled_char, TextStyle};
///
/// assert_eq!(styled_char('R', TextStyle::Bb), 'ℝ');
/// ```
#[must_use]
pub fn styled_char(ch: char, style: TextStyle) -> char {
    match style {
        TextStyle::Rm | TextStyle::Text | TextStyle::Pmb => ch,
        TextStyle::Bf => bold(ch).unwrap_or(ch),
        TextStyle::It => italic(ch).unwrap_or(ch),
        TextStyle::Sf => sans(ch).unwrap_or(ch),
        TextStyle::Tt => mono(ch).unwrap_or(ch),
        TextStyle::Bb => double_struck(ch).unwrap_or(ch),
        TextStyle::Cal | TextStyle::Scr => script(ch).unwrap_or(ch),
        TextStyle::Frak => fraktur(ch).unwrap_or(ch),
        TextStyle::Boldsymbol => bold_italic(ch).or_else(|| bold(ch)).unwrap_or(ch),
    }
}

/// Letters that `\mathrm` / `\mathbf` / … restyle.
#[must_use]
pub fn is_stylable(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || is_greek_letter(ch)
}

fn cp(n: u32) -> Option<char> {
    char::from_u32(n)
}

fn latin_offset(ch: char) -> Option<(bool, u32)> {
    if ch.is_ascii_uppercase() {
        Some((true, u32::from(ch) - u32::from('A')))
    } else if ch.is_ascii_lowercase() {
        Some((false, u32::from(ch) - u32::from('a')))
    } else {
        None
    }
}

fn digit_offset(ch: char) -> Option<u32> {
    if ch.is_ascii_digit() {
        Some(u32::from(ch) - u32::from('0'))
    } else {
        None
    }
}

fn is_greek_letter(ch: char) -> bool {
    let u = u32::from(ch);
    (0x0391..=0x03A9).contains(&u) && u != 0x03A2
        || (0x03B1..=0x03C9).contains(&u)
        || matches!(
            ch,
            'ϵ' | 'ϑ' | 'ϰ' | 'ϕ' | 'ϱ' | 'ϖ' | 'ϴ' | '∇' | '∂' | 'ϝ' | 'Ϝ'
        )
}

fn plane(base: u32, off: u32) -> Option<char> {
    cp(base + off)
}

fn bold(ch: char) -> Option<char> {
    if let Some((upper, off)) = latin_offset(ch) {
        return plane(if upper { 0x1D400 } else { 0x1D41A }, off);
    }
    if let Some(off) = digit_offset(ch) {
        return plane(0x1D7CE, off);
    }
    greek_bold(ch)
}

fn italic(ch: char) -> Option<char> {
    if let Some((upper, off)) = latin_offset(ch) {
        if !upper && off == 7 {
            return cp(0x210E);
        }
        return plane(if upper { 0x1D434 } else { 0x1D44E }, off);
    }
    greek_style(ch, 0x3A)
}

fn bold_italic(ch: char) -> Option<char> {
    if let Some((upper, off)) = latin_offset(ch) {
        return plane(if upper { 0x1D468 } else { 0x1D482 }, off);
    }
    greek_style(ch, 0x74)
}

fn sans(ch: char) -> Option<char> {
    if let Some((upper, off)) = latin_offset(ch) {
        return plane(if upper { 0x1D5A0 } else { 0x1D5BA }, off);
    }
    digit_offset(ch).and_then(|off| plane(0x1D7E2, off))
}

fn mono(ch: char) -> Option<char> {
    if let Some((upper, off)) = latin_offset(ch) {
        return plane(if upper { 0x1D670 } else { 0x1D68A }, off);
    }
    digit_offset(ch).and_then(|off| plane(0x1D7F6, off))
}

fn double_struck(ch: char) -> Option<char> {
    match ch {
        'C' => cp(0x2102),
        'H' => cp(0x210D),
        'N' => cp(0x2115),
        'P' => cp(0x2119),
        'Q' => cp(0x211A),
        'R' => cp(0x211D),
        'Z' => cp(0x2124),
        _ => {
            if let Some((upper, off)) = latin_offset(ch) {
                plane(if upper { 0x1D538 } else { 0x1D552 }, off)
            } else {
                digit_offset(ch).and_then(|off| plane(0x1D7D8, off))
            }
        }
    }
}

fn script(ch: char) -> Option<char> {
    match ch {
        'B' => cp(0x212C),
        'E' => cp(0x2130),
        'F' => cp(0x2131),
        'H' => cp(0x210B),
        'I' => cp(0x2110),
        'L' => cp(0x2112),
        'M' => cp(0x2133),
        'R' => cp(0x211B),
        'e' => cp(0x212F),
        'g' => cp(0x210A),
        'o' => cp(0x2134),
        _ => latin_offset(ch)
            .and_then(|(upper, off)| plane(if upper { 0x1D49C } else { 0x1D4B6 }, off)),
    }
}

fn fraktur(ch: char) -> Option<char> {
    match ch {
        'C' => cp(0x212D),
        'H' => cp(0x210C),
        'I' => cp(0x2111),
        'R' => cp(0x211C),
        'Z' => cp(0x2128),
        _ => latin_offset(ch)
            .and_then(|(upper, off)| plane(if upper { 0x1D504 } else { 0x1D51E }, off)),
    }
}

fn greek_bold(ch: char) -> Option<char> {
    let u = u32::from(ch);
    if (0x0391..=0x03A9).contains(&u) && u != 0x03A2 {
        return cp(0x1D6A8 + (u - 0x0391));
    }
    if (0x03B1..=0x03C9).contains(&u) {
        return cp(0x1D6C2 + (u - 0x03B1));
    }
    match ch {
        'ϴ' => cp(0x1D6B9),
        '∇' => cp(0x1D6C1),
        '∂' => cp(0x1D6DB),
        'ϵ' => cp(0x1D6DC),
        'ϑ' => cp(0x1D6DD),
        'ϰ' => cp(0x1D6DE),
        'ϕ' => cp(0x1D6DF),
        'ϱ' => cp(0x1D6E0),
        'ϖ' => cp(0x1D6E1),
        _ => None,
    }
}

fn greek_style(ch: char, delta: u32) -> Option<char> {
    greek_bold(ch).and_then(|c| cp(u32::from(c) + delta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blackboard_exceptions() {
        assert_eq!(styled_char('R', TextStyle::Bb), 'ℝ');
        assert_eq!(styled_char('C', TextStyle::Bb), 'ℂ');
        assert_eq!(styled_char('N', TextStyle::Bb), 'ℕ');
        assert_eq!(styled_char('Z', TextStyle::Bb), 'ℤ');
        assert_eq!(styled_char('Q', TextStyle::Bb), 'ℚ');
        assert_eq!(styled_char('P', TextStyle::Bb), 'ℙ');
        assert_eq!(styled_char('H', TextStyle::Bb), 'ℍ');
        assert_eq!(styled_char('A', TextStyle::Bb), '𝔸');
    }

    #[test]
    fn script_and_fraktur() {
        assert_eq!(styled_char('L', TextStyle::Cal), 'ℒ');
        assert_eq!(styled_char('H', TextStyle::Scr), 'ℋ');
        assert_eq!(styled_char('g', TextStyle::Frak), '𝔤');
        assert_eq!(styled_char('C', TextStyle::Frak), 'ℭ');
    }

    #[test]
    fn latin_weights() {
        assert_eq!(styled_char('d', TextStyle::Rm), 'd');
        assert_eq!(styled_char('x', TextStyle::Bf), '𝐱');
        assert_eq!(styled_char('h', TextStyle::It), 'ℎ');
        assert_eq!(styled_char('A', TextStyle::Sf), '𝖠');
        assert_eq!(styled_char('A', TextStyle::Tt), '𝙰');
    }

    #[test]
    fn bold_greek_alpha() {
        assert_eq!(styled_char('α', TextStyle::Bf), '𝛂');
        assert_eq!(styled_char('α', TextStyle::Boldsymbol), '𝜶');
        assert_eq!(styled_char('Γ', TextStyle::Bf), '𝚪');
    }
}

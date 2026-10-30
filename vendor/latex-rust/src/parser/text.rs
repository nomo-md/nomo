//! Text-mode lexer for the body of `\text{…}`, `\mbox{…}` and `\intertext{…}`.
//!
//! The tokenizer hands the raw body over as one [`Token::TextArg`](super::Token)
//! so no math-mode pass ever reads it. This module turns that body into a
//! [`TextRun`] by the grammar in `documents/TEXT_MODE_SPEC.md` section 2.
//! Nesting uses an explicit stack. Inline math goes back to the math parser
//! through a callback.

use super::ast::{MathNode, TextItem, TextRun, TextStyle};
use crate::error::ParseError;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Close {
    /// `$…$`
    Dollar,
    /// `\(…\)`
    Paren,
}

/// What a control sequence in text means.
enum Ctl {
    /// Push this item.
    Item(TextItem),
    /// Push this char (escaped special).
    Char(char),
    /// A nested text command; its `{` is consumed.
    Open(TextStyle),
    /// `\(`: inline math starts after it.
    Math,
}

/// Whitespace in text (spec 2.1 `space`).
fn is_text_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
}

fn char_at(raw: &str, i: usize) -> Option<char> {
    raw.get(i..).and_then(|s| s.chars().next())
}

/// Lexes a text body (the bytes between the braces) into a run.
///
/// `levels` is how many nesting levels are still allowed: each `{…}` group,
/// nested text command, and inline math uses one; going past it returns
/// `too_deep()`, which builds the error on demand. `math` parses inline math source with the levels left.
pub(crate) fn lex_body(
    raw: &str,
    levels: usize,
    too_deep: &dyn Fn() -> ParseError,
    math: &mut dyn FnMut(&str, usize) -> Result<MathNode, ParseError>,
) -> Result<TextRun, ParseError> {
    let mut f = Frames::default();
    let mut i = 0;

    while let Some(ch) = char_at(raw, i) {
        match ch {
            c if is_text_space(c) => {
                let mut j = i;
                while let Some(c) = char_at(raw, j).filter(|c| is_text_space(*c)) {
                    j += c.len_utf8();
                }
                f.cur().push(TextItem::Space);
                i = j;
            }
            '%' => {
                while let Some(c) = char_at(raw, i) {
                    i += c.len_utf8();
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' => {
                if f.stack.len() >= levels {
                    return Err(too_deep());
                }
                f.stack.push((None, TextRun::default()));
                i += 1;
            }
            '}' => {
                let Some((style, run)) = f.stack.pop() else {
                    return Err(ParseError::Malformed("unmatched '}'".into()));
                };
                let parent = f.cur();
                match style {
                    Some(style) => parent.push(TextItem::Nested(style, run)),
                    None => parent.append(run),
                }
                i += 1;
            }
            '~' => {
                f.cur().push(TextItem::NoBreakSpace);
                i += 1;
            }
            '$' => {
                i = push_math(raw, i + 1, Close::Dollar, levels, too_deep, math, &mut f)?;
            }
            '^' | '_' | '&' | '#' => {
                return Err(ParseError::Malformed(format!(
                    "unexpected token {ch} in text"
                )));
            }
            '\\' => {
                let (ctl, next) = control(raw, i)?;
                match ctl {
                    Ctl::Item(item) => {
                        f.cur().push(item);
                        i = next;
                    }
                    Ctl::Char(c) => {
                        f.cur().push_char(c);
                        i = next;
                    }
                    Ctl::Open(style) => {
                        if f.stack.len() >= levels {
                            return Err(too_deep());
                        }
                        f.stack.push((Some(style), TextRun::default()));
                        i = next;
                    }
                    Ctl::Math => {
                        i = push_math(raw, next, Close::Paren, levels, too_deep, math, &mut f)?;
                    }
                }
            }
            _ => {
                f.cur().push_char(ch);
                i += ch.len_utf8();
            }
        }
    }

    if !f.stack.is_empty() {
        return Err(ParseError::Malformed("unmatched '{'".into()));
    }
    Ok(f.root)
}

/// Open frames (`None` style: a plain `{…}` group) above the root run.
#[derive(Default)]
struct Frames {
    stack: Vec<(Option<TextStyle>, TextRun)>,
    root: TextRun,
}

impl Frames {
    /// The run that receives items: the innermost open frame, else the root.
    fn cur(&mut self) -> &mut TextRun {
        match self.stack.last_mut() {
            Some((_, run)) => run,
            None => &mut self.root,
        }
    }
}

/// Parses inline math starting at byte `start` and pushes it. Returns the
/// byte index after the close.
fn push_math(
    raw: &str,
    start: usize,
    close: Close,
    levels: usize,
    too_deep: &dyn Fn() -> ParseError,
    math: &mut dyn FnMut(&str, usize) -> Result<MathNode, ParseError>,
    f: &mut Frames,
) -> Result<usize, ParseError> {
    let Some((src, end)) = scan_math(raw, start, close) else {
        return Err(ParseError::Malformed("unmatched math shift in text".into()));
    };
    if f.stack.len() >= levels {
        return Err(too_deep());
    }
    let node = math(src, levels - f.stack.len() - 1)?;
    f.cur().push(TextItem::Math(Box::new(node)));
    Ok(end)
}

/// Finds the end of inline math that starts at byte `start` of `raw` (just
/// after the opening `$` or `\(`). Returns the math source and the byte index
/// just after the closing `$` or `\)`, or `None` when there is no close.
fn scan_math(raw: &str, start: usize, close: Close) -> Option<(&str, usize)> {
    let mut chars = raw.get(start..)?.char_indices().peekable();
    let mut depth = 0;

    while let Some((pos, ch)) = chars.next() {
        match ch {
            '\\' => {
                if close == Close::Paren && depth == 0 {
                    if let Some((_, next_ch)) = chars.peek() {
                        if *next_ch == ')' {
                            return Some((&raw[start..start + pos], start + pos + 2));
                        }
                    }
                }
                // Skip the next character (escape sequence or control word)
                chars.next();
            }
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            '$' if close == Close::Dollar && depth == 0 => {
                return Some((&raw[start..start + pos], start + pos + 1));
            }
            _ => (),
        }
    }
    None
}

/// Reads the control sequence whose backslash is at byte `at` of `raw`.
/// Returns its meaning and the byte index after it.
fn control(raw: &str, at: usize) -> Result<(Ctl, usize), ParseError> {
    let start = at + 1;
    let Some(first) = char_at(raw, start) else {
        return Err(ParseError::TrailingBackslash);
    };
    if !first.is_ascii_alphabetic() {
        let next = start + first.len_utf8();
        return match first {
            '{' | '}' | '$' | '%' | '&' | '#' | '_' => Ok((Ctl::Char(first), next)),
            c if is_text_space(c) => Ok((Ctl::Item(TextItem::ControlSpace), next)),
            '\\' => Err(ParseError::Malformed("line break in text".into())),
            '(' => Ok((Ctl::Math, next)),
            ')' => Err(ParseError::Malformed("unmatched math shift in text".into())),
            _ => Err(ParseError::Unknown(format!("\\{first}"))),
        };
    }
    let mut end = start;
    while char_at(raw, end).is_some_and(|c| c.is_ascii_alphabetic()) {
        end += 1;
    }
    let name = raw.get(start..end).unwrap_or_default();
    let mut next = end;
    while char_at(raw, next).is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
        next += 1;
    }
    let style = match name {
        "text" | "mbox" => TextStyle::Text,
        "textrm" => TextStyle::Rm,
        "textbf" => TextStyle::Bf,
        "textit" => TextStyle::It,
        "textsf" => TextStyle::Sf,
        "texttt" => TextStyle::Tt,
        "ldots" | "dots" => return Ok((Ctl::Item(TextItem::Symbol("ldots")), next)),
        "quad" => return Ok((Ctl::Item(TextItem::Symbol("quad")), next)),
        "qquad" => return Ok((Ctl::Item(TextItem::Symbol("qquad")), next)),
        "textbackslash" => return Ok((Ctl::Char('\\'), next)),
        "textasciicircum" => return Ok((Ctl::Char('^'), next)),
        "textasciitilde" => return Ok((Ctl::Char('~'), next)),
        _ => return Err(ParseError::Unknown(format!("\\{name}"))),
    };
    if char_at(raw, next) == Some('{') {
        Ok((Ctl::Open(style), next + 1))
    } else {
        Err(ParseError::Malformed("expected '{'".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(raw: &str) -> Result<TextRun, ParseError> {
        let deep = || ParseError::Malformed("deep".into());
        lex_body(raw, 4, &deep, &mut |src, _| {
            Ok(MathNode::Symbol(src.into()))
        })
    }

    #[test]
    fn groups_splice_and_spaces_collapse() {
        let want = TextRun::new(vec![
            TextItem::Str("a".into()),
            TextItem::Space,
            TextItem::Str("b{".into()),
        ]);
        assert_eq!(lex("a {\t\n b}\\{"), Ok(want));
    }

    #[test]
    fn nested_math_and_symbols() {
        let want = TextRun::new(vec![
            TextItem::Nested(TextStyle::Bf, TextRun::plain("x")),
            TextItem::Math(Box::new(MathNode::Symbol("y\\}".into()))),
            TextItem::Symbol("ldots"),
            TextItem::Str("z".into()),
            TextItem::ControlSpace,
        ]);
        assert_eq!(lex("\\textbf {x}\\(y\\}\\)\\dots  z\\ "), Ok(want));
    }

    #[test]
    fn errors() {
        assert_eq!(lex("\\"), Err(ParseError::TrailingBackslash));
        assert_eq!(
            lex("{{{{{x}}}}}"),
            Err(ParseError::Malformed("deep".into()))
        );
        assert_eq!(
            lex("$x"),
            Err(ParseError::Malformed("unmatched math shift in text".into()))
        );
        assert_eq!(
            lex("{$x}$"),
            Err(ParseError::Malformed("unmatched math shift in text".into()))
        );
        assert_eq!(lex("\\rm x"), Err(ParseError::Unknown("\\rm".into())));
        assert_eq!(
            lex("\\text x"),
            Err(ParseError::Malformed("expected '{'".into()))
        );
    }
}

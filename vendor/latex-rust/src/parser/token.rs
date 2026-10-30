//! LaTeX math tokenizer.

use core::fmt;
use core::iter::Peekable;
use core::str::Chars;

use crate::error::ParseError;

/// A single TeX-style math token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// Ordinary character (letter, digit, or other).
    Char(char),
    /// Control sequence without the leading backslash (`frac`, `[`, `,`).
    Command(String),
    /// `{`
    BeginGroup,
    /// `}`
    EndGroup,
    /// `^`
    Superscript,
    /// `_`
    Subscript,
    /// `&`
    AlignmentTab,
    /// Single `$`
    MathShift,
    /// `$$`
    DisplayShift,
    /// A space character (kept for `\text`; skipped in math lists).
    Space,
    /// Raw body of a text-mode command (`\text`, `\mbox`, `\intertext`), braces excluded.
    TextArg(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(c) => write!(f, "char:{c}"),
            Self::Command(name) => write!(f, "cmd:{name}"),
            Self::BeginGroup => f.write_str("{"),
            Self::EndGroup => f.write_str("}"),
            Self::Superscript => f.write_str("^"),
            Self::Subscript => f.write_str("_"),
            Self::AlignmentTab => f.write_str("&"),
            Self::MathShift => f.write_str("$"),
            Self::DisplayShift => f.write_str("$$"),
            Self::Space => f.write_str("space"),
            Self::TextArg(s) => write!(f, "text:{s}"),
        }
    }
}

/// Format a token stream as a gold-stable string.
///
/// # Examples
///
/// ```
/// use latex_rust::{format_tokens, tokenize};
///
/// let t = tokenize(r"a^2").unwrap();
/// assert_eq!(format_tokens(&t), "char:a ^ char:2");
/// ```
#[must_use]
pub fn format_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&t.to_string());
    }
    out
}

/// Tokenize a LaTeX math string. Whitespace and `%` line comments are skipped.
///
/// Supported delimiters are tokenized, not interpreted:
/// `$...$`, `$$...$$`, `\[...\]`, `\(...\)`.
///
/// # Arguments
///
/// * `input` — math source.
///
/// # Returns
///
/// The token list in source order.
///
/// # Errors
///
/// [`ParseError::TrailingBackslash`] if `input` ends with a stray `\`.
///
/// # Examples
///
/// ```
/// use latex_rust::{tokenize, Token};
///
/// let t = tokenize(r"\frac{1}{2}").unwrap();
/// assert!(matches!(&t[0], Token::Command(s) if s == "frac"));
/// ```
pub fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut chars = input.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '%' => skip_line(&mut chars),
            ' ' => out.push(Token::Space),
            '\t' | '\n' | '\r' | '\u{b}' | '\u{c}' => {}
            '{' => out.push(Token::BeginGroup),
            '}' => out.push(Token::EndGroup),
            '^' => out.push(Token::Superscript),
            '_' => out.push(Token::Subscript),
            '&' => out.push(Token::AlignmentTab),
            '$' => {
                if chars.peek() == Some(&'$') {
                    chars.next();
                    out.push(Token::DisplayShift);
                } else {
                    out.push(Token::MathShift);
                }
            }
            '\\' => {
                let token = command(&mut chars)?;
                let text = matches!(&token, Token::Command(name)
                    if name == "text" || name == "mbox" || name == "intertext");
                out.push(token);
                if text && chars.peek() == Some(&'{') {
                    chars.next();
                    out.push(Token::TextArg(text_arg(&mut chars)?));
                }
            }
            other => out.push(Token::Char(other)),
        }
    }
    Ok(out)
}

/// Reads a text-mode body up to its matching `}` (consumed, not included)
/// and returns it unchanged. `\` hides the next char from the brace count,
/// and a `%` comment runs to the end of the line, as in the text lexer.
fn text_arg(chars: &mut Peekable<Chars<'_>>) -> Result<String, ParseError> {
    let mut result = String::new();
    let mut brace_depth = 1;
    while brace_depth > 0 {
        match chars.next() {
            Some('{') => {
                brace_depth += 1;
                result.push('{');
            }
            Some('}') => {
                brace_depth -= 1;
                if brace_depth > 0 {
                    result.push('}');
                }
            }
            Some('\\') => {
                let Some(next) = chars.next() else {
                    return Err(ParseError::TrailingBackslash);
                };
                result.push('\\');
                result.push(next);
            }
            Some('%') => {
                result.push('%');
                loop {
                    match chars.next() {
                        Some('\n') => {
                            result.push('\n');
                            break;
                        }
                        None => return Err(ParseError::Malformed("unmatched '{'".into())),
                        Some(c) => result.push(c),
                    }
                }
            }
            None => return Err(ParseError::Malformed("unmatched '{'".into())),
            Some(c) => result.push(c),
        }
    }
    Ok(result)
}

fn skip_line(chars: &mut Peekable<Chars<'_>>) {
    for c in chars.by_ref() {
        if c == '\n' {
            break;
        }
    }
}

fn command(chars: &mut Peekable<Chars<'_>>) -> Result<Token, ParseError> {
    let Some(&first) = chars.peek() else {
        return Err(ParseError::TrailingBackslash);
    };
    if first.is_ascii_alphabetic() {
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
        while matches!(chars.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            chars.next();
        }
        Ok(Token::Command(name))
    } else {
        chars.next();
        Ok(Token::Command(first.to_string()))
    }
}

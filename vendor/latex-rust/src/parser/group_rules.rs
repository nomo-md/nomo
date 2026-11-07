//! Token rules that used to be string rewrites in `preprocess` (spec 3.1).
//!
//! They run on the token stream, so escapes are already tokens (`\{` is
//! `Command("{")`) and a text body is one opaque `TextArg`. Neither can be
//! matched by a rule. Every rule is one linear pass.

use super::token::Token;

/// Applies the rules in the order the old string passes ran.
pub(crate) fn apply(tokens: Vec<Token>) -> Vec<Token> {
    let tokens = unwrap_delimiters(tokens);
    let tokens = font_scopes(tokens);
    let tokens = convert_infix(tokens, InfixKind::Over);
    convert_infix(tokens, InfixKind::Choose)
}

/// `{ (}`, `{ )}`, `{ ( }`, `{ ) }`, `{[}` and `{]}` become the bare delimiter.
fn unwrap_delimiters(mut tokens: Vec<Token>) -> Vec<Token> {
    use Token::{BeginGroup as B, Char, EndGroup as E, Space as S};
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let found = match tokens.get(i..) {
            Some([B, S, Char(c @ ('(' | ')')), E, ..]) => Some((*c, 4)),
            Some([B, S, Char(c @ ('(' | ')')), S, E, ..]) => Some((*c, 5)),
            Some([B, Char(c @ ('[' | ']')), E, ..]) => Some((*c, 3)),
            _ => None,
        };
        match found {
            Some((c, len)) => {
                out.push(Char(c));
                i += len;
            }
            None => {
                out.extend(tokens.get_mut(i).map(|t| std::mem::replace(t, S)));
                i += 1;
            }
        }
    }
    out
}

fn font_command(name: &str) -> Option<&'static str> {
    Some(match name {
        "rm" => "mathrm",
        "bf" => "mathbf",
        "cal" => "mathcal",
        "it" => "mathit",
        "sf" => "mathsf",
        "tt" => "mathtt",
        _ => return None,
    })
}

/// `{\rm x}` → `\mathrm{x}` (likewise `bf cal it sf tt`). The group's content,
/// spaces trimmed, is the argument. An unclosed group is left alone.
fn font_scopes(mut tokens: Vec<Token>) -> Vec<Token> {
    // Which `}` closes each `{`.
    let mut closes = vec![false; tokens.len()];
    let mut matched = vec![false; tokens.len()];
    let mut open = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        match tok {
            Token::BeginGroup => open.push(i),
            Token::EndGroup => {
                if let Some(o) = open.pop() {
                    if let Some(m) = matched.get_mut(o) {
                        *m = true;
                    }
                    if let Some(c) = closes.get_mut(i) {
                        *c = true;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(tokens.len());
    // For each open group: does its `}` end a font scope, and where its
    // content starts in `out`.
    let mut groups: Vec<Option<usize>> = Vec::new();
    let mut i = 0;
    while let Some(tok) = tokens.get(i) {
        match tok {
            Token::BeginGroup if matched.get(i) == Some(&true) => {
                let font = match tokens.get(i + 1) {
                    Some(Token::Command(name)) => font_command(name),
                    _ => None,
                };
                if let Some(cmd) = font {
                    out.push(Token::Command(cmd.into()));
                    out.push(Token::BeginGroup);
                    i += 2;
                    while matches!(tokens.get(i), Some(Token::Space)) {
                        i += 1;
                    }
                    groups.push(Some(out.len()));
                    continue;
                }
                groups.push(None);
                out.push(Token::BeginGroup);
            }
            Token::EndGroup if closes.get(i) == Some(&true) => {
                if let Some(Some(start)) = groups.pop() {
                    while out.len() > start && out.last() == Some(&Token::Space) {
                        out.pop();
                    }
                }
                out.push(Token::EndGroup);
            }
            _ => out.extend(
                tokens
                    .get_mut(i)
                    .map(|t| std::mem::replace(t, Token::Space)),
            ),
        }
        i += 1;
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InfixKind {
    Over,
    Choose,
}

impl InfixKind {
    fn name(self) -> &'static str {
        match self {
            Self::Over => "over",
            Self::Choose => "choose",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Over => "frac",
            Self::Choose => "binom",
        }
    }
}

/// `{a \over b}` → `\frac{a}{b}`, `{n \choose k}` → `\binom{n}{k}`.
///
/// The innermost `{` before the command opens the fraction. A second
/// unbraced command in the same group is peeled in the same pass:
/// `{a \over b \over c}` becomes `\frac{a}\frac{b}{c}`. An unclosed group
/// keeps the original tokens.
fn convert_infix(tokens: Vec<Token>, kind: InfixKind) -> Vec<Token> {
    let mut layers = vec![Layer::base()];
    for tok in tokens {
        if matches!(&tok, Token::Command(name) if name == kind.name())
            && (start_denominator(&mut layers, kind) || peel(&mut layers, kind))
        {
            continue;
        }
        if tok == Token::EndGroup && close_denominator(&mut layers) {
            continue;
        }
        push_token(&mut layers, tok);
    }
    collapse(layers)
}

struct Pending {
    head: FracHead,
    /// Earlier unbraced commands in this same group.
    earlier: Vec<FracHead>,
}

struct FracHead {
    kind: InfixKind,
    num: Vec<Token>,
}

impl FracHead {
    fn emit(&self, den: &[Token], out: &mut Vec<Token>) {
        self.open_only(out);
        out.push(Token::BeginGroup);
        out.extend_from_slice(trim_spaces(den));
        out.push(Token::EndGroup);
    }

    fn open_only(&self, out: &mut Vec<Token>) {
        out.push(Token::Command(self.kind.command().into()));
        out.push(Token::BeginGroup);
        out.extend_from_slice(trim_spaces(&self.num));
        out.push(Token::EndGroup);
    }

    fn command(&self) -> Token {
        Token::Command(self.kind.name().into())
    }
}

struct Layer {
    out: Vec<Token>,
    braces: Vec<usize>,
    pending: Option<Pending>,
}

impl Layer {
    fn base() -> Self {
        Layer {
            out: vec![],
            braces: vec![],
            pending: None,
        }
    }

    fn den(kind: InfixKind, num: Vec<Token>) -> Self {
        Layer {
            out: vec![],
            braces: vec![],
            pending: Some(Pending {
                head: FracHead { kind, num },
                earlier: vec![],
            }),
        }
    }
}

/// The innermost `{` before the command opens the fraction. Its body is the
/// numerator; the rest of the group is scanned by a fresh layer.
fn start_denominator(layers: &mut Vec<Layer>, kind: InfixKind) -> bool {
    let Some(layer) = layers.last_mut() else {
        return false;
    };
    let Some(open) = layer.braces.pop() else {
        return false;
    };
    let num = layer.out.split_off(open + 1);
    layer.out.truncate(open);
    layers.push(Layer::den(kind, num));
    true
}

/// A second unbraced command in the denominator: the tokens so far are the
/// next numerator, and the previous numerator keeps only its first argument.
fn peel(layers: &mut Vec<Layer>, kind: InfixKind) -> bool {
    let ready = matches!(
        layers.last(),
        Some(layer) if layer.pending.is_some() && layer.braces.is_empty()
    );
    if !ready || layers.len() < 2 {
        return false;
    }
    let Some(done) = layers.pop() else {
        return false;
    };
    let Some(pending) = done.pending else {
        return false;
    };
    let mut earlier = pending.earlier;
    earlier.push(pending.head);
    layers.push(Layer {
        out: vec![],
        braces: vec![],
        pending: Some(Pending {
            head: FracHead {
                kind,
                num: done.out,
            },
            earlier,
        }),
    });
    true
}

/// A `}` that belongs to the fraction, not to a group inside the denominator.
fn close_denominator(layers: &mut Vec<Layer>) -> bool {
    let ready = matches!(
        layers.last(),
        Some(layer) if layer.pending.is_some() && layer.braces.is_empty()
    );
    if !ready || layers.len() < 2 {
        return false;
    }
    let Some(done) = layers.pop() else {
        return false;
    };
    let Some(pending) = done.pending else {
        return false;
    };
    let Some(parent) = layers.last_mut() else {
        return false;
    };
    for open in &pending.earlier {
        open.open_only(&mut parent.out);
    }
    pending.head.emit(&done.out, &mut parent.out);
    true
}

fn push_token(layers: &mut [Layer], token: Token) {
    let Some(layer) = layers.last_mut() else {
        return;
    };
    match token {
        Token::BeginGroup => layer.braces.push(layer.out.len()),
        Token::EndGroup => {
            layer.braces.pop();
        }
        _ => {}
    }
    layer.out.push(token);
}

/// An unclosed `{a \over` keeps the original tokens.
fn collapse(mut layers: Vec<Layer>) -> Vec<Token> {
    while layers.len() > 1 {
        let Some(done) = layers.pop() else {
            break;
        };
        let Some(parent) = layers.last_mut() else {
            return done.out;
        };
        match done.pending {
            Some(pending) => {
                parent.out.push(Token::BeginGroup);
                for open in &pending.earlier {
                    parent.out.extend_from_slice(&open.num);
                    parent.out.push(open.command());
                }
                parent.out.extend_from_slice(&pending.head.num);
                parent.out.push(pending.head.command());
                parent.out.extend(done.out);
            }
            None => parent.out.extend(done.out),
        }
    }
    layers.pop().map(|layer| layer.out).unwrap_or_default()
}

fn trim_spaces(t: &[Token]) -> &[Token] {
    let start = t.iter().take_while(|tok| **tok == Token::Space).count();
    let rest = t.get(start..).unwrap_or_default();
    let end = rest.len()
        - rest
            .iter()
            .rev()
            .take_while(|tok| **tok == Token::Space)
            .count();
    rest.get(..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::tokenize;

    /// The rules turn `from` into exactly the tokens of `to`. These are the
    /// string-level cases the old `preprocess` tests pinned.
    fn rule(from: &str, to: &str) {
        let got = tokenize(from).map(apply);
        assert_eq!(got, tokenize(to), "{from:?}");
    }

    #[test]
    fn over_becomes_frac() {
        rule(r"{a \over b}", r"\frac{a}{b}");
        rule(r"{a \over {b \over c}}", r"\frac{a}{\frac{b}{c}}");
        rule(r"{ {a \over b} \over c }", r"\frac{\frac{a}{b}}{c}");
        rule(r"{a \over b}{c \over d}", r"\frac{a}{b}\frac{c}{d}");
        rule(r"{a \over b \over c}", r"\frac{a}\frac{b}{c}");
        rule(
            r"{a \over b \over c \over d}",
            r"\frac{a}\frac{b}\frac{c}{d}",
        );
        rule(
            r"{a \over b \over {c \over d}}",
            r"\frac{a}\frac{b}{\frac{c}{d}}",
        );
        rule(r"{a \over {b \over c}+d}", r"\frac{a}{\frac{b}{c}+d}");
        rule(r"{a \over b+{c \over d}}", r"\frac{a}{b+\frac{c}{d}}");
        rule(r"{a \over b \over c+d}", r"\frac{a}\frac{b}{c+d}");
        rule(r"{a \over}", r"\frac{a}{}");
        rule(r"{a \over", r"{a \over");
        rule(r"{a \over b \over", r"{a \over b \over");
        rule(r"a \over b", r"a \over b");
        rule(r"{a \over \ }", r"\frac{a}{\ }");
        // whole commands only, and escapes are tokens
        rule(r"\overline{z}", r"\overline{z}");
        rule(r"{\overset{a}{b}}", r"{\overset{a}{b}}");
        rule(r"\{a \over b\}", r"\{a \over b\}");
        rule(r"{a\\over b}", r"{a\\over b}");
    }

    #[test]
    fn choose_becomes_binom() {
        rule(r"{n \choose k}", r"\binom{n}{k}");
        rule(r"{n \choose k \choose m}", r"\binom{n}\binom{k}{m}");
        rule(r"{a \choose b \over c}", r"\frac\binom{a}{b}{c}");
        rule(r"{a \over b \choose c}", r"\frac{a}\binom{b}{c}");
        rule(r"{a \choose {b \over c}}", r"\binom{a}{\frac{b}{c}}");
        rule(r"{a \over {b \choose c}}", r"\frac{a}{\binom{b}{c}}");
    }

    #[test]
    fn delimiter_groups_unwrap() {
        rule(r"{ (}a{ )}", r"(a)");
        rule(r"{ ( }a{ ) }", r"(a)");
        rule(r"{[}a{]}", r"[a]");
        rule(r"{(}", r"{(}");
        rule(r"\{ )}", r"\{ )}");
        // a text body is opaque
        rule(r"\text{{ )}}", r"\text{{ )}}");
    }

    #[test]
    fn font_scopes_become_commands() {
        rule(r"{\rm x}", r"\mathrm{x}");
        rule(r"{\bf  x y }", r"\mathbf{x y}");
        rule(r"{\it a{\rm b}c}", r"\mathit{a\mathrm{b}c}");
        rule(
            r"{\cal A}{\sf B}{\tt C}",
            r"\mathcal{A}\mathsf{B}\mathtt{C}",
        );
        rule(r"{\rm x", r"{\rm x");
        rule(r"{\rmfoo x}", r"{\rmfoo x}");
        rule(r"\text{{\rm a}}", r"\text{{\rm a}}");
    }
}

//! LaTeX math string → [`MathNode`].
//!
//! Binding of `_` / `^` / primes is Pratt-style (tight postfix on the nucleus).
//! The math list itself is a TeX-style row of atoms, not an arithmetic tree.

use super::ast::{
    AccentKind, AtomKind, ColSpec, DelimSize, Delimiter, EnvRow, EqNumber, FracStyle, IntegralKind,
    MathNode, TextRun, TextStyle,
};
use super::group_rules;
use super::preproc::preprocess;
use super::text::lex_body;
use super::token::{tokenize, Token};
use crate::color::{parse_color_spec, Color, ColorTable};

mod drive;
pub(crate) mod lexicon;
use crate::dim::Dim;
use crate::error::{Error, ParseError};
use crate::symbols::{lookup, SymbolKind as CatalogKind};

/// Default deepest nesting [`parse`] and [`layout`](crate::layout()) accept.
///
/// Past the limit both return an error. Input this deep is pathological, and
/// a crate that refuses to invent a render should not abort the process either.
///
/// Parsing keeps its nesting on a heap stack. Whether an input is inside this
/// limit depends only on the input and the limit, not on frame size, the build
/// profile, or the calling thread's stack.
///
/// The count is of parser recursion levels rather than of LaTeX constructs, and
/// a braced argument costs two of them (one for the argument and one for the
/// group), so the default admits `\frac{1}{…}` nested 15 deep. Real mathematics
/// rarely nests beyond five levels.
///
/// Layout, `gold`, `to_latex`, and render walk an explicit heap stack, the
/// same way parsing does. A tree this limit accepts can be laid out and
/// rendered on a 256 KiB thread. `max_depth` is still the only depth limit:
/// raise it with [`layout_with_max_depth`](crate::layout_with_max_depth), and
/// raise parsing separately with [`ParseOptions::with_max_depth`].
pub const DEFAULT_MAX_NESTING_DEPTH: usize = 32;

/// Options for [`parse_with_options`].
///
/// # Examples
///
/// ```
/// use latex_rust::{parse_with_options, ParseOptions};
///
/// let deep = "{".repeat(40) + "x" + &"}".repeat(40);
/// assert!(parse_with_options(&deep, &ParseOptions::new()).is_err());
/// let roomy = ParseOptions::new().with_max_depth(128);
/// assert!(parse_with_options(&deep, &roomy).is_ok());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseOptions {
    /// Deepest parser recursion accepted. Defaults to [`DEFAULT_MAX_NESTING_DEPTH`].
    pub max_depth: usize,
    /// Map adjacent `<=`, `>=`, and `!=` to `\le`, `\ge`, and `\ne`. Default off.
    pub ascii_relations: bool,
    /// Read a whole ASCII-letter run as an operator, `\zeta`, or cases `otherwise`.
    /// Default off.
    pub bare_operators: bool,
    /// Map a fixed set of unicode glyphs to symbol commands. Default off.
    pub unicode_symbols: bool,
    /// Spell `'` as `(symbol prime)`. A run such as `f'''` is one flat row. Default off.
    pub prime_symbol: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_NESTING_DEPTH,
            ascii_relations: false,
            bare_operators: false,
            unicode_symbols: false,
            prime_symbol: false,
        }
    }
}

impl ParseOptions {
    /// Options with every field at its default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the deepest parser nesting accepted.
    ///
    /// Parsing uses a heap stack, so this limit is exact. The same input is
    /// [`Ok`](Result::Ok) or [`ParseError::Malformed`] in every build profile
    /// and on every thread stack, including 256 KiB. A brace group or a
    /// `\left` body is one level, so 3,531 of them are inside a limit of
    /// 20,000 and 20,000 of them are not. A braced argument such as `\frac`
    /// or a superscript costs two levels, so 9,999 of those are inside 20,000
    /// and 10,000 are not.
    ///
    /// Layout uses its own heap stack and its own limit. See
    /// [`DEFAULT_MAX_NESTING_DEPTH`].
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Map adjacent `<=`, `>=`, and `!=` to the same symbols as `\le`, `\ge`, and `\ne`.
    ///
    /// Off by default, so those digraphs stay two atoms. The two characters
    /// must be adjacent: `x <= 0` matches and `x < = 0` does not. `\le` and
    /// `\leq` already share a tree with this flag off.
    #[must_use]
    pub fn with_ascii_relations(mut self, on: bool) -> Self {
        self.ascii_relations = on;
        self
    }

    /// Read a maximal run of ASCII letters as one identifier.
    ///
    /// The whole run is looked up in the operator table (`\sin`, `\sinh`,
    /// `\log`, the big operators, and the rest of that list), plus `\zeta`.
    /// `sinh` is `\sinh`, not `\sin` followed by `h`, because the longer
    /// spelling is its own entry and the match is the whole run, not a
    /// prefix. A run that is not in the table stays one atom per letter, so
    /// `xsin` and `sinx` stay letters and `infty` is not `\inf`. A space
    /// ends the run, so `x sin` is `x` times `\sin`. `ω` is not an ASCII
    /// letter, so it is never part of the run: `sinω` is `\sin` followed by
    /// `ω`, and `ω` itself stays an ordinary atom.
    ///
    /// Inside `cases`, the identifier `otherwise` is the text node of
    /// `\text{otherwise}`. Outside `cases` that word stays letters.
    ///
    /// Off by default. `\sin(x)` is the same tree either way.
    #[must_use]
    pub fn with_bare_operators(mut self, on: bool) -> Self {
        self.bare_operators = on;
        self
    }

    /// Map `α`, `Γ`, `π`, `≤`, `≥`, `≠`, `∞`, `×`, `·` (U+00B7), and `⋅`
    /// (U+22C5) to the symbol nodes of `\alpha`, `\Gamma`, `\pi`, `\leq`,
    /// `\geq`, `\neq`, `\infty`, `\times`, and `\cdot`.
    ///
    /// Off by default, so each glyph stays an atom with its current class.
    /// `ω` is not in this set.
    #[must_use]
    pub fn with_unicode_symbols(mut self, on: bool) -> Self {
        self.unicode_symbols = on;
        self
    }

    /// Spell `'` as `(symbol prime)`, the node of `\prime`.
    ///
    /// Off by default, so `f'` stays a superscript of U+2032. `f''` is a row
    /// of two primes and `f'''` a flat row of three, with or without this flag.
    #[must_use]
    pub fn with_prime_symbol(mut self, on: bool) -> Self {
        self.prime_symbol = on;
        self
    }
}

/// Parse a LaTeX math string into a [`MathNode`] using a fresh color table.
///
/// Accepts raw math, `$...$`, `$$...$$`, `\(...\)`, or `\[...\]`. Binding of
/// `_` / `^` / primes is Pratt-style (tight postfix on the nucleus).
///
/// `√x` is a square root, a run of superscript digits or letters is a
/// superscript, and U+2212 is a binary minus. `<=`, a bare `sin`, `α`, and
/// `'` stay today's atoms unless [`ParseOptions`] turns that lexicon on.
///
/// # Arguments
///
/// * `input` — math source, with or without delimiter fences.
///
/// # Returns
///
/// The typed AST, or a [`ParseError`] naming the problem.
///
/// # Errors
///
/// * [`ParseError::Unknown`] — command is not in the catalog.
/// * [`ParseError::Unsupported`] — known construct this crate will not invent.
/// * [`ParseError::Malformed`] — syntactically invalid input.
/// * [`ParseError::UnmatchedDelimiter`] — `\left` without `\right` (or vice versa).
/// * [`ParseError::TrailingBackslash`] — input ended with a stray `\`.
///
/// # Examples
///
/// ```
/// use latex_rust::parse;
///
/// let ast = parse(r"\frac{1}{2}").unwrap();
/// assert_eq!(ast.gold(), r#"(frac (atom Ord "1") (atom Ord "2"))"#);
/// ```
pub fn parse(input: &str) -> Result<MathNode, ParseError> {
    parse_with_colors(input).map(|(n, _)| n)
}

/// Parse a math string, returning the AST and the color table after `\definecolor`.
///
/// # Arguments
///
/// * `input` — math source, with or without delimiter fences.
///
/// # Returns
///
/// The AST and the color table including any `\definecolor` names from `input`.
///
/// # Errors
///
/// Same as [`parse`].
///
/// # Examples
///
/// ```
/// use latex_rust::parse_with_colors;
///
/// let (ast, table) = parse_with_colors(r"\definecolor{ok}{named}{red}x").unwrap();
/// assert!(table.get("ok").is_ok());
/// let _ = ast;
/// ```
pub fn parse_with_colors(input: &str) -> Result<(MathNode, ColorTable), ParseError> {
    parse_with_options(input, &ParseOptions::default())
}

/// Parse a math string with explicit [`ParseOptions`], returning the AST and
/// the color table after `\definecolor`.
///
/// # Errors
///
/// Same as [`parse`]. Input nesting deeper than `options.max_depth` returns
/// [`ParseError::Malformed`].
///
/// # Examples
///
/// ```
/// use latex_rust::{parse_with_options, ParseOptions};
///
/// // The top level and each `\frac` cost one level, braced or not.
/// let deep = r"\frac{1}{\frac{1}{\frac{1}{2}}}";
/// assert!(parse_with_options(deep, &ParseOptions::new().with_max_depth(3)).is_err());
/// assert!(parse_with_options(deep, &ParseOptions::new().with_max_depth(4)).is_ok());
/// ```
pub fn parse_with_options(
    input: &str,
    options: &ParseOptions,
) -> Result<(MathNode, ColorTable), ParseError> {
    let sanitized = preprocess(input);
    if sanitized.is_empty() {
        return Err(ParseError::Malformed("empty input".into()));
    }
    let tokens = group_rules::apply(tokenize(&sanitized)?);
    let tokens = strip_fences(&tokens)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
        max_depth: options.max_depth,
        colors: ColorTable::new(),
        ascii_relations: options.ascii_relations,
        bare_operators: options.bare_operators,
        unicode_symbols: options.unicode_symbols,
        prime_symbol: options.prime_symbol,
        cases_depth: 0,
        plain_letters_until: 0,
        text_left: TEXT_LEVELS,
        stop: Stop::eof(),
    };
    let node = drive::parse_top(&mut p)?;
    p.skip_ws();
    if p.pos < p.tokens.len() {
        return Err(ParseError::Malformed(format!(
            "unexpected leftover token {}",
            p.tokens[p.pos]
        )));
    }
    Ok((node, p.colors))
}

#[derive(Clone, Copy)]
pub(crate) struct Stop {
    end_group: bool,
    amp: bool,
    cr: bool,
    right: bool,
    end_env: bool,
    rbracket: bool,
}

impl Stop {
    pub(crate) fn eof() -> Self {
        Self {
            end_group: false,
            amp: false,
            cr: false,
            right: false,
            end_env: false,
            rbracket: false,
        }
    }

    /// True when `tok` ends a list read with this stop set.
    pub(crate) fn hits(self, tok: &Token) -> bool {
        match tok {
            Token::EndGroup if self.end_group => true,
            Token::AlignmentTab if self.amp => true,
            Token::Command(s) if s == "\\" && self.cr => true,
            Token::Command(s) if s == "cr" && self.cr => true,
            Token::Command(s) if s == "right" && self.right => true,
            Token::Command(s) if s == "end" && self.end_env => true,
            Token::Char(']') if self.rbracket => true,
            _ => false,
        }
    }

    fn group() -> Self {
        Self {
            end_group: true,
            ..Self::eof()
        }
    }

    fn cell() -> Self {
        Self {
            amp: true,
            cr: true,
            end_env: true,
            ..Self::eof()
        }
    }

    fn delim() -> Self {
        Self {
            right: true,
            ..Self::eof()
        }
    }

    fn index() -> Self {
        Self {
            rbracket: true,
            ..Self::eof()
        }
    }

    fn substack_line() -> Self {
        Self {
            end_group: true,
            cr: true,
            ..Self::eof()
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    colors: ColorTable,
    /// Current nesting depth, bounded by `max_depth`.
    depth: usize,
    /// Deepest nesting accepted, from [`ParseOptions::max_depth`].
    max_depth: usize,
    ascii_relations: bool,
    bare_operators: bool,
    unicode_symbols: bool,
    prime_symbol: bool,
    /// `\begin{cases}` bodies currently open. `otherwise` is text only here.
    cases_depth: usize,
    /// Index past a rejected ASCII-letter run. Letters before it stay atoms.
    plain_letters_until: usize,
    /// Text nesting levels still allowed (starts at [`TEXT_LEVELS`]).
    text_left: usize,
    /// The stop set of the list now reading atoms (`&`, `\\`, `\end`, `\right`,
    /// an option's `]`): a paren lookahead ends there (spec 5, Q1).
    stop: Stop,
}

/// Bounds the native recursion text → inline math → text. Text nesting also
/// counts toward `max_depth`; this cap holds when `max_depth` is raised.
const TEXT_LEVELS: usize = 32;

impl Parser {
    fn skip_ws(&mut self) {
        while matches!(self.tokens.get(self.pos), Some(Token::Space)) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_ws(&mut self) -> Option<&Token> {
        self.skip_ws();
        self.peek()
    }

    fn bump(&mut self) -> Option<Token> {
        self.skip_ws();
        self.bump_raw()
    }

    fn bump_raw(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned()?;
        self.pos += 1;
        Some(t)
    }

    fn is_stop(&self, tok: &Token, stop: Stop) -> bool {
        stop.hits(tok)
    }

    /// True when the `(` at `pos` starts a scripted paren group; see
    /// [`paren_script_match`].
    fn script_follows_parens(&self) -> bool {
        paren_script_match(&self.tokens, self.pos, self.stop).is_some()
    }

    /// Lexes the [`Token::TextArg`] after a text command (spec 3.2).
    fn text_run(&mut self) -> Result<TextRun, ParseError> {
        self.skip_ws();
        let pos = self.pos;
        let raw = match self.bump_raw() {
            Some(Token::TextArg(raw)) => raw,
            _ => {
                self.pos = pos;
                return Err(ParseError::Malformed("expected '{'".into()));
            }
        };
        let max_depth = self.max_depth;
        let nested_by_input = max_depth.saturating_sub(self.depth) <= self.text_left;
        let levels = max_depth.saturating_sub(self.depth).min(self.text_left);
        let too_deep = move || {
            if nested_by_input {
                ParseError::Malformed(format!("input nests deeper than {max_depth} levels"))
            } else {
                ParseError::Malformed(format!("text nests deeper than {TEXT_LEVELS} levels"))
            }
        };
        let mut math = |src: &str, left: usize| self.sub_parse(src, left);
        lex_body(&raw, levels, &too_deep, &mut math)
    }

    /// Parses inline math from a text body with `left` levels to spare.
    ///
    /// The sub-parser takes the color table and hands it back, so a color
    /// defined inside text math stays defined (like `\definecolor` in TeX).
    fn sub_parse(&mut self, src: &str, left: usize) -> Result<MathNode, ParseError> {
        let tokens = group_rules::apply(tokenize(src)?);
        let mut parser = Parser {
            tokens,
            pos: 0,
            colors: std::mem::replace(&mut self.colors, ColorTable::empty()),
            depth: self.max_depth.saturating_sub(left),
            max_depth: self.max_depth,
            ascii_relations: self.ascii_relations,
            bare_operators: self.bare_operators,
            unicode_symbols: self.unicode_symbols,
            prime_symbol: self.prime_symbol,
            cases_depth: 0,
            plain_letters_until: 0,
            text_left: left,
            stop: Stop::eof(),
        };
        let result = parse_whole(&mut parser);
        self.colors = parser.colors;
        result
    }

    fn parse_text(&mut self, style: TextStyle) -> Result<MathNode, ParseError> {
        Ok(MathNode::Text(self.text_run()?, style))
    }

    fn parse_sized_delim(&mut self, name: &str) -> Result<MathNode, ParseError> {
        let size = DelimSize::from_command(name)
            .ok_or_else(|| ParseError::Malformed(format!("unknown delimiter size \\{name}")))?;
        let d = self.parse_delimiter()?;
        let class = DelimSize::class_from_command(name).unwrap_or_else(|| match &d {
            Delimiter::Char(c) => atom_kind(*c),
            Delimiter::Named(n) if n == "{" => AtomKind::Open,
            Delimiter::Named(n) if n == "}" => AtomKind::Close,
            _ => AtomKind::Open,
        });
        Ok(MathNode::SizedDelim(d, size, class))
    }

    fn parse_delimiter(&mut self) -> Result<Delimiter, ParseError> {
        self.skip_ws();
        match self.bump() {
            Some(Token::Char('.')) => Ok(Delimiter::Empty),
            Some(Token::Char(c)) if is_delimiter_char(c) => Ok(Delimiter::Char(c)),
            Some(Token::Command(n)) => {
                if n.as_str() == "." {
                    Ok(Delimiter::Empty)
                } else if is_delimiter_name(n.as_str()) {
                    Ok(Delimiter::Named(n))
                } else {
                    Err(ParseError::Malformed(format!("unknown delimiter \\{n}")))
                }
            }
            Some(other) => Err(ParseError::Malformed(format!(
                "expected delimiter, found {other}"
            ))),
            None => Err(ParseError::Malformed("expected delimiter".into())),
        }
    }

    fn expect_end(&mut self, name: &str) -> Result<(), ParseError> {
        match self.bump() {
            Some(Token::Command(n)) if n == "end" => {}
            _ => return Err(ParseError::Malformed(format!("expected \\end{{{name}}}"))),
        }
        let got = self.collect_group_text()?;
        if got != name {
            return Err(ParseError::Malformed(format!(
                "\\begin{{{name}}} closed by \\end{{{got}}}"
            )));
        }
        Ok(())
    }

    fn parse_color_from_cmd(&mut self) -> Result<Color, ParseError> {
        self.skip_ws();
        let model = if matches!(self.peek(), Some(Token::Char('['))) {
            self.bump();
            let m = self.collect_until_char(']')?;
            match self.bump() {
                Some(Token::Char(']')) => {}
                _ => {
                    return Err(ParseError::Malformed(
                        "expected ']' after color model".into(),
                    ))
                }
            }
            m
        } else {
            "named".into()
        };
        let spec = self.collect_group_text()?;
        parse_color_spec(&model, &spec, Some(&self.colors)).map_err(color_err)
    }

    fn parse_definecolor(&mut self) -> Result<(), ParseError> {
        let name = self.collect_group_text()?;
        let model = self.collect_group_text()?;
        let spec = self.collect_group_text()?;
        self.colors
            .define(&name, &model, &spec)
            .map_err(color_err)?;
        Ok(())
    }

    fn collect_group_text(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        match self.bump_raw() {
            Some(Token::Space) => {
                self.pos -= 1;
                self.skip_ws();
                return self.collect_group_text();
            }
            Some(Token::BeginGroup) => {}
            _ => return Err(ParseError::Malformed("expected '{'".into())),
        }
        let mut s = String::new();
        let mut depth = 1;
        while depth > 0 {
            match self.bump_raw() {
                None => return Err(ParseError::Malformed("unmatched '{'".into())),
                Some(Token::BeginGroup) => {
                    depth += 1;
                    s.push('{');
                }
                Some(Token::EndGroup) => {
                    depth -= 1;
                    if depth > 0 {
                        s.push('}');
                    }
                }
                Some(Token::Space) => s.push(' '),
                Some(Token::Char(c)) => s.push(c),
                Some(Token::Command(n)) => {
                    if n.len() == 1 {
                        let c = n.chars().next().unwrap_or('\\');
                        if c.is_whitespace() {
                            s.push(' ');
                        } else {
                            s.push(c);
                        }
                    } else {
                        s.push('\\');
                        s.push_str(&n);
                    }
                }
                Some(other) => {
                    return Err(ParseError::Malformed(format!(
                        "unexpected token {other} in group text"
                    )))
                }
            }
        }
        Ok(s)
    }

    fn collect_until_char(&mut self, end: char) -> Result<String, ParseError> {
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(ParseError::Malformed(format!("expected '{end}'"))),
                Some(Token::Char(c)) if *c == end => break,
                Some(Token::Char(c)) => {
                    s.push(*c);
                    self.bump_raw();
                }
                Some(Token::Space) => {
                    s.push(' ');
                    self.bump_raw();
                }
                Some(other) => {
                    return Err(ParseError::Malformed(format!(
                        "unexpected token {other} in optional argument"
                    )))
                }
            }
        }
        Ok(s)
    }

    fn parse_symbol_or_unknown(&self, name: &str) -> Result<MathNode, ParseError> {
        let canon = alias(name);
        if let Some(e) = lookup(canon).or_else(|| lookup(name)) {
            match e.kind {
                CatalogKind::Container | CatalogKind::Modifier => {
                    return Err(ParseError::Unsupported(format!("\\{name}")));
                }
                CatalogKind::Symbol | CatalogKind::Operator => {
                    return Ok(MathNode::Symbol(canon.to_string()));
                }
            }
        }
        if is_extra_symbol(canon) {
            return Ok(MathNode::Symbol(canon.to_string()));
        }
        Err(ParseError::Unknown(format!("\\{name}")))
    }
}

/// The index of the `)` matching the `(` at `pos`, when a script signal
/// (`^`, `_`, `'` or a superscript character) follows it, so the parser reads
/// `(…)` as one scripted group. The printer runs the same scan on its own
/// output (spec 5.4).
///
/// Parens inside a brace group belong to that group: the scan skips
/// balanced groups and stops at an unbalanced `}` (spec 3.3). It also
/// skips the delimiter after `\left`, `\right`, `\middle` and the `\big`
/// family, and the `[…]` option of a command that takes one, since those
/// parens belong to the command (spec 5, Q1). The scan stops at any token
/// that ends the list it started in (`stop`: `&`, `\\`, `\end`, `\right`,
/// an option's `]`), so a `(` in one matrix cell never pairs with a `)` in
/// the next.
pub(crate) fn paren_script_match(tokens: &[Token], pos: usize, stop: Stop) -> Option<usize> {
    let mut i = pos;
    let mut depth = 0i32;
    let mut braces = 0usize;
    while let Some(tok) = tokens.get(i) {
        match tok {
            Token::BeginGroup => braces += 1,
            Token::EndGroup => {
                if braces == 0 {
                    return None;
                }
                braces -= 1;
            }
            _ if braces > 0 => {}
            _ if stop.hits(tok) => return None,
            Token::Space => {}
            Token::Command(name) if is_delimiter_command(name) => {
                i = skip_spaces_from(tokens, i + 1);
            }
            Token::Command(name) if takes_option(name) => {
                let next = skip_spaces_from(tokens, i + 1);
                if matches!(tokens.get(next), Some(Token::Char('['))) {
                    i = option_end(tokens, next)?;
                }
            }
            Token::Char('(') => depth += 1,
            Token::Char(')') => {
                depth -= 1;
                if depth == 0 {
                    let close = i;
                    let next = skip_spaces_from(tokens, i + 1);
                    return match tokens.get(next) {
                        Some(Token::Superscript | Token::Subscript | Token::Char('\'')) => {
                            Some(close)
                        }
                        Some(Token::Char(c)) if lexicon::is_superscript_char(*c) => Some(close),
                        _ => None,
                    };
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The first index at or after `i` that is not a space.
fn skip_spaces_from(tokens: &[Token], mut i: usize) -> usize {
    while matches!(tokens.get(i), Some(Token::Space)) {
        i += 1;
    }
    i
}

/// The index of the `]` closing the option whose `[` is at `open`, skipping
/// brace groups and the options of nested option commands (`\sqrt[…]` inside
/// the option owns its own `]`); `None` when it is unclosed.
fn option_end(tokens: &[Token], open: usize) -> Option<usize> {
    let mut braces = 0usize;
    let mut nested = 0usize;
    let mut i = open + 1;
    while let Some(tok) = tokens.get(i) {
        match tok {
            Token::BeginGroup => braces += 1,
            Token::EndGroup => braces = braces.checked_sub(1)?,
            Token::Command(name) if braces == 0 && takes_option(name) => {
                let next = skip_spaces_from(tokens, i + 1);
                if matches!(tokens.get(next), Some(Token::Char('['))) {
                    nested += 1;
                    i = next;
                }
            }
            Token::Char(']') if braces == 0 => {
                if nested == 0 {
                    return Some(i);
                }
                nested -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Parses all of `parser`'s tokens as one math list; leftover tokens are an error.
fn parse_whole(parser: &mut Parser) -> Result<MathNode, ParseError> {
    let result = drive::parse_top(parser)?;
    parser.skip_ws();
    if let Some(tok) = parser.tokens.get(parser.pos) {
        return Err(ParseError::Malformed(format!(
            "unexpected leftover token {tok}"
        )));
    }
    Ok(result)
}

/// True when `tokens`, read as the body of a `[…]` option followed by its
/// closing `]`, would end the option early or not parse: the parser's own
/// option-body reader must stop exactly at the final `]`. Which `]` an
/// argument or a nested option command consumes is the parser's business, so
/// this asks it (spec 5.3, optional argument). The printer groups such a body.
pub(crate) fn option_ends_early(mut tokens: Vec<Token>) -> bool {
    tokens.push(Token::Char(']'));
    let end = tokens.len() - 1;
    let defaults = ParseOptions::default();
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
        max_depth: usize::MAX,
        colors: ColorTable::new(),
        ascii_relations: defaults.ascii_relations,
        bare_operators: defaults.bare_operators,
        unicode_symbols: defaults.unicode_symbols,
        prime_symbol: defaults.prime_symbol,
        cases_depth: 0,
        plain_letters_until: 0,
        text_left: TEXT_LEVELS,
        stop: Stop::eof(),
    };
    let read = drive::parse_option_body(&mut parser).is_ok();
    !(read && parser.pos == end)
}

/// A command whose next token is a delimiter (`\left(`, `\big)`).
fn is_delimiter_command(name: &str) -> bool {
    matches!(
        name,
        "left"
            | "right"
            | "middle"
            | "big"
            | "Big"
            | "bigg"
            | "Bigg"
            | "bigl"
            | "bigr"
            | "Bigl"
            | "Bigr"
            | "biggl"
            | "biggr"
            | "Biggl"
            | "Biggr"
            | "bigm"
            | "Bigm"
            | "biggm"
            | "Biggm"
    )
}

/// A command that reads a `[…]` option.
fn takes_option(name: &str) -> bool {
    matches!(
        name,
        "sqrt" | "xrightarrow" | "xleftarrow" | "color" | "textcolor" | "colorbox" | "fcolorbox"
    )
}

fn strip_fences(tokens: &[Token]) -> Result<Vec<Token>, ParseError> {
    let t = trim_spaces(tokens);
    if t.len() >= 2 {
        let inner = match (&t[0], &t[t.len() - 1]) {
            (Token::MathShift, Token::MathShift) => Some(&t[1..t.len() - 1]),
            (Token::DisplayShift, Token::DisplayShift) => Some(&t[1..t.len() - 1]),
            (Token::Command(a), Token::Command(b)) if a == "[" && b == "]" => {
                Some(&t[1..t.len() - 1])
            }
            (Token::Command(a), Token::Command(b)) if a == "(" && b == ")" => {
                Some(&t[1..t.len() - 1])
            }
            _ => None,
        };
        if let Some(inner) = inner {
            return Ok(trim_spaces(inner).to_vec());
        }
        if matches!(t[0], Token::MathShift | Token::DisplayShift)
            || matches!(&t[0], Token::Command(s) if s == "[" || s == "(")
        {
            return Err(ParseError::Malformed("unmatched math delimiter".into()));
        }
    }
    Ok(t.to_vec())
}

fn trim_spaces(tokens: &[Token]) -> &[Token] {
    let mut a = 0;
    let mut b = tokens.len();
    while a < b && tokens[a] == Token::Space {
        a += 1;
    }
    while b > a && tokens[b - 1] == Token::Space {
        b -= 1;
    }
    &tokens[a..b]
}

fn wrap_row(mut items: Vec<MathNode>) -> MathNode {
    if items.len() == 1 {
        items.remove(0)
    } else {
        MathNode::Row(items)
    }
}

fn parse_colspec(s: &str) -> Result<Vec<ColSpec>, ParseError> {
    let mut out = Vec::new();
    for c in s.chars() {
        match c {
            'l' => out.push(ColSpec::Left),
            'c' => out.push(ColSpec::Center),
            'r' => out.push(ColSpec::Right),
            '|' => out.push(ColSpec::VRule),
            ' ' | '\t' => {}
            '@' | '!' | '>' | '<' | 'p' | 'm' | 'b' | '*' => {
                return Err(ParseError::Unsupported(format!("array preamble `{c}`")))
            }
            other => return Err(ParseError::Malformed(format!("array preamble `{other}`"))),
        }
    }
    if out.is_empty() {
        return Err(ParseError::Malformed("empty array preamble".into()));
    }
    Ok(out)
}

fn peel_row_meta(mut node: MathNode, number: &mut EqNumber, labels: &mut Vec<String>) -> MathNode {
    enum Kind {
        NoNumber,
        Tag,
        Label,
        Hline,
        Intertext,
        Row,
        Other,
    }
    let kind = match &node {
        MathNode::NoNumber => Kind::NoNumber,
        MathNode::Tag { .. } => Kind::Tag,
        MathNode::Label(_) => Kind::Label,
        MathNode::Hline => Kind::Hline,
        MathNode::Intertext(_) => Kind::Intertext,
        MathNode::Row(_) => Kind::Row,
        _ => Kind::Other,
    };
    match kind {
        Kind::NoNumber => {
            *number = EqNumber::Suppress;
            MathNode::Row(Vec::new())
        }
        Kind::Tag => {
            if let MathNode::Tag { star, body } = &mut node {
                *number = EqNumber::Tag {
                    star: *star,
                    body: std::mem::replace(body, Box::new(MathNode::Hline)),
                };
            }
            MathNode::Row(Vec::new())
        }
        Kind::Label => {
            if let MathNode::Label(k) = &mut node {
                labels.push(std::mem::take(k));
            }
            MathNode::Row(Vec::new())
        }
        Kind::Hline => MathNode::Hline,
        Kind::Intertext => {
            let body = if let MathNode::Intertext(body) = &mut node {
                std::mem::replace(body, Box::new(MathNode::Hline))
            } else {
                Box::new(MathNode::Hline)
            };
            MathNode::Intertext(body)
        }
        Kind::Row => {
            let items = if let MathNode::Row(items) = &mut node {
                std::mem::take(items)
            } else {
                Vec::new()
            };
            let mut kept = Vec::new();
            for it in items {
                let peeled = peel_row_meta(it, number, labels);
                if !is_empty_node(&peeled) {
                    kept.push(peeled);
                }
            }
            wrap_row(kept)
        }
        Kind::Other => node,
    }
}

fn finish_env_row(cells: Vec<MathNode>, number: EqNumber, labels: Vec<String>) -> EnvRow {
    if cells.len() == 1 && matches!(cells[0], MathNode::Hline) {
        return EnvRow::Hline;
    }
    if cells.len() == 1 {
        if let MathNode::Intertext(n) = &cells[0] {
            return EnvRow::Intertext(n.clone());
        }
    }
    EnvRow::Cells {
        cells,
        number,
        labels,
    }
}

fn is_empty_node(n: &MathNode) -> bool {
    match n {
        MathNode::Row(v) => v.is_empty() || v.iter().all(is_empty_node),
        MathNode::Space(_) | MathNode::NoNumber | MathNode::Label(_) => true,
        _ => false,
    }
}

fn apply_scripts(mut nucleus: MathNode, sub: Option<MathNode>, sup: Option<MathNode>) -> MathNode {
    enum Base {
        Sum,
        Product,
        Integral(IntegralKind),
        Limit,
        Brace(AccentKind),
        Other,
    }
    let base = match &nucleus {
        MathNode::Sum(None, None) => Base::Sum,
        MathNode::Product(None, None) => Base::Product,
        MathNode::Integral(k, None, None) => Base::Integral(*k),
        MathNode::Limit(None) => Base::Limit,
        MathNode::Accent(_, k @ (AccentKind::Overbrace | AccentKind::Underbrace)) => {
            Base::Brace(*k)
        }
        _ => Base::Other,
    };
    match base {
        Base::Sum => MathNode::Sum(sub.map(Box::new), sup.map(Box::new)),
        Base::Product => MathNode::Product(sub.map(Box::new), sup.map(Box::new)),
        Base::Integral(k) => MathNode::Integral(k, sub.map(Box::new), sup.map(Box::new)),
        Base::Limit => {
            let lim = MathNode::Limit(sub.map(Box::new));
            match sup {
                Some(s) => MathNode::Superscript(Box::new(lim), Box::new(s)),
                None => lim,
            }
        }
        Base::Brace(k) => {
            let body = if let MathNode::Accent(body, _) = &mut nucleus {
                std::mem::replace(body, Box::new(MathNode::Hline))
            } else {
                Box::new(MathNode::Hline)
            };
            match (sub, sup) {
                (None, None) => MathNode::Accent(body, k),
                (s, e) => MathNode::OverUnder(
                    Box::new(MathNode::Accent(body, k)),
                    e.map(Box::new),
                    s.map(Box::new),
                ),
            }
        }
        Base::Other => match (sub, sup) {
            (None, None) => nucleus,
            (Some(s), None) => MathNode::Subscript(Box::new(nucleus), Box::new(s)),
            (None, Some(e)) => MathNode::Superscript(Box::new(nucleus), Box::new(e)),
            (Some(s), Some(e)) => MathNode::SubSup(Box::new(nucleus), Box::new(s), Box::new(e)),
        },
    }
}

/// Apply a math font style to a whole subformula.
///
/// A LaTeX font command such as `\mathrm` switches the font for everything in
/// its argument, not only for the characters at the top of the argument's list,
/// so this descends into scripts, fractions, radicals, accents and delimited
/// bodies as well as rows. Delimiters themselves are `Delimiter` values rather
/// than nodes and are left alone, as they are in LaTeX.
///
/// The descent is an explicit heap stack. A styled fraction nested up to
/// `max_depth` does not grow the calling thread's stack.
fn apply_text_style(node: MathNode, style: TextStyle) -> MathNode {
    enum Task {
        Go(MathNode),
        Join(Join),
    }
    enum Join {
        Superscript,
        Subscript,
        SubSup,
        Fraction(FracStyle),
        Radical(bool),
        Accent(AccentKind),
        Delim(Delimiter, Delimiter),
        Row(usize),
        Substack(usize),
    }
    enum Exploded {
        Leaf(MathNode),
        Kids { join: Join, kids: Vec<MathNode> },
    }
    fn take_box(slot: &mut Box<MathNode>) -> MathNode {
        *std::mem::replace(slot, Box::new(MathNode::Hline))
    }
    let explode = |node: &mut MathNode| -> Exploded {
        enum Shape {
            StyledAtom,
            Keep,
            Text,
            Symbol,
            Row,
            Substack,
            Sup,
            Sub,
            SubSup,
            Frac,
            Radical,
            Accent,
            Delim,
        }
        let shape = match &*node {
            MathNode::Atom(c, _) if crate::style_map::is_stylable(*c) => Shape::StyledAtom,
            MathNode::Text(_, _) => Shape::Text,
            MathNode::Symbol(_) => Shape::Symbol,
            MathNode::Row(_) => Shape::Row,
            MathNode::Substack(_) => Shape::Substack,
            MathNode::Superscript(_, _) => Shape::Sup,
            MathNode::Subscript(_, _) => Shape::Sub,
            MathNode::SubSup(_, _, _) => Shape::SubSup,
            MathNode::Fraction(_, _, _) => Shape::Frac,
            MathNode::Radical(_, _) => Shape::Radical,
            MathNode::Accent(_, _) => Shape::Accent,
            MathNode::Delimited(_, _, _) => Shape::Delim,
            _ => Shape::Keep,
        };
        match shape {
            Shape::StyledAtom => {
                let ch = match &*node {
                    MathNode::Atom(c, _) => *c,
                    _ => ' ',
                };
                Exploded::Leaf(MathNode::Text(TextRun::plain(&ch.to_string()), style))
            }
            Shape::Keep => Exploded::Leaf(std::mem::replace(node, MathNode::Hline)),
            Shape::Text => {
                let s = if let MathNode::Text(s, _) = &mut *node {
                    std::mem::take(s)
                } else {
                    TextRun::default()
                };
                Exploded::Leaf(MathNode::Text(s, style))
            }
            Shape::Symbol => {
                let name = if let MathNode::Symbol(name) = &mut *node {
                    std::mem::take(name)
                } else {
                    String::new()
                };
                let styled = if let Some(ch) = crate::symbols::glyph_char(&name) {
                    if crate::style_map::is_stylable(ch) {
                        MathNode::Text(TextRun::plain(&ch.to_string()), style)
                    } else {
                        MathNode::Symbol(name)
                    }
                } else {
                    MathNode::Symbol(name)
                };
                Exploded::Leaf(styled)
            }
            Shape::Row => {
                let kids = if let MathNode::Row(v) = &mut *node {
                    std::mem::take(v)
                } else {
                    Vec::new()
                };
                let n = kids.len();
                Exploded::Kids {
                    join: Join::Row(n),
                    kids,
                }
            }
            Shape::Substack => {
                let kids = if let MathNode::Substack(v) = &mut *node {
                    std::mem::take(v)
                } else {
                    Vec::new()
                };
                let n = kids.len();
                Exploded::Kids {
                    join: Join::Substack(n),
                    kids,
                }
            }
            Shape::Sup => {
                let kids = if let MathNode::Superscript(b, sup) = &mut *node {
                    vec![take_box(b), take_box(sup)]
                } else {
                    Vec::new()
                };
                Exploded::Kids {
                    join: Join::Superscript,
                    kids,
                }
            }
            Shape::Sub => {
                let kids = if let MathNode::Subscript(b, sub) = &mut *node {
                    vec![take_box(b), take_box(sub)]
                } else {
                    Vec::new()
                };
                Exploded::Kids {
                    join: Join::Subscript,
                    kids,
                }
            }
            Shape::SubSup => {
                let kids = if let MathNode::SubSup(b, sub, sup) = &mut *node {
                    vec![take_box(b), take_box(sub), take_box(sup)]
                } else {
                    Vec::new()
                };
                Exploded::Kids {
                    join: Join::SubSup,
                    kids,
                }
            }
            Shape::Frac => {
                let (style, kids) = if let MathNode::Fraction(style, num, den) = &mut *node {
                    (*style, vec![take_box(num), take_box(den)])
                } else {
                    (FracStyle::Frac, Vec::new())
                };
                Exploded::Kids {
                    join: Join::Fraction(style),
                    kids,
                }
            }
            Shape::Radical => {
                let (had_index, kids) = if let MathNode::Radical(index, body) = &mut *node {
                    let had = index.is_some();
                    let mut kids = Vec::new();
                    if let Some(index) = index.take() {
                        kids.push(*index);
                    }
                    kids.push(take_box(body));
                    (had, kids)
                } else {
                    (false, Vec::new())
                };
                Exploded::Kids {
                    join: Join::Radical(had_index),
                    kids,
                }
            }
            Shape::Accent => {
                let (kind, kids) = if let MathNode::Accent(body, kind) = &mut *node {
                    (*kind, vec![take_box(body)])
                } else {
                    (AccentKind::Hat, Vec::new())
                };
                Exploded::Kids {
                    join: Join::Accent(kind),
                    kids,
                }
            }
            Shape::Delim => {
                let (open, close, kids) = if let MathNode::Delimited(open, body, close) = &mut *node
                {
                    (
                        std::mem::replace(open, Delimiter::Empty),
                        std::mem::replace(close, Delimiter::Empty),
                        vec![take_box(body)],
                    )
                } else {
                    (Delimiter::Empty, Delimiter::Empty, Vec::new())
                };
                Exploded::Kids {
                    join: Join::Delim(open, close),
                    kids,
                }
            }
        }
    };
    let mut tasks = vec![Task::Go(node)];
    let mut vals = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            Task::Go(mut node) => match explode(&mut node) {
                Exploded::Leaf(leaf) => vals.push(leaf),
                Exploded::Kids { join, kids } => {
                    tasks.push(Task::Join(join));
                    for child in kids.into_iter().rev() {
                        tasks.push(Task::Go(child));
                    }
                }
            },
            Task::Join(join) => {
                let node = match join {
                    Join::Superscript => {
                        let sup = take_styled(&mut vals);
                        let base = take_styled(&mut vals);
                        MathNode::Superscript(Box::new(base), Box::new(sup))
                    }
                    Join::Subscript => {
                        let sub = take_styled(&mut vals);
                        let base = take_styled(&mut vals);
                        MathNode::Subscript(Box::new(base), Box::new(sub))
                    }
                    Join::SubSup => {
                        let sup = take_styled(&mut vals);
                        let sub = take_styled(&mut vals);
                        let base = take_styled(&mut vals);
                        MathNode::SubSup(Box::new(base), Box::new(sub), Box::new(sup))
                    }
                    Join::Fraction(style) => {
                        let den = take_styled(&mut vals);
                        let num = take_styled(&mut vals);
                        MathNode::Fraction(style, Box::new(num), Box::new(den))
                    }
                    Join::Radical(true) => {
                        let body = take_styled(&mut vals);
                        let index = take_styled(&mut vals);
                        MathNode::Radical(Some(Box::new(index)), Box::new(body))
                    }
                    Join::Radical(false) => {
                        let body = take_styled(&mut vals);
                        MathNode::Radical(None, Box::new(body))
                    }
                    Join::Accent(kind) => {
                        let body = take_styled(&mut vals);
                        MathNode::Accent(Box::new(body), kind)
                    }
                    Join::Delim(open, close) => {
                        let body = take_styled(&mut vals);
                        MathNode::Delimited(open, Box::new(body), close)
                    }
                    Join::Row(n) => {
                        let built = take_many(&mut vals, n);
                        collapse_text(MathNode::Row(built))
                    }
                    Join::Substack(n) => MathNode::Substack(take_many(&mut vals, n)),
                };
                vals.push(node);
            }
        }
    }
    match vals.pop() {
        Some(node) => node,
        None => MathNode::Row(Vec::new()),
    }
}

fn take_styled(vals: &mut Vec<MathNode>) -> MathNode {
    match vals.pop() {
        Some(node) => node,
        None => MathNode::Row(Vec::new()),
    }
}

fn take_many(vals: &mut Vec<MathNode>, n: usize) -> Vec<MathNode> {
    let start = vals.len().saturating_sub(n);
    vals.split_off(start)
}

fn collapse_text(mut node: MathNode) -> MathNode {
    let is_row = matches!(node, MathNode::Row(_));
    if !is_row {
        return node;
    }
    let v = if let MathNode::Row(v) = &mut node {
        std::mem::take(v)
    } else {
        Vec::new()
    };
    let mut out: Vec<MathNode> = Vec::new();
    for mut n in v {
        match (out.last_mut(), &mut n) {
            (Some(MathNode::Text(a, sa)), MathNode::Text(b, sb)) if sa == sb => {
                a.append(std::mem::take(b));
            }
            _ => out.push(n),
        }
    }
    wrap_row(out)
}

fn atom_kind(c: char) -> AtomKind {
    match c {
        '+' | '-' | '*' | '±' | '∓' | '·' | '×' | '÷' => AtomKind::Bin,
        '=' | '<' | '>' | '≠' | '≤' | '≥' | '≈' | '≡' => AtomKind::Rel,
        '(' | '[' | '{' => AtomKind::Open,
        ')' | ']' | '}' => AtomKind::Close,
        ',' | ';' | '!' | '?' | ':' => AtomKind::Punct,
        _ => AtomKind::Ord,
    }
}

fn is_delimiter_char(c: char) -> bool {
    matches!(c, '(' | ')' | '[' | ']' | '|' | '/' | '<' | '>')
}

fn is_delimiter_name(name: &str) -> bool {
    matches!(
        name,
        "{" | "}"
            | "|"
            | "langle"
            | "rangle"
            | "lfloor"
            | "rfloor"
            | "lceil"
            | "rceil"
            | "lvert"
            | "rvert"
            | "lVert"
            | "rVert"
            | "vert"
            | "Vert"
            | "uparrow"
            | "downarrow"
            | "Uparrow"
            | "Downarrow"
            | "updownarrow"
            | "Updownarrow"
            | "backslash"
            | "lgroup"
            | "rgroup"
            | "lmoustache"
            | "rmoustache"
    )
}

fn delim_from_text(s: &str) -> Result<Delimiter, ParseError> {
    let s = s.trim();
    if s.is_empty() || s == "." || s == "\\." {
        return Ok(Delimiter::Empty);
    }
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if is_delimiter_char(c) {
            return Ok(Delimiter::Char(c));
        }
        // The group text of `\{` is `{`: the brace delimiter, as in `\left\{`.
        if matches!(c, '{' | '}') {
            return Ok(Delimiter::Named(c.to_string()));
        }
        return Err(ParseError::Malformed(format!(
            "expected delimiter, found char:{c}"
        )));
    }
    let name = s.strip_prefix('\\').unwrap_or(s);
    if is_delimiter_name(name) {
        Ok(Delimiter::Named(name.to_string()))
    } else {
        Err(ParseError::Malformed(format!("unknown delimiter \\{name}")))
    }
}

fn parse_tex_dim(s: &str) -> Result<Dim, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::Malformed("empty dimension".into()));
    }
    let mut i = 0;
    let b = s.as_bytes();
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    // At least one digit and at most one point; `.` or `1.2.3` would make a
    // NaN that prints as `NaNem`.
    let mut points = 0usize;
    let mut has_digit = false;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
        if b[i] == b'.' {
            points += 1;
        } else {
            has_digit = true;
        }
        i += 1;
    }
    if !has_digit || points > 1 {
        return Err(ParseError::Malformed(format!("invalid dimension `{s}`")));
    }
    let num = Dim::parse(&s[..i]);
    let unit = s[i..].trim();
    // `Dim::in_em` is the one definition of the units (em, mu, pt, bp).
    let dim = num
        .in_em(unit)
        .ok_or_else(|| ParseError::Unsupported(format!("dimension unit {unit}")))?;
    // A number too large for the exact rational is NaN, which prints as
    // `NaNem` and cannot be read back.
    if dim.is_nan() {
        return Err(ParseError::Malformed(format!(
            "dimension out of range `{s}`"
        )));
    }
    Ok(dim)
}

fn color_err(e: Error) -> ParseError {
    match e {
        Error::Unsupported { what } => ParseError::Unsupported(what),
        Error::Parse(p) => p,
        Error::Malformed { what } => ParseError::Malformed(what),
        Error::InvalidOption { what } => ParseError::Malformed(what),
        other => ParseError::Malformed(other.to_string()),
    }
}

fn alias(name: &str) -> &str {
    match name {
        "le" => "leq",
        "ge" => "geq",
        "ne" => "neq",
        "dots" => "ldots",
        "lnot" => "neg",
        "dag" => "dagger",
        "ddag" => "ddagger",
        "owns" => "ni",
        _ => name,
    }
}

fn is_extra_symbol(name: &str) -> bool {
    matches!(
        name,
        "Gamma"
            | "Delta"
            | "Theta"
            | "Lambda"
            | "Xi"
            | "Pi"
            | "Sigma"
            | "Upsilon"
            | "Phi"
            | "Psi"
            | "Omega"
            | "varepsilon"
            | "vartheta"
            | "varpi"
            | "varrho"
            | "varsigma"
            | "varphi"
            | "ldots"
            | "cdots"
            | "vdots"
            | "ddots"
            | "colon"
            | "mid"
            | "lvert"
            | "rvert"
            | "lVert"
            | "rVert"
            | "vert"
            | "Vert"
            | "implies"
            | "iff"
            | "to"
            | "gets"
            | "neq"
            | "leq"
            | "geq"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frac_gold() {
        let n = parse(r"\frac{1}{2}").unwrap();
        assert_eq!(n.gold(), r#"(frac (atom Ord "1") (atom Ord "2"))"#);
    }

    #[test]
    fn brace_depth_follows_max_depth() {
        let braces = |n: usize| "{".repeat(n) + "x" + &"}".repeat(n);
        let opts = ParseOptions::new().with_max_depth(32);
        assert!(parse_with_options(&braces(31), &opts).is_ok());
        match parse_with_options(&braces(32), &opts) {
            Err(ParseError::Malformed(msg)) => {
                assert_eq!(msg, "input nests deeper than 32 levels");
            }
            other => panic!("32 braces: {other:?}"),
        }
    }

    #[test]
    fn frac_depth_costs_one_level_braced_or_bare() {
        // A braced argument costs the same as a bare one (spec 5, Q2).
        let frac = |n: usize| "\\frac{1}{".repeat(n) + "2" + &"}".repeat(n);
        let bare = |n: usize| "\\frac 1".repeat(n) + "2";
        let opts = ParseOptions::default();
        assert!(parse_with_options(&frac(31), &opts).is_ok());
        assert!(parse_with_options(&frac(32), &opts).is_err());
        assert!(parse_with_options(&bare(31), &opts).is_ok());
        assert!(parse_with_options(&bare(32), &opts).is_err());
    }
}

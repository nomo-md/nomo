//! Heap stack for parser nesting.
//!
//! Each recursive descent step is a frame on `Vec`, so the native stack stays
//! one frame deep. Acceptance is `max_depth` and the tokens, on every thread
//! stack and in every build profile.
//!
//! `inline(never)` keeps those steps from being glued back into one native
//! frame. A debug build gives every local its own slot; one merged frame would
//! grow with the number of states and could overflow a 256 KiB thread.

use std::mem;

use crate::color::Color;
use crate::dim::Dim;
use crate::error::ParseError;
use crate::parser::ast::{
    AccentKind, AtomKind, ColSpec, Delimiter, EnvRow, EqNumber, FracStyle, IntegralKind, MathNode,
    MatrixStyle, PhantomKind, SpaceKind, TextStyle,
};
use crate::parser::token::Token;

use super::{
    apply_scripts, apply_text_style, atom_kind, collapse_text, delim_from_text, finish_env_row,
    is_empty_node, parse_colspec, parse_tex_dim, peel_row_meta, wrap_row, Stop,
};

/// Run the parser from the current token position.
///
/// Nesting counts toward the configured maximum. The native stack does not.
pub(super) fn parse_top(parser: &mut super::Parser) -> Result<MathNode, ParseError> {
    let mut frames = Vec::new();
    push(&mut frames, parser, Kind::list(Stop::eof()), true)?;
    drive(parser, &mut frames)
}

/// Reads one `[…]` option body: a list that ends at a `]` outside nested
/// groups and nested options, as the option readers do.
pub(super) fn parse_option_body(parser: &mut super::Parser) -> Result<MathNode, ParseError> {
    let mut frames = Vec::new();
    push(&mut frames, parser, Kind::list(Stop::index()), true)?;
    drive(parser, &mut frames)
}

struct Frame {
    counts: bool,
    kind: Kind,
}

enum Kind {
    List(Box<ListState>),
    Group(GroupPhase),
    Arg(ArgPhase),
    Atom(AtomPhase),
    Nucleus(NuclPhase),
    Paren(Box<ParenState>),
    Scripts(Box<ScriptState>),
    Cmd(Box<Cmd>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupPhase {
    Open,
    /// A group that is itself a command argument: the argument frame
    /// already counts one level, so the group's list does not (spec 5, Q2).
    ArgOpen,
    Wait,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgPhase {
    Dispatch,
    Wait,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AtomPhase {
    Start,
    GotNucleus,
    GotScripts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NuclPhase {
    Dispatch,
    Wait,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListPhase {
    Scan,
    WaitAtom,
    WaitRest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParenPhase {
    Scan,
    WaitAtom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptPhase {
    Scan,
    WaitSub,
    WaitSup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seq {
    First,
    Second,
    Finish,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum One {
    Need,
    Got,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptSeq {
    Bracket,
    AfterBracket,
    Body,
    Finish,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paint {
    Text,
    Box,
    Fbox,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubPhase {
    Open,
    Scan,
    WaitLine,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BeginPhase {
    Row,
    Cell,
}

struct ListState {
    stop: Stop,
    items: Vec<MathNode>,
    phase: ListPhase,
    color: Option<Color>,
}

struct ParenState {
    items: Vec<MathNode>,
    depth: i32,
    started: bool,
    phase: ParenPhase,
}

struct ScriptState {
    nucleus: Option<Box<MathNode>>,
    sub: Option<Box<MathNode>>,
    sup: Option<Box<MathNode>>,
    sup_from_prime: bool,
    phase: ScriptPhase,
}

struct SubstackState {
    lines: Vec<MathNode>,
    phase: SubPhase,
}

struct BeginState {
    name: String,
    style: MatrixStyle,
    colspec: Vec<ColSpec>,
    rows: Vec<EnvRow>,
    cells: Vec<MathNode>,
    number: EqNumber,
    labels: Vec<String>,
    phase: BeginPhase,
}

enum Cmd {
    Frac {
        style: FracStyle,
        num: Option<Box<MathNode>>,
        phase: Seq,
    },
    Binom {
        num: Option<Box<MathNode>>,
        phase: Seq,
    },
    Genfrac {
        left: String,
        right: String,
        num: Option<Box<MathNode>>,
        phase: Seq,
    },
    Accent {
        kind: AccentKind,
        phase: One,
    },
    CancelTo {
        value: Option<Box<MathNode>>,
        phase: Seq,
    },
    Not {
        phase: One,
    },
    Stack {
        over: bool,
        first: Option<Box<MathNode>>,
        phase: Seq,
    },
    Font {
        style: TextStyle,
        phase: One,
    },
    Phantom {
        kind: PhantomKind,
        phase: One,
    },
    Paint {
        which: Paint,
        a: Color,
        b: Color,
        phase: One,
    },
    Tag {
        star: bool,
        phase: One,
    },
    Sqrt {
        index: Option<Box<MathNode>>,
        phase: OptSeq,
    },
    Xarrow {
        arrow: String,
        under: Option<Box<MathNode>>,
        phase: OptSeq,
    },
    Delim {
        open: Delimiter,
        phase: One,
    },
    Tail {
        phase: One,
    },
    Substack(Box<SubstackState>),
    Begin(Box<BeginState>),
}

enum Step {
    Call(Kind, bool),
    Again,
    Done(MathNode),
}

enum Classified {
    Node(MathNode),
    Cmd(Cmd),
}

enum OneOut {
    Step(Step),
    Node(MathNode),
}

enum Pair {
    Step(Step),
    Both(MathNode, MathNode),
}

impl Kind {
    fn list(stop: Stop) -> Self {
        Self::List(Box::new(ListState {
            stop,
            items: Vec::new(),
            phase: ListPhase::Scan,
            color: None,
        }))
    }

    fn group() -> Self {
        Self::Group(GroupPhase::Open)
    }

    fn arg_group() -> Self {
        Self::Group(GroupPhase::ArgOpen)
    }

    fn arg() -> Self {
        Self::Arg(ArgPhase::Dispatch)
    }

    fn atom() -> Self {
        Self::Atom(AtomPhase::Start)
    }

    fn nucleus() -> Self {
        Self::Nucleus(NuclPhase::Dispatch)
    }

    fn paren() -> Self {
        Self::Paren(Box::new(ParenState {
            items: Vec::new(),
            depth: 0,
            started: false,
            phase: ParenPhase::Scan,
        }))
    }

    fn scripts(nucleus: MathNode) -> Self {
        Self::Scripts(Box::new(ScriptState {
            nucleus: Some(Box::new(nucleus)),
            sub: None,
            sup: None,
            sup_from_prime: false,
            phase: ScriptPhase::Scan,
        }))
    }
}

fn internal(what: &str) -> ParseError {
    ParseError::Malformed(format!("internal: {what}"))
}

fn take_node(ready: &mut Option<MathNode>) -> Result<MathNode, ParseError> {
    ready.take().ok_or_else(|| internal("missing child value"))
}

fn push(
    frames: &mut Vec<Frame>,
    parser: &mut super::Parser,
    kind: Kind,
    counts: bool,
) -> Result<(), ParseError> {
    if counts && parser.depth >= parser.max_depth {
        return Err(ParseError::Malformed(format!(
            "input nests deeper than {} levels",
            parser.max_depth
        )));
    }
    if counts {
        parser.depth += 1;
    }
    frames.push(Frame { counts, kind });
    Ok(())
}

#[inline(never)]
fn drive(parser: &mut super::Parser, frames: &mut Vec<Frame>) -> Result<MathNode, ParseError> {
    let mut ready = None;
    loop {
        let Some(mut frame) = frames.pop() else {
            return Err(internal("empty parse stack"));
        };
        let action = step(parser, &mut frame.kind, &mut ready)?;
        match action {
            Step::Call(kind, counts) => {
                frames.push(frame);
                push(frames, parser, kind, counts)?;
                ready = None;
            }
            Step::Again => frames.push(frame),
            Step::Done(node) => {
                if frame.counts {
                    parser.depth -= 1;
                }
                if frames.is_empty() {
                    return Ok(node);
                }
                ready = Some(node);
            }
        }
    }
}

#[inline(never)]
fn step(
    parser: &mut super::Parser,
    kind: &mut Kind,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match kind {
        Kind::List(st) => step_list(parser, st, ready),
        Kind::Group(phase) => step_group(parser, phase, ready),
        Kind::Arg(phase) => step_arg(parser, phase, ready),
        Kind::Atom(phase) => step_atom(parser, phase, ready),
        Kind::Nucleus(phase) => step_nucleus(parser, phase, ready),
        Kind::Paren(st) => step_paren(parser, st, ready),
        Kind::Scripts(st) => step_scripts(parser, st, ready),
        Kind::Cmd(cmd) => step_cmd(parser, cmd, ready),
    }
}

#[inline(never)]
fn step_list(
    parser: &mut super::Parser,
    st: &mut ListState,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match st.phase {
        ListPhase::WaitAtom => {
            st.items.push(take_node(ready)?);
            st.phase = ListPhase::Scan;
            return Ok(Step::Again);
        }
        ListPhase::WaitRest => {
            let rest = take_node(ready)?;
            let Some(c) = st.color.take() else {
                return Err(internal("color"));
            };
            st.items.push(MathNode::Color(c, Box::new(rest)));
            return Ok(Step::Done(wrap_row(mem::take(&mut st.items))));
        }
        ListPhase::Scan => {}
    }
    parser.skip_ws();
    let look = match parser.peek() {
        None => 0,
        Some(tok) if parser.is_stop(tok, st.stop) => 0,
        Some(Token::MathShift | Token::DisplayShift) => 1,
        Some(Token::Command(n)) if n == "color" => 2,
        Some(Token::Command(n)) if n == "definecolor" => 3,
        Some(_) => 4,
    };
    match look {
        0 => Ok(Step::Done(wrap_row(mem::take(&mut st.items)))),
        1 => Err(ParseError::Malformed("unexpected math shift".into())),
        2 => {
            parser.bump();
            let c = parser.parse_color_from_cmd()?;
            st.color = Some(c);
            st.phase = ListPhase::WaitRest;
            Ok(Step::Call(Kind::list(st.stop), true))
        }
        3 => {
            parser.bump();
            parser.parse_definecolor()?;
            Ok(Step::Again)
        }
        _ => {
            parser.stop = st.stop;
            st.phase = ListPhase::WaitAtom;
            Ok(Step::Call(Kind::atom(), false))
        }
    }
}

#[inline(never)]
fn step_group(
    parser: &mut super::Parser,
    phase: &mut GroupPhase,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        GroupPhase::Open | GroupPhase::ArgOpen => {
            let counts = *phase == GroupPhase::Open;
            match parser.bump() {
                Some(Token::BeginGroup) => {}
                _ => return Err(ParseError::Malformed("expected '{'".into())),
            }
            *phase = GroupPhase::Wait;
            Ok(Step::Call(Kind::list(Stop::group()), counts))
        }
        GroupPhase::Wait => match parser.bump() {
            Some(Token::EndGroup) => Ok(Step::Done(take_node(ready)?)),
            _ => Err(ParseError::Malformed("unmatched '{'".into())),
        },
    }
}

#[inline(never)]
fn step_arg(
    parser: &mut super::Parser,
    phase: &mut ArgPhase,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        ArgPhase::Dispatch => {
            parser.skip_ws();
            let look = match parser.peek() {
                Some(Token::BeginGroup) => 1,
                Some(Token::Char('(')) => 2,
                Some(_) => 3,
                None => 0,
            };
            *phase = ArgPhase::Wait;
            match look {
                1 => Ok(Step::Call(Kind::arg_group(), false)),
                2 => Ok(Step::Call(Kind::paren(), false)),
                3 => Ok(Step::Call(Kind::nucleus(), false)),
                _ => Err(ParseError::Malformed("missing argument".into())),
            }
        }
        ArgPhase::Wait => Ok(Step::Done(take_node(ready)?)),
    }
}

#[inline(never)]
fn step_atom(
    parser: &mut super::Parser,
    phase: &mut AtomPhase,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        AtomPhase::Start => {
            parser.skip_ws();
            if matches!(parser.peek(), Some(Token::Char('('))) && parser.script_follows_parens() {
                *phase = AtomPhase::GotNucleus;
                Ok(Step::Call(Kind::paren(), true))
            } else {
                *phase = AtomPhase::GotNucleus;
                Ok(Step::Call(Kind::nucleus(), false))
            }
        }
        AtomPhase::GotNucleus => {
            let mut nucleus = take_node(ready)?;
            let mut limits = None;
            loop {
                let look = match parser.peek_ws() {
                    Some(Token::Command(n)) if n == "limits" => 1,
                    Some(Token::Command(n)) if n == "nolimits" => 2,
                    _ => 0,
                };
                match look {
                    1 => {
                        parser.bump();
                        limits = Some(true);
                    }
                    2 => {
                        parser.bump();
                        limits = Some(false);
                    }
                    _ => break,
                }
            }
            if let Some(flag) = limits {
                if let MathNode::Operator(_, slot) = &mut nucleus {
                    *slot = flag;
                }
            }
            *phase = AtomPhase::GotScripts;
            Ok(Step::Call(Kind::scripts(nucleus), false))
        }
        AtomPhase::GotScripts => Ok(Step::Done(take_node(ready)?)),
    }
}

#[inline(never)]
fn step_nucleus(
    parser: &mut super::Parser,
    phase: &mut NuclPhase,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        NuclPhase::Wait => Ok(Step::Done(take_node(ready)?)),
        NuclPhase::Dispatch => {
            parser.skip_ws();
            match parser.peek().cloned() {
                None => Err(ParseError::Malformed("unexpected end of input".into())),
                Some(Token::Char('\'') | Token::Superscript | Token::Subscript) => {
                    Err(ParseError::Malformed("script has no base".into()))
                }
                Some(Token::BeginGroup) => {
                    *phase = NuclPhase::Wait;
                    Ok(Step::Call(Kind::group(), false))
                }
                Some(Token::Char(c)) => match nucleus_char(parser, c)? {
                    CharNucleus::Node(node) => Ok(Step::Done(node)),
                    CharNucleus::Sqrt => {
                        *phase = NuclPhase::Wait;
                        Ok(Step::Call(
                            Kind::Cmd(Box::new(Cmd::Sqrt {
                                index: None,
                                phase: OptSeq::Body,
                            })),
                            false,
                        ))
                    }
                },
                Some(Token::Command(name)) => {
                    parser.bump();
                    match classify(parser, &name)? {
                        Classified::Node(node) => Ok(Step::Done(node)),
                        Classified::Cmd(cmd) => {
                            *phase = NuclPhase::Wait;
                            Ok(Step::Call(Kind::Cmd(Box::new(cmd)), false))
                        }
                    }
                }
                Some(other) => Err(ParseError::Malformed(format!("unexpected token {other}"))),
            }
        }
    }
}

enum CharNucleus {
    Node(MathNode),
    Sqrt,
}

fn nucleus_char(parser: &mut super::Parser, c: char) -> Result<CharNucleus, ParseError> {
    if super::lexicon::is_superscript_char(c) {
        return Err(ParseError::Malformed("script has no base".into()));
    }
    if c == '\u{221A}' {
        parser.bump();
        return Ok(CharNucleus::Sqrt);
    }
    if c == '\u{2212}' {
        parser.bump();
        return Ok(CharNucleus::Node(MathNode::Atom('-', atom_kind('-'))));
    }
    if parser.bare_operators && c.is_ascii_alphabetic() {
        if let Some(node) = take_bare_word(parser) {
            return Ok(CharNucleus::Node(node));
        }
    }
    if let Some(name) = ascii_relation(parser, c) {
        return Ok(CharNucleus::Node(MathNode::Symbol(name.into())));
    }
    if parser.unicode_symbols {
        if let Some(name) = super::lexicon::unicode_symbol(c) {
            parser.bump();
            return Ok(CharNucleus::Node(MathNode::Symbol(name.into())));
        }
    }
    parser.bump();
    Ok(CharNucleus::Node(MathNode::Atom(c, atom_kind(c))))
}

fn take_bare_word(parser: &mut super::Parser) -> Option<MathNode> {
    if parser.pos < parser.plain_letters_until {
        return None;
    }
    let (end, word) = ascii_letter_run(parser);
    if let Some(node) = super::lexicon::bare_identifier(&word, parser.cases_depth > 0) {
        parser.pos = end;
        Some(node)
    } else {
        parser.plain_letters_until = end;
        None
    }
}

fn ascii_letter_run(parser: &super::Parser) -> (usize, String) {
    let mut end = parser.pos;
    let mut word = String::new();
    while let Some(Token::Char(c)) = parser.tokens.get(end) {
        if !c.is_ascii_alphabetic() {
            break;
        }
        word.push(*c);
        end += 1;
    }
    (end, word)
}

/// Adjacent `<=`, `>=`, and `!=` when `ascii_relations` is on. A space between
/// the two characters is not a match.
fn ascii_relation(parser: &mut super::Parser, c: char) -> Option<&'static str> {
    if !parser.ascii_relations {
        return None;
    }
    let name = match c {
        '<' => "leq",
        '>' => "geq",
        '!' => "neq",
        _ => return None,
    };
    match parser.tokens.get(parser.pos + 1) {
        Some(Token::Char('=')) => {
            parser.pos += 2;
            Some(name)
        }
        _ => None,
    }
}

fn peek_superscript_base(parser: &super::Parser) -> Option<char> {
    match parser.peek() {
        Some(Token::Char(c)) => super::lexicon::superscript_base(*c),
        _ => None,
    }
}

fn take_superscript_run(parser: &mut super::Parser) -> Result<MathNode, ParseError> {
    parser.skip_ws();
    let mut items = Vec::new();
    while let Some(base) = peek_superscript_base(parser) {
        parser.bump_raw();
        items.push(MathNode::Atom(base, atom_kind(base)));
    }
    if items.is_empty() {
        return Err(internal("superscript run"));
    }
    Ok(wrap_row(items))
}

#[inline(never)]
fn step_paren(
    parser: &mut super::Parser,
    st: &mut ParenState,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    if st.phase == ParenPhase::WaitAtom {
        st.items.push(take_node(ready)?);
        st.phase = ParenPhase::Scan;
        return Ok(Step::Again);
    }
    if !st.started {
        match parser.bump() {
            Some(Token::Char('(')) => {}
            _ => return Err(ParseError::Malformed("expected '('".into())),
        }
        st.items.push(MathNode::Atom('(', AtomKind::Open));
        st.depth = 1;
        st.started = true;
    }
    parser.skip_ws();
    let look = match parser.peek() {
        None => 0,
        Some(Token::Char(')')) => 1,
        Some(Token::Char('(')) => 2,
        Some(_) => 3,
    };
    match look {
        0 => Err(ParseError::Malformed("unmatched '('".into())),
        1 => {
            parser.bump();
            st.items.push(MathNode::Atom(')', AtomKind::Close));
            st.depth -= 1;
            if st.depth == 0 {
                Ok(Step::Done(wrap_row(mem::take(&mut st.items))))
            } else {
                Ok(Step::Again)
            }
        }
        2 if parser.script_follows_parens() => {
            st.phase = ParenPhase::WaitAtom;
            Ok(Step::Call(Kind::atom(), false))
        }
        2 => {
            parser.bump();
            st.items.push(MathNode::Atom('(', AtomKind::Open));
            st.depth += 1;
            Ok(Step::Again)
        }
        _ => {
            st.phase = ParenPhase::WaitAtom;
            Ok(Step::Call(Kind::atom(), false))
        }
    }
}

#[inline(never)]
fn step_scripts(
    parser: &mut super::Parser,
    st: &mut ScriptState,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match st.phase {
        ScriptPhase::WaitSub => {
            st.sub = Some(Box::new(take_node(ready)?));
            st.phase = ScriptPhase::Scan;
            return Ok(Step::Again);
        }
        ScriptPhase::WaitSup => {
            st.sup = Some(Box::new(take_node(ready)?));
            st.phase = ScriptPhase::Scan;
            return Ok(Step::Again);
        }
        ScriptPhase::Scan => {}
    }
    let sig = match parser.peek_ws() {
        Some(Token::Subscript) => 1,
        Some(Token::Superscript) => 2,
        Some(Token::Char('\'')) => 3,
        Some(Token::Char(c)) if super::lexicon::is_superscript_char(*c) => 4,
        _ => 0,
    };
    match sig {
        1 => {
            parser.bump();
            if st.sub.is_some() {
                return Err(ParseError::Malformed("double subscript".into()));
            }
            st.phase = ScriptPhase::WaitSub;
            Ok(Step::Call(Kind::arg(), true))
        }
        2 => {
            parser.bump();
            if st.sup_from_prime {
                let nucleus = unbox(st.nucleus.take(), "script nucleus")?;
                st.nucleus = Some(Box::new(apply_scripts(
                    nucleus,
                    st.sub.take().map(|n| *n),
                    st.sup.take().map(|n| *n),
                )));
                st.sup_from_prime = false;
                st.phase = ScriptPhase::WaitSup;
                Ok(Step::Call(Kind::arg(), true))
            } else if st.sup.is_some() {
                Err(ParseError::Malformed("double superscript".into()))
            } else {
                st.phase = ScriptPhase::WaitSup;
                Ok(Step::Call(Kind::arg(), true))
            }
        }
        3 => {
            parser.bump();
            let prime = if parser.prime_symbol {
                MathNode::Symbol("prime".into())
            } else {
                MathNode::Atom('′', AtomKind::Ord)
            };
            st.sup = Some(Box::new(match st.sup.take() {
                None => prime,
                Some(prev) => push_prime(*prev, prime, st.sup_from_prime),
            }));
            st.sup_from_prime = true;
            Ok(Step::Again)
        }
        4 => {
            let body = take_superscript_run(parser)?;
            if st.sup_from_prime {
                let nucleus = unbox(st.nucleus.take(), "script nucleus")?;
                st.nucleus = Some(Box::new(apply_scripts(
                    nucleus,
                    st.sub.take().map(|n| *n),
                    st.sup.take().map(|n| *n),
                )));
                st.sup = Some(Box::new(body));
                st.sup_from_prime = false;
                Ok(Step::Again)
            } else if st.sup.is_some() {
                Err(ParseError::Malformed("double superscript".into()))
            } else {
                st.sup = Some(Box::new(body));
                Ok(Step::Again)
            }
        }
        _ => {
            let nucleus = unbox(st.nucleus.take(), "script nucleus")?;
            Ok(Step::Done(apply_scripts(
                nucleus,
                st.sub.take().map(|n| *n),
                st.sup.take().map(|n| *n),
            )))
        }
    }
}

fn unbox(node: Option<Box<MathNode>>, what: &str) -> Result<MathNode, ParseError> {
    match node {
        Some(node) => Ok(*node),
        None => Err(internal(what)),
    }
}

/// Append one prime to a superscript. When the previous token was also a
/// prime, `prev` is the run so far, so the run stays one flat row and the
/// printer never nests deeper than the source. `MathNode` implements `Drop`,
/// so the row is extended in place rather than destructured.
fn push_prime(mut prev: MathNode, prime: MathNode, run: bool) -> MathNode {
    if run {
        if let MathNode::Row(items) = &mut prev {
            items.push(prime);
            return prev;
        }
    }
    MathNode::Row(vec![prev, prime])
}

fn pair(
    num: &mut Option<Box<MathNode>>,
    phase: &mut Seq,
    ready: &mut Option<MathNode>,
) -> Result<Pair, ParseError> {
    match *phase {
        Seq::First => {
            *phase = Seq::Second;
            Ok(Pair::Step(Step::Call(Kind::arg(), true)))
        }
        Seq::Second => {
            *num = Some(Box::new(take_node(ready)?));
            *phase = Seq::Finish;
            Ok(Pair::Step(Step::Call(Kind::arg(), true)))
        }
        Seq::Finish => {
            let second = take_node(ready)?;
            let first = unbox(num.take(), "pair")?;
            Ok(Pair::Both(first, second))
        }
    }
}

fn one_call(
    phase: &mut One,
    ready: &mut Option<MathNode>,
    counts: bool,
    kind: impl FnOnce() -> Kind,
) -> Result<OneOut, ParseError> {
    match *phase {
        One::Need => {
            *phase = One::Got;
            Ok(OneOut::Step(Step::Call(kind(), counts)))
        }
        One::Got => Ok(OneOut::Node(take_node(ready)?)),
    }
}

#[inline(never)]
fn step_cmd(
    parser: &mut super::Parser,
    cmd: &mut Cmd,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match cmd {
        Cmd::Frac { style, num, phase } => match pair(num, phase, ready)? {
            Pair::Step(step) => Ok(step),
            Pair::Both(n, d) => {
                if is_empty_node(&n) && is_empty_node(&d) {
                    return Err(ParseError::Malformed("empty fraction".into()));
                }
                Ok(Step::Done(MathNode::Fraction(
                    *style,
                    Box::new(n),
                    Box::new(d),
                )))
            }
        },
        Cmd::Binom { num, phase } => match pair(num, phase, ready)? {
            Pair::Step(step) => Ok(step),
            Pair::Both(n, k) => {
                if is_empty_node(&n) && is_empty_node(&k) {
                    return Err(ParseError::Malformed("empty binomial".into()));
                }
                Ok(Step::Done(MathNode::Delimited(
                    Delimiter::Char('('),
                    Box::new(MathNode::Fraction(
                        FracStyle::Frac,
                        Box::new(n),
                        Box::new(k),
                    )),
                    Delimiter::Char(')'),
                )))
            }
        },
        Cmd::Genfrac {
            left,
            right,
            num,
            phase,
        } => match pair(num, phase, ready)? {
            Pair::Step(step) => Ok(step),
            Pair::Both(n, d) => {
                let frac = MathNode::Fraction(FracStyle::Frac, Box::new(n), Box::new(d));
                if left.is_empty() && right.is_empty() {
                    return Ok(Step::Done(frac));
                }
                Ok(Step::Done(MathNode::Delimited(
                    delim_from_text(left)?,
                    Box::new(frac),
                    delim_from_text(right)?,
                )))
            }
        },
        Cmd::CancelTo { value, phase } => match pair(value, phase, ready)? {
            Pair::Step(step) => Ok(step),
            Pair::Both(value, expr) => {
                if is_empty_node(&expr) {
                    return Err(ParseError::Malformed("empty accent base".into()));
                }
                Ok(Step::Done(MathNode::CancelTo(
                    Box::new(value),
                    Box::new(expr),
                )))
            }
        },
        Cmd::Stack { over, first, phase } => match pair(first, phase, ready)? {
            Pair::Step(step) => Ok(step),
            Pair::Both(extra, base) => {
                let (above, below) = if *over {
                    (Some(Box::new(extra)), None)
                } else {
                    (None, Some(Box::new(extra)))
                };
                Ok(Step::Done(MathNode::OverUnder(
                    Box::new(base),
                    above,
                    below,
                )))
            }
        },
        Cmd::Accent { kind, phase } => match one_call(phase, ready, true, Kind::arg)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => {
                if is_empty_node(&body) {
                    return Err(ParseError::Malformed("empty accent base".into()));
                }
                Ok(Step::Done(MathNode::Accent(Box::new(body), *kind)))
            }
        },
        Cmd::Not { phase } => match one_call(phase, ready, false, Kind::nucleus)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => Ok(Step::Done(MathNode::Accent(
                Box::new(body),
                AccentKind::Not,
            ))),
        },
        Cmd::Tail { phase } => match one_call(phase, ready, false, Kind::nucleus)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => Ok(Step::Done(body)),
        },
        Cmd::Font { style, phase } => match one_call(phase, ready, true, Kind::arg)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(inner) => Ok(Step::Done(collapse_text(apply_text_style(inner, *style)))),
        },
        Cmd::Phantom { kind, phase } => match one_call(phase, ready, true, Kind::arg)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => Ok(Step::Done(MathNode::Phantom(*kind, Box::new(body)))),
        },
        Cmd::Paint { which, a, b, phase } => match one_call(phase, ready, true, Kind::arg)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => {
                let body = Box::new(body);
                Ok(Step::Done(match *which {
                    Paint::Text => MathNode::TextColor(*a, body),
                    Paint::Box => MathNode::ColorBox(*a, body),
                    Paint::Fbox => MathNode::FColorBox(*a, *b, body),
                }))
            }
        },
        Cmd::Tag { star, phase } => match one_call(phase, ready, true, Kind::arg)? {
            OneOut::Step(step) => Ok(step),
            OneOut::Node(body) => Ok(Step::Done(MathNode::Tag {
                star: *star,
                body: Box::new(body),
            })),
        },
        Cmd::Sqrt { index, phase } => step_sqrt(parser, index, phase, ready),
        Cmd::Xarrow {
            arrow,
            under,
            phase,
        } => step_xarrow(parser, arrow, under, phase, ready),
        Cmd::Delim { open, phase } => step_delim(parser, open, phase, ready),
        Cmd::Substack(st) => step_substack(parser, st, ready),
        Cmd::Begin(st) => step_begin(parser, st, ready),
    }
}

fn step_sqrt(
    parser: &mut super::Parser,
    index: &mut Option<Box<MathNode>>,
    phase: &mut OptSeq,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        OptSeq::Bracket => {
            *phase = OptSeq::AfterBracket;
            Ok(Step::Call(Kind::list(Stop::index()), true))
        }
        OptSeq::AfterBracket => {
            *index = Some(Box::new(take_node(ready)?));
            match parser.bump() {
                Some(Token::Char(']')) => {}
                _ => {
                    return Err(ParseError::Malformed(
                        "expected ']' after \\sqrt index".into(),
                    ))
                }
            }
            *phase = OptSeq::Finish;
            Ok(Step::Call(Kind::arg(), true))
        }
        OptSeq::Body => {
            *phase = OptSeq::Finish;
            Ok(Step::Call(Kind::arg(), true))
        }
        OptSeq::Finish => {
            let body = take_node(ready)?;
            Ok(Step::Done(MathNode::Radical(index.take(), Box::new(body))))
        }
    }
}

fn step_xarrow(
    parser: &mut super::Parser,
    arrow: &mut String,
    under: &mut Option<Box<MathNode>>,
    phase: &mut OptSeq,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match *phase {
        OptSeq::Bracket => {
            *phase = OptSeq::AfterBracket;
            Ok(Step::Call(Kind::list(Stop::index()), true))
        }
        OptSeq::AfterBracket => {
            *under = Some(Box::new(take_node(ready)?));
            match parser.bump() {
                Some(Token::Char(']')) => {}
                _ => {
                    return Err(ParseError::Malformed(
                        "expected ']' after x-arrow optional argument".into(),
                    ))
                }
            }
            *phase = OptSeq::Finish;
            Ok(Step::Call(Kind::arg(), true))
        }
        OptSeq::Body => {
            *phase = OptSeq::Finish;
            Ok(Step::Call(Kind::arg(), true))
        }
        OptSeq::Finish => {
            let over = take_node(ready)?;
            Ok(Step::Done(MathNode::OverUnder(
                Box::new(MathNode::Symbol(mem::take(arrow))),
                Some(Box::new(over)),
                under.take(),
            )))
        }
    }
}

fn step_delim(
    parser: &mut super::Parser,
    open: &mut Delimiter,
    phase: &mut One,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match one_call(phase, ready, true, || Kind::list(Stop::delim()))? {
        OneOut::Step(step) => Ok(step),
        OneOut::Node(body) => {
            match parser.bump() {
                Some(Token::Command(n)) if n == "right" => {}
                _ => return Err(ParseError::UnmatchedDelimiter),
            }
            let close = parser.parse_delimiter()?;
            let open = mem::replace(open, Delimiter::Empty);
            Ok(Step::Done(MathNode::Delimited(open, Box::new(body), close)))
        }
    }
}

#[inline(never)]
fn step_substack(
    parser: &mut super::Parser,
    st: &mut SubstackState,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    match st.phase {
        SubPhase::Open => {
            match parser.bump() {
                Some(Token::BeginGroup) => {}
                _ => {
                    return Err(ParseError::Malformed(
                        "expected '{' after \\substack".into(),
                    ))
                }
            }
            st.phase = SubPhase::Scan;
            Ok(Step::Again)
        }
        SubPhase::Scan => {
            parser.skip_ws();
            if matches!(parser.peek(), Some(Token::EndGroup)) {
                parser.bump();
                return finish_substack(st);
            }
            st.phase = SubPhase::WaitLine;
            Ok(Step::Call(Kind::list(Stop::substack_line()), true))
        }
        SubPhase::WaitLine => {
            st.lines.push(take_node(ready)?);
            parser.skip_ws();
            let look = match parser.peek() {
                Some(Token::Command(n)) if n == "\\" || n == "cr" => 1,
                Some(Token::EndGroup) => 2,
                None => 0,
                Some(_) => 3,
            };
            match look {
                1 => {
                    parser.bump();
                    st.phase = SubPhase::Scan;
                    Ok(Step::Again)
                }
                2 => {
                    parser.bump();
                    finish_substack(st)
                }
                0 => Err(ParseError::Malformed("unmatched '{' in \\substack".into())),
                _ => {
                    let Some(tok) = parser.peek().cloned() else {
                        return Err(internal("substack token"));
                    };
                    Err(ParseError::Malformed(format!(
                        "unexpected token {tok} in \\substack"
                    )))
                }
            }
        }
    }
}

fn finish_substack(st: &mut SubstackState) -> Result<Step, ParseError> {
    if st.lines.is_empty() {
        return Err(ParseError::Malformed("empty \\substack".into()));
    }
    Ok(Step::Done(MathNode::Substack(mem::take(&mut st.lines))))
}

#[inline(never)]
fn step_begin(
    parser: &mut super::Parser,
    st: &mut BeginState,
    ready: &mut Option<MathNode>,
) -> Result<Step, ParseError> {
    if st.phase == BeginPhase::Cell {
        let cell = take_node(ready)?;
        let cell = peel_row_meta(cell, &mut st.number, &mut st.labels);
        st.cells.push(cell);
        parser.skip_ws();
        let look = match parser.peek() {
            Some(Token::AlignmentTab) => 1,
            Some(Token::Command(n)) if n == "\\" || n == "cr" => 2,
            Some(Token::Command(n)) if n == "end" => 3,
            None => 0,
            Some(_) => 4,
        };
        return match look {
            1 => {
                parser.bump();
                Ok(Step::Call(Kind::list(Stop::cell()), true))
            }
            2 => {
                parser.bump();
                push_row(st);
                parser.skip_ws();
                if matches!(parser.peek(), Some(Token::Command(n)) if n == "end") {
                    finish_begin(parser, st)
                } else {
                    st.phase = BeginPhase::Row;
                    Ok(Step::Again)
                }
            }
            3 => {
                push_row(st);
                finish_begin(parser, st)
            }
            0 => Err(ParseError::Malformed(
                "unmatched \\begin (missing \\end)".into(),
            )),
            _ => {
                let Some(tok) = parser.peek().cloned() else {
                    return Err(internal("environment token"));
                };
                Err(ParseError::Malformed(format!(
                    "unexpected token {tok} in environment body"
                )))
            }
        };
    }
    parser.skip_ws();
    let look = match parser.peek() {
        Some(Token::Command(n)) if n == "end" => 1,
        Some(Token::Command(n)) if n == "hline" => 2,
        Some(Token::Command(n)) if n == "intertext" => 3,
        _ => 0,
    };
    match look {
        1 => finish_begin(parser, st),
        2 => {
            parser.bump();
            st.rows.push(EnvRow::Hline);
            Ok(Step::Again)
        }
        3 => {
            parser.bump();
            let run = parser.text_run()?;
            st.rows.push(EnvRow::Intertext(Box::new(MathNode::Text(
                run,
                TextStyle::Text,
            ))));
            Ok(Step::Again)
        }
        _ => {
            st.phase = BeginPhase::Cell;
            Ok(Step::Call(Kind::list(Stop::cell()), true))
        }
    }
}

fn push_row(st: &mut BeginState) {
    let cells = mem::take(&mut st.cells);
    let number = mem::replace(&mut st.number, EqNumber::Default);
    let labels = mem::take(&mut st.labels);
    st.rows.push(finish_env_row(cells, number, labels));
}

fn finish_begin(parser: &mut super::Parser, st: &mut BeginState) -> Result<Step, ParseError> {
    parser.expect_end(&st.name)?;
    if matches!(st.style, MatrixStyle::Cases) {
        parser.cases_depth = parser.cases_depth.saturating_sub(1);
    }
    Ok(Step::Done(MathNode::Matrix(
        st.style,
        mem::take(&mut st.colspec),
        mem::take(&mut st.rows),
    )))
}

#[inline(never)]
fn classify(parser: &mut super::Parser, name: &str) -> Result<Classified, ParseError> {
    match name {
        "frac" | "dfrac" | "tfrac" | "cfrac" => {
            let style = match name {
                "dfrac" => FracStyle::Dfrac,
                "tfrac" => FracStyle::Tfrac,
                "cfrac" => FracStyle::Cfrac,
                _ => FracStyle::Frac,
            };
            Ok(Classified::Cmd(Cmd::Frac {
                style,
                num: None,
                phase: Seq::First,
            }))
        }
        "\\" | "cr" => Err(ParseError::RowBreakOutside),
        "binom" | "dbinom" | "tbinom" => Ok(Classified::Cmd(Cmd::Binom {
            num: None,
            phase: Seq::First,
        })),
        "genfrac" => {
            let left = parser.collect_group_text()?;
            let right = parser.collect_group_text()?;
            let _thickness = parser.collect_group_text()?;
            let _style = parser.collect_group_text()?;
            Ok(Classified::Cmd(Cmd::Genfrac {
                left,
                right,
                num: None,
                phase: Seq::First,
            }))
        }
        "sqrt" => {
            let phase = if matches!(parser.peek_ws(), Some(Token::Char('['))) {
                parser.bump();
                OptSeq::Bracket
            } else {
                OptSeq::Body
            };
            Ok(Classified::Cmd(Cmd::Sqrt { index: None, phase }))
        }
        "left" => {
            let open = parser.parse_delimiter()?;
            Ok(Classified::Cmd(Cmd::Delim {
                open,
                phase: One::Need,
            }))
        }
        "right" => Err(ParseError::UnmatchedDelimiter),
        "begin" => start_begin(parser),
        "end" => Err(ParseError::Malformed("unexpected \\end".into())),
        "over" => Err(ParseError::Malformed("\\over outside a group".into())),
        "choose" => Err(ParseError::Malformed("\\choose outside a group".into())),
        "hat" => accent(AccentKind::Hat),
        "check" => accent(AccentKind::Check),
        "breve" => accent(AccentKind::Breve),
        "acute" => accent(AccentKind::Acute),
        "grave" => accent(AccentKind::Grave),
        "tilde" => accent(AccentKind::Tilde),
        "bar" => accent(AccentKind::Bar),
        "vec" => accent(AccentKind::Vec),
        "dot" => accent(AccentKind::Dot),
        "ddot" => accent(AccentKind::Ddot),
        "dddot" => accent(AccentKind::Dddot),
        "ddddot" => accent(AccentKind::Ddddot),
        "widehat" => accent(AccentKind::WideHat),
        "widetilde" => accent(AccentKind::WideTilde),
        "overline" => accent(AccentKind::Overline),
        "underline" => accent(AccentKind::Underline),
        "overbrace" => accent(AccentKind::Overbrace),
        "underbrace" => accent(AccentKind::Underbrace),
        "overleftarrow" => accent(AccentKind::Overleftarrow),
        "overrightarrow" => accent(AccentKind::Overrightarrow),
        "overleftrightarrow" => accent(AccentKind::Overleftrightarrow),
        "underleftarrow" => accent(AccentKind::Underleftarrow),
        "underrightarrow" => accent(AccentKind::Underrightarrow),
        "underleftrightarrow" => accent(AccentKind::Underleftrightarrow),
        "cancel" => accent(AccentKind::Cancel),
        "bcancel" => accent(AccentKind::BCancel),
        "xcancel" => accent(AccentKind::XCancel),
        "boxed" | "fbox" => accent(AccentKind::Boxed),
        "mathring" => accent(AccentKind::Ring),
        "cancelto" => Ok(Classified::Cmd(Cmd::CancelTo {
            value: None,
            phase: Seq::First,
        })),
        "not" => Ok(Classified::Cmd(Cmd::Not { phase: One::Need })),
        "overset" | "stackrel" => Ok(Classified::Cmd(Cmd::Stack {
            over: true,
            first: None,
            phase: Seq::First,
        })),
        "underset" => Ok(Classified::Cmd(Cmd::Stack {
            over: false,
            first: None,
            phase: Seq::First,
        })),
        "mathrm" | "textrm" => font(TextStyle::Rm),
        "mathbf" | "textbf" => font(TextStyle::Bf),
        "mathit" | "textit" => font(TextStyle::It),
        "mathsf" | "textsf" => font(TextStyle::Sf),
        "mathtt" | "texttt" => font(TextStyle::Tt),
        "mathbb" => font(TextStyle::Bb),
        "mathcal" => font(TextStyle::Cal),
        "mathfrak" => font(TextStyle::Frak),
        "mathscr" => font(TextStyle::Scr),
        "boldsymbol" => font(TextStyle::Boldsymbol),
        "pmb" => font(TextStyle::Pmb),
        "xrightarrow" => start_xarrow(parser, "longrightarrow"),
        "xleftarrow" => start_xarrow(parser, "longleftarrow"),
        "text" | "mbox" => Ok(Classified::Node(parser.parse_text(TextStyle::Text)?)),
        "operatorname" => {
            let op = parser.collect_group_text()?;
            Ok(Classified::Node(MathNode::Operator(op, false)))
        }
        "," => Ok(Classified::Node(MathNode::Space(SpaceKind::Thin))),
        ":" | ">" => Ok(Classified::Node(MathNode::Space(SpaceKind::Medium))),
        ";" => Ok(Classified::Node(MathNode::Space(SpaceKind::Thick))),
        "!" => Ok(Classified::Node(MathNode::Space(SpaceKind::NegThin))),
        "quad" => Ok(Classified::Node(MathNode::Space(SpaceKind::Quad))),
        "qquad" => Ok(Classified::Node(MathNode::Space(SpaceKind::Qquad))),
        " " => Ok(Classified::Node(MathNode::Space(SpaceKind::ControlSpace))),
        "hspace" => {
            let spec = parser.collect_group_text()?;
            let d = parse_tex_dim(&spec)?;
            Ok(Classified::Node(MathNode::Space(SpaceKind::Hspace(d))))
        }
        "phantom" => phantom(PhantomKind::Full),
        "vphantom" => phantom(PhantomKind::Vertical),
        "hphantom" => phantom(PhantomKind::Horizontal),
        "strut" => Ok(Classified::Node(MathNode::Strut(
            Dim::ratio(7, 10),
            Dim::ratio(3, 10),
        ))),
        "rule" => {
            let _w = parse_tex_dim(&parser.collect_group_text()?)?;
            let h = parse_tex_dim(&parser.collect_group_text()?)?;
            Ok(Classified::Node(MathNode::Strut(h, Dim::zero())))
        }
        "textcolor" => {
            let c = parser.parse_color_from_cmd()?;
            Ok(Classified::Cmd(Cmd::Paint {
                which: Paint::Text,
                a: c,
                b: c,
                phase: One::Need,
            }))
        }
        "colorbox" => {
            let c = parser.parse_color_from_cmd()?;
            Ok(Classified::Cmd(Cmd::Paint {
                which: Paint::Box,
                a: c,
                b: c,
                phase: One::Need,
            }))
        }
        "fcolorbox" => {
            let border = parser.parse_color_from_cmd()?;
            let fill = parser.parse_color_from_cmd()?;
            Ok(Classified::Cmd(Cmd::Paint {
                which: Paint::Fbox,
                a: border,
                b: fill,
                phase: One::Need,
            }))
        }
        "sum" => Ok(Classified::Node(MathNode::Sum(None, None))),
        "prod" => Ok(Classified::Node(MathNode::Product(None, None))),
        "int" => Ok(Classified::Node(MathNode::Integral(
            IntegralKind::Int,
            None,
            None,
        ))),
        "iint" => Ok(Classified::Node(MathNode::Integral(
            IntegralKind::Iint,
            None,
            None,
        ))),
        "iiint" => Ok(Classified::Node(MathNode::Integral(
            IntegralKind::Iiint,
            None,
            None,
        ))),
        "oint" => Ok(Classified::Node(MathNode::Integral(
            IntegralKind::Oint,
            None,
            None,
        ))),
        "oiint" => Ok(Classified::Node(MathNode::Integral(
            IntegralKind::Oiint,
            None,
            None,
        ))),
        "lim" => Ok(Classified::Node(MathNode::Limit(None))),
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh"
        | "cosh" | "tanh" | "coth" | "log" | "ln" | "lg" | "exp" | "limsup" | "liminf" | "sup"
        | "inf" | "max" | "min" | "det" | "dim" | "ker" | "deg" | "gcd" | "lcm" | "Pr" | "arg" => {
            Ok(Classified::Node(MathNode::Operator(
                name.to_string(),
                false,
            )))
        }
        "coprod" | "bigcup" | "bigcap" | "bigsqcup" | "bigvee" | "bigwedge" | "bigoplus"
        | "bigotimes" | "biguplus" => {
            Ok(Classified::Node(MathNode::Operator(name.to_string(), true)))
        }
        "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr"
        | "Biggl" | "Biggr" | "bigm" | "Bigm" | "biggm" | "Biggm" => {
            Ok(Classified::Node(parser.parse_sized_delim(name)?))
        }
        "tag" => {
            let star = matches!(parser.peek_ws(), Some(Token::Char('*')));
            if star {
                parser.bump();
            }
            Ok(Classified::Cmd(Cmd::Tag {
                star,
                phase: One::Need,
            }))
        }
        "label" => {
            let key = parser.collect_group_text()?;
            Ok(Classified::Node(MathNode::Label(key)))
        }
        "ref" => {
            let key = parser.collect_group_text()?;
            Ok(Classified::Node(MathNode::Ref(key)))
        }
        "nonumber" | "notag" => Ok(Classified::Node(MathNode::NoNumber)),
        "hline" => Ok(Classified::Node(MathNode::Hline)),
        "intertext" => {
            let run = parser.text_run()?;
            Ok(Classified::Node(MathNode::Intertext(Box::new(
                MathNode::Text(run, TextStyle::Text),
            ))))
        }
        "substack" => Ok(Classified::Cmd(Cmd::Substack(Box::new(SubstackState {
            lines: Vec::new(),
            phase: SubPhase::Open,
        })))),
        "displaystyle" | "textstyle" | "scriptstyle" | "scriptscriptstyle" | "limits"
        | "nolimits" => Ok(Classified::Cmd(Cmd::Tail { phase: One::Need })),
        "{" | "}" => {
            let (ch, kind) = if name == "{" {
                ('{', AtomKind::Open)
            } else {
                ('}', AtomKind::Close)
            };
            Ok(Classified::Node(MathNode::Atom(ch, kind)))
        }
        "|" => Ok(Classified::Node(MathNode::Symbol("Vert".into()))),
        "backslash" => Ok(Classified::Node(MathNode::Symbol("backslash".into()))),
        _ => {
            if name.starts_with("math")
                && name.len() > 4
                && name.chars().all(|c| c.is_ascii_alphabetic())
            {
                return Err(ParseError::Unsupported(format!("font style {name}")));
            }
            if name.starts_with("wide") {
                return Err(ParseError::Unsupported(format!("accent {name}")));
            }
            Ok(Classified::Node(parser.parse_symbol_or_unknown(name)?))
        }
    }
}

fn accent(kind: AccentKind) -> Result<Classified, ParseError> {
    Ok(Classified::Cmd(Cmd::Accent {
        kind,
        phase: One::Need,
    }))
}

fn font(style: TextStyle) -> Result<Classified, ParseError> {
    Ok(Classified::Cmd(Cmd::Font {
        style,
        phase: One::Need,
    }))
}

fn phantom(kind: PhantomKind) -> Result<Classified, ParseError> {
    Ok(Classified::Cmd(Cmd::Phantom {
        kind,
        phase: One::Need,
    }))
}

fn start_xarrow(parser: &mut super::Parser, arrow: &str) -> Result<Classified, ParseError> {
    let phase = if matches!(parser.peek_ws(), Some(Token::Char('['))) {
        parser.bump();
        OptSeq::Bracket
    } else {
        OptSeq::Body
    };
    Ok(Classified::Cmd(Cmd::Xarrow {
        arrow: arrow.to_string(),
        under: None,
        phase,
    }))
}

fn start_begin(parser: &mut super::Parser) -> Result<Classified, ParseError> {
    let name = parser.collect_group_text()?;
    let colspec = if name == "array" {
        let preamble = parser.collect_group_text()?;
        parse_colspec(&preamble)?
    } else {
        Vec::new()
    };
    let style = match name.as_str() {
        "matrix" => MatrixStyle::Matrix,
        "pmatrix" => MatrixStyle::Pmatrix,
        "bmatrix" => MatrixStyle::Bmatrix,
        "vmatrix" => MatrixStyle::Vmatrix,
        "Vmatrix" => MatrixStyle::VVmatrix,
        "Bmatrix" => MatrixStyle::BBmatrix,
        "cases" => MatrixStyle::Cases,
        "array" => MatrixStyle::Array,
        "aligned" => MatrixStyle::Aligned,
        "align" => MatrixStyle::Align,
        "gather" => MatrixStyle::Gather,
        "multline" => MatrixStyle::Multline,
        "equation" => MatrixStyle::Equation,
        "split" => MatrixStyle::Split,
        other => {
            return Err(ParseError::Unsupported(format!("environment {other}")));
        }
    };
    if matches!(style, MatrixStyle::Cases) {
        parser.cases_depth = parser.cases_depth.saturating_add(1);
    }
    Ok(Classified::Cmd(Cmd::Begin(Box::new(BeginState {
        name,
        style,
        colspec,
        rows: Vec::new(),
        cells: Vec::new(),
        number: EqNumber::Default,
        labels: Vec::new(),
        phase: BeginPhase::Row,
    }))))
}

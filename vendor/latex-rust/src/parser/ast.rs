//! Typed math AST produced by the parser.

use crate::color::Color;
use crate::dim::Dim;
use crate::parser::group_rules;
use crate::parser::parse::lexicon::is_superscript_char;
use crate::parser::parse::{option_ends_early, paren_script_match, ParseOptions, Stop};
use crate::parser::token::{tokenize, Token};

/// TeX math atom class (Appendix G).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomKind {
    /// Ordinary (`x`, `1`, `\alpha` as a letter-like glyph).
    Ord,
    /// Large operator class.
    Op,
    /// Binary operator (`+`, `\times`).
    Bin,
    /// Relation (`=`, `\leq`).
    Rel,
    /// Opening delimiter.
    Open,
    /// Closing delimiter.
    Close,
    /// Punctuation (`,`).
    Punct,
    /// Inner (fraction-like).
    Inner,
}

impl AtomKind {
    fn gold(self) -> &'static str {
        match self {
            Self::Ord => "Ord",
            Self::Op => "Op",
            Self::Bin => "Bin",
            Self::Rel => "Rel",
            Self::Open => "Open",
            Self::Close => "Close",
            Self::Punct => "Punct",
            Self::Inner => "Inner",
        }
    }
}

/// Accent or decoration applied to a nucleus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccentKind {
    /// `\hat`
    Hat,
    /// `\check`
    Check,
    /// `\breve`
    Breve,
    /// `\acute`
    Acute,
    /// `\grave`
    Grave,
    /// `\tilde`
    Tilde,
    /// `\bar`
    Bar,
    /// `\vec`
    Vec,
    /// `\dot`
    Dot,
    /// `\ddot`
    Ddot,
    /// `\dddot`
    Dddot,
    /// `\ddddot`
    Ddddot,
    /// `\widehat`
    WideHat,
    /// `\widetilde`
    WideTilde,
    /// `\overline`
    Overline,
    /// `\underline`
    Underline,
    /// `\overbrace`
    Overbrace,
    /// `\underbrace`
    Underbrace,
    /// `\overleftarrow`
    Overleftarrow,
    /// `\overrightarrow`
    Overrightarrow,
    /// `\overleftrightarrow`
    Overleftrightarrow,
    /// `\underleftarrow`
    Underleftarrow,
    /// `\underrightarrow`
    Underrightarrow,
    /// `\underleftrightarrow`
    Underleftrightarrow,
    /// `\cancel`
    Cancel,
    /// `\bcancel`
    BCancel,
    /// `\xcancel`
    XCancel,
    /// `\boxed`
    Boxed,
    /// `\mathring`
    Ring,
    /// `\not`
    Not,
}

impl AccentKind {
    pub(crate) fn gold(self) -> &'static str {
        match self {
            Self::Hat => "hat",
            Self::Check => "check",
            Self::Breve => "breve",
            Self::Acute => "acute",
            Self::Grave => "grave",
            Self::Tilde => "tilde",
            Self::Bar => "bar",
            Self::Vec => "vec",
            Self::Dot => "dot",
            Self::Ddot => "ddot",
            Self::Dddot => "dddot",
            Self::Ddddot => "ddddot",
            Self::WideHat => "widehat",
            Self::WideTilde => "widetilde",
            Self::Overline => "overline",
            Self::Underline => "underline",
            Self::Overbrace => "overbrace",
            Self::Underbrace => "underbrace",
            Self::Overleftarrow => "overleftarrow",
            Self::Overrightarrow => "overrightarrow",
            Self::Overleftrightarrow => "overleftrightarrow",
            Self::Underleftarrow => "underleftarrow",
            Self::Underrightarrow => "underrightarrow",
            Self::Underleftrightarrow => "underleftrightarrow",
            Self::Cancel => "cancel",
            Self::BCancel => "bcancel",
            Self::XCancel => "xcancel",
            Self::Boxed => "boxed",
            Self::Ring => "mathring",
            Self::Not => "not",
        }
    }
}

/// Font / text style for a run of characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// `\mathrm`
    Rm,
    /// `\mathbf`
    Bf,
    /// `\mathit`
    It,
    /// `\mathsf`
    Sf,
    /// `\mathtt`
    Tt,
    /// `\mathbb`
    Bb,
    /// `\mathcal`
    Cal,
    /// `\mathfrak`
    Frak,
    /// `\mathscr`
    Scr,
    /// `\boldsymbol`
    Boldsymbol,
    /// `\pmb`
    Pmb,
    /// `\text`
    Text,
}

impl TextStyle {
    fn gold(self) -> &'static str {
        match self {
            Self::Rm => "rm",
            Self::Bf => "bf",
            Self::It => "it",
            Self::Sf => "sf",
            Self::Tt => "tt",
            Self::Bb => "bb",
            Self::Cal => "cal",
            Self::Frak => "frak",
            Self::Scr => "scr",
            Self::Boldsymbol => "boldsymbol",
            Self::Pmb => "pmb",
            Self::Text => "text",
        }
    }
}

/// One item of a text-mode run (`\text{…}` body).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextItem {
    /// A maximal run of plain characters (no whitespace).
    Str(String),
    /// Collapsed inter-word space.
    Space,
    /// A `\ ` control space.
    ControlSpace,
    /// A `~` no-break space.
    NoBreakSpace,
    /// A text symbol such as `\ldots`, `\quad`, `\qquad`.
    Symbol(&'static str),
    /// A nested text command with its own style.
    Nested(TextStyle, TextRun),
    /// Math embedded in text with `\(…\)` or `$…$`.
    Math(Box<MathNode>),
}

/// Body of a text-mode command. Adjacent `Str` items are merged, no `Str` is
/// empty, and no `Space` follows a `Space`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRun(Vec<TextItem>);

impl TextRun {
    /// Creates a new `TextRun` from a vector of items, enforcing invariants.
    #[must_use]
    pub fn new(items: Vec<TextItem>) -> Self {
        let mut run = Self::default();
        for item in items {
            run.push(item);
        }
        run
    }

    /// Creates a `TextRun` from a plain string, converting spaces to `Space` items.
    #[must_use]
    pub fn plain(s: &str) -> Self {
        let mut run = Self::default();
        for c in s.chars() {
            if c == ' ' {
                run.push(TextItem::Space);
            } else {
                run.push_char(c);
            }
        }
        run
    }

    /// Appends an item to the run, maintaining invariants.
    pub fn push(&mut self, item: TextItem) {
        match (self.0.last_mut(), item) {
            (_, TextItem::Str(s)) if s.is_empty() => {}
            (Some(TextItem::Str(prev)), TextItem::Str(s)) => prev.push_str(&s),
            (Some(TextItem::Space), TextItem::Space) => {}
            (_, item) => self.0.push(item),
        }
    }

    /// Appends a character to the last `Str` item or creates a new one.
    pub fn push_char(&mut self, c: char) {
        if let Some(TextItem::Str(s)) = self.0.last_mut() {
            s.push(c);
        } else {
            self.0.push(TextItem::Str(c.to_string()));
        }
    }

    /// Appends all items from another `TextRun`.
    pub fn append(&mut self, other: TextRun) {
        for item in other.0 {
            self.push(item);
        }
    }

    /// Returns a slice of the items in this run.
    #[must_use]
    pub fn items(&self) -> &[TextItem] {
        &self.0
    }

    /// Checks if the run is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns `Some` only when the run contains exactly one `Str` item.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self.0.as_slice() {
            [TextItem::Str(s)] => Some(s),
            _ => None,
        }
    }

    /// Converts the run to a plain string representation.
    #[must_use]
    pub fn to_plain_string(&self) -> String {
        let mut result = String::new();
        for item in &self.0 {
            match item {
                TextItem::Str(s) => result.push_str(s),
                TextItem::Space | TextItem::ControlSpace | TextItem::NoBreakSpace => {
                    result.push(' ');
                }
                TextItem::Symbol(name) => {
                    if *name == "ldots" {
                        result.push('\u{2026}');
                    } else {
                        result.push(' ');
                    }
                }
                TextItem::Nested(_, run) => result.push_str(&run.to_plain_string()),
                TextItem::Math(_) => {}
            }
        }
        result
    }
}

/// Horizontal skip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpaceKind {
    /// `\,`
    Thin,
    /// `\:` or `\>`
    Medium,
    /// `\;`
    Thick,
    /// `\!`
    NegThin,
    /// `\quad`
    Quad,
    /// `\qquad`
    Qquad,
    /// `\ ` (control space)
    ControlSpace,
    /// `\hspace{...}` in em.
    Hspace(Dim),
}

/// Matrix / alignment environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixStyle {
    /// `{matrix}`
    Matrix,
    /// `{pmatrix}`
    Pmatrix,
    /// `{bmatrix}`
    Bmatrix,
    /// `{vmatrix}`
    Vmatrix,
    /// `{Vmatrix}`
    VVmatrix,
    /// `{Bmatrix}`
    BBmatrix,
    /// `{cases}`
    Cases,
    /// `{array}`
    Array,
    /// `{aligned}`
    Aligned,
    /// `{align}`
    Align,
    /// `{gather}`
    Gather,
    /// `{multline}`
    Multline,
    /// `{equation}`
    Equation,
    /// `{split}`
    Split,
}

impl MatrixStyle {
    fn gold(self) -> &'static str {
        match self {
            Self::Matrix => "matrix",
            Self::Pmatrix => "pmatrix",
            Self::Bmatrix => "bmatrix",
            Self::Vmatrix => "vmatrix",
            Self::VVmatrix => "Vmatrix",
            Self::BBmatrix => "Bmatrix",
            Self::Cases => "cases",
            Self::Array => "array",
            Self::Aligned => "aligned",
            Self::Align => "align",
            Self::Gather => "gather",
            Self::Multline => "multline",
            Self::Equation => "equation",
            Self::Split => "split",
        }
    }

    /// `align` / `gather` / `multline` / `equation` take display style and numbers.
    #[must_use]
    pub fn is_display_env(self) -> bool {
        matches!(
            self,
            Self::Align | Self::Gather | Self::Multline | Self::Equation
        )
    }

    /// Rows are numbered unless `\nonumber` / `\notag` / `\tag` says otherwise.
    #[must_use]
    pub fn numbers_rows(self) -> bool {
        matches!(self, Self::Align | Self::Gather)
    }

    /// One equation number for the whole environment (last line for `multline`).
    #[must_use]
    pub fn numbers_once(self) -> bool {
        matches!(self, Self::Equation | Self::Multline)
    }
}

/// One column of an `{array}` preamble (`l`, `c`, `r`, `|`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColSpec {
    /// `l`
    Left,
    /// `c`
    Center,
    /// `r`
    Right,
    /// `|`
    VRule,
}

impl ColSpec {
    fn gold(self) -> char {
        match self {
            Self::Left => 'l',
            Self::Center => 'c',
            Self::Right => 'r',
            Self::VRule => '|',
        }
    }

    /// True for `|`.
    #[must_use]
    pub fn is_rule(self) -> bool {
        matches!(self, Self::VRule)
    }
}

/// Per-row equation number in numbered environments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EqNumber {
    /// Auto number when the environment numbers rows; none otherwise.
    Default,
    /// `\nonumber` / `\notag`
    Suppress,
    /// `\tag{...}` or `\tag*{...}`
    Tag {
        /// `\tag*` — no parentheses around the tag.
        star: bool,
        /// Tag body (text style at layout).
        body: Box<MathNode>,
    },
}

/// One row of a matrix / alignment environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvRow {
    /// Alignment cells (`&`-separated).
    Cells {
        /// Column entries.
        cells: Vec<MathNode>,
        /// Number / tag for this row.
        number: EqNumber,
        /// `\label{...}` keys bound to this row's number.
        labels: Vec<String>,
    },
    /// `\hline`
    Hline,
    /// `\intertext{...}`
    Intertext(Box<MathNode>),
}

impl EnvRow {
    /// A data row with default numbering and no labels.
    #[must_use]
    pub fn cells(cells: Vec<MathNode>) -> Self {
        Self::Cells {
            cells,
            number: EqNumber::Default,
            labels: Vec::new(),
        }
    }
}

/// Which integral glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegralKind {
    /// `\int`
    Int,
    /// `\iint`
    Iint,
    /// `\iiint`
    Iiint,
    /// `\oint`
    Oint,
    /// `\oiint`
    Oiint,
}

impl IntegralKind {
    fn gold(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Iint => "iint",
            Self::Iiint => "iiint",
            Self::Oint => "oint",
            Self::Oiint => "oiint",
        }
    }
}

/// `\phantom` / `\vphantom` / `\hphantom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhantomKind {
    /// `\phantom`
    Full,
    /// `\vphantom`
    Vertical,
    /// `\hphantom`
    Horizontal,
}

impl PhantomKind {
    fn gold(self) -> &'static str {
        match self {
            Self::Full => "phantom",
            Self::Vertical => "vphantom",
            Self::Horizontal => "hphantom",
        }
    }
}

/// A `\left` / `\right` delimiter (or `.` for empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delimiter {
    /// `\left.` / `\right.`
    Empty,
    /// Literal character (`(`, `[`, `|`).
    Char(char),
    /// Named delimiter (`langle`, `{` from `\{`).
    Named(String),
}

impl Delimiter {
    fn gold(&self) -> String {
        match self {
            Self::Empty => ".".into(),
            Self::Char(c) => c.to_string(),
            Self::Named(n) => {
                if n == "{" || n == "}" || n == "|" {
                    format!("\\{n}")
                } else {
                    n.clone()
                }
            }
        }
    }
}

/// `\big` / `\Big` / `\bigg` / `\Bigg` (and `l`/`r`/`m` siblings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelimSize {
    /// `\big` — about 1.2 em.
    Big,
    /// `\Big` — about 1.8 em.
    Big2,
    /// `\bigg` — about 2.4 em.
    Bigg,
    /// `\Bigg` — about 3.0 em.
    Bigg2,
}

impl DelimSize {
    fn gold(self) -> &'static str {
        match self {
            Self::Big => "big",
            Self::Big2 => "Big",
            Self::Bigg => "bigg",
            Self::Bigg2 => "Bigg",
        }
    }

    /// Map a control sequence to a size. `None` if it is not a `\big` family command.
    #[must_use]
    pub fn from_command(name: &str) -> Option<Self> {
        match name {
            "big" | "bigl" | "bigr" | "bigm" => Some(Self::Big),
            "Big" | "Bigl" | "Bigr" | "Bigm" => Some(Self::Big2),
            "bigg" | "biggl" | "biggr" | "biggm" => Some(Self::Bigg),
            "Bigg" | "Biggl" | "Biggr" | "Biggm" => Some(Self::Bigg2),
            _ => None,
        }
    }

    /// Open / Close / Rel from the `l` / `r` / `m` suffix; `None` for unsuffixed `\big`.
    #[must_use]
    pub fn class_from_command(name: &str) -> Option<AtomKind> {
        if name.ends_with('l') {
            Some(AtomKind::Open)
        } else if name.ends_with('r') {
            Some(AtomKind::Close)
        } else if name.ends_with('m') {
            Some(AtomKind::Rel)
        } else {
            None
        }
    }
}

/// Which command built a [`MathNode::Fraction`].
///
/// `\frac` and `{a \over b}` are [`FracStyle::Frac`]. `\dfrac`, `\tfrac`, and
/// `\cfrac` keep their own style, which layout uses and which [`MathNode::gold`]
/// and [`MathNode::to_latex`] print back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FracStyle {
    /// `\frac` and infix `\over`.
    Frac,
    /// `\dfrac`. Layout uses display style.
    Dfrac,
    /// `\tfrac`. Layout uses text style.
    Tfrac,
    /// `\cfrac`. Layout uses display style.
    Cfrac,
}

impl FracStyle {
    pub(crate) fn gold(self) -> &'static str {
        match self {
            Self::Frac => "frac",
            Self::Dfrac => "dfrac",
            Self::Tfrac => "tfrac",
            Self::Cfrac => "cfrac",
        }
    }
}

/// Typed math-mode syntax tree.
///
/// Produced by [`crate::parse()`]. [`MathNode::gold`] is the gold-stable debug form.
/// The enum is non-exhaustive so a later release can add a variant. A
/// downstream match needs a wildcard.
///
/// # Examples
///
/// ```
/// use latex_rust::parse;
///
/// let n = parse("x^2").unwrap();
/// assert!(n.gold().contains("sup"));
/// ```
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MathNode {
    /// Single character with a TeX atom class.
    Atom(char, AtomKind),
    /// `\frac` / `\dfrac` / `\tfrac` / `\cfrac` / `{a \over b}`.
    ///
    /// The style is the command. `\over` is [`FracStyle::Frac`].
    Fraction(FracStyle, Box<MathNode>, Box<MathNode>),
    /// `\sqrt` / `\sqrt[n]`. Index `None` is a square root.
    Radical(Option<Box<MathNode>>, Box<MathNode>),
    /// `x^{}`
    Superscript(Box<MathNode>, Box<MathNode>),
    /// `x_{}`
    Subscript(Box<MathNode>, Box<MathNode>),
    /// `x_{}^{}`
    SubSup(Box<MathNode>, Box<MathNode>, Box<MathNode>),
    /// `\left ... \right`
    Delimited(Delimiter, Box<MathNode>, Delimiter),
    /// `\big` / `\Big` / `\bigg` / `\Bigg` (and `l`/`r`/`m` forms).
    SizedDelim(Delimiter, DelimSize, AtomKind),
    /// Horizontal list of nodes.
    Row(Vec<MathNode>),
    /// Matrix or multiline environment. `colspec` is the `{array}` preamble (empty otherwise).
    Matrix(MatrixStyle, Vec<ColSpec>, Vec<EnvRow>),
    /// `\substack{...}` — stacked script-style lines.
    Substack(Vec<MathNode>),
    /// `\ref{key}`
    Ref(String),
    /// `\tag{...}` / `\tag*{...}` (peeled into [`EnvRow`] when inside an environment).
    Tag {
        /// `\tag*`
        star: bool,
        /// Tag body.
        body: Box<MathNode>,
    },
    /// `\label{key}`
    Label(String),
    /// `\nonumber` / `\notag`
    NoNumber,
    /// `\hline` (peeled into [`EnvRow::Hline`] in environments).
    Hline,
    /// `\intertext{...}`
    Intertext(Box<MathNode>),
    /// `\sum` with optional lower / upper limits.
    Sum(Option<Box<MathNode>>, Option<Box<MathNode>>),
    /// `\int` family with optional limits.
    Integral(IntegralKind, Option<Box<MathNode>>, Option<Box<MathNode>>),
    /// `\prod` with optional limits.
    Product(Option<Box<MathNode>>, Option<Box<MathNode>>),
    /// `\lim` with optional subscript.
    Limit(Option<Box<MathNode>>),
    /// `\overset` / `\underset` / `\stackrel` (base, over, under).
    OverUnder(Box<MathNode>, Option<Box<MathNode>>, Option<Box<MathNode>>),
    /// Accent or decoration on a nucleus.
    Accent(Box<MathNode>, AccentKind),
    /// `\cancelto{value}{expr}`
    CancelTo(Box<MathNode>, Box<MathNode>),
    /// Styled character run (`\mathrm`, `\text`, …).
    Text(TextRun, TextStyle),
    /// Explicit math skip.
    Space(SpaceKind),
    /// Named operator (`\sin`). The flag is `\limits` vs `\nolimits`.
    Operator(String, bool),
    /// Named glyph (`\alpha`, `\times`). The string is the control-sequence name.
    Symbol(String),
    /// `\color` applied to a following math list.
    Color(Color, Box<MathNode>),
    /// `\textcolor{...}{...}`
    TextColor(Color, Box<MathNode>),
    /// `\colorbox{...}{...}`
    ColorBox(Color, Box<MathNode>),
    /// `\fcolorbox{border}{fill}{body}`
    FColorBox(Color, Color, Box<MathNode>),
    /// Vertical strut (height, depth) in em.
    Strut(Dim, Dim),
    /// `\phantom` family.
    Phantom(PhantomKind, Box<MathNode>),
}

impl Clone for MathNode {
    fn clone(&self) -> Self {
        clone_node(self)
    }
}

impl Drop for MathNode {
    fn drop(&mut self) {
        // Box drop walks children on the native stack. A 10,000-deep `\left`
        // or `\frac` is a valid tree under a raised `max_depth` and must drop
        // on a 256 KiB thread, so the children move to a heap stack first.
        let mut stack = Vec::new();
        self.drain_children(&mut stack);
        while let Some(mut child) = stack.pop() {
            child.drain_children(&mut stack);
        }
    }
}

fn push_box(out: &mut Vec<MathNode>, slot: &mut Box<MathNode>) {
    out.push(std::mem::replace(slot.as_mut(), MathNode::Hline));
}

fn push_opt(out: &mut Vec<MathNode>, slot: &mut Option<Box<MathNode>>) {
    if let Some(child) = slot.take() {
        out.push(*child);
    }
}

impl MathNode {
    fn drain_children(&mut self, out: &mut Vec<MathNode>) {
        match self {
            Self::Fraction(_, a, b) | Self::Superscript(a, b) | Self::Subscript(a, b) => {
                push_box(out, a);
                push_box(out, b);
            }
            Self::CancelTo(a, b) => {
                push_box(out, a);
                push_box(out, b);
            }
            Self::SubSup(a, b, c) => {
                push_box(out, a);
                push_box(out, b);
                push_box(out, c);
            }
            Self::Radical(index, body) => {
                push_opt(out, index);
                push_box(out, body);
            }
            Self::Delimited(_, body, _) | Self::Intertext(body) => push_box(out, body),
            Self::Row(items) | Self::Substack(items) => out.append(items),
            Self::Matrix(_, _, rows) => {
                for row in rows {
                    row.drain_nodes(out);
                }
            }
            Self::Tag { body, .. } => push_box(out, body),
            Self::Sum(lower, upper) | Self::Product(lower, upper) => {
                push_opt(out, lower);
                push_opt(out, upper);
            }
            Self::Integral(_, lower, upper) => {
                push_opt(out, lower);
                push_opt(out, upper);
            }
            Self::Limit(lower) => push_opt(out, lower),
            Self::OverUnder(base, over, under) => {
                push_box(out, base);
                push_opt(out, over);
                push_opt(out, under);
            }
            Self::Accent(body, _)
            | Self::Color(_, body)
            | Self::TextColor(_, body)
            | Self::ColorBox(_, body)
            | Self::Phantom(_, body) => push_box(out, body),
            Self::FColorBox(_, _, body) => push_box(out, body),
            Self::Atom(_, _)
            | Self::SizedDelim(_, _, _)
            | Self::Ref(_)
            | Self::Label(_)
            | Self::NoNumber
            | Self::Hline
            | Self::Text(_, _)
            | Self::Space(_)
            | Self::Operator(_, _)
            | Self::Symbol(_)
            | Self::Strut(_, _) => {}
        }
    }
}

impl EnvRow {
    fn drain_nodes(&mut self, out: &mut Vec<MathNode>) {
        match self {
            Self::Cells { cells, number, .. } => {
                out.append(cells);
                number.drain_node(out);
            }
            Self::Hline => {}
            Self::Intertext(body) => push_box(out, body),
        }
    }
}

impl EqNumber {
    fn drain_node(&mut self, out: &mut Vec<MathNode>) {
        if let Self::Tag { body, .. } = self {
            push_box(out, body);
        }
    }
}

enum CloneTask<'a> {
    Node(&'a MathNode),
    Finish(Finish),
}

enum Finish {
    Pair(PairKind),
    Radical {
        has_index: bool,
    },
    SubSup,
    Delimited(Delimiter, Delimiter),
    List(ListKind, usize),
    Matrix {
        style: MatrixStyle,
        cols: Vec<ColSpec>,
        rows: Vec<RowPlan>,
    },
    Tag {
        star: bool,
    },
    Limits {
        kind: LimitKind,
        has_lo: bool,
        has_hi: bool,
    },
    Limit {
        has_lo: bool,
    },
    OverUnder {
        has_over: bool,
        has_under: bool,
    },
    Accent(AccentKind),
    Wrap(WrapKind),
    Intertext,
}

enum PairKind {
    Fraction(FracStyle),
    Superscript,
    Subscript,
    CancelTo,
}

enum ListKind {
    Row,
    Substack,
}

enum LimitKind {
    Sum,
    Product,
    Integral(IntegralKind),
}

enum WrapKind {
    Color(Color),
    TextColor(Color),
    ColorBox(Color),
    FColorBox(Color, Color),
    Phantom(PhantomKind),
}

enum RowPlan {
    Hline,
    Intertext,
    Cells {
        n: usize,
        number: NumPlan,
        labels: Vec<String>,
    },
}

enum NumPlan {
    Default,
    Suppress,
    Tag { star: bool },
}

fn clone_node(root: &MathNode) -> MathNode {
    let mut tasks = vec![CloneTask::Node(root)];
    let mut done = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            CloneTask::Node(node) => schedule_clone(node, &mut tasks, &mut done),
            CloneTask::Finish(finish) => finish_clone(finish, &mut done),
        }
    }
    done.pop().unwrap_or(MathNode::Hline)
}

fn pop_node(done: &mut Vec<MathNode>) -> MathNode {
    done.pop().unwrap_or(MathNode::Hline)
}

fn schedule_clone<'a>(
    node: &'a MathNode,
    tasks: &mut Vec<CloneTask<'a>>,
    done: &mut Vec<MathNode>,
) {
    match node {
        MathNode::Atom(ch, kind) => done.push(MathNode::Atom(*ch, *kind)),
        MathNode::SizedDelim(delim, size, kind) => {
            done.push(MathNode::SizedDelim(delim.clone(), *size, *kind));
        }
        MathNode::Ref(key) => done.push(MathNode::Ref(key.clone())),
        MathNode::Label(key) => done.push(MathNode::Label(key.clone())),
        MathNode::NoNumber => done.push(MathNode::NoNumber),
        MathNode::Hline => done.push(MathNode::Hline),
        MathNode::Text(text, style) => done.push(MathNode::Text(text.clone(), *style)),
        MathNode::Space(kind) => done.push(MathNode::Space(kind.clone())),
        MathNode::Operator(name, limits) => {
            done.push(MathNode::Operator(name.clone(), *limits));
        }
        MathNode::Symbol(name) => done.push(MathNode::Symbol(name.clone())),
        MathNode::Strut(height, depth) => {
            done.push(MathNode::Strut(height.clone(), depth.clone()));
        }
        MathNode::Fraction(style, a, b) => {
            tasks.push(CloneTask::Finish(Finish::Pair(PairKind::Fraction(*style))));
            tasks.push(CloneTask::Node(b));
            tasks.push(CloneTask::Node(a));
        }
        MathNode::Superscript(a, b) => {
            tasks.push(CloneTask::Finish(Finish::Pair(PairKind::Superscript)));
            tasks.push(CloneTask::Node(b));
            tasks.push(CloneTask::Node(a));
        }
        MathNode::Subscript(a, b) => {
            tasks.push(CloneTask::Finish(Finish::Pair(PairKind::Subscript)));
            tasks.push(CloneTask::Node(b));
            tasks.push(CloneTask::Node(a));
        }
        MathNode::CancelTo(a, b) => {
            tasks.push(CloneTask::Finish(Finish::Pair(PairKind::CancelTo)));
            tasks.push(CloneTask::Node(b));
            tasks.push(CloneTask::Node(a));
        }
        MathNode::SubSup(a, b, c) => {
            tasks.push(CloneTask::Finish(Finish::SubSup));
            tasks.push(CloneTask::Node(c));
            tasks.push(CloneTask::Node(b));
            tasks.push(CloneTask::Node(a));
        }
        MathNode::Radical(index, body) => {
            tasks.push(CloneTask::Finish(Finish::Radical {
                has_index: index.is_some(),
            }));
            tasks.push(CloneTask::Node(body));
            if let Some(index) = index {
                tasks.push(CloneTask::Node(index));
            }
        }
        MathNode::Delimited(left, body, right) => {
            tasks.push(CloneTask::Finish(Finish::Delimited(
                left.clone(),
                right.clone(),
            )));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::Row(items) => {
            tasks.push(CloneTask::Finish(Finish::List(ListKind::Row, items.len())));
            for item in items.iter().rev() {
                tasks.push(CloneTask::Node(item));
            }
        }
        MathNode::Substack(items) => {
            tasks.push(CloneTask::Finish(Finish::List(
                ListKind::Substack,
                items.len(),
            )));
            for item in items.iter().rev() {
                tasks.push(CloneTask::Node(item));
            }
        }
        MathNode::Matrix(style, cols, rows) => schedule_matrix(*style, cols, rows, tasks),
        MathNode::Tag { star, body } => {
            tasks.push(CloneTask::Finish(Finish::Tag { star: *star }));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::Sum(lower, upper) => {
            schedule_limits(LimitKind::Sum, lower, upper, tasks);
        }
        MathNode::Product(lower, upper) => {
            schedule_limits(LimitKind::Product, lower, upper, tasks);
        }
        MathNode::Integral(kind, lower, upper) => {
            schedule_limits(LimitKind::Integral(*kind), lower, upper, tasks);
        }
        MathNode::Limit(lower) => {
            tasks.push(CloneTask::Finish(Finish::Limit {
                has_lo: lower.is_some(),
            }));
            if let Some(lower) = lower {
                tasks.push(CloneTask::Node(lower));
            }
        }
        MathNode::OverUnder(base, over, under) => {
            tasks.push(CloneTask::Finish(Finish::OverUnder {
                has_over: over.is_some(),
                has_under: under.is_some(),
            }));
            if let Some(under) = under {
                tasks.push(CloneTask::Node(under));
            }
            if let Some(over) = over {
                tasks.push(CloneTask::Node(over));
            }
            tasks.push(CloneTask::Node(base));
        }
        MathNode::Accent(body, kind) => {
            tasks.push(CloneTask::Finish(Finish::Accent(*kind)));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::Color(color, body) => {
            tasks.push(CloneTask::Finish(Finish::Wrap(WrapKind::Color(*color))));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::TextColor(color, body) => {
            tasks.push(CloneTask::Finish(Finish::Wrap(WrapKind::TextColor(*color))));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::ColorBox(color, body) => {
            tasks.push(CloneTask::Finish(Finish::Wrap(WrapKind::ColorBox(*color))));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::FColorBox(border, fill, body) => {
            tasks.push(CloneTask::Finish(Finish::Wrap(WrapKind::FColorBox(
                *border, *fill,
            ))));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::Phantom(kind, body) => {
            tasks.push(CloneTask::Finish(Finish::Wrap(WrapKind::Phantom(*kind))));
            tasks.push(CloneTask::Node(body));
        }
        MathNode::Intertext(body) => {
            tasks.push(CloneTask::Finish(Finish::Intertext));
            tasks.push(CloneTask::Node(body));
        }
    }
}

fn schedule_limits<'a>(
    kind: LimitKind,
    lower: &'a Option<Box<MathNode>>,
    upper: &'a Option<Box<MathNode>>,
    tasks: &mut Vec<CloneTask<'a>>,
) {
    tasks.push(CloneTask::Finish(Finish::Limits {
        kind,
        has_lo: lower.is_some(),
        has_hi: upper.is_some(),
    }));
    if let Some(upper) = upper {
        tasks.push(CloneTask::Node(upper));
    }
    if let Some(lower) = lower {
        tasks.push(CloneTask::Node(lower));
    }
}

fn schedule_matrix<'a>(
    style: MatrixStyle,
    cols: &[ColSpec],
    rows: &'a [EnvRow],
    tasks: &mut Vec<CloneTask<'a>>,
) {
    let mut plan = Vec::with_capacity(rows.len());
    let mut kids = Vec::new();
    for row in rows {
        match row {
            EnvRow::Hline => plan.push(RowPlan::Hline),
            EnvRow::Intertext(body) => {
                plan.push(RowPlan::Intertext);
                kids.push(body.as_ref());
            }
            EnvRow::Cells {
                cells,
                number,
                labels,
            } => {
                let num = match number {
                    EqNumber::Default => NumPlan::Default,
                    EqNumber::Suppress => NumPlan::Suppress,
                    EqNumber::Tag { star, .. } => NumPlan::Tag { star: *star },
                };
                plan.push(RowPlan::Cells {
                    n: cells.len(),
                    number: num,
                    labels: labels.clone(),
                });
                for cell in cells {
                    kids.push(cell);
                }
                if let EqNumber::Tag { body, .. } = number {
                    kids.push(body);
                }
            }
        }
    }
    tasks.push(CloneTask::Finish(Finish::Matrix {
        style,
        cols: cols.to_vec(),
        rows: plan,
    }));
    for kid in kids.into_iter().rev() {
        tasks.push(CloneTask::Node(kid));
    }
}

fn finish_clone(finish: Finish, done: &mut Vec<MathNode>) {
    match finish {
        Finish::Pair(kind) => {
            let b = Box::new(pop_node(done));
            let a = Box::new(pop_node(done));
            done.push(match kind {
                PairKind::Fraction(style) => MathNode::Fraction(style, a, b),
                PairKind::Superscript => MathNode::Superscript(a, b),
                PairKind::Subscript => MathNode::Subscript(a, b),
                PairKind::CancelTo => MathNode::CancelTo(a, b),
            });
        }
        Finish::SubSup => {
            let c = Box::new(pop_node(done));
            let b = Box::new(pop_node(done));
            let a = Box::new(pop_node(done));
            done.push(MathNode::SubSup(a, b, c));
        }
        Finish::Radical { has_index } => {
            let body = Box::new(pop_node(done));
            let index = if has_index {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            done.push(MathNode::Radical(index, body));
        }
        Finish::Delimited(left, right) => {
            let body = Box::new(pop_node(done));
            done.push(MathNode::Delimited(left, body, right));
        }
        Finish::List(kind, n) => {
            let start = done.len().saturating_sub(n);
            let items = done.split_off(start);
            done.push(match kind {
                ListKind::Row => MathNode::Row(items),
                ListKind::Substack => MathNode::Substack(items),
            });
        }
        Finish::Tag { star } => {
            let body = Box::new(pop_node(done));
            done.push(MathNode::Tag { star, body });
        }
        Finish::Limits {
            kind,
            has_lo,
            has_hi,
        } => {
            let upper = if has_hi {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            let lower = if has_lo {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            done.push(match kind {
                LimitKind::Sum => MathNode::Sum(lower, upper),
                LimitKind::Product => MathNode::Product(lower, upper),
                LimitKind::Integral(integral) => MathNode::Integral(integral, lower, upper),
            });
        }
        Finish::Limit { has_lo } => {
            let lower = if has_lo {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            done.push(MathNode::Limit(lower));
        }
        Finish::OverUnder {
            has_over,
            has_under,
        } => {
            let under = if has_under {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            let over = if has_over {
                Some(Box::new(pop_node(done)))
            } else {
                None
            };
            let base = Box::new(pop_node(done));
            done.push(MathNode::OverUnder(base, over, under));
        }
        Finish::Accent(kind) => {
            let body = Box::new(pop_node(done));
            done.push(MathNode::Accent(body, kind));
        }
        Finish::Wrap(kind) => {
            let body = Box::new(pop_node(done));
            done.push(match kind {
                WrapKind::Color(color) => MathNode::Color(color, body),
                WrapKind::TextColor(color) => MathNode::TextColor(color, body),
                WrapKind::ColorBox(color) => MathNode::ColorBox(color, body),
                WrapKind::FColorBox(border, fill) => MathNode::FColorBox(border, fill, body),
                WrapKind::Phantom(phantom) => MathNode::Phantom(phantom, body),
            });
        }
        Finish::Intertext => {
            let body = Box::new(pop_node(done));
            done.push(MathNode::Intertext(body));
        }
        Finish::Matrix { style, cols, rows } => {
            let n = matrix_child_count(&rows);
            let start = done.len().saturating_sub(n);
            let kids = done.split_off(start);
            let mut kids = kids.into_iter();
            let mut built = Vec::with_capacity(rows.len());
            for row in rows {
                built.push(match row {
                    RowPlan::Hline => EnvRow::Hline,
                    RowPlan::Intertext => {
                        EnvRow::Intertext(Box::new(kids.next().unwrap_or(MathNode::Hline)))
                    }
                    RowPlan::Cells { n, number, labels } => {
                        let mut cells = Vec::with_capacity(n);
                        for _ in 0..n {
                            cells.push(kids.next().unwrap_or(MathNode::Hline));
                        }
                        let number = match number {
                            NumPlan::Default => EqNumber::Default,
                            NumPlan::Suppress => EqNumber::Suppress,
                            NumPlan::Tag { star } => EqNumber::Tag {
                                star,
                                body: Box::new(kids.next().unwrap_or(MathNode::Hline)),
                            },
                        };
                        EnvRow::Cells {
                            cells,
                            number,
                            labels,
                        }
                    }
                });
            }
            done.push(MathNode::Matrix(style, cols, built));
        }
    }
}

fn matrix_child_count(rows: &[RowPlan]) -> usize {
    let mut n = 0usize;
    for row in rows {
        match row {
            RowPlan::Hline => {}
            RowPlan::Intertext => n = n.saturating_add(1),
            RowPlan::Cells {
                n: cells, number, ..
            } => {
                n = n.saturating_add(*cells);
                if matches!(number, NumPlan::Tag { .. }) {
                    n = n.saturating_add(1);
                }
            }
        }
    }
    n
}

impl MathNode {
    /// Gold-stable S-expression. One space between items; no trailing space.
    ///
    /// The walk is an explicit heap stack, so a tree `parse` accepted cannot
    /// overflow the caller's stack here.
    #[must_use]
    pub fn gold(&self) -> String {
        let mut stack = vec![GoldOp::Node(self)];
        let mut out = String::new();
        while let Some(op) = stack.pop() {
            match op {
                GoldOp::Text(text) => out.push_str(&text),
                GoldOp::Node(node) => {
                    let mut forward = Vec::new();
                    gold_expand(node, &mut forward);
                    for op in forward.into_iter().rev() {
                        stack.push(op);
                    }
                }
            }
        }
        out
    }

    /// LaTeX reprint of this tree.
    ///
    /// The method is total: every node prints, and it does not return
    /// [`Err`](crate::Error) or panic. Braces and spaces may differ from the
    /// source that built the tree, so compare trees, then compare a reprint
    /// to a reprint. It is not a byte-preserving formatter.
    ///
    /// Braces follow one rule based on the parser's grouping rules
    /// (documents/TEXT_MODE_SPEC.md section 5), and a reprint never nests
    /// deeper than any source of the same tree. Command arguments always
    /// print braced (`\frac {1}{2}`). A script argument that is a single
    /// nucleus prints bare (`x_a`), a row is one group, and an empty
    /// argument is `{}`. Row items print bare unless one would change
    /// meaning: a nested row, a superscript character, or a `)` that the
    /// parser's paren lookahead would pair with an earlier `(`. Superscript
    /// primes print as `'` marks. An alphabetic control word is followed by
    /// a space, so `\sin x` does not become `\sinx`, and two bare letters
    /// are split by a space. `\overline` is that full command.
    ///
    /// A [`MathNode::Fraction`] prints as the command in its [`FracStyle`].
    /// `\over` is stored as [`FracStyle::Frac`], so it prints as `\frac`;
    /// `\choose` and `\binom` print as `\binom`.
    ///
    /// A [`MathNode::Color`] is its own group. A color nested in a row
    /// therefore does not swallow the nodes that follow it.
    #[must_use]
    pub fn to_latex(&self) -> String {
        self.to_latex_with(&ParseOptions::new())
    }

    /// [`to_latex`](Self::to_latex) for a tree that will be read back with
    /// `options`. Only [`ParseOptions::prime_symbol`] changes the output:
    /// with it on, `'` reads back as `\prime`, so a `′` superscript prints
    /// as `^{′}` instead of a `'` mark.
    #[must_use]
    pub fn to_latex_with(&self, options: &ParseOptions) -> String {
        let mut out = self.print_raw(options.prime_symbol);
        // The parser strips `\(…\)` and `\[…\]` fences around the whole
        // input and rejects an unclosed one, so a print that only starts
        // with `\(` or `\[` is one group. A print that starts or ends with a
        // whitespace atom the tokenizer does not skip (U+00A0, U+2007, `\x0b`;
        // a comment hid it from the `preprocess` trim) is one group too, so
        // the braces are its ends. A space, tab or newline is not an atom:
        // `<`, `>` and `!` print with a trailing space the tokenizer skips.
        let fenced = ["\\(", "\\["]
            .iter()
            .any(|open| out.starts_with(open) && out.len() > open.len());
        let trimmed = |c: Option<char>| {
            c.is_some_and(|c| c.is_whitespace() && !matches!(c, ' ' | '\t' | '\n' | '\r'))
        };
        if fenced || trimmed(out.chars().next()) || trimmed(out.chars().next_back()) {
            out.insert(0, '{');
            out.push('}');
        }
        out
    }

    /// The print of `self` as it is written inside a larger print: no group
    /// is added around a leading `\(` or `\[`.
    fn print_raw(&self, literal_primes: bool) -> String {
        let mut buf = LatexBuf::new(literal_primes);
        let mut stack = vec![LatexOp::Node(self)];
        while let Some(op) = stack.pop() {
            match op {
                LatexOp::Text(text) => buf.out.push_str(&text),
                LatexOp::Node(node) => {
                    let mut forward = LatexBuf::new(buf.literal_primes);
                    write_node(&mut forward, node);
                    for op in forward.ops.into_iter().rev() {
                        stack.push(op);
                    }
                }
            }
        }
        buf.out
    }
}

enum LatexOp<'a> {
    Node(&'a MathNode),
    Text(String),
}

struct LatexBuf<'a> {
    ops: Vec<LatexOp<'a>>,
    out: String,
    /// A `′` superscript prints as `^{′}`, not as `'` marks.
    literal_primes: bool,
}

impl<'a> LatexBuf<'a> {
    fn new(literal_primes: bool) -> Self {
        Self {
            ops: Vec::new(),
            out: String::new(),
            literal_primes,
        }
    }

    fn push_str(&mut self, text: &str) {
        if let Some(LatexOp::Text(prev)) = self.ops.last_mut() {
            prev.push_str(text);
            return;
        }
        self.ops.push(LatexOp::Text(text.to_string()));
    }

    fn push(&mut self, c: char) {
        if let Some(LatexOp::Text(prev)) = self.ops.last_mut() {
            prev.push(c);
            return;
        }
        self.ops.push(LatexOp::Text(c.to_string()));
    }

    fn node(&mut self, node: &'a MathNode) {
        self.ops.push(LatexOp::Node(node));
    }
}

enum GoldOp<'a> {
    Node(&'a MathNode),
    Text(String),
}

fn gold_text(buf: &mut Vec<GoldOp<'_>>, text: impl Into<String>) {
    buf.push(GoldOp::Text(text.into()));
}

fn gold_node<'a>(buf: &mut Vec<GoldOp<'a>>, node: &'a MathNode) {
    buf.push(GoldOp::Node(node));
}

fn gold_opt<'a>(buf: &mut Vec<GoldOp<'a>>, node: &'a Option<Box<MathNode>>) {
    match node {
        None => gold_text(buf, "_"),
        Some(node) => gold_node(buf, node),
    }
}

fn gold_expand<'a>(node: &'a MathNode, buf: &mut Vec<GoldOp<'a>>) {
    match node {
        MathNode::Atom(c, k) => gold_text(buf, format!("(atom {} {})", k.gold(), quote_atom(*c))),
        MathNode::Fraction(style, n, d) => {
            gold_text(buf, format!("({} ", style.gold()));
            gold_node(buf, n);
            gold_text(buf, " ");
            gold_node(buf, d);
            gold_text(buf, ")");
        }
        MathNode::Radical(None, r) => {
            gold_text(buf, "(sqrt ");
            gold_node(buf, r);
            gold_text(buf, ")");
        }
        MathNode::Radical(Some(i), r) => {
            gold_text(buf, "(sqrtn ");
            gold_node(buf, i);
            gold_text(buf, " ");
            gold_node(buf, r);
            gold_text(buf, ")");
        }
        MathNode::Superscript(b, e) => {
            gold_text(buf, "(sup ");
            gold_node(buf, b);
            gold_text(buf, " ");
            gold_node(buf, e);
            gold_text(buf, ")");
        }
        MathNode::Subscript(b, s) => {
            gold_text(buf, "(sub ");
            gold_node(buf, b);
            gold_text(buf, " ");
            gold_node(buf, s);
            gold_text(buf, ")");
        }
        MathNode::SubSup(b, s, e) => {
            gold_text(buf, "(subsup ");
            gold_node(buf, b);
            gold_text(buf, " ");
            gold_node(buf, s);
            gold_text(buf, " ");
            gold_node(buf, e);
            gold_text(buf, ")");
        }
        MathNode::Delimited(l, b, r) => {
            gold_text(buf, format!("(delim {} ", l.gold()));
            gold_node(buf, b);
            gold_text(buf, format!(" {})", r.gold()));
        }
        MathNode::SizedDelim(d, sz, k) => {
            gold_text(
                buf,
                format!("(big {} {} {})", sz.gold(), k.gold(), quote_delim(d)),
            );
        }
        MathNode::Row(items) => {
            if items.is_empty() {
                gold_text(buf, "(row)");
            } else {
                gold_text(buf, "(row");
                for it in items {
                    gold_text(buf, " ");
                    gold_node(buf, it);
                }
                gold_text(buf, ")");
            }
        }
        MathNode::Matrix(style, spec, rows) => {
            let mut head = format!("(matrix {}", style.gold());
            if !spec.is_empty() {
                head.push(' ');
                for c in spec {
                    head.push(c.gold());
                }
            }
            gold_text(buf, head);
            for row in rows {
                gold_text(buf, " ");
                gold_row(buf, row);
            }
            gold_text(buf, ")");
        }
        MathNode::Substack(lines) => {
            gold_text(buf, "(substack");
            for ln in lines {
                gold_text(buf, " ");
                gold_node(buf, ln);
            }
            gold_text(buf, ")");
        }
        MathNode::Ref(k) => gold_text(buf, format!("(ref {k})")),
        MathNode::Tag { star: false, body } => {
            gold_text(buf, "(tag ");
            gold_node(buf, body);
            gold_text(buf, ")");
        }
        MathNode::Tag { star: true, body } => {
            gold_text(buf, "(tagstar ");
            gold_node(buf, body);
            gold_text(buf, ")");
        }
        MathNode::Label(k) => gold_text(buf, format!("(label {k})")),
        MathNode::NoNumber => gold_text(buf, "(nonumber)"),
        MathNode::Hline => gold_text(buf, "(hline)"),
        MathNode::Intertext(n) => {
            gold_text(buf, "(intertext ");
            gold_node(buf, n);
            gold_text(buf, ")");
        }
        MathNode::Sum(lo, hi) => {
            gold_text(buf, "(sum ");
            gold_opt(buf, lo);
            gold_text(buf, " ");
            gold_opt(buf, hi);
            gold_text(buf, ")");
        }
        MathNode::Integral(k, lo, hi) => {
            gold_text(buf, format!("({} ", k.gold()));
            gold_opt(buf, lo);
            gold_text(buf, " ");
            gold_opt(buf, hi);
            gold_text(buf, ")");
        }
        MathNode::Product(lo, hi) => {
            gold_text(buf, "(prod ");
            gold_opt(buf, lo);
            gold_text(buf, " ");
            gold_opt(buf, hi);
            gold_text(buf, ")");
        }
        MathNode::Limit(lo) => {
            gold_text(buf, "(lim ");
            gold_opt(buf, lo);
            gold_text(buf, ")");
        }
        MathNode::OverUnder(b, over, under) => {
            gold_text(buf, "(overunder ");
            gold_node(buf, b);
            gold_text(buf, " ");
            gold_opt(buf, over);
            gold_text(buf, " ");
            gold_opt(buf, under);
            gold_text(buf, ")");
        }
        MathNode::Accent(b, a) => {
            gold_text(buf, format!("(accent {} ", a.gold()));
            gold_node(buf, b);
            gold_text(buf, ")");
        }
        MathNode::CancelTo(v, e) => {
            gold_text(buf, "(cancelto ");
            gold_node(buf, v);
            gold_text(buf, " ");
            gold_node(buf, e);
            gold_text(buf, ")");
        }
        MathNode::Text(t, st) => {
            let plain = t
                .items()
                .iter()
                .all(|item| matches!(item, TextItem::Str(_) | TextItem::Space));
            if plain {
                gold_text(
                    buf,
                    format!("(text {} {})", st.gold(), quote_text(&t.to_plain_string())),
                );
            } else {
                gold_text(buf, format!("(text {} ", st.gold()));
                gold_run(buf, t);
                gold_text(buf, ")");
            }
        }
        MathNode::Space(SpaceKind::Thin) => gold_text(buf, "(space thin)"),
        MathNode::Space(SpaceKind::Medium) => gold_text(buf, "(space medium)"),
        MathNode::Space(SpaceKind::Thick) => gold_text(buf, "(space thick)"),
        MathNode::Space(SpaceKind::NegThin) => gold_text(buf, "(space negthin)"),
        MathNode::Space(SpaceKind::Quad) => gold_text(buf, "(space quad)"),
        MathNode::Space(SpaceKind::Qquad) => gold_text(buf, "(space qquad)"),
        MathNode::Space(SpaceKind::ControlSpace) => gold_text(buf, "(space control)"),
        MathNode::Space(SpaceKind::Hspace(d)) => {
            gold_text(buf, format!("(space hspace {})", dim_gold(d)));
        }
        MathNode::Operator(name, false) => gold_text(buf, format!("(op {name})")),
        MathNode::Operator(name, true) => gold_text(buf, format!("(op {name} limits)")),
        MathNode::Symbol(name) => gold_text(buf, format!("(symbol {name})")),
        MathNode::Color(c, b) => {
            gold_text(buf, format!("(color {} ", c.css_hex()));
            gold_node(buf, b);
            gold_text(buf, ")");
        }
        MathNode::TextColor(c, b) => {
            gold_text(buf, format!("(textcolor {} ", c.css_hex()));
            gold_node(buf, b);
            gold_text(buf, ")");
        }
        MathNode::ColorBox(c, b) => {
            gold_text(buf, format!("(colorbox {} ", c.css_hex()));
            gold_node(buf, b);
            gold_text(buf, ")");
        }
        MathNode::FColorBox(border, fill, b) => {
            gold_text(
                buf,
                format!("(fcolorbox {} {} ", border.css_hex(), fill.css_hex()),
            );
            gold_node(buf, b);
            gold_text(buf, ")");
        }
        MathNode::Strut(h, d) => {
            gold_text(buf, format!("(strut {} {})", dim_gold(h), dim_gold(d)));
        }
        MathNode::Phantom(k, b) => {
            gold_text(buf, format!("({} ", k.gold()));
            gold_node(buf, b);
            gold_text(buf, ")");
        }
    }
}

fn gold_row<'a>(buf: &mut Vec<GoldOp<'a>>, row: &'a EnvRow) {
    match row {
        EnvRow::Hline => gold_text(buf, "(hline)"),
        EnvRow::Intertext(n) => {
            gold_text(buf, "(intertext ");
            gold_node(buf, n);
            gold_text(buf, ")");
        }
        EnvRow::Cells {
            cells,
            number,
            labels,
        } => {
            gold_text(buf, "(");
            for (i, cell) in cells.iter().enumerate() {
                if i > 0 {
                    gold_text(buf, " ");
                }
                gold_node(buf, cell);
            }
            match number {
                EqNumber::Default => {}
                EqNumber::Suppress => gold_text(buf, " (nonumber)"),
                EqNumber::Tag { star: false, body } => {
                    gold_text(buf, " (tag ");
                    gold_node(buf, body);
                    gold_text(buf, ")");
                }
                EqNumber::Tag { star: true, body } => {
                    gold_text(buf, " (tagstar ");
                    gold_node(buf, body);
                    gold_text(buf, ")");
                }
            }
            for lab in labels {
                gold_text(buf, format!(" (label {lab})"));
            }
            gold_text(buf, ")");
        }
    }
}

fn quote_delim(d: &Delimiter) -> String {
    match d {
        Delimiter::Empty => ".".into(),
        Delimiter::Char(c) => quote_atom(*c),
        Delimiter::Named(n) => n.clone(),
    }
}

fn quote_atom(c: char) -> String {
    match c {
        '"' => "'\"'".into(),
        '\'' => "\"'\"".into(),
        _ => format!("\"{c}\""),
    }
}

/// `(run item…)` for a text run that is not plain characters and spaces.
fn gold_run<'a>(buf: &mut Vec<GoldOp<'a>>, run: &'a TextRun) {
    gold_text(buf, "(run");
    for item in run.items() {
        match item {
            TextItem::Str(s) => gold_text(buf, format!(" {}", quote_text(s))),
            TextItem::Space => gold_text(buf, " space"),
            TextItem::ControlSpace => gold_text(buf, " ctrlspace"),
            TextItem::NoBreakSpace => gold_text(buf, " nbsp"),
            TextItem::Symbol(n) => gold_text(buf, format!(" (symbol {n})")),
            TextItem::Nested(st, r) => {
                gold_text(buf, format!(" (nested {} ", st.gold()));
                gold_run(buf, r);
                gold_text(buf, ")");
            }
            TextItem::Math(m) => {
                gold_text(buf, " (math ");
                gold_node(buf, m);
                gold_text(buf, ")");
            }
        }
    }
    gold_text(buf, ")");
}

fn quote_text(t: &str) -> String {
    format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
}

fn dim_gold(d: &Dim) -> String {
    const RATIOS: [(i64, i64); 10] = [
        (0, 1),
        (1, 1),
        (2, 1),
        (1, 2),
        (1, 18),
        (2, 18),
        (3, 18),
        (7, 10),
        (3, 10),
        (1, 10),
    ];
    for (n, den) in RATIOS {
        if d.eq_dim(&Dim::ratio(n, den)) {
            if den == 1 {
                return n.to_string();
            }
            return format!("{n}/{den}");
        }
    }
    for i in -64i64..65 {
        if d.eq_dim(&Dim::from_i64(i)) {
            return i.to_string();
        }
    }
    d.to_dec_string()
}

fn write_node<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    match node {
        MathNode::Atom(c, _) => write_atom(out, *c),
        MathNode::Fraction(style, num, den) => {
            write_cmd(out, style.gold());
            write_arg(out, num);
            write_arg(out, den);
        }
        MathNode::Radical(index, body) => {
            write_cmd(out, "sqrt");
            if let Some(index) = index {
                write_option(out, index);
            }
            write_arg_after_option(out, body);
        }
        MathNode::Superscript(base, sup) => write_scripts(out, base, None, Some(sup)),
        MathNode::Subscript(base, sub) => write_scripts(out, base, Some(sub), None),
        MathNode::SubSup(base, sub, sup) => write_scripts(out, base, Some(sub), Some(sup)),
        MathNode::Delimited(open, body, close) => {
            // `\binom` parses to this tree and costs one level less.
            if let (
                Delimiter::Char('('),
                Delimiter::Char(')'),
                MathNode::Fraction(FracStyle::Frac, n, d),
            ) = (open, close, body.as_ref())
            {
                write_cmd(out, "binom");
                write_arg(out, n);
                write_arg(out, d);
            } else {
                write_cmd(out, "left");
                write_delim(out, open);
                out.node(body);
                write_cmd(out, "right");
                write_delim(out, close);
            }
        }
        MathNode::SizedDelim(delim, size, kind) => {
            out.push('\\');
            out.push_str(size.gold());
            out.push_str(sized_suffix(*kind, delim));
            out.push(' ');
            write_delim(out, delim);
        }
        MathNode::Row(items) => write_row(out, items),
        MathNode::Matrix(style, spec, rows) => write_matrix(out, *style, spec, rows),
        MathNode::Substack(lines) => {
            write_cmd(out, "substack");
            out.push('{');
            match lines.as_slice() {
                // The `{` is required, and `{]}` unwraps to a bare `]` (G3),
                // so a lone bracket keeps two groups.
                [line @ MathNode::Atom('[' | ']', _)] => write_group(out, line),
                _ => {
                    for (i, line) in lines.iter().enumerate() {
                        if i > 0 {
                            out.push_str("\\\\");
                        }
                        out.node(line);
                    }
                }
            }
            out.push('}');
        }
        MathNode::Ref(key) => write_braced_text(out, "ref", key),
        MathNode::Tag { star, body } => {
            write_cmd(out, "tag");
            if *star {
                out.push('*');
            }
            write_group(out, body);
        }
        MathNode::Label(key) => write_braced_text(out, "label", key),
        MathNode::NoNumber => write_cmd(out, "nonumber"),
        MathNode::Hline => write_cmd(out, "hline"),
        MathNode::Intertext(body) => write_intertext(out, body),
        MathNode::Sum(lower, upper) => {
            write_op_scripts(out, "sum", lower.as_deref(), upper.as_deref())
        }
        MathNode::Product(lower, upper) => {
            write_op_scripts(out, "prod", lower.as_deref(), upper.as_deref())
        }
        MathNode::Integral(kind, lower, upper) => {
            write_op_scripts(out, kind.gold(), lower.as_deref(), upper.as_deref())
        }
        MathNode::Limit(lower) => write_op_scripts(out, "lim", lower.as_deref(), None),
        MathNode::OverUnder(base, over, under) => {
            write_over_under(out, base, over.as_deref(), under.as_deref())
        }
        MathNode::Accent(body, kind) => {
            write_cmd(out, kind.gold());
            write_arg(out, body);
        }
        MathNode::CancelTo(value, expr) => {
            write_cmd(out, "cancelto");
            write_arg(out, value);
            write_arg(out, expr);
        }
        MathNode::Text(text, style) => {
            if *style == TextStyle::Text
                || text
                    .as_str()
                    .is_some_and(|t| t.chars().all(|c| c.is_ascii_alphabetic()))
            {
                write_cmd(out, style_cmd(*style));
                out.push('{');
                print_run(out, text);
                out.push('}');
            } else {
                write_cmd(out, style_cmd(*style));
                out.push('{');
                write_cmd(out, "text");
                out.push('{');
                print_run(out, text);
                out.push('}');
                out.push('}');
            }
        }
        MathNode::Space(kind) => write_space(out, kind),
        MathNode::Operator(name, limits) => write_operator(out, name, *limits),
        MathNode::Symbol(name) => write_cmd(out, name),
        MathNode::Color(color, body) => {
            out.push('{');
            write_cmd(out, "color");
            push_html(out, *color);
            out.node(body);
            out.push('}');
        }
        MathNode::TextColor(color, body) => write_paint(out, "textcolor", *color, None, body),
        MathNode::ColorBox(color, body) => write_paint(out, "colorbox", *color, None, body),
        MathNode::FColorBox(border, fill, body) => {
            write_paint(out, "fcolorbox", *border, Some(*fill), body)
        }
        MathNode::Strut(height, depth) => write_strut(out, height, depth),
        MathNode::Phantom(kind, body) => {
            write_cmd(out, kind.gold());
            write_arg(out, body);
        }
    }
}

/// The items of a row of two or more primes of one spelling (all U+2032
/// atoms, or all `\prime` symbols). Such a run prints unbraced inside one
/// `^{…}` group, so its reprint costs one brace level for any run length.
fn prime_run(node: &MathNode) -> Option<&[MathNode]> {
    match node {
        MathNode::Row(items)
            if items.len() >= 2
                && (items
                    .iter()
                    .all(|item| matches!(item, MathNode::Atom('′', AtomKind::Ord)))
                    || items
                        .iter()
                        .all(|item| matches!(item, MathNode::Symbol(name) if name == "prime"))) =>
        {
            Some(items)
        }
        _ => None,
    }
}

/// True for a superscript that prints as `'` marks: one `′` atom or a run.
fn is_primes(sup: &MathNode) -> bool {
    matches!(sup, MathNode::Atom('′', AtomKind::Ord))
        || prime_run(sup).is_some_and(|items| {
            items
                .iter()
                .all(|item| matches!(item, MathNode::Atom('′', AtomKind::Ord)))
        })
}

fn write_scripts<'a>(
    out: &mut LatexBuf<'a>,
    base: &'a MathNode,
    sub: Option<&'a MathNode>,
    sup: Option<&'a MathNode>,
) {
    match base {
        MathNode::Row(items) if items.is_empty() => out.push_str("{}"),
        // `x'^a` parses as `(x')^a` and `x_b'^a` as `(x_b')^a`, so a primed
        // base needs no braces when a `^` superscript follows. More marks
        // would join the base's primes instead.
        MathNode::Superscript(_, prime) | MathNode::SubSup(_, _, prime)
            if sub.is_none()
                && !out.literal_primes
                && is_primes(prime)
                && !sup.is_some_and(is_primes) =>
        {
            out.node(base)
        }
        // A lone bracket is a nucleus here, though not as an argument.
        MathNode::Atom('(' | ')' | '[', _) => out.node(base),
        _ if bare_arg(base) => out.node(base),
        _ => write_group(out, base),
    }
    if let Some(sub) = sub {
        out.push('_');
        write_script_arg(out, sub);
    }
    let Some(sup) = sup else {
        return;
    };
    if !out.literal_primes && matches!(sup, MathNode::Atom('′', AtomKind::Ord)) {
        out.push('\'');
        return;
    }
    match prime_run(sup) {
        Some(items)
            if !out.literal_primes
                && items
                    .iter()
                    .all(|item| matches!(item, MathNode::Atom('′', AtomKind::Ord))) =>
        {
            for _ in items {
                out.push('\'');
            }
        }
        Some(items) => {
            out.push_str("^{");
            for item in items {
                out.node(item);
            }
            out.push('}');
        }
        None => {
            // Primes after a superscript join it: `x^a''` is the row of the
            // old superscript and the primes. Print them as marks after the
            // argument.
            let (core, marks) = match sup {
                MathNode::Row(items) if !out.literal_primes => match items.split_first() {
                    Some((rest, tail))
                        if !tail.is_empty()
                            && tail
                                .iter()
                                .all(|item| matches!(item, MathNode::Atom('′', AtomKind::Ord))) =>
                    {
                        (rest, tail.len())
                    }
                    _ => (sup, 0),
                },
                _ => (sup, 0),
            };
            out.push('^');
            write_script_arg(out, core);
            for _ in 0..marks {
                out.push('\'');
            }
        }
    }
}

fn write_op_scripts<'a>(
    out: &mut LatexBuf<'a>,
    name: &str,
    sub: Option<&'a MathNode>,
    sup: Option<&'a MathNode>,
) {
    write_cmd(out, name);
    if let Some(sub) = sub {
        out.push('_');
        write_script_arg(out, sub);
    }
    if let Some(sup) = sup {
        out.push('^');
        write_script_arg(out, sup);
    }
}

fn write_group<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    out.push('{');
    out.node(node);
    out.push('}');
}

fn write_row<'a>(out: &mut LatexBuf<'a>, items: &'a [MathNode]) {
    if items.is_empty() {
        out.push_str("{}");
        return;
    }
    write_row_items(out, items);
}

/// Row items print bare unless the parser would read them back differently
/// (spec 5.3, row item): a nested row stays a group, and a prime or
/// superscript character would attach to its neighbour. Two bare letters are
/// split by a space so they do not form an operator word. Then the row paren
/// pass (spec 5.4) groups any item that would close a scripted paren group.
fn write_row_items_at<'a>(out: &mut LatexBuf<'a>, items: &'a [MathNode], row_start: bool) {
    let mut braced: Vec<bool> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if row_start && i == 0 && matches!(item, MathNode::Hline | MathNode::Intertext(_)) {
                true
            } else {
                match item {
                    MathNode::Row(_) => true,
                    MathNode::Atom(c, _) => *c == '\'' || is_superscript_char(*c),
                    _ => false,
                }
            }
        })
        .collect();
    // The pass prints every item once; those strings are reused below, so a
    // nested row is printed once per level, not twice (that was 2^k).
    let prints = paren_groups(items, &mut braced, out.literal_primes);
    let mut prev_letter = false;
    for (i, (item, braced)) in items.iter().zip(braced).enumerate() {
        let letter = !braced && matches!(item, MathNode::Atom(c, _) if c.is_ascii_alphabetic());
        if letter && prev_letter {
            out.push(' ');
        }
        prev_letter = letter;
        match (item, prints.as_ref().and_then(|p| p.get(i))) {
            (MathNode::Row(inner), _) if braced && inner.is_empty() => out.push_str("{}"),
            (_, Some(print)) if braced => {
                out.push('{');
                out.push_str(print);
                out.push('}');
            }
            (_, Some(print)) => out.push_str(print),
            _ if braced => write_group(out, item),
            _ => out.node(item),
        }
    }
}

fn write_row_items<'a>(out: &mut LatexBuf<'a>, items: &'a [MathNode]) {
    write_row_items_at(out, items, false);
}

/// The first cell of an environment row: the parser reads a leading `\hline` or
/// `\intertext` as a row rule or a text row, so such an item prints braced.
fn write_cell<'a>(out: &mut LatexBuf<'a>, cell: &'a MathNode) {
    match cell {
        MathNode::Hline | MathNode::Intertext(_) => write_group(out, cell),
        MathNode::Row(items) if !items.is_empty() => write_row_items_at(out, items, true),
        _ => out.node(cell),
    }
}

/// True when the print of `node` starts with a bare `(` that begins an atom:
/// the `(` itself, or a scripted node whose innermost base is `(`.
fn starts_with_open_paren(node: &MathNode) -> bool {
    let mut current = node;
    loop {
        match current {
            MathNode::Superscript(base, _)
            | MathNode::Subscript(base, _)
            | MathNode::SubSup(base, _, _) => current = base,
            MathNode::Atom(c, _) => return *c == '(',
            _ => return false,
        }
    }
}

/// The parser's tokens for a print: tokenized and passed through the group
/// rules, as `parse_with_options` does.
fn item_tokens(printed: &str) -> Option<Vec<Token>> {
    tokenize(printed).ok().map(group_rules::apply)
}

/// The row paren pass (spec 5.4). The row is tokenized as printed, and the
/// parser's own lookahead ([`paren_script_match`]) runs from every bare `(`
/// that starts an item. Paren-script groups always print braced, so any match
/// it finds is a hazard: the item holding the matching `)` is grouped, which
/// hides it, and the scan repeats until no `(` matches. Returns the print of
/// each item when the pass ran, so the caller does not print them again.
fn paren_groups(
    items: &[MathNode],
    braced: &mut [bool],
    literal_primes: bool,
) -> Option<Vec<String>> {
    let starts = |braced: &[bool]| {
        items
            .iter()
            .zip(braced)
            .any(|(item, braced)| !braced && starts_with_open_paren(item))
    };
    if !starts(braced) {
        return None;
    }
    let prints: Vec<String> = items
        .iter()
        .map(|item| item.print_raw(literal_primes))
        .collect();
    let mut item_toks = Vec::with_capacity(items.len());
    for (print, flag) in prints.iter().zip(braced.iter_mut()) {
        match item_tokens(print) {
            Some(toks) => item_toks.push(toks),
            None => {
                // Not expected for printer output; grouping is always safe.
                *flag = true;
                item_toks.push(Vec::new());
            }
        }
    }
    loop {
        let mut tokens = Vec::new();
        let mut owner = Vec::new();
        let mut first = Vec::with_capacity(items.len());
        let mut prev_letter = false;
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(items.len());
        for (i, ((item, toks), flag)) in items
            .iter()
            .zip(item_toks.iter_mut())
            .zip(braced.iter())
            .enumerate()
        {
            let letter = !flag && matches!(item, MathNode::Atom(c, _) if c.is_ascii_alphabetic());
            if letter && prev_letter {
                tokens.push(Token::Space);
                owner.push(i);
            }
            prev_letter = letter;
            if *flag {
                tokens.push(Token::BeginGroup);
                owner.push(i);
            }
            let skip = toks
                .iter()
                .take_while(|t| matches!(t, Token::Space))
                .count();
            first.push(tokens.len() + skip);
            spans.push((tokens.len(), toks.len()));
            tokens.append(toks);
            owner.resize(tokens.len(), i);
            if *flag {
                tokens.push(Token::EndGroup);
                owner.push(i);
            }
        }
        let hit = items
            .iter()
            .zip(braced.iter())
            .zip(&first)
            .filter(|((item, flag), _)| !**flag && starts_with_open_paren(item))
            .find_map(|(_, pos)| paren_script_match(&tokens, *pos, Stop::eof()))
            .and_then(|m| owner.get(m).copied());
        let grouped = match hit.and_then(|j| braced.get_mut(j)) {
            Some(flag) if !*flag => {
                *flag = true;
                true
            }
            _ => false,
        };
        if !grouped {
            return Some(prints);
        }
        for ((start, len), toks) in spans.into_iter().rev().zip(item_toks.iter_mut().rev()) {
            let mut piece = tokens.split_off(start);
            piece.truncate(len);
            *toks = piece;
        }
    }
}

/// True when `node` prints as exactly one nucleus, so an argument needs no
/// braces around it.
fn bare_arg(node: &MathNode) -> bool {
    let mut node = node;
    loop {
        return match node {
            MathNode::Atom(c, _) => {
                !c.is_whitespace()
                    && !is_superscript_char(*c)
                    && !matches!(
                        *c,
                        '(' | ')' | '[' | '\'' | '^' | '_' | '&' | '#' | '~' | '$'
                    )
            }
            MathNode::Row(_)
            | MathNode::Superscript(..)
            | MathNode::Subscript(..)
            | MathNode::SubSup(..)
            | MathNode::Color(..)
            | MathNode::Ref(_)
            | MathNode::Tag { .. }
            | MathNode::Label(_)
            | MathNode::NoNumber
            | MathNode::Hline
            | MathNode::Intertext(_) => false,
            MathNode::Operator(name, limits) => match operator_limits(name) {
                Some(default) => default == *limits,
                None => !*limits,
            },
            MathNode::Sum(lo, hi) | MathNode::Product(lo, hi) | MathNode::Integral(_, lo, hi) => {
                lo.is_none() && hi.is_none()
            }
            MathNode::Limit(lo) => lo.is_none(),
            MathNode::OverUnder(base, None, None) => {
                node = base;
                continue;
            }
            MathNode::OverUnder(base, ..) => !matches!(
                base.as_ref(),
                MathNode::Accent(_, AccentKind::Overbrace | AccentKind::Underbrace)
            ),
            _ => true,
        };
    }
}

/// True when `printed` contains `pattern` outside every brace group, not
/// as the second character of a control symbol.
fn has_top_level(printed: &str, pattern: &[char]) -> bool {
    let chars: Vec<char> = printed.chars().collect();
    let mut braces = 0usize;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if braces == 0 && chars.get(i..i + pattern.len()) == Some(pattern) {
            return true;
        }
        match c {
            '\\' => i += 1,
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    false
}

/// `[…]` around an optional argument (spec 5.3). A `]` at brace level zero
/// would end the option early, so such content is grouped; a lone `]` keeps
/// two groups, since `{]}` is read as a bare `]` (G3). Parens need nothing:
/// the paren lookahead skips options (Q1).
fn write_option<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    out.push('[');
    match node {
        // `[]` reads as an empty row; `[{}]` would cost one more level.
        MathNode::Row(items) if items.is_empty() => {}
        MathNode::Atom(']', _) => {
            out.push('{');
            write_group(out, node);
            out.push('}');
        }
        _ => {
            // Print the body once and reuse it: printing it again through
            // `out.node` made nested options cost 2^k.
            let body = node.print_raw(out.literal_primes);
            if item_tokens(&body).is_none_or(option_ends_early) {
                out.push('{');
                out.push_str(&body);
                out.push('}');
            } else {
                out.push_str(&body);
            }
        }
    }
    out.push(']');
}

/// An argument right after a command that takes `[…]`: a lone `[` keeps two
/// groups, since `{[}` is read as a bare `[` and would open the option (G3).
fn write_arg_after_option<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    if matches!(node, MathNode::Atom('[', _)) {
        out.push('{');
        write_group(out, node);
        out.push('}');
    } else {
        write_arg(out, node);
    }
}

/// A command argument: always braced (spec 5, Q2). An empty row is `{}`, a
/// row is one group, and a color is already its own group. A braced argument
/// costs no more depth than a bare one, so this never nests deeper than the
/// source.
fn write_arg<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    match node {
        MathNode::Row(items) if items.is_empty() => out.push_str("{}"),
        MathNode::Row(items) => {
            out.push('{');
            write_row_items(out, items);
            out.push('}');
        }
        MathNode::Color(..) => out.node(node),
        _ => write_group(out, node),
    }
}

/// A sub- or superscript argument, bare first (spec 5.3, script): bare when it
/// prints as exactly one nucleus (G1), otherwise one group.
fn write_script_arg<'a>(out: &mut LatexBuf<'a>, node: &'a MathNode) {
    match node {
        MathNode::Row(items) if items.is_empty() => out.push_str("{}"),
        MathNode::Row(items) => {
            out.push('{');
            write_row_items(out, items);
            out.push('}');
        }
        MathNode::Color(..) => out.node(node),
        _ if bare_arg(node) => out.node(node),
        _ => write_group(out, node),
    }
}

fn write_matrix<'a>(
    out: &mut LatexBuf<'a>,
    style: MatrixStyle,
    spec: &[ColSpec],
    rows: &'a [EnvRow],
) {
    write_cmd(out, "begin");
    out.push('{');
    out.push_str(style.gold());
    out.push('}');
    if matches!(style, MatrixStyle::Array) || !spec.is_empty() {
        out.push('{');
        for col in spec {
            out.push(col.gold());
        }
        out.push('}');
    }
    let mut prev_cells = false;
    for row in rows {
        let cells = matches!(row, EnvRow::Cells { .. });
        if prev_cells {
            out.push_str("\\\\");
        }
        prev_cells = cells;
        match row {
            EnvRow::Hline => write_cmd(out, "hline"),
            EnvRow::Intertext(body) => write_intertext(out, body),
            EnvRow::Cells {
                cells,
                number,
                labels,
            } => {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        out.push('&');
                    }
                    if i == 0 {
                        write_cell(out, cell);
                    } else {
                        out.node(cell);
                    }
                    if i + 1 == cells.len() {
                        write_eq_meta(out, number, labels);
                    }
                }
                if cells.is_empty() {
                    write_eq_meta(out, number, labels);
                }
            }
        }
    }
    write_cmd(out, "end");
    out.push('{');
    out.push_str(style.gold());
    out.push('}');
}

fn write_eq_meta<'a>(out: &mut LatexBuf<'a>, number: &'a EqNumber, labels: &[String]) {
    for key in labels {
        write_braced_text(out, "label", key);
    }
    match number {
        EqNumber::Default => {}
        EqNumber::Suppress => write_cmd(out, "nonumber"),
        EqNumber::Tag { star, body } => {
            write_cmd(out, "tag");
            if *star {
                out.push('*');
            }
            write_group(out, body);
        }
    }
}

fn write_over_under<'a>(
    out: &mut LatexBuf<'a>,
    base: &'a MathNode,
    over: Option<&'a MathNode>,
    under: Option<&'a MathNode>,
) {
    if let MathNode::Accent(body, kind @ (AccentKind::Overbrace | AccentKind::Underbrace)) = base {
        write_cmd(out, kind.gold());
        write_arg(out, body);
        if let Some(under) = under {
            out.push('_');
            write_arg(out, under);
        }
        if let Some(over) = over {
            out.push('^');
            write_arg(out, over);
        }
        return;
    }
    if let MathNode::Symbol(name) = base {
        let cmd = match (name.as_str(), over) {
            ("longrightarrow", Some(_)) => Some("xrightarrow"),
            ("longleftarrow", Some(_)) => Some("xleftarrow"),
            _ => None,
        };
        if let Some(cmd) = cmd {
            write_cmd(out, cmd);
            if let Some(under) = under {
                write_option(out, under);
            }
            if let Some(over) = over {
                write_arg_after_option(out, over);
            }
            return;
        }
    }
    match (over, under) {
        (Some(over), Some(under)) => {
            write_cmd(out, "overset");
            write_arg(out, over);
            write_cmd(out, "underset");
            write_arg(out, under);
            write_arg(out, base);
        }
        (Some(over), None) => {
            write_cmd(out, "overset");
            write_arg(out, over);
            write_arg(out, base);
        }
        (None, Some(under)) => {
            write_cmd(out, "underset");
            write_arg(out, under);
            write_arg(out, base);
        }
        (None, None) => out.node(base),
    }
}

fn write_intertext<'a>(out: &mut LatexBuf<'a>, body: &'a MathNode) {
    write_cmd(out, "intertext");
    out.push('{');
    if let MathNode::Text(text, _) = body {
        print_run(out, text);
    } else {
        out.node(body);
    }
    out.push('}');
}

fn write_paint<'a>(
    out: &mut LatexBuf<'a>,
    cmd: &str,
    first: Color,
    second: Option<Color>,
    body: &'a MathNode,
) {
    write_cmd(out, cmd);
    push_html(out, first);
    if let Some(second) = second {
        push_html(out, second);
    }
    write_arg(out, body);
}

fn write_operator(out: &mut LatexBuf<'_>, name: &str, limits: bool) {
    match operator_limits(name) {
        Some(default) => {
            write_cmd(out, name);
            if limits != default {
                write_cmd(out, if limits { "limits" } else { "nolimits" });
            }
        }
        None => {
            write_cmd(out, "operatorname");
            out.push('{');
            push_escaped(out, name);
            out.push('}');
            if limits {
                write_cmd(out, "limits");
            }
        }
    }
}

fn operator_limits(name: &str) -> Option<bool> {
    Some(match name {
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

fn write_space(out: &mut LatexBuf<'_>, kind: &SpaceKind) {
    match kind {
        SpaceKind::Thin => out.push_str("\\,"),
        SpaceKind::Medium => out.push_str("\\:"),
        SpaceKind::Thick => out.push_str("\\;"),
        SpaceKind::NegThin => out.push_str("\\!"),
        SpaceKind::Quad => write_cmd(out, "quad"),
        SpaceKind::Qquad => write_cmd(out, "qquad"),
        // The space is the control symbol, not disposable padding. `preprocess`
        // keeps one trailing ASCII space after an odd run of backslashes, so
        // this reprint parses again. Do not drop the space or replace it with
        // a token that trimming would delete.
        SpaceKind::ControlSpace => out.push_str("\\ "),
        SpaceKind::Hspace(dim) => {
            write_cmd(out, "hspace");
            out.push('{');
            out.push_str(&dim.to_tex_string());
            out.push('}');
        }
    }
}

fn write_strut(out: &mut LatexBuf<'_>, height: &Dim, depth: &Dim) {
    if height.eq_dim(&Dim::ratio(7, 10)) && depth.eq_dim(&Dim::ratio(3, 10)) {
        write_cmd(out, "strut");
        return;
    }
    write_cmd(out, "rule");
    out.push_str("{0em}{");
    out.push_str(&height.to_tex_string());
    out.push('}');
}

fn write_atom(out: &mut LatexBuf<'_>, c: char) {
    match c {
        '{' | '}' | '%' | '\\' => {
            out.push('\\');
            out.push(c);
        }
        '<' | '>' | '!' => {
            out.push(c);
            out.push(' ');
        }
        _ => out.push(c),
    }
}

fn write_delim(out: &mut LatexBuf<'_>, delim: &Delimiter) {
    match delim {
        Delimiter::Empty => out.push('.'),
        Delimiter::Char(c) => write_atom(out, *c),
        Delimiter::Named(name) if name == "{" || name == "}" || name == "|" => {
            out.push('\\');
            out.push_str(name);
        }
        Delimiter::Named(name) => write_cmd(out, name),
    }
}

fn sized_suffix(kind: AtomKind, delim: &Delimiter) -> &'static str {
    if kind == delim_class(delim) {
        return "";
    }
    match kind {
        AtomKind::Open => "l",
        AtomKind::Close => "r",
        AtomKind::Rel => "m",
        AtomKind::Ord | AtomKind::Op | AtomKind::Bin | AtomKind::Punct | AtomKind::Inner => "",
    }
}

fn delim_class(delim: &Delimiter) -> AtomKind {
    match delim {
        Delimiter::Char(c) => atom_class(*c),
        Delimiter::Named(name) if name == "{" => AtomKind::Open,
        Delimiter::Named(name) if name == "}" => AtomKind::Close,
        Delimiter::Empty | Delimiter::Named(_) => AtomKind::Open,
    }
}

fn atom_class(c: char) -> AtomKind {
    match c {
        '+' | '-' | '*' | '±' | '∓' | '·' | '×' | '÷' => AtomKind::Bin,
        '=' | '<' | '>' | '≠' | '≤' | '≥' | '≈' | '≡' => AtomKind::Rel,
        '(' | '[' | '{' => AtomKind::Open,
        ')' | ']' | '}' => AtomKind::Close,
        ',' | ';' | '!' | '?' | ':' => AtomKind::Punct,
        _ => AtomKind::Ord,
    }
}

fn style_cmd(style: TextStyle) -> &'static str {
    match style {
        TextStyle::Rm => "mathrm",
        TextStyle::Bf => "mathbf",
        TextStyle::It => "mathit",
        TextStyle::Sf => "mathsf",
        TextStyle::Tt => "mathtt",
        TextStyle::Bb => "mathbb",
        TextStyle::Cal => "mathcal",
        TextStyle::Frak => "mathfrak",
        TextStyle::Scr => "mathscr",
        TextStyle::Boldsymbol => "boldsymbol",
        TextStyle::Pmb => "pmb",
        TextStyle::Text => "text",
    }
}

fn write_cmd(out: &mut LatexBuf<'_>, name: &str) {
    out.push('\\');
    out.push_str(name);
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()) {
        out.push(' ');
    }
}

fn write_braced_text(out: &mut LatexBuf<'_>, cmd: &str, text: &str) {
    write_cmd(out, cmd);
    out.push('{');
    push_escaped(out, text);
    out.push('}');
}

/// Escapes special LaTeX characters in text.
///
/// The first character is escaped if it is a space, '[', or ']'.
/// All characters are escaped if they are '\', '{', '}', '%', '$', '&', '_', or '^'.
/// Command for a nested text run of `style` inside text.
fn text_cmd(style: TextStyle) -> &'static str {
    match style {
        TextStyle::Text => "text",
        TextStyle::Rm => "textrm",
        TextStyle::Bf => "textbf",
        TextStyle::It => "textit",
        TextStyle::Sf => "textsf",
        TextStyle::Tt => "texttt",
        _ => "text",
    }
}

/// Canonical text-mode print of a run (spec 2.5). `parse` reads it back to
/// the same run, and math-level code never looks inside it.
fn print_run<'a>(out: &mut LatexBuf<'a>, run: &'a TextRun) {
    let mut stack = vec![run.items().iter()];

    while let Some(mut iter) = stack.pop() {
        if let Some(item) = iter.next() {
            stack.push(iter);

            match item {
                TextItem::Str(s) => {
                    for c in s.chars() {
                        match c {
                            '\\' => out.push_str(r"\textbackslash{}"),
                            '^' => out.push_str(r"\textasciicircum{}"),
                            '~' => out.push_str(r"\textasciitilde{}"),
                            '{' | '}' | '$' | '%' | '&' | '#' | '_' => {
                                out.push('\\');
                                out.push(c);
                            }
                            _ => out.push(c),
                        }
                    }
                }
                TextItem::Space => out.push(' '),
                TextItem::ControlSpace => out.push_str(r"\ "),
                TextItem::NoBreakSpace => out.push('~'),
                TextItem::Symbol(name) => {
                    out.push('\\');
                    out.push_str(name);
                    out.push_str("{}");
                }
                TextItem::Nested(style, nested_run) => {
                    write_cmd(out, text_cmd(*style));
                    out.push('{');
                    stack.push(nested_run.items().iter());
                }
                TextItem::Math(node) => {
                    // A `\)` in the body would close `\(` early. Such a body
                    // uses `$…$`, or one group when it also has a `$`.
                    let body = node.print_raw(out.literal_primes);
                    let close = has_top_level(&body, &['\\', ')']);
                    // The body is printed once (`out.node` again would cost
                    // 2^k for text nested in math nested in text).
                    if close && !has_top_level(&body, &['$']) {
                        out.push('$');
                        out.push_str(&body);
                        out.push('$');
                    } else if close {
                        out.push_str(r"\({");
                        out.push_str(&body);
                        out.push_str(r"}\)");
                    } else {
                        out.push_str(r"\(");
                        out.push_str(&body);
                        out.push_str(r"\)");
                    }
                }
            }
        } else if !stack.is_empty() {
            out.push('}');
        }
    }
}

fn push_escaped(out: &mut LatexBuf<'_>, text: &str) {
    for (i, c) in text.chars().enumerate() {
        match c {
            ' ' | '[' | ']' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            '\\' | '{' | '}' | '%' | '$' | '&' | '_' | '^' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
}

fn push_html(out: &mut LatexBuf<'_>, color: Color) {
    out.push_str("[HTML]{");
    let hex = color.css_hex();
    match hex.strip_prefix('#') {
        Some(body) => out.push_str(body),
        None => out.push_str(&hex),
    }
    out.push('}');
}

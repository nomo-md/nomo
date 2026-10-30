//! Equation counters, `\tag`, and `\label` / `\ref` (two-pass).

use std::collections::HashMap;

use crate::parser::{EnvRow, EqNumber, MathNode, MatrixStyle};

/// How auto equation numbers are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberStyle {
    /// `(1)`, `(2)`, …
    Arabic,
    /// `(i)`, `(ii)`, …
    Roman,
    /// `(a)`, `(b)`, …
    Alphabetic,
}

/// Wrapper around the number body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberFormat {
    /// `(1)`
    Parenthesized,
    /// `[1]`
    Bracketed,
    /// `1`
    Plain,
}

/// Counter style for a render (or a sequence of [`layout_with_numbering`](super::layout_with_numbering) calls).
///
/// # Examples
///
/// ```
/// use latex_rust::{NumberFormat, NumberStyle, NumberingConfig};
///
/// let cfg = NumberingConfig::new();
/// assert_eq!(cfg.style, NumberStyle::Arabic);
/// assert_eq!(cfg.start, 1);
/// assert_eq!(cfg.format, NumberFormat::Parenthesized);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberingConfig {
    /// Digit / roman / letter.
    pub style: NumberStyle,
    /// First auto number (usually 1).
    pub start: usize,
    /// Parentheses, brackets, or none.
    pub format: NumberFormat,
}

impl Default for NumberingConfig {
    fn default() -> Self {
        Self {
            style: NumberStyle::Arabic,
            start: 1,
            format: NumberFormat::Parenthesized,
        }
    }
}

impl NumberingConfig {
    /// Arabic, start at 1, parenthesized.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Mutable numbering / label table. Survives across `layout_with_numbering` calls.
///
/// # Examples
///
/// ```
/// use latex_rust::{NumberingConfig, NumberingState};
///
/// let state = NumberingState::new(NumberingConfig::new());
/// assert!(state.label("eq:1").is_none());
/// ```
#[derive(Clone, Debug)]
pub struct NumberingState {
    config: NumberingConfig,
    next: usize,
    labels: HashMap<String, String>,
    assigned: Vec<Option<String>>,
}

impl Default for NumberingState {
    fn default() -> Self {
        Self::new(NumberingConfig::default())
    }
}

impl NumberingState {
    /// Counter starts at `config.start`.
    #[must_use]
    pub fn new(config: NumberingConfig) -> Self {
        let next = config.start;
        Self {
            config,
            next,
            labels: HashMap::new(),
            assigned: Vec::new(),
        }
    }

    /// Formatted number bound to `key`, if `\label{key}` was seen.
    #[must_use]
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    pub(crate) fn lookup(&self, key: &str) -> Option<&str> {
        self.label(key)
    }

    pub(crate) fn assigned(&self, i: usize) -> Option<&str> {
        self.assigned.get(i).and_then(|o| o.as_deref())
    }

    /// Walk `node`, assign numbers, fill labels. Returns the index of the first
    /// new assignment (for this tree).
    pub fn collect(&mut self, node: &MathNode) -> usize {
        let start = self.assigned.len();
        collect_node(node, self);
        start
    }

    fn wrap(&self, body: &str) -> String {
        match self.config.format {
            NumberFormat::Parenthesized => format!("({body})"),
            NumberFormat::Bracketed => format!("[{body}]"),
            NumberFormat::Plain => body.to_string(),
        }
    }

    fn auto_body(&self, n: usize) -> String {
        match self.config.style {
            NumberStyle::Arabic => n.to_string(),
            NumberStyle::Roman => to_roman(n),
            NumberStyle::Alphabetic => to_alpha(n),
        }
    }

    fn auto_display(&mut self) -> String {
        let n = self.next;
        self.next = self.next.saturating_add(1);
        self.wrap(&self.auto_body(n))
    }

    fn bind(&mut self, labels: &[String], display: &str) {
        for k in labels {
            self.labels.insert(k.clone(), display.to_string());
        }
    }
}

enum Collect<'a> {
    Node(&'a MathNode),
    NumberMatrix(MatrixStyle, &'a [crate::parser::EnvRow]),
}

fn collect_node(node: &MathNode, st: &mut NumberingState) {
    let mut stack = vec![Collect::Node(node)];
    while let Some(task) = stack.pop() {
        match task {
            Collect::NumberMatrix(style, rows) => number_matrix(style, rows, st),
            Collect::Node(node) => push_collect(node, &mut stack),
        }
    }
}

fn push_collect<'a>(node: &'a MathNode, stack: &mut Vec<Collect<'a>>) {
    match node {
        MathNode::Matrix(style, _, rows) => {
            stack.push(Collect::NumberMatrix(*style, rows));
            for row in rows.iter().rev() {
                match row {
                    EnvRow::Cells { cells, .. } => {
                        for cell in cells.iter().rev() {
                            stack.push(Collect::Node(cell));
                        }
                    }
                    EnvRow::Intertext(body) => stack.push(Collect::Node(body)),
                    EnvRow::Hline => {}
                }
            }
        }
        MathNode::Row(v) | MathNode::Substack(v) => {
            for child in v.iter().rev() {
                stack.push(Collect::Node(child));
            }
        }
        MathNode::Fraction(_, a, b)
        | MathNode::Superscript(a, b)
        | MathNode::Subscript(a, b)
        | MathNode::CancelTo(a, b) => {
            stack.push(Collect::Node(b));
            stack.push(Collect::Node(a));
        }
        MathNode::SubSup(a, b, c) => {
            stack.push(Collect::Node(c));
            stack.push(Collect::Node(b));
            stack.push(Collect::Node(a));
        }
        MathNode::Radical(deg, body) => {
            stack.push(Collect::Node(body));
            if let Some(deg) = deg {
                stack.push(Collect::Node(deg));
            }
        }
        MathNode::Delimited(_, body, _)
        | MathNode::Accent(body, _)
        | MathNode::Color(_, body)
        | MathNode::TextColor(_, body)
        | MathNode::ColorBox(_, body)
        | MathNode::Phantom(_, body)
        | MathNode::Intertext(body)
        | MathNode::Tag { body, .. }
        | MathNode::FColorBox(_, _, body) => stack.push(Collect::Node(body)),
        MathNode::Sum(lo, hi) | MathNode::Product(lo, hi) | MathNode::Integral(_, lo, hi) => {
            if let Some(hi) = hi {
                stack.push(Collect::Node(hi));
            }
            if let Some(lo) = lo {
                stack.push(Collect::Node(lo));
            }
        }
        MathNode::Limit(lo) => {
            if let Some(lo) = lo {
                stack.push(Collect::Node(lo));
            }
        }
        MathNode::OverUnder(base, over, under) => {
            if let Some(under) = under {
                stack.push(Collect::Node(under));
            }
            if let Some(over) = over {
                stack.push(Collect::Node(over));
            }
            stack.push(Collect::Node(base));
        }
        MathNode::Atom(_, _)
        | MathNode::SizedDelim(_, _, _)
        | MathNode::Text(_, _)
        | MathNode::Space(_)
        | MathNode::Operator(_, _)
        | MathNode::Symbol(_)
        | MathNode::Strut(_, _)
        | MathNode::Ref(_)
        | MathNode::Label(_)
        | MathNode::NoNumber
        | MathNode::Hline => {}
    }
}

fn number_matrix(style: MatrixStyle, rows: &[EnvRow], st: &mut NumberingState) {
    if style.numbers_rows() {
        for row in rows {
            match row {
                EnvRow::Intertext(_) | EnvRow::Hline => {}
                EnvRow::Cells { number, labels, .. } => {
                    let display = assign(number, st);
                    if let Some(d) = &display {
                        st.bind(labels, d);
                    }
                    st.assigned.push(display);
                }
            }
        }
    } else if style.numbers_once() {
        let mut number = EqNumber::Default;
        let mut labels = Vec::new();
        for row in rows {
            if let EnvRow::Cells {
                number: n,
                labels: l,
                ..
            } = row
            {
                match n {
                    EqNumber::Default => {}
                    other => number = other.clone(),
                }
                labels.extend(l.iter().cloned());
            }
        }
        let display = assign(&number, st);
        if let Some(d) = &display {
            st.bind(&labels, d);
        }
        st.assigned.push(display);
    }
}

fn assign(number: &EqNumber, st: &mut NumberingState) -> Option<String> {
    match number {
        EqNumber::Suppress => None,
        EqNumber::Tag { star, body } => {
            let plain = node_plain(body);
            let display = if *star { plain } else { st.wrap(&plain) };
            Some(display)
        }
        EqNumber::Default => Some(st.auto_display()),
    }
}

fn node_plain(n: &MathNode) -> String {
    let mut stack = vec![n];
    let mut out = String::new();
    while let Some(n) = stack.pop() {
        match n {
            MathNode::Atom(c, _) => out.push(*c),
            MathNode::Text(s, _) => out.push_str(&s.to_plain_string()),
            MathNode::Symbol(name) => out.push_str(name),
            MathNode::Row(v) => {
                for child in v.iter().rev() {
                    stack.push(child);
                }
            }
            MathNode::Tag { body, .. } => stack.push(body),
            other => out.push_str(&other.gold()),
        }
    }
    out
}

fn to_roman(mut n: usize) -> String {
    if n == 0 {
        return "0".into();
    }
    let pairs: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, g) in pairs {
        while n >= v {
            s.push_str(g);
            n -= v;
        }
    }
    s
}

fn to_alpha(mut n: usize) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, char::from(b'a' + (n % 26) as u8));
        n /= 26;
    }
    s
}

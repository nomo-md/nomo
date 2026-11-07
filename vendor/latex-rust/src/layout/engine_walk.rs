//! Heap-stack layout walk.
//!
//! `Engine::item_inner` still assembles one node. Children are laid out first
//! and handed back through [`Engine::take_box`], so a chain of any depth the
//! parser accepts stays on the heap.

use super::{is_tex_accent, ArenaNode, Engine, Item, RowKind};
use crate::error::Error;
use crate::layout::{MathBox, MathStyle};
use crate::parser::{EnvRow, MathNode, MatrixStyle, SpaceKind, TextItem, TextRun};

#[derive(Clone, Copy)]
enum Src<'a> {
    Tree(&'a MathNode),
    /// Index into [`Engine::arena`]. The boxed node does not move when the
    /// arena appends.
    Heap(usize),
}

struct ChildJob<'a> {
    src: Src<'a>,
    style: MathStyle,
}

enum Work<'a> {
    Enter {
        src: Src<'a>,
        style: MathStyle,
        depth: usize,
    },
    Exit {
        src: Src<'a>,
        style: MathStyle,
        n: usize,
    },
    Align(AlignWalk<'a>),
}

enum Step<'a> {
    Kids(Vec<ChildJob<'a>>),
    Align(AlignWalk<'a>),
}

struct AlignWalk<'a> {
    rows: &'a [EnvRow],
    style: MathStyle,
    numbered: bool,
    depth: usize,
    index: usize,
    /// `0` schedules the current row. `1` consumes that row's boxes.
    phase: u8,
    pending: usize,
    math_rows: Vec<Vec<MathBox>>,
    nums: Vec<Option<MathBox>>,
    extras: Vec<RowKind>,
    ncols: usize,
}

impl Engine<'_> {
    pub(super) fn drive(&self, root: &MathNode, style: MathStyle) -> Result<Item, Error> {
        let mut stack = vec![Work::Enter {
            src: Src::Tree(root),
            style,
            depth: 0,
        }];
        let mut results: Vec<Item> = Vec::new();
        while let Some(work) = stack.pop() {
            match work {
                Work::Enter { src, style, depth } => {
                    self.enter(src, style, depth, &mut stack)?;
                }
                Work::Exit { src, style, n } => {
                    self.exit(src, style, n, &mut results)?;
                }
                Work::Align(walk) => {
                    self.step_align(walk, &mut stack, &mut results)?;
                }
            }
        }
        if results.len() == 1 {
            if let Some(item) = results.pop() {
                return Ok(item);
            }
        }
        Err(Error::Unsupported {
            what: "internal: layout produced no box".into(),
        })
    }

    pub(super) fn take_item(&self) -> Result<Item, Error> {
        let mut kids = self.kids.borrow_mut();
        let i = self.kid_i.get();
        let Some(slot) = kids.get_mut(i) else {
            return Err(Error::Unsupported {
                what: "internal: layout child missing".into(),
            });
        };
        self.kid_i.set(i + 1);
        Ok(std::mem::replace(
            slot,
            Item {
                bx: MathBox::empty(),
                class: None,
            },
        ))
    }

    pub(super) fn take_box(&self) -> Result<MathBox, Error> {
        Ok(self.take_item()?.bx)
    }

    fn enter<'s>(
        &'s self,
        src: Src<'s>,
        style: MathStyle,
        depth: usize,
        stack: &mut Vec<Work<'s>>,
    ) -> Result<(), Error> {
        if depth >= self.max_depth {
            return Err(Error::Unsupported {
                what: format!("tree nests deeper than {} levels", self.max_depth),
            });
        }
        match self.classify(src, style, depth)? {
            Step::Align(walk) => stack.push(Work::Align(walk)),
            Step::Kids(jobs) => {
                let n = jobs.len();
                stack.push(Work::Exit { src, style, n });
                for job in jobs.into_iter().rev() {
                    stack.push(Work::Enter {
                        src: job.src,
                        style: job.style,
                        depth: depth + 1,
                    });
                }
            }
        }
        Ok(())
    }

    fn exit<'s>(
        &'s self,
        src: Src<'s>,
        style: MathStyle,
        n: usize,
        results: &mut Vec<Item>,
    ) -> Result<(), Error> {
        if results.len() < n {
            return Err(Error::Unsupported {
                what: "internal: layout child missing".into(),
            });
        }
        let kids = results.split_off(results.len() - n);
        *self.kids.borrow_mut() = kids;
        self.kid_i.set(0);
        results.push(self.assemble(src, style)?);
        Ok(())
    }

    fn assemble<'s>(&'s self, src: Src<'s>, style: MathStyle) -> Result<Item, Error> {
        match src {
            Src::Tree(node) => self.item_inner(node, style),
            Src::Heap(i) => {
                let ptr = self.heap_ptr(i)?;
                // SAFETY: `ptr` addresses a `Box<MathNode>` stored in
                // `self.arena`. `drive` only appends to that arena, so the
                // allocation is not moved or freed until `drive` returns.
                // `item_inner` does not replace the slot.
                let node = unsafe { &*ptr };
                self.item_inner(node, style)
            }
        }
    }

    fn classify<'s>(
        &'s self,
        src: Src<'s>,
        style: MathStyle,
        depth: usize,
    ) -> Result<Step<'s>, Error> {
        match src {
            Src::Tree(node) => self.classify_node(node, style, depth),
            Src::Heap(i) => {
                let ptr = self.heap_ptr(i)?;
                // SAFETY: same arena invariant as [`Self::assemble`].
                let node = unsafe { &*ptr };
                self.classify_node(node, style, depth)
            }
        }
    }

    fn classify_node<'s>(
        &'s self,
        node: &'s MathNode,
        style: MathStyle,
        depth: usize,
    ) -> Result<Step<'s>, Error> {
        if let Some(walk) = self.start_align(node, style, depth) {
            return Ok(Step::Align(walk));
        }
        Ok(Step::Kids(self.plan_node(node, style)?))
    }

    fn heap_ptr(&self, index: usize) -> Result<*const MathNode, Error> {
        let arena = self.arena.borrow();
        let Some(node) = arena.get(index) else {
            return Err(Error::Unsupported {
                what: "internal: layout arena".into(),
            });
        };
        Ok(&*node.0 as *const MathNode)
    }

    fn push_owned(&self, node: MathNode) -> usize {
        let mut arena = self.arena.borrow_mut();
        let index = arena.len();
        arena.push(ArenaNode(Box::new(node)));
        index
    }

    fn start_align<'s>(
        &'s self,
        node: &'s MathNode,
        style: MathStyle,
        depth: usize,
    ) -> Option<AlignWalk<'s>> {
        let MathNode::Matrix(ms, _, rows) = node else {
            return None;
        };
        let body = if ms.is_display_env() {
            MathStyle::Display
        } else {
            style
        };
        let numbered = match ms {
            MatrixStyle::Align => true,
            MatrixStyle::Split => false,
            MatrixStyle::Aligned if rows.iter().any(|row| !matches!(row, EnvRow::Cells { .. })) => {
                false
            }
            _ => return None,
        };
        Some(AlignWalk {
            rows,
            style: body,
            numbered,
            depth,
            index: 0,
            phase: 0,
            pending: 0,
            math_rows: Vec::new(),
            nums: Vec::new(),
            extras: Vec::new(),
            ncols: 0,
        })
    }

    fn step_align<'s>(
        &'s self,
        mut walk: AlignWalk<'s>,
        stack: &mut Vec<Work<'s>>,
        results: &mut Vec<Item>,
    ) -> Result<(), Error> {
        if walk.phase == 0 {
            if walk.index >= walk.rows.len() {
                results.push(self.finish_align(
                    walk.math_rows,
                    walk.nums,
                    walk.extras,
                    walk.ncols,
                    walk.style,
                    walk.numbered,
                )?);
                return Ok(());
            }
            enum RowPlan<'a> {
                Hline,
                Intertext(&'a MathNode),
                Cells(Vec<&'a MathNode>),
            }
            let plan = match &walk.rows[walk.index] {
                EnvRow::Hline => RowPlan::Hline,
                EnvRow::Intertext(body) => RowPlan::Intertext(body),
                EnvRow::Cells { cells, .. } => RowPlan::Cells(cells.iter().collect()),
            };
            match plan {
                RowPlan::Hline => {
                    walk.extras.push(RowKind::Hline);
                    walk.index += 1;
                    stack.push(Work::Align(walk));
                }
                RowPlan::Intertext(body) => {
                    walk.phase = 1;
                    walk.pending = 1;
                    let depth = walk.depth;
                    stack.push(Work::Align(walk));
                    stack.push(Work::Enter {
                        src: Src::Tree(body),
                        style: MathStyle::Text,
                        depth: depth + 1,
                    });
                }
                RowPlan::Cells(cells) => {
                    let pending = cells.len();
                    walk.phase = 1;
                    walk.pending = pending;
                    let depth = walk.depth;
                    let style = walk.style;
                    stack.push(Work::Align(walk));
                    for cell in cells.into_iter().rev() {
                        stack.push(Work::Enter {
                            src: Src::Tree(cell),
                            style,
                            depth: depth + 1,
                        });
                    }
                }
            }
            return Ok(());
        }
        if results.len() < walk.pending {
            return Err(Error::Unsupported {
                what: "internal: layout child missing".into(),
            });
        }
        let kids = results.split_off(results.len() - walk.pending);
        match &walk.rows[walk.index] {
            EnvRow::Hline => {
                return Err(Error::Unsupported {
                    what: "internal: align row".into(),
                });
            }
            EnvRow::Intertext(_) => {
                let Some(item) = kids.into_iter().next() else {
                    return Err(Error::Unsupported {
                        what: "internal: layout child missing".into(),
                    });
                };
                walk.extras.push(RowKind::Intertext(Box::new(item.bx)));
            }
            EnvRow::Cells { .. } => {
                let rboxes: Vec<MathBox> = kids.into_iter().map(|item| item.bx).collect();
                walk.ncols = walk.ncols.max(rboxes.len());
                walk.math_rows.push(rboxes);
                walk.extras.push(RowKind::Cells);
                if walk.numbered {
                    let num = match self.take_number() {
                        Some(text) => Some(self.number_box(&text, MathStyle::Text)?),
                        None => None,
                    };
                    walk.nums.push(num);
                }
            }
        }
        walk.index += 1;
        walk.phase = 0;
        stack.push(Work::Align(walk));
        Ok(())
    }

    fn plan_node<'s>(
        &'s self,
        node: &'s MathNode,
        style: MathStyle,
    ) -> Result<Vec<ChildJob<'s>>, Error> {
        let mut jobs = Vec::new();
        match node {
            MathNode::Row(items) => {
                for item in items {
                    jobs.push(tree(item, style));
                }
            }
            MathNode::Fraction(kind, num, den) => {
                let styled = super::frac_math_style(*kind, style);
                jobs.push(tree(num, styled.numerator()));
                jobs.push(tree(den, styled.denominator()));
            }
            MathNode::Radical(deg, rad) => {
                jobs.push(tree(rad, style.cramp()));
                if let Some(deg) = deg {
                    jobs.push(tree(deg, MathStyle::ScriptScript));
                }
            }
            MathNode::Superscript(base, exp) => {
                plan_scripts(style, base, None, Some(exp), &mut jobs);
            }
            MathNode::Subscript(base, sub) => {
                plan_scripts(style, base, Some(sub), None, &mut jobs);
            }
            MathNode::SubSup(base, sub, exp) => {
                plan_scripts(style, base, Some(sub), Some(exp), &mut jobs);
            }
            MathNode::Delimited(_, body, _)
            | MathNode::Phantom(_, body)
            | MathNode::Color(_, body)
            | MathNode::TextColor(_, body)
            | MathNode::ColorBox(_, body)
            | MathNode::FColorBox(_, _, body) => jobs.push(tree(body, style)),
            MathNode::Intertext(body) | MathNode::Tag { body, .. } => {
                jobs.push(tree(body, MathStyle::Text));
            }
            MathNode::Accent(base, kind) => {
                let nucleus = if is_tex_accent(*kind) {
                    style.cramp()
                } else {
                    style
                };
                jobs.push(tree(base, nucleus));
            }
            MathNode::CancelTo(value, expr) => {
                jobs.push(tree(expr, style));
                jobs.push(tree(value, style.into_script()));
            }
            MathNode::Sum(lo, hi) | MathNode::Product(lo, hi) => {
                plan_large_limits(style, true, lo, hi, &mut jobs);
            }
            MathNode::Integral(_, lo, hi) => {
                let script = style.into_script();
                if let Some(hi) = hi {
                    jobs.push(tree(hi, script.cramp()));
                }
                if let Some(lo) = lo {
                    jobs.push(tree(lo, script));
                }
            }
            MathNode::Limit(sub) => {
                if let Some(sub) = sub {
                    jobs.push(tree(sub, style.into_script()));
                }
            }
            MathNode::OverUnder(base, over, under) => {
                jobs.push(tree(base, style));
                let script = style.into_script();
                if let Some(over) = over {
                    jobs.push(tree(over, script));
                }
                if let Some(under) = under {
                    jobs.push(tree(under, script));
                }
                if let Some(over) = over {
                    jobs.push(tree(over, script));
                }
                if let Some(under) = under {
                    jobs.push(tree(under, script));
                }
            }
            MathNode::Substack(lines) => {
                for line in lines {
                    jobs.push(tree(line, MathStyle::Script));
                }
            }
            MathNode::Matrix(ms, _, rows) => self.plan_matrix(*ms, rows, style, &mut jobs)?,
            MathNode::Text(run, _) => plan_text(run, style, &mut jobs),
            MathNode::Atom(_, _)
            | MathNode::Symbol(_)
            | MathNode::SizedDelim(_, _, _)
            | MathNode::Space(_)
            | MathNode::Strut(_, _)
            | MathNode::Operator(_, _)
            | MathNode::Ref(_)
            | MathNode::Label(_)
            | MathNode::NoNumber
            | MathNode::Hline => {}
        }
        Ok(jobs)
    }

    fn plan_matrix<'s>(
        &'s self,
        ms: MatrixStyle,
        rows: &'s [EnvRow],
        style: MathStyle,
        jobs: &mut Vec<ChildJob<'s>>,
    ) -> Result<(), Error> {
        let body = if ms.is_display_env() {
            MathStyle::Display
        } else {
            style
        };
        match ms {
            MatrixStyle::Align | MatrixStyle::Split => Ok(()),
            MatrixStyle::Aligned => {
                if rows.iter().any(|row| !matches!(row, EnvRow::Cells { .. })) {
                    return Ok(());
                }
                for row in rows {
                    let EnvRow::Cells { cells, .. } = row else {
                        continue;
                    };
                    for (column, cell) in cells.iter().enumerate() {
                        jobs.push(tree(cell, MathStyle::Display));
                        if column % 2 == 1 {
                            jobs.push(tree(cell, MathStyle::Display));
                            let index = self.push_owned(prefixed_cell(cell));
                            jobs.push(ChildJob {
                                src: Src::Heap(index),
                                style: MathStyle::Display,
                            });
                        }
                    }
                }
                Ok(())
            }
            MatrixStyle::Gather => {
                for row in rows {
                    match row {
                        EnvRow::Hline => {}
                        EnvRow::Intertext(body) => jobs.push(tree(body, MathStyle::Text)),
                        EnvRow::Cells { cells, .. } => {
                            self.push_row_job(cells, body, jobs);
                        }
                    }
                }
                Ok(())
            }
            MatrixStyle::Multline => {
                if rows.iter().any(|row| !matches!(row, EnvRow::Cells { .. })) {
                    return Ok(());
                }
                for row in rows {
                    if let EnvRow::Cells { cells, .. } = row {
                        self.push_row_job(cells, body, jobs);
                    }
                }
                Ok(())
            }
            MatrixStyle::Equation => {
                if rows.iter().any(|row| !matches!(row, EnvRow::Cells { .. })) {
                    return Ok(());
                }
                self.push_equation(rows, body, jobs);
                Ok(())
            }
            MatrixStyle::Array => {
                for row in rows {
                    match row {
                        EnvRow::Hline => {}
                        EnvRow::Intertext(body) => jobs.push(tree(body, MathStyle::Text)),
                        EnvRow::Cells { cells, .. } => {
                            for cell in cells {
                                jobs.push(tree(cell, body));
                            }
                        }
                    }
                }
                Ok(())
            }
            MatrixStyle::Cases
            | MatrixStyle::Matrix
            | MatrixStyle::Pmatrix
            | MatrixStyle::Bmatrix
            | MatrixStyle::Vmatrix
            | MatrixStyle::VVmatrix
            | MatrixStyle::BBmatrix => {
                if rows.iter().any(|row| !matches!(row, EnvRow::Cells { .. })) {
                    return Ok(());
                }
                for row in rows {
                    if let EnvRow::Cells { cells, .. } = row {
                        for cell in cells {
                            jobs.push(tree(cell, MathStyle::Text));
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn push_row_job<'s>(
        &'s self,
        cells: &'s [MathNode],
        style: MathStyle,
        jobs: &mut Vec<ChildJob<'s>>,
    ) {
        if cells.len() == 1 {
            jobs.push(tree(&cells[0], style));
            return;
        }
        let index = self.push_owned(MathNode::Row(cells.to_vec()));
        jobs.push(ChildJob {
            src: Src::Heap(index),
            style,
        });
    }

    fn push_equation<'s>(
        &'s self,
        rows: &'s [EnvRow],
        style: MathStyle,
        jobs: &mut Vec<ChildJob<'s>>,
    ) {
        let mut only: Option<&'s MathNode> = None;
        let mut count = 0usize;
        for row in rows {
            if let EnvRow::Cells { cells, .. } = row {
                for cell in cells {
                    count = count.saturating_add(1);
                    only = Some(cell);
                }
            }
        }
        if count == 0 {
            let index = self.push_owned(MathNode::Row(Vec::new()));
            jobs.push(ChildJob {
                src: Src::Heap(index),
                style,
            });
            return;
        }
        if count == 1 {
            if let Some(cell) = only {
                jobs.push(tree(cell, style));
                return;
            }
        }
        let mut parts = Vec::new();
        for row in rows {
            if let EnvRow::Cells { cells, .. } = row {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        parts.push(MathNode::Space(SpaceKind::Quad));
                    }
                    parts.push(cell.clone());
                }
            }
        }
        let index = self.push_owned(MathNode::Row(parts));
        jobs.push(ChildJob {
            src: Src::Heap(index),
            style,
        });
    }
}

/// Inline math inside a text run, in the order `text_items` takes the boxes.
fn plan_text<'s>(run: &'s TextRun, style: MathStyle, jobs: &mut Vec<ChildJob<'s>>) {
    let mut stack = vec![run.items().iter()];

    while let Some(mut iter) = stack.pop() {
        while let Some(item) = iter.next() {
            match item {
                TextItem::Math(m) => {
                    jobs.push(tree(m, style));
                }
                TextItem::Nested(_, r) => {
                    stack.push(iter);
                    stack.push(r.items().iter());
                    break;
                }
                _ => {}
            }
        }
    }
}

fn tree(node: &MathNode, style: MathStyle) -> ChildJob<'_> {
    ChildJob {
        src: Src::Tree(node),
        style,
    }
}

fn plan_scripts<'a>(
    style: MathStyle,
    base: &'a MathNode,
    sub: Option<&'a MathNode>,
    exp: Option<&'a MathNode>,
    jobs: &mut Vec<ChildJob<'a>>,
) {
    jobs.push(tree(base, style));
    let script = style.into_script();
    if let Some(exp) = exp {
        jobs.push(tree(exp, script.cramp()));
    }
    if let Some(sub) = sub {
        jobs.push(tree(sub, script));
    }
}

fn plan_large_limits<'a>(
    style: MathStyle,
    limits_in_display: bool,
    lo: &'a Option<Box<MathNode>>,
    hi: &'a Option<Box<MathNode>>,
    jobs: &mut Vec<ChildJob<'a>>,
) {
    let script = style.into_script();
    let use_limits = limits_in_display && style.is_display();
    if let Some(hi) = hi {
        let hi_style = if use_limits { script } else { script.cramp() };
        jobs.push(tree(hi, hi_style));
    }
    if let Some(lo) = lo {
        jobs.push(tree(lo, script));
    }
}

fn prefixed_cell(cell: &MathNode) -> MathNode {
    let empty_ord = MathNode::Row(Vec::new());
    match cell {
        MathNode::Row(items) => {
            let mut with_ord = Vec::with_capacity(items.len().saturating_add(1));
            with_ord.push(empty_ord);
            with_ord.extend(items.iter().cloned());
            MathNode::Row(with_ord)
        }
        _ => MathNode::Row(vec![empty_ord, cell.clone()]),
    }
}

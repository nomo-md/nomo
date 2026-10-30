//! TeX-faithful math box model. Dimensions are [`Dim`](crate::Dim).

mod engine;
mod metrics;
mod numbering;
mod space;
mod style;

pub use engine::{
    layout, layout_with_em_size_pt, layout_with_glyph_fallback, layout_with_max_depth, layout_with_numbering,
    layout_with_numbering_and_em_size_pt,
};
pub use metrics::MathParams;
pub use numbering::{NumberFormat, NumberStyle, NumberingConfig, NumberingState};
pub use style::MathStyle;

use crate::color::Color;
use crate::dim::Dim;
use crate::error::Error;
use crate::font::MathFont;

/// A host-provided glyph. Metrics are normalized em units; the outline uses
/// `units_per_em` font units with an upward Y axis and an origin on the baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackGlyph {
    /// Horizontal advance in em.
    pub advance: Dim,
    /// Ink above the baseline in em.
    pub height: Dim,
    /// Ink below the baseline in em.
    pub depth: Dim,
    /// Font units per em, which must be nonzero.
    pub units_per_em: u16,
    /// Trusted, numeric SVG path data from a font outline, never raw document XML.
    pub outline: std::sync::Arc<str>,
}

/// Optional per-layout missing-glyph resolver. `None` preserves the original error.
pub type GlyphFallback<'a> = dyn Fn(char, crate::TextStyle) -> Result<Option<FallbackGlyph>, Error> + 'a;

/// What a box contains.
///
/// Glyphs carry an OpenType id from the math face. Lists compose children.
/// Color wrappers do not change dimensions. [`BoxContent::Line`] and
/// [`BoxContent::Frame`] are decorations from cancel / boxed constructs.
#[derive(Debug, PartialEq, Eq)]
pub enum BoxContent {
    /// Empty box (strut or placeholder).
    Empty,
    /// Solid rule (fraction bar, vinculum). No glyph.
    Rule,
    /// A single character whose metrics came from the math font.
    ///
    /// Build with [`BoxContent::glyph`]; the variant is non-exhaustive.
    #[non_exhaustive]
    Glyph {
        /// Character.
        ch: char,
        /// OpenType glyph id.
        glyph_id: u16,
        /// Outline scale relative to the em: 1 at text size, smaller at script
        /// and scriptscript size. Box dimensions are already scaled; renderers
        /// draw the outline at this factor.
        scale: Dim,
        /// Host outline; its scale is converted to the primary font units.
        outline: Option<std::sync::Arc<str>>,
    },
    /// Horizontal list. Width is the sum of children.
    HList(Vec<MathBox>),
    /// Vertical list. Height/depth stack on the baseline of the first box.
    VList(Vec<MathBox>),
    /// Horizontal kern (atom spacing, `\hspace`).
    Kern(Dim),
    /// Color wrapper. Dimensions match the inner box. Sets SVG `fill`.
    Color(Color, Box<MathBox>),
    /// Background color (`\colorbox`). Inner glyphs keep the default fill.
    BackColor(Color, Box<MathBox>),
    /// Children share the left edge; each child's [`MathBox::shift`] is its baseline.
    Overlap(Vec<MathBox>),
    /// Diagonal or free line in em, relative to the box left and baseline (`y` up).
    Line {
        /// Start x (em from left).
        x1: Dim,
        /// Start y (em above baseline).
        y1: Dim,
        /// End x.
        x2: Dim,
        /// End y.
        y2: Dim,
        /// Stroke thickness (em).
        thickness: Dim,
    },
    /// Stroked rectangle around the inner box. Inner is laid out at the same origin.
    Frame {
        /// Rule thickness (em).
        thickness: Dim,
        /// Border color. `None` inherits the current fill (`\boxed`).
        stroke: Option<Color>,
        /// Contents inside the frame.
        inner: Box<MathBox>,
    },
}

/// TeX-style box: width, height above baseline, depth below, italic correction.
///
/// # Examples
///
/// ```
/// use latex_rust::{Dim, MathBox};
///
/// let packed = MathBox::hpack(vec![
///     MathBox::rule(Dim::one(), Dim::zero(), Dim::zero()),
///     MathBox::rule(Dim::ratio(1, 2), Dim::zero(), Dim::zero()),
/// ]);
/// assert_eq!(packed.width, Dim::ratio(3, 2));
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct MathBox {
    /// Width.
    pub width: Dim,
    /// Height above the baseline.
    pub height: Dim,
    /// Depth below the baseline.
    pub depth: Dim,
    /// Italic correction.
    pub italic: Dim,
    /// Baseline raise relative to the parent list (positive is up).
    pub shift: Dim,
    /// Payload.
    pub content: BoxContent,
}

impl BoxContent {
    /// Glyph content drawn at `scale` times the em (1 for text size).
    #[must_use]
    pub fn glyph(ch: char, glyph_id: u16, scale: Dim) -> Self {
        Self::Glyph {
            ch,
            glyph_id,
            scale,
            outline: None,
        }
    }
}

impl MathBox {
    /// Zero-size empty box.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            width: Dim::zero(),
            height: Dim::zero(),
            depth: Dim::zero(),
            italic: Dim::zero(),
            shift: Dim::zero(),
            content: BoxContent::Empty,
        }
    }

    /// Rule with explicit dimensions (fraction bar, strut).
    #[must_use]
    pub fn rule(width: Dim, height: Dim, depth: Dim) -> Self {
        Self {
            width,
            height,
            depth,
            italic: Dim::zero(),
            shift: Dim::zero(),
            content: BoxContent::Rule,
        }
    }

    /// Horizontal kern of `width` (zero height and depth).
    #[must_use]
    pub fn kern(width: Dim) -> Self {
        Self {
            width: width.clone(),
            height: Dim::zero(),
            depth: Dim::zero(),
            italic: Dim::zero(),
            shift: Dim::zero(),
            content: BoxContent::Kern(width),
        }
    }

    /// Box from a font glyph. Errors if the face has no glyph for `ch`.
    pub fn from_glyph(font: &MathFont, ch: char) -> Result<Self, Error> {
        let g = font.glyph(ch)?;
        Ok(Self {
            width: g.advance,
            height: g.height,
            depth: g.depth,
            italic: font.italic_correction(g.glyph_id),
            shift: Dim::zero(),
            content: BoxContent::glyph(ch, g.glyph_id, Dim::one()),
        })
    }

    /// Pack boxes in a row. Width sums; height and depth are maxima.
    #[must_use]
    pub fn hpack(children: Vec<Self>) -> Self {
        let mut width = Dim::zero();
        let mut height = Dim::zero();
        let mut depth = Dim::zero();
        for c in &children {
            width = &width + &c.width;
            height = height.max(&c.height);
            depth = depth.max(&c.depth);
        }
        Self {
            width,
            height,
            depth,
            italic: Dim::zero(),
            shift: Dim::zero(),
            content: BoxContent::HList(children),
        }
    }

    /// Pack boxes in a column, first child on the baseline.
    ///
    /// Subsequent children sit below the previous (height + depth stacked).
    #[must_use]
    pub fn vpack(children: Vec<Self>) -> Self {
        if children.is_empty() {
            return Self::empty();
        }
        let mut width = Dim::zero();
        let height = children[0].height.clone();
        let mut depth = children[0].depth.clone();
        for c in children.iter().skip(1) {
            width = width.max(&c.width);
            depth = &depth + &c.height;
            depth = &depth + &c.depth;
        }
        width = width.max(&children[0].width);
        Self {
            width,
            height,
            depth,
            italic: Dim::zero(),
            shift: Dim::zero(),
            content: BoxContent::VList(children),
        }
    }

    /// Raise this box's baseline by `shift` (positive is up).
    #[must_use]
    pub fn with_shift(mut self, shift: Dim) -> Self {
        self.shift = shift;
        self
    }

    /// Gold-stable width/height/depth decimal string.
    #[must_use]
    pub fn dim_gold(&self) -> String {
        format!(
            "w={} h={} d={}",
            self.width.to_dec_string(),
            self.height.to_dec_string(),
            self.depth.to_dec_string()
        )
    }
}

impl Clone for MathBox {
    fn clone(&self) -> Self {
        clone_math_box(self)
    }
}

impl Clone for BoxContent {
    fn clone(&self) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Rule => Self::Rule,
            Self::Kern(w) => Self::Kern(w.clone()),
            Self::Glyph {
                ch,
                glyph_id,
                scale,
                outline,
            } => Self::Glyph { ch: *ch, glyph_id: *glyph_id, scale: scale.clone(), outline: outline.clone() },
            Self::Line {
                x1,
                y1,
                x2,
                y2,
                thickness,
            } => Self::Line {
                x1: x1.clone(),
                y1: y1.clone(),
                x2: x2.clone(),
                y2: y2.clone(),
                thickness: thickness.clone(),
            },
            Self::HList(children) => Self::HList(children.iter().map(MathBox::clone).collect()),
            Self::VList(children) => Self::VList(children.iter().map(MathBox::clone).collect()),
            Self::Overlap(children) => Self::Overlap(children.iter().map(MathBox::clone).collect()),
            Self::Color(color, inner) => Self::Color(*color, Box::new(MathBox::clone(inner))),
            Self::BackColor(color, inner) => {
                Self::BackColor(*color, Box::new(MathBox::clone(inner)))
            }
            Self::Frame {
                thickness,
                stroke,
                inner,
            } => Self::Frame {
                thickness: thickness.clone(),
                stroke: *stroke,
                inner: Box::new(MathBox::clone(inner)),
            },
        }
    }
}

impl Drop for MathBox {
    fn drop(&mut self) {
        // Derived drop walks `HList` / `Overlap` / `Frame` on the native
        // stack. A fraction chain the parser accepts is that deep, so the
        // children move to a heap stack first.
        let mut stack = Vec::new();
        drain_box(self, &mut stack);
        while let Some(mut child) = stack.pop() {
            drain_box(&mut child, &mut stack);
        }
    }
}

fn drain_box(bx: &mut MathBox, out: &mut Vec<MathBox>) {
    match &mut bx.content {
        BoxContent::HList(children)
        | BoxContent::VList(children)
        | BoxContent::Overlap(children) => out.append(children),
        BoxContent::Color(_, inner) | BoxContent::BackColor(_, inner) => {
            out.push(std::mem::replace(inner.as_mut(), MathBox::empty()));
        }
        BoxContent::Frame { inner, .. } => {
            out.push(std::mem::replace(inner.as_mut(), MathBox::empty()));
        }
        BoxContent::Empty
        | BoxContent::Rule
        | BoxContent::Kern(_)
        | BoxContent::Glyph { .. }
        | BoxContent::Line { .. } => {}
    }
}

enum CloneTask<'a> {
    Box(&'a MathBox),
    List {
        width: Dim,
        height: Dim,
        depth: Dim,
        italic: Dim,
        shift: Dim,
        kind: ListKind,
        n: usize,
    },
    Wrap {
        width: Dim,
        height: Dim,
        depth: Dim,
        italic: Dim,
        shift: Dim,
        kind: WrapKind,
    },
}

enum ListKind {
    H,
    V,
    Overlap,
}

enum WrapKind {
    Color(Color),
    BackColor(Color),
    Frame {
        thickness: Dim,
        stroke: Option<Color>,
    },
}

fn clone_math_box(root: &MathBox) -> MathBox {
    let mut tasks = vec![CloneTask::Box(root)];
    let mut done: Vec<MathBox> = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            CloneTask::Box(bx) => push_box_task(bx, &mut tasks, &mut done),
            CloneTask::List {
                width,
                height,
                depth,
                italic,
                shift,
                kind,
                n,
            } => {
                let start = done.len().saturating_sub(n);
                let children = done.split_off(start);
                let content = match kind {
                    ListKind::H => BoxContent::HList(children),
                    ListKind::V => BoxContent::VList(children),
                    ListKind::Overlap => BoxContent::Overlap(children),
                };
                done.push(MathBox {
                    width,
                    height,
                    depth,
                    italic,
                    shift,
                    content,
                });
            }
            CloneTask::Wrap {
                width,
                height,
                depth,
                italic,
                shift,
                kind,
            } => {
                let inner = done.pop().unwrap_or_else(MathBox::empty);
                let content = match kind {
                    WrapKind::Color(c) => BoxContent::Color(c, Box::new(inner)),
                    WrapKind::BackColor(c) => BoxContent::BackColor(c, Box::new(inner)),
                    WrapKind::Frame { thickness, stroke } => BoxContent::Frame {
                        thickness,
                        stroke,
                        inner: Box::new(inner),
                    },
                };
                done.push(MathBox {
                    width,
                    height,
                    depth,
                    italic,
                    shift,
                    content,
                });
            }
        }
    }
    done.pop().unwrap_or_else(MathBox::empty)
}

fn push_box_task<'a>(bx: &'a MathBox, tasks: &mut Vec<CloneTask<'a>>, done: &mut Vec<MathBox>) {
    let width = bx.width.clone();
    let height = bx.height.clone();
    let depth = bx.depth.clone();
    let italic = bx.italic.clone();
    let shift = bx.shift.clone();
    match &bx.content {
        BoxContent::Empty => done.push(MathBox {
            width,
            height,
            depth,
            italic,
            shift,
            content: BoxContent::Empty,
        }),
        BoxContent::Rule => done.push(MathBox {
            width,
            height,
            depth,
            italic,
            shift,
            content: BoxContent::Rule,
        }),
        BoxContent::Kern(w) => done.push(MathBox {
            width,
            height,
            depth,
            italic,
            shift,
            content: BoxContent::Kern(w.clone()),
        }),
        BoxContent::Glyph {
            ch,
            glyph_id,
            scale,
            outline,
        } => done.push(MathBox {
            width,
            height,
            depth,
            italic,
            shift,
            content: BoxContent::Glyph { ch: *ch, glyph_id: *glyph_id, scale: scale.clone(), outline: outline.clone() },
        }),
        BoxContent::Line {
            x1,
            y1,
            x2,
            y2,
            thickness,
        } => done.push(MathBox {
            width,
            height,
            depth,
            italic,
            shift,
            content: BoxContent::Line {
                x1: x1.clone(),
                y1: y1.clone(),
                x2: x2.clone(),
                y2: y2.clone(),
                thickness: thickness.clone(),
            },
        }),
        BoxContent::HList(children) => {
            tasks.push(CloneTask::List {
                width,
                height,
                depth,
                italic,
                shift,
                kind: ListKind::H,
                n: children.len(),
            });
            for child in children.iter().rev() {
                tasks.push(CloneTask::Box(child));
            }
        }
        BoxContent::VList(children) => {
            tasks.push(CloneTask::List {
                width,
                height,
                depth,
                italic,
                shift,
                kind: ListKind::V,
                n: children.len(),
            });
            for child in children.iter().rev() {
                tasks.push(CloneTask::Box(child));
            }
        }
        BoxContent::Overlap(children) => {
            tasks.push(CloneTask::List {
                width,
                height,
                depth,
                italic,
                shift,
                kind: ListKind::Overlap,
                n: children.len(),
            });
            for child in children.iter().rev() {
                tasks.push(CloneTask::Box(child));
            }
        }
        BoxContent::Color(c, inner) => {
            tasks.push(CloneTask::Wrap {
                width,
                height,
                depth,
                italic,
                shift,
                kind: WrapKind::Color(*c),
            });
            tasks.push(CloneTask::Box(inner));
        }
        BoxContent::BackColor(c, inner) => {
            tasks.push(CloneTask::Wrap {
                width,
                height,
                depth,
                italic,
                shift,
                kind: WrapKind::BackColor(*c),
            });
            tasks.push(CloneTask::Box(inner));
        }
        BoxContent::Frame {
            thickness,
            stroke,
            inner,
        } => {
            tasks.push(CloneTask::Wrap {
                width,
                height,
                depth,
                italic,
                shift,
                kind: WrapKind::Frame {
                    thickness: thickness.clone(),
                    stroke: *stroke,
                },
            });
            tasks.push(CloneTask::Box(inner));
        }
    }
}

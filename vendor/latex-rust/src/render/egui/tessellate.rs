//! TrueType outline → triangle mesh in font units (y-up).
//!
//! The scaler delivers `f32` outline points; those bits become [`Dim`]
//! immediately, then integer millifont-units for flatten / earcut (render path,
//! not layout). Vertices are stored as [`Dim`] for emission.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use skrifa::outline::OutlinePen;

use crate::dim::Dim;
use crate::error::{Error, FontError};
use crate::font::MathFont;

/// Sub-font-unit scale for integer flatten / earcut.
const SCALE: i64 = 64;
const FLAT_STEPS: i64 = 8;

type Ipt = (i64, i64);

/// Cached tessellation: font-unit vertices (y-up) and triangle indices.
#[derive(Clone, Debug)]
pub(super) struct GlyphTris {
    pub vertices: Vec<(Dim, Dim)>,
    pub indices: Vec<u32>,
}

fn glyph_cache() -> &'static Mutex<HashMap<u16, GlyphTris>> {
    static CACHE: OnceLock<Mutex<HashMap<u16, GlyphTris>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Tessellate `glyph_id`, reusing a process-wide triangle cache on later calls.
pub(super) fn tessellate(font: &MathFont, glyph_id: u16) -> Result<GlyphTris, Error> {
    if let Ok(guard) = glyph_cache().lock() {
        if let Some(t) = guard.get(&glyph_id) {
            return Ok(t.clone());
        }
    }
    let tris = tessellate_uncached(font, glyph_id)?;
    if let Ok(mut guard) = glyph_cache().lock() {
        guard.insert(glyph_id, tris.clone());
    }
    Ok(tris)
}

fn tessellate_uncached(font: &MathFont, glyph_id: u16) -> Result<GlyphTris, Error> {
    let mut b = ContourBuilder::new();
    if !font.draw_glyph(glyph_id, &mut b) {
        return Err(FontError::MissingGlyph { ch: '\u{FFFD}' }.into());
    }
    b.close_current();
    let contours = b.contours;
    if contours.is_empty() {
        return Err(FontError::MissingGlyph { ch: '\u{FFFD}' }.into());
    }
    let (vertices, indices) = match ear_mesh(contours.clone()) {
        Some((points, indices)) => (points.into_iter().map(from_fix).collect(), indices),
        None => {
            let t = scanline_mesh(&contours)?;
            (t.vertices, t.indices)
        }
    };
    if indices.is_empty() {
        return Err(Error::Unsupported {
            what: "glyph tessellation produced no triangles".into(),
        });
    }
    Ok(GlyphTris { vertices, indices })
}

fn tessellation_error() -> Error {
    Error::Unsupported {
        what: "glyph tessellation".into(),
    }
}

fn tessellation_index_error() -> Error {
    Error::Unsupported {
        what: "glyph tessellation exceeds u32 index range".into(),
    }
}

fn tessellation_index(index: usize) -> Result<u32, Error> {
    u32::try_from(index).map_err(|_| tessellation_index_error())
}

fn tessellation_index_offset(base: u32, offset: u32) -> Result<u32, Error> {
    base.checked_add(offset)
        .ok_or_else(tessellation_index_error)
}

fn from_fix(p: Ipt) -> (Dim, Dim) {
    let s = Dim::from_i64(SCALE);
    (Dim::from_i64(p.0) / &s, Dim::from_i64(p.1) / s)
}

fn floor_i64(d: &Dim) -> i64 {
    if d.is_nan() {
        return 0;
    }
    let neg = matches!(d.cmp(&Dim::zero()), Some(core::cmp::Ordering::Less));
    let abs = d.abs();
    let n = i64::from(abs.floor_to_u32().unwrap_or(0));
    if !neg {
        n
    } else if abs.eq_dim(&Dim::from_i64(n)) {
        -n
    } else {
        -n - 1
    }
}

fn to_fix(x: f32) -> i64 {
    let d = Dim::from_ieee32_bits(x.to_bits()) * Dim::from_i64(SCALE);
    floor_i64(&d)
}

struct ContourBuilder {
    contours: Vec<Vec<Ipt>>,
    current: Vec<Ipt>,
    last: Option<Ipt>,
}

impl ContourBuilder {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            current: Vec::new(),
            last: None,
        }
    }

    fn pt(x: f32, y: f32) -> Ipt {
        (to_fix(x), to_fix(y))
    }

    fn push(&mut self, p: Ipt) {
        if let Some(last) = self.current.last() {
            if *last == p {
                return;
            }
        }
        self.current.push(p);
        self.last = Some(p);
    }

    fn close_current(&mut self) {
        if self.current.len() >= 3 {
            if let (Some(&first), Some(&last)) = (self.current.first(), self.current.last()) {
                if first == last {
                    self.current.pop();
                }
            }
            if self.current.len() >= 3 {
                self.contours.push(std::mem::take(&mut self.current));
            } else {
                self.current.clear();
            }
        } else {
            self.current.clear();
        }
        self.last = None;
    }
}

impl OutlinePen for ContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_current();
        let p = Self::pt(x, y);
        self.push(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(Self::pt(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let Some(p0) = self.last else {
            return;
        };
        let p1 = Self::pt(x1, y1);
        let p2 = Self::pt(x, y);
        flatten_quad(p0, p1, p2, self);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let Some(p0) = self.last else {
            return;
        };
        let p1 = Self::pt(x1, y1);
        let p2 = Self::pt(x2, y2);
        let p3 = Self::pt(x, y);
        flatten_cubic(p0, p1, p2, p3, self);
    }

    fn close(&mut self) {
        self.close_current();
    }
}

fn lerp(a: Ipt, b: Ipt, t_num: i64, t_den: i64) -> Ipt {
    (
        a.0 + (b.0 - a.0) * t_num / t_den,
        a.1 + (b.1 - a.1) * t_num / t_den,
    )
}

fn eval_quad(p0: Ipt, p1: Ipt, p2: Ipt, t_num: i64, t_den: i64) -> Ipt {
    let a = lerp(p0, p1, t_num, t_den);
    let b = lerp(p1, p2, t_num, t_den);
    lerp(a, b, t_num, t_den)
}

fn eval_cubic(p0: Ipt, p1: Ipt, p2: Ipt, p3: Ipt, t_num: i64, t_den: i64) -> Ipt {
    let a = lerp(p0, p1, t_num, t_den);
    let b = lerp(p1, p2, t_num, t_den);
    let c = lerp(p2, p3, t_num, t_den);
    let d = lerp(a, b, t_num, t_den);
    let e = lerp(b, c, t_num, t_den);
    lerp(d, e, t_num, t_den)
}

fn flatten_quad(p0: Ipt, p1: Ipt, p2: Ipt, b: &mut ContourBuilder) {
    for i in 1..=FLAT_STEPS {
        b.push(eval_quad(p0, p1, p2, i, FLAT_STEPS));
    }
}

fn flatten_cubic(p0: Ipt, p1: Ipt, p2: Ipt, p3: Ipt, b: &mut ContourBuilder) {
    for i in 1..=FLAT_STEPS {
        b.push(eval_cubic(p0, p1, p2, p3, i, FLAT_STEPS));
    }
}

fn signed_area(poly: &[Ipt]) -> i64 {
    let n = poly.len();
    let mut a = 0i64;
    for i in 0..n {
        let j = (i + 1) % n;
        a += poly[i].0 * poly[j].1 - poly[j].0 * poly[i].1;
    }
    a
}

fn dist2(a: Ipt, b: Ipt) -> i64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx * dx + dy * dy
}

fn point_in_poly(poly: &[Ipt], p: Ipt) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let a_below = a.1 <= p.1;
        let b_above = b.1 > p.1;
        let b_below = b.1 <= p.1;
        let a_above = a.1 > p.1;
        if (a_below && b_above) || (b_below && a_above) {
            let dy = b.1 - a.1;
            if dy == 0 {
                continue;
            }
            let xint = a.0 + (p.1 - a.1) * (b.0 - a.0) / dy;
            if xint > p.0 {
                inside = !inside;
            }
        }
    }
    inside
}

fn combine_contours(contours: Vec<Vec<Ipt>>) -> Result<Vec<Vec<Ipt>>, Error> {
    if contours.is_empty() {
        return Ok(Vec::new());
    }
    let areas: Vec<i64> = contours.iter().map(|c| signed_area(c)).collect();
    let mut outer_idx = Vec::new();
    let mut hole_idx = Vec::new();
    for (i, a) in areas.iter().enumerate() {
        if *a == 0 {
            continue;
        }
        if *a < 0 {
            hole_idx.push(i);
        } else {
            outer_idx.push(i);
        }
    }
    if outer_idx.is_empty() && !hole_idx.is_empty() {
        outer_idx = hole_idx;
        hole_idx = Vec::new();
    }
    if outer_idx.is_empty() {
        return Ok(contours);
    }
    let outer_sign_pos = areas[outer_idx[0]] > 0;
    outer_idx.clear();
    hole_idx.clear();
    for (i, a) in areas.iter().enumerate() {
        if *a == 0 {
            continue;
        }
        if (*a > 0) == outer_sign_pos {
            outer_idx.push(i);
        } else {
            hole_idx.push(i);
        }
    }
    let mut outers: Vec<Vec<Ipt>> = outer_idx.into_iter().map(|i| contours[i].clone()).collect();
    for h in hole_idx {
        let hole = &contours[h];
        let Some(&pt) = hole.first() else {
            continue;
        };
        let mut host = 0usize;
        let mut found = false;
        for (oi, outer) in outers.iter().enumerate() {
            if point_in_poly(outer, pt) {
                host = oi;
                found = true;
                break;
            }
        }
        if !found {
            let mut best = 0i64;
            for (oi, outer) in outers.iter().enumerate() {
                let aa = signed_area(outer).abs();
                if aa > best {
                    best = aa;
                    host = oi;
                }
            }
        }
        insert_hole(&mut outers[host], hole);
    }
    Ok(outers)
}

fn insert_hole(outer: &mut Vec<Ipt>, hole: &[Ipt]) {
    if hole.len() < 3 || outer.len() < 3 {
        return;
    }
    let mut best_i = 0usize;
    let mut best_j = 0usize;
    let mut best_d = dist2(outer[0], hole[0]);
    for (i, &op) in outer.iter().enumerate() {
        for (j, &hp) in hole.iter().enumerate() {
            let d = dist2(op, hp);
            if d < best_d {
                best_d = d;
                best_i = i;
                best_j = j;
            }
        }
    }
    let mut spliced = Vec::with_capacity(outer.len() + hole.len() + 2);
    spliced.extend_from_slice(&outer[..=best_i]);
    let hn = hole.len();
    for k in 0..hn {
        spliced.push(hole[(best_j + k) % hn]);
    }
    spliced.push(hole[best_j]);
    spliced.push(outer[best_i]);
    spliced.extend_from_slice(&outer[best_i + 1..]);
    *outer = spliced;
}

fn cross(a: Ipt, b: Ipt, c: Ipt) -> i64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn triangle_contains(a: Ipt, b: Ipt, c: Ipt, p: Ipt) -> bool {
    let c1 = cross(a, b, p);
    let c2 = cross(b, c, p);
    let c3 = cross(c, a, p);
    (c1 > 0 && c2 > 0 && c3 > 0) || (c1 < 0 && c2 < 0 && c3 < 0)
}

fn earcut(poly: &[Ipt]) -> Result<Vec<[u32; 3]>, Error> {
    let n0 = poly.len();
    if n0 < 3 {
        return Ok(Vec::new());
    }
    let area = signed_area(poly);
    let ccw = area > 0;
    let mut idx: Vec<usize> = (0..n0).collect();
    if !ccw {
        idx.reverse();
    }
    let mut tris = Vec::new();
    let mut guard = 0usize;
    let max_guard = n0 * n0 + 8;
    while idx.len() > 3 {
        guard += 1;
        if guard > max_guard {
            return Err(Error::Unsupported {
                what: "glyph tessellation".into(),
            });
        }
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let prev = idx[(i + m - 1) % m];
            let cur = idx[i];
            let next = idx[(i + 1) % m];
            let a = poly[prev];
            let b = poly[cur];
            let c = poly[next];
            if cross(a, b, c) <= 0 {
                continue;
            }
            let mut empty = true;
            for (k, &vi) in idx.iter().enumerate() {
                if k == (i + m - 1) % m || k == i || k == (i + 1) % m {
                    continue;
                }
                if triangle_contains(a, b, c, poly[vi]) {
                    empty = false;
                    break;
                }
            }
            if !empty {
                continue;
            }
            tris.push([
                tessellation_index(prev)?,
                tessellation_index(cur)?,
                tessellation_index(next)?,
            ]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return Err(Error::Unsupported {
                what: "glyph tessellation".into(),
            });
        }
    }
    if idx.len() == 3 {
        tris.push([
            tessellation_index(idx[0])?,
            tessellation_index(idx[1])?,
            tessellation_index(idx[2])?,
        ]);
    }
    Ok(tris)
}

/// Tessellate `contours` by ear clipping, and keep the result only if it can be
/// shown exact.
///
/// Every clip replaces the ring by a triangle plus a smaller ring, so the
/// winding number of the input around any point equals the sum of the
/// triangles' winding numbers there. If every triangle is counter-clockwise
/// (or flat), the triangles therefore cover each point exactly as many times
/// as the outline winds around it: once inside the glyph, never outside.
/// A clockwise triangle means the clipper cut outside the outline. That
/// happens when an ear's closing diagonal runs through another vertex (the
/// ear test only rejects vertices strictly inside the triangle), as on the
/// straight stems of `H` or `π`; the clipper then draws ink outside the glyph
/// or gets stuck with no ear left.
fn ear_mesh(contours: Vec<Vec<Ipt>>) -> Option<(Vec<Ipt>, Vec<u32>)> {
    let polys = combine_contours(contours).ok()?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for poly in polys {
        let tris = earcut(&poly).ok()?;
        let base = tessellation_index(vertices.len()).ok()?;
        if let Some(last) = poly.len().checked_sub(1) {
            let last = tessellation_index(last).ok()?;
            tessellation_index_offset(base, last).ok()?;
        }
        if tris
            .iter()
            .any(|&[a, b, c]| cross(poly[a as usize], poly[b as usize], poly[c as usize]) < 0)
        {
            return None;
        }
        vertices.extend(poly);
        for [a, b, c] in tris {
            indices.push(tessellation_index_offset(base, a).ok()?);
            indices.push(tessellation_index_offset(base, b).ok()?);
            indices.push(tessellation_index_offset(base, c).ok()?);
        }
    }
    Some((vertices, indices))
}

/// An outline edge with `y0 < y1`, and `+1` or `-1` for whether the contour
/// runs up or down along it.
#[derive(Clone, Copy, Debug)]
struct Edge {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    wind: i32,
}

/// An exact rational `n / d` with `d > 0`, in outline units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frac {
    n: i128,
    d: i128,
}

impl Frac {
    fn int(v: i64) -> Self {
        Self {
            n: i128::from(v),
            d: 1,
        }
    }

    fn cmp(self, other: Self) -> Option<core::cmp::Ordering> {
        Some(
            self.n
                .checked_mul(other.d)?
                .cmp(&other.n.checked_mul(self.d)?),
        )
    }

    /// The same value in lowest terms.
    fn reduced(self) -> Self {
        let (mut a, mut b) = (self.n.unsigned_abs(), self.d.unsigned_abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        match i128::try_from(a) {
            Ok(g) if g > 1 => Self {
                n: self.n / g,
                d: self.d / g,
            },
            _ => self,
        }
    }

    fn to_dim(self) -> Option<Dim> {
        let n = i64::try_from(self.n).ok()?;
        let d = i64::try_from(self.d).ok()?;
        let v = Dim::from_i64(n) / Dim::from_i64(d) / Dim::from_i64(SCALE);
        (!v.is_nan()).then_some(v)
    }
}

impl Edge {
    /// `x` on the edge at height `y`.
    fn x_at(&self, y: Frac) -> Option<Frac> {
        let dy = i128::from(self.y1 - self.y0);
        let dx = i128::from(self.x1 - self.x0);
        let rise = y.n.checked_sub(i128::from(self.y0).checked_mul(y.d)?)?;
        let n = i128::from(self.x0)
            .checked_mul(dy)?
            .checked_mul(y.d)?
            .checked_add(rise.checked_mul(dx)?)?;
        Some(Frac {
            n,
            d: dy.checked_mul(y.d)?,
        })
    }

    /// Order of `self` and `other` along `x` at height `y`.
    fn cmp_at(&self, other: &Self, y: Frac) -> Option<core::cmp::Ordering> {
        self.x_at(y)?.cmp(other.x_at(y)?)
    }

    /// Height at which the lines through `self` and `other` cross, if they
    /// are not parallel.
    fn crossing(&self, other: &Self) -> Option<Frac> {
        let (ady, adx) = (i128::from(self.y1 - self.y0), i128::from(self.x1 - self.x0));
        let (bdy, bdx) = (
            i128::from(other.y1 - other.y0),
            i128::from(other.x1 - other.x0),
        );
        let mut d = adx.checked_mul(bdy)?.checked_sub(bdx.checked_mul(ady)?)?;
        if d == 0 {
            return None;
        }
        let mut n = ady
            .checked_mul(bdy)?
            .checked_mul(i128::from(other.x0 - self.x0))?
            .checked_sub(i128::from(other.y0).checked_mul(bdx)?.checked_mul(ady)?)?
            .checked_add(i128::from(self.y0).checked_mul(adx)?.checked_mul(bdy)?)?;
        if d < 0 {
            n = -n;
            d = -d;
        }
        Some(Frac { n, d }.reduced())
    }
}

/// Triangles with exact rational corners, as built by [`scanline`].
#[derive(Default)]
struct FracMesh {
    vertices: Vec<(Frac, Frac)>,
    indices: Vec<u32>,
}

/// Tessellate `contours` exactly under the non-zero winding rule, the rule the
/// SVG and PNG backends fill glyphs with, converted to [`Dim`] vertices.
fn scanline_mesh(contours: &[Vec<Ipt>]) -> Result<GlyphTris, Error> {
    let mesh = scanline(contours).ok_or_else(tessellation_error)?;
    let mut vertices = Vec::with_capacity(mesh.vertices.len());
    for (x, y) in mesh.vertices {
        vertices.push((
            x.to_dim().ok_or_else(tessellation_error)?,
            y.to_dim().ok_or_else(tessellation_error)?,
        ));
    }
    Ok(GlyphTris {
        vertices,
        indices: mesh.indices,
    })
}

/// Cut the outline into horizontal slabs at every vertex height (and at
/// every height where two edges cross). Inside a slab no edge starts, ends or
/// crosses another, so the edges keep one left-to-right order; walking it and
/// summing each edge's direction gives the filled spans, and each span is a
/// trapezoid split into two triangles. Corners are exact rationals, so
/// neighbouring trapezoids meet without gaps or overlaps. `None` if a
/// coordinate overflows.
///
/// This makes more triangles than ear clipping, so it is used only for
/// outlines whose ear clipping cannot be shown exact.
fn scanline(contours: &[Vec<Ipt>]) -> Option<FracMesh> {
    let mut edges = Vec::new();
    let mut heights = Vec::new();
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (p, q) = (c[i], c[(i + 1) % n]);
            heights.push(p.1);
            if p.1 == q.1 {
                continue;
            }
            let (lo, hi, wind) = if p.1 < q.1 { (p, q, 1) } else { (q, p, -1) };
            edges.push(Edge {
                x0: lo.0,
                y0: lo.1,
                x1: hi.0,
                y1: hi.1,
                wind,
            });
        }
    }
    heights.sort_unstable();
    heights.dedup();
    let mut out = FracMesh::default();
    for w in heights.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let active: Vec<Edge> = edges
            .iter()
            .filter(|e| e.y0 <= lo && e.y1 >= hi)
            .copied()
            .collect();
        slab(active, Frac::int(lo), Frac::int(hi), &mut out)?;
    }
    Some(out)
}

/// Fill the slab between heights `lo` and `hi`, splitting it where edges
/// cross.
fn slab(mut order: Vec<Edge>, mut lo: Frac, hi: Frac, out: &mut FracMesh) -> Option<()> {
    use core::cmp::Ordering;
    loop {
        let mut overflow = false;
        order.sort_by(|a, b| {
            let o = a.cmp_at(b, lo).and_then(|o| Some(o.then(a.cmp_at(b, hi)?)));
            o.unwrap_or_else(|| {
                overflow = true;
                Ordering::Equal
            })
        });
        if overflow {
            return None;
        }
        // Two edges cross inside the slab exactly when they are neighbours in
        // this order at `lo` and swapped at `hi`; cut at the lowest crossing.
        let mut cut: Option<Frac> = None;
        for pair in order.windows(2) {
            if pair[0].cmp_at(&pair[1], hi)? != Ordering::Greater {
                continue;
            }
            let Some(y) = pair[0].crossing(&pair[1]) else {
                continue;
            };
            let inside = lo.cmp(y)? == Ordering::Less && y.cmp(hi)? == Ordering::Less;
            let lower = match cut {
                Some(c) => y.cmp(c)? == Ordering::Less,
                None => true,
            };
            if inside && lower {
                cut = Some(y);
            }
        }
        match cut {
            Some(y) => {
                spans(&order, lo, y, out)?;
                lo = y;
            }
            None => return spans(&order, lo, hi, out),
        }
    }
}

/// Emit the filled spans of one slab whose edges keep the order `order`.
fn spans(order: &[Edge], lo: Frac, hi: Frac, out: &mut FracMesh) -> Option<()> {
    let mut wind = 0i32;
    let mut left: Option<&Edge> = None;
    for e in order {
        let before = wind;
        wind += e.wind;
        if before == 0 && wind != 0 {
            left = Some(e);
        } else if before != 0 && wind == 0 {
            trapezoid(left?, e, lo, hi, out)?;
        }
    }
    Some(())
}

/// Two counter-clockwise triangles for the span between edges `l` and `r`
/// from height `lo` to `hi`, leaving out a triangle of zero area where the
/// span narrows to a point.
fn trapezoid(l: &Edge, r: &Edge, lo: Frac, hi: Frac, out: &mut FracMesh) -> Option<()> {
    use core::cmp::Ordering;
    let (bl, br) = (l.x_at(lo)?.reduced(), r.x_at(lo)?.reduced());
    let (tl, tr) = (l.x_at(hi)?.reduced(), r.x_at(hi)?.reduced());
    let bottom = br.cmp(bl)? == Ordering::Greater;
    let top = tr.cmp(tl)? == Ordering::Greater;
    if !bottom && !top {
        return Some(());
    }
    let base = tessellation_index(out.vertices.len()).ok()?;
    let i1 = tessellation_index_offset(base, 1).ok()?;
    let i2 = tessellation_index_offset(base, 2).ok()?;
    let i3 = tessellation_index_offset(base, 3).ok()?;
    out.vertices
        .extend([(bl, lo), (br, lo), (tr, hi), (tl, hi)]);
    if bottom {
        out.indices.extend([base, i1, i2]);
    }
    if top {
        out.indices.extend([base, i2, i3]);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    //! Exact checks: every orientation test below is done in integers.

    use core::cmp::Ordering;

    use super::*;

    type P = (Frac, Frac);

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn tessellation_index_rejects_u32_overflow() {
        assert!(tessellation_index(usize::MAX).is_err());
    }

    #[test]
    fn tessellation_index_offset_rejects_u32_overflow() {
        assert!(tessellation_index_offset(u32::MAX, 1).is_err());
    }

    fn outline(font: &MathFont, glyph_id: u16) -> Vec<Vec<Ipt>> {
        let mut b = ContourBuilder::new();
        if !font.draw_glyph(glyph_id, &mut b) {
            return Vec::new();
        }
        b.close_current();
        b.contours
    }

    fn int_pt(p: Ipt) -> P {
        (Frac::int(p.0), Frac::int(p.1))
    }

    fn sub(a: Frac, b: Frac) -> Frac {
        Frac {
            n: a.n * b.d - b.n * a.d,
            d: a.d * b.d,
        }
        .reduced()
    }

    fn mul(a: Frac, b: Frac) -> Frac {
        Frac {
            n: a.n * b.n,
            d: a.d * b.d,
        }
        .reduced()
    }

    /// Sign of the cross product `(b - a) x (c - a)`, exactly.
    fn orient(a: P, b: P, c: P) -> Ordering {
        let t1 = mul(sub(b.0, a.0), sub(c.1, a.1));
        let t2 = mul(sub(b.1, a.1), sub(c.0, a.0));
        t1.cmp(t2).expect("no overflow")
    }

    /// Triangles of the mesh `tessellate_uncached` builds, with exact corners.
    fn exact_mesh(contours: &[Vec<Ipt>]) -> (Vec<P>, Vec<u32>, bool) {
        match ear_mesh(contours.to_vec()) {
            Some((pts, idx)) => (pts.into_iter().map(int_pt).collect(), idx, true),
            None => {
                let m = scanline(contours).expect("scanline");
                (m.vertices, m.indices, false)
            }
        }
    }

    /// Non-zero winding of the outline around `q`.
    fn winding(contours: &[Vec<Ipt>], q: P) -> i32 {
        let mut w = 0;
        for c in contours {
            for i in 0..c.len() {
                let (a, b) = (int_pt(c[i]), int_pt(c[(i + 1) % c.len()]));
                let a_le = a.1.cmp(q.1) != Some(Ordering::Greater);
                let b_le = b.1.cmp(q.1) != Some(Ordering::Greater);
                if a_le && !b_le && orient(a, b, q) == Ordering::Greater {
                    w += 1;
                } else if !a_le && b_le && orient(a, b, q) == Ordering::Less {
                    w -= 1;
                }
            }
        }
        w
    }

    /// Check on a grid of off-grid sample points that the mesh covers every
    /// point the outline winds around exactly once and nothing else. Points on
    /// a triangle edge are skipped.
    fn assert_covers_outline(what: &str, contours: &[Vec<Ipt>], pts: &[P], idx: &[u32], n: i64) {
        let all = contours.iter().flatten();
        let (x0, x1) = all
            .clone()
            .fold((i64::MAX, i64::MIN), |(l, h), p| (l.min(p.0), h.max(p.0)));
        let (y0, y1) = all.fold((i64::MAX, i64::MIN), |(l, h), p| (l.min(p.1), h.max(p.1)));
        let pad = 256;
        let mut inside_seen = 0;
        for i in 0..n {
            for j in 0..n {
                let gx = x0 - pad + (x1 - x0 + 2 * pad) * i / n;
                let gy = y0 - pad + (y1 - y0 + 2 * pad) * j / n;
                let q = (
                    Frac {
                        n: i128::from(gx) * 97 + 31,
                        d: 97,
                    },
                    Frac {
                        n: i128::from(gy) * 89 + 53,
                        d: 89,
                    },
                );
                let mut covered = 0;
                let mut on_edge = false;
                for t in idx.chunks(3) {
                    let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
                    let o = [orient(a, b, q), orient(b, c, q), orient(c, a, q)];
                    if o.contains(&Ordering::Equal) {
                        on_edge = true;
                        break;
                    }
                    if o.iter().all(|&s| s == Ordering::Greater) {
                        covered += 1;
                    }
                }
                if on_edge {
                    continue;
                }
                let inside = i32::from(winding(contours, q) != 0);
                inside_seen += inside;
                assert_eq!(covered, inside, "{what}: grid point ({i}, {j})");
            }
        }
        assert!(inside_seen > 0, "{what}: no sample point landed inside");
    }

    fn glyph(font: &MathFont, ch: char) -> u16 {
        font.glyph(ch).expect("glyph in STIX Two Math").glyph_id
    }

    /// Glyphs whose ear clipping got stuck (the "glyph tessellation" error),
    /// then glyphs where it finished with a clockwise triangle that put ink
    /// outside the glyph.
    const BROKEN: &[char] = &[
        'H',
        '\u{1D461}',
        '\u{03C0}',
        '\u{03C3}',
        '#',
        '$',
        '\u{20AC}',
        '\u{22A3}',
        '\u{266E}',
        '\u{1D407}',
        '\u{1D6D1}',
        '\u{1D6BD}',
        '\u{1D539}',
        '\u{23E9}',
        '\u{25A9}',
        'B',
        'k',
        '\u{03C4}',
        '\u{21A6}',
    ];

    #[test]
    fn known_broken_glyphs_cover_exactly_their_outline() {
        let font = MathFont::stix_two_math().expect("STIX Two Math");
        for &ch in BROKEN {
            let gid = glyph(&font, ch);
            let t = tessellate_uncached(&font, gid)
                .unwrap_or_else(|e| panic!("U+{:04X}: {e:?}", ch as u32));
            assert!(!t.indices.is_empty(), "U+{:04X}", ch as u32);
            let contours = outline(&font, gid);
            let (pts, idx, _) = exact_mesh(&contours);
            assert_covers_outline(&format!("U+{:04X}", ch as u32), &contours, &pts, &idx, 20);
        }
    }

    #[test]
    fn every_glyph_in_the_embedded_font_tessellates() {
        let font = MathFont::stix_two_math().expect("STIX Two Math");
        let (mut ear, mut scan) = (0, 0);
        for gid in 0..font.glyph_count() {
            let contours = outline(&font, gid);
            if contours.is_empty() {
                continue;
            }
            let t =
                tessellate_uncached(&font, gid).unwrap_or_else(|e| panic!("glyph {gid}: {e:?}"));
            assert!(!t.indices.is_empty(), "glyph {gid}");
            let (pts, idx, by_ears) = exact_mesh(&contours);
            assert_eq!(idx, t.indices, "glyph {gid}");
            for k in idx.chunks(3) {
                let o = orient(pts[k[0] as usize], pts[k[1] as usize], pts[k[2] as usize]);
                assert_ne!(o, Ordering::Less, "glyph {gid}: clockwise triangle");
            }
            if by_ears {
                // Integer corners: the covered area must equal the outline's.
                let pts: Vec<Ipt> = pts.iter().map(|p| (p.0.n as i64, p.1.n as i64)).collect();
                let sum: i64 = idx
                    .chunks(3)
                    .map(|k| cross(pts[k[0] as usize], pts[k[1] as usize], pts[k[2] as usize]))
                    .sum();
                let want: i64 = contours.iter().map(|c| signed_area(c)).sum();
                assert_eq!(sum, want.abs(), "glyph {gid}");
                ear += 1;
            } else {
                scan += 1;
            }
        }
        assert!(ear > 6000 && scan > 0, "ear {ear}, scanline {scan}");
    }

    #[test]
    fn scanline_covers_what_ear_clipping_covers() {
        let font = MathFont::stix_two_math().expect("STIX Two Math");
        for ch in ['a', 'g', '8', '\u{2211}', '\u{1D465}'] {
            let contours = outline(&font, glyph(&font, ch));
            assert!(ear_mesh(contours.clone()).is_some(), "U+{:04X}", ch as u32);
            let m = scanline(&contours).expect("scanline");
            assert_covers_outline(
                &format!("U+{:04X}", ch as u32),
                &contours,
                &m.vertices,
                &m.indices,
                16,
            );
        }
    }

    #[test]
    fn pinched_contour_is_filled_on_both_sides() {
        // One contour visiting (320, 320) twice: two triangles touching at a
        // point, as in U+23E9. Ear clipping cannot be shown exact here.
        let pinch = vec![vec![
            (640, 320),
            (320, 640),
            (320, 320),
            (0, 640),
            (0, 0),
            (320, 320),
            (320, 0),
        ]];
        assert!(ear_mesh(pinch.clone()).is_none());
        let t = scanline_mesh(&pinch).expect("tessellates");
        assert!(!t.indices.is_empty());
        let m = scanline(&pinch).expect("scanline");
        assert_covers_outline("pinch", &pinch, &m.vertices, &m.indices, 24);
    }

    #[test]
    fn crossing_edges_are_split_where_they_cross() {
        // A bow tie: the contour crosses itself at (320, 320).
        let bow = vec![vec![(0, 0), (640, 640), (640, 0), (0, 640)]];
        let m = scanline(&bow).expect("scanline");
        assert!(m
            .vertices
            .iter()
            .any(|v| v.1.cmp(Frac::int(320)) == Some(Ordering::Equal)));
        assert_covers_outline("bow tie", &bow, &m.vertices, &m.indices, 24);
    }
}

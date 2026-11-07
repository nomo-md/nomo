//! Color models for math mode. Channel arithmetic is [`Dim`](crate::Dim) only.
//!
//! SVG `fill` / `stroke`, PNG (`tiny-skia`), and egui (`Color32`) all convert
//! through [`Color::to_rgba8`]. This module resolves a color to 8-bit sRGB so
//! every backend shares one contract.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use crate::dim::Dim;
use crate::error::Error;

// dvips names from drivers.dtx (`%<*dvipsnames>` / dvipsnam.def), LPPL-1.3c
// or later. Lines in the table that start with `#` are the modification
// notice. See NOTICE.
const DVIPS: &str = include_str!("../data/dvipsnames.tsv");

/// 8-bit sRGB color. Values are integers; conversion from unit intervals uses `Dim`.
///
/// SVG `fill` / `stroke`, PNG (`tiny-skia`), and egui (`Color32`) all convert
/// through [`Color::to_rgba8`].
///
/// # Examples
///
/// ```
/// use latex_rust::Color;
///
/// let c = Color::rgb(255, 0, 0);
/// assert_eq!(c.css_hex(), "#ff0000");
/// assert_eq!(c.to_rgba8(), [255, 0, 0, 255]);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// Red 0–255.
    pub r: u8,
    /// Green 0–255.
    pub g: u8,
    /// Blue 0–255.
    pub b: u8,
}

impl Color {
    /// Opaque sRGB triple.
    ///
    /// # Examples
    ///
    /// ```
    /// use latex_rust::Color;
    /// assert_eq!(Color::rgb(0, 0, 0).css_hex(), "#000000");
    /// ```
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// CSS hex `#rrggbb`.
    #[must_use]
    pub fn css_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Opaque sRGB bytes for PNG (`tiny-skia`) and egui (`Color32`) backends.
    #[must_use]
    pub const fn to_rgba8(self) -> [u8; 4] {
        [self.r, self.g, self.b, 255]
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.css_hex())
    }
}

const BASE: &[(&str, Color)] = &[
    ("black", Color::rgb(0, 0, 0)),
    ("white", Color::rgb(255, 255, 255)),
    ("red", Color::rgb(255, 0, 0)),
    ("green", Color::rgb(0, 255, 0)),
    ("blue", Color::rgb(0, 0, 255)),
    ("cyan", Color::rgb(0, 255, 255)),
    ("magenta", Color::rgb(255, 0, 255)),
    ("yellow", Color::rgb(255, 255, 0)),
];

/// Named-color registry, including `\definecolor` overrides.
///
/// # Examples
///
/// ```
/// use latex_rust::ColorTable;
///
/// let mut t = ColorTable::new();
/// assert!(!t.is_empty());
/// t.define("ok", "named", "red").unwrap();
/// assert_eq!(t.get("ok").unwrap().css_hex(), "#ff0000");
/// ```
#[derive(Clone, Debug)]
pub struct ColorTable {
    named: BTreeMap<String, Color>,
}

impl Default for ColorTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorTable {
    /// Standard LaTeX names plus the 68 dvipsnames (CMYK → sRGB via `Dim`).
    #[must_use]
    pub fn new() -> Self {
        builtin().clone()
    }

    /// A table with no names, as a cheap placeholder while the real table is
    /// moved elsewhere.
    pub(crate) fn empty() -> Self {
        Self {
            named: BTreeMap::new(),
        }
    }

    /// Number of registered names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.named.len()
    }

    /// True when no names are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.named.is_empty()
    }

    /// Look up a named color. Unknown names are [`Error::Unsupported`].
    pub fn get(&self, name: &str) -> Result<Color, Error> {
        self.named
            .get(name)
            .copied()
            .ok_or_else(|| Error::Unsupported {
                what: format!("named color {name}"),
            })
    }

    /// `\definecolor{name}{model}{spec}`.
    pub fn define(&mut self, name: &str, model: &str, spec: &str) -> Result<Color, Error> {
        let c = parse_color_spec(model, spec, Some(self))?;
        self.named.insert(name.to_string(), c);
        Ok(c)
    }
}

fn dvipsnames() -> &'static [(String, Color)] {
    static T: OnceLock<Vec<(String, Color)>> = OnceLock::new();
    T.get_or_init(load_dvips).as_slice()
}

fn load_dvips() -> Vec<(String, Color)> {
    let mut out = Vec::new();
    let mut lines = DVIPS.lines().filter(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    });
    let header = lines.next().expect("dvipsnames header");
    assert_eq!(header, "name\tc\tm\ty\tk", "dvipsnames.tsv schema");
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut cols = line.split('\t');
        let name = cols.next().expect("name").to_string();
        let c = cols.next().expect("c");
        let m = cols.next().expect("m");
        let y = cols.next().expect("y");
        let k = cols.next().expect("k");
        let color = cmyk_to_rgb(
            &Dim::parse(c),
            &Dim::parse(m),
            &Dim::parse(y),
            &Dim::parse(k),
        )
        .expect("dvipsnames cmyk");
        out.push((name, color));
    }
    assert_eq!(out.len(), 68, "dvipsnames must be the full 68-color set");
    out
}

/// Parse `{model}{spec}` as in `\definecolor` / `\color[model]{spec}`.
///
/// `named` uses `table` when provided, otherwise a fresh standard table.
///
/// # Arguments
///
/// * `model` — `named`, `rgb`, `RGB`, `HTML`, `cmyk`, or `gray`.
/// * `spec` — model-specific components (comma-separated, or a name / hex).
/// * `table` — optional named-color registry for the `named` model.
///
/// # Returns
///
/// An opaque sRGB [`Color`].
///
/// # Errors
///
/// * [`Error::Unsupported`] — unknown model or unknown named color.
/// * [`Error::Malformed`] — wrong component count or out-of-range value.
///
/// # Examples
///
/// ```
/// use latex_rust::parse_color_spec;
///
/// let c = parse_color_spec("RGB", "255,0,0", None).unwrap();
/// assert_eq!(c.css_hex(), "#ff0000");
/// ```
pub fn parse_color_spec(
    model: &str,
    spec: &str,
    table: Option<&ColorTable>,
) -> Result<Color, Error> {
    let model = model.trim();
    let spec = spec.trim();
    match model {
        "named" => match table {
            Some(t) => t.get(spec),
            None => builtin().get(spec),
        },
        "rgb" => {
            let v = unit_components(spec, 3)?;
            Ok(Color::rgb(
                unit_to_u8(&v[0])?,
                unit_to_u8(&v[1])?,
                unit_to_u8(&v[2])?,
            ))
        }
        "RGB" => {
            let v = unit_components(spec, 3)?;
            Ok(Color::rgb(
                byte_channel(&v[0])?,
                byte_channel(&v[1])?,
                byte_channel(&v[2])?,
            ))
        }
        "HTML" => parse_html(spec),
        "cmyk" => {
            let v = unit_components(spec, 4)?;
            cmyk_to_rgb(&v[0], &v[1], &v[2], &v[3])
        }
        "gray" => {
            let v = unit_components(spec, 1)?;
            let g = unit_to_u8(&v[0])?;
            Ok(Color::rgb(g, g, g))
        }
        other => Err(Error::Unsupported {
            what: format!("color model {other}"),
        }),
    }
}

fn builtin() -> &'static ColorTable {
    static T: OnceLock<ColorTable> = OnceLock::new();
    T.get_or_init(|| {
        let mut named = BTreeMap::new();
        for (n, c) in BASE {
            named.insert((*n).to_string(), *c);
        }
        for (n, c) in dvipsnames() {
            named.insert(n.clone(), *c);
        }
        ColorTable { named }
    })
}

/// Named color from the standard table (`red`, `RoyalBlue`, …).
///
/// # Arguments
///
/// * `name` — LaTeX or dvipsnames identifier (case-sensitive as stored).
///
/// # Returns
///
/// The registered sRGB color.
///
/// # Errors
///
/// [`Error::Unsupported`] when the name is not in the table.
///
/// # Examples
///
/// ```
/// use latex_rust::named_color;
///
/// assert_eq!(named_color("red").unwrap().css_hex(), "#ff0000");
/// assert!(named_color("not-a-color").is_err());
/// ```
pub fn named_color(name: &str) -> Result<Color, Error> {
    builtin().get(name)
}

fn parse_html(spec: &str) -> Result<Color, Error> {
    let s = spec.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(malformed(format!("HTML color {spec}")));
    }
    let n = u32::from_str_radix(s, 16).map_err(|_| malformed(format!("HTML color {spec}")))?;
    Ok(Color::rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

fn unit_components(spec: &str, n: usize) -> Result<Vec<Dim>, Error> {
    let parts: Vec<&str> = spec
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != n {
        return Err(malformed(format!(
            "color spec `{spec}` (need {n} components)"
        )));
    }
    Ok(parts.iter().map(|p| Dim::parse(p)).collect())
}

fn cmyk_to_rgb(c: &Dim, m: &Dim, y: &Dim, k: &Dim) -> Result<Color, Error> {
    let one = Dim::one();
    let r = &(&one - c) * &(&one - k);
    let g = &(&one - m) * &(&one - k);
    let b = &(&one - y) * &(&one - k);
    Ok(Color::rgb(
        unit_to_u8(&r)?,
        unit_to_u8(&g)?,
        unit_to_u8(&b)?,
    ))
}

fn unit_to_u8(d: &Dim) -> Result<u8, Error> {
    if d.is_nan() {
        return Err(malformed("color component NaN"));
    }
    let zero = Dim::zero();
    let one = Dim::one();
    let clamped = if matches!(d.cmp(&zero), Some(core::cmp::Ordering::Less)) {
        zero
    } else if matches!(d.cmp(&one), Some(core::cmp::Ordering::Greater)) {
        one
    } else {
        d.clone()
    };
    let scaled = clamped * Dim::from_i64(255);
    let rounded = &scaled + &Dim::ratio(1, 2);
    Ok(floor_u8(&rounded))
}

fn byte_channel(d: &Dim) -> Result<u8, Error> {
    if d.is_nan() {
        return Err(malformed("RGB component NaN"));
    }
    let zero = Dim::zero();
    let max = Dim::from_i64(255);
    if matches!(d.cmp(&zero), Some(core::cmp::Ordering::Less))
        || matches!(d.cmp(&max), Some(core::cmp::Ordering::Greater))
    {
        return Err(malformed("RGB component out of 0..255"));
    }
    let rounded = d + &Dim::ratio(1, 2);
    Ok(floor_u8(&rounded))
}

fn malformed(what: impl Into<String>) -> Error {
    Error::Malformed { what: what.into() }
}

fn floor_u8(d: &Dim) -> u8 {
    let mut ans = 0u8;
    let mut bit = 128u8;
    while bit > 0 {
        let cand = ans.saturating_add(bit);
        if Dim::from_i64(i64::from(cand))
            .cmp(d)
            .is_some_and(|o| o != core::cmp::Ordering::Greater)
        {
            ans = cand;
        }
        bit /= 2;
    }
    ans
}

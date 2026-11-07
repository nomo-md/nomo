//! OpenType math font metrics. Integer font units → [`Dim`](crate::Dim).

mod math;

use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::*;
use skrifa::raw::tables::gsub::SubstitutionSubtables;
use skrifa::raw::TableProvider;
use skrifa::MetadataProvider;

use self::math::MathTable;
use crate::dim::Dim;
use crate::error::{Error, FontError};

/// Embedded STIX Two Math Regular 2.13 (SIL OFL 1.1).
///
/// A `static` rather than a `const`, so that the font's bytes are placed in the binary once and every use refers
/// to that one copy. A `const` is inlined at each use site, and a crate that reads these bytes as well as calling
/// [`MathFont::stix_two_math`] carries the 839 KB font twice.
pub static STIX_TWO_MATH_OTF: &[u8] =
    include_bytes!("../../fonts/stix-two-math/STIXTwoMath-Regular.otf");

/// SHA-256 (hex) of [`STIX_TWO_MATH_OTF`]. Locked by gold.
pub const STIX_TWO_MATH_SHA256: &str =
    "f2076b9f1676438439dd41e23676f5ab99056e83d6b8f8c27841591ef2ccfa72";

/// Face name as shipped.
pub const STIX_TWO_MATH_NAME: &str = "STIX Two Math";

/// Horizontal glyph metrics in font units and em.
///
/// # Examples
///
/// ```
/// use latex_rust::MathFont;
///
/// let font = MathFont::stix_two_math().unwrap();
/// let g = font.glyph('x').unwrap();
/// assert_eq!(g.ch, 'x');
/// assert!(!g.advance.is_zero());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphMetrics {
    /// Character requested.
    pub ch: char,
    /// OpenType glyph id.
    pub glyph_id: u16,
    /// Horizontal advance, font units.
    pub advance_fu: u16,
    /// Advance in em.
    pub advance: Dim,
    /// Height above baseline in em (`max(y_max, 0)`).
    pub height: Dim,
    /// Depth below baseline in em (`max(-y_min, 0)`).
    pub depth: Dim,
}

#[derive(Clone, Copy)]
struct ScriptAlternateEntry {
    glyph_id: u16,
    alternates: [Option<u16>; 2],
}

/// Loaded math face.
///
/// Outlines and horizontal metrics come from [`skrifa`]. The MATH table is
/// read in this crate: the `skrifa` 0.39 line this toolchain can build does
/// not expose it.
///
/// # Examples
///
/// ```
/// use latex_rust::MathFont;
///
/// let font = MathFont::stix_two_math().unwrap();
/// assert_eq!(font.units_per_em(), 1000);
/// ```
pub struct MathFont {
    raw: &'static [u8],
    face: FontRef<'static>,
    units_per_em: u16,
    ascender_fu: i16,
    descender_fu: i16,
    x_height_fu: Option<i16>,
    glyph_count: u16,
    math: Option<MathTable>,
    script_alternates: Vec<ScriptAlternateEntry>,
}

impl MathFont {
    /// Load the embedded STIX Two Math Regular face.
    ///
    /// # Errors
    ///
    /// [`crate::FontError::InvalidFace`] if the embedded bytes are not a usable OpenType face.
    ///
    /// # Examples
    ///
    /// ```
    /// use latex_rust::MathFont;
    /// assert!(MathFont::stix_two_math().is_ok());
    /// ```
    pub fn stix_two_math() -> Result<Self, Error> {
        Self::from_bytes(STIX_TWO_MATH_OTF)
    }

    /// Parse OpenType bytes. The buffer must be `'static` so the face can
    /// borrow it for the life of this value.
    pub fn from_bytes(raw: &'static [u8]) -> Result<Self, Error> {
        let face = FontRef::new(raw).map_err(|_| FontError::InvalidFace)?;
        let units_per_em = face
            .head()
            .map_err(|_| FontError::InvalidFace)?
            .units_per_em();
        if units_per_em == 0 {
            return Err(FontError::InvalidFace.into());
        }
        let hhea = face.hhea().map_err(|_| FontError::InvalidFace)?;
        let ascender_fu = hhea.ascender().to_i16();
        let descender_fu = hhea.descender().to_i16();
        let x_height_fu = face.os2().ok().and_then(|os2| os2.sx_height());
        let glyph_count = face
            .maxp()
            .map_err(|_| FontError::InvalidFace)?
            .num_glyphs();
        let math = face
            .table_data(Tag::new(b"MATH"))
            .and_then(|data| MathTable::parse(data.as_bytes()));
        let script_alternates = load_script_alternates(&face);

        Ok(Self {
            raw,
            face,
            units_per_em,
            ascender_fu,
            descender_fu,
            x_height_fu,
            glyph_count,
            math,
            script_alternates,
        })
    }

    pub(crate) fn math_constants(&self) -> Option<math::MathConstants> {
        self.math.as_ref().and_then(MathTable::constants)
    }

    pub(crate) fn script_alternate_glyph_id(&self, glyph_id: u16, script_level: u8) -> Option<u16> {
        let alternate_index = match script_level {
            1 => 0,
            2 => 1,
            _ => return None,
        };

        let index = self
            .script_alternates
            .binary_search_by_key(&glyph_id, |entry| entry.glyph_id)
            .ok()?;

        self.script_alternates[index].alternates[alternate_index]
    }

    /// OpenType bytes this face was parsed from.
    #[must_use]
    pub fn bytes(&self) -> &'static [u8] {
        self.raw
    }

    /// `unitsPerEm` from the `head` table.
    #[must_use]
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// `hhea` ascender in font units.
    #[must_use]
    pub fn ascender_fu(&self) -> i16 {
        self.ascender_fu
    }

    /// `hhea` descender in font units (typically negative).
    #[must_use]
    pub fn descender_fu(&self) -> i16 {
        self.descender_fu
    }

    /// Ascender in em.
    #[must_use]
    pub fn ascender(&self) -> Dim {
        Dim::from_font_units(i64::from(self.ascender_fu), self.units_per_em)
    }

    /// Depth below baseline from `hhea` descender, in em (non-negative).
    #[must_use]
    pub fn descender(&self) -> Dim {
        let d = i64::from(self.descender_fu);
        Dim::from_font_units(-d, self.units_per_em)
    }

    /// Number of glyphs in `maxp`.
    #[must_use]
    pub fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// OS/2 x-height in font units, when the table version records one.
    #[must_use]
    pub fn x_height_fu(&self) -> Option<i16> {
        self.x_height_fu
    }

    /// Metrics for `ch`, or [`FontError::MissingGlyph`].
    pub fn glyph(&self, ch: char) -> Result<GlyphMetrics, Error> {
        let gid = self
            .face
            .charmap()
            .map(ch)
            .ok_or(FontError::MissingGlyph { ch })?;
        self.metrics_for(ch, gid.to_u32())
    }

    /// Metrics for OpenType glyph id `gid`, tagged with `ch` for the box payload.
    pub fn glyph_id(&self, ch: char, gid: u16) -> Result<GlyphMetrics, Error> {
        self.metrics_for(ch, u32::from(gid))
    }

    /// Draw `glyph_id` in font units. `false` when the face has no outline.
    pub(crate) fn draw_glyph(&self, glyph_id: u16, pen: &mut impl OutlinePen) -> bool {
        let Some(outline) = self
            .face
            .outline_glyphs()
            .get(GlyphId::new(u32::from(glyph_id)))
        else {
            return false;
        };
        let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
        outline.draw(settings, pen).is_ok()
    }

    /// MATH italic correction for `glyph_id`, or zero.
    pub fn italic_correction(&self, glyph_id: u16) -> Dim {
        let Some(value) = self.math.as_ref().and_then(|math| math.italic(glyph_id)) else {
            return Dim::zero();
        };
        Dim::from_font_units(i64::from(value), self.units_per_em)
    }

    /// MATH top-accent attachment (em from glyph left), if present.
    pub fn top_accent_attachment(&self, glyph_id: u16) -> Option<Dim> {
        let value = self.math.as_ref()?.top_accent(glyph_id)?;
        Some(Dim::from_font_units(i64::from(value), self.units_per_em))
    }

    /// Horizontal glyph-assembly parts: `(gid, start_connector, end_connector, advance, extender)`.
    /// Lengths are font units.
    pub fn horizontal_assembly_parts(&self, glyph_id: u16) -> Vec<(u16, u16, u16, u16, bool)> {
        let Some(found) = self
            .math
            .as_ref()
            .and_then(|math| math.construction(glyph_id, true))
        else {
            return Vec::new();
        };
        found
            .parts
            .into_iter()
            .map(|part| {
                (
                    part.glyph_id,
                    part.start_connector,
                    part.end_connector,
                    part.full_advance,
                    part.extender,
                )
            })
            .collect()
    }

    /// Horizontal MATH variants of `glyph_id`, including the base glyph first.
    pub fn horizontal_variants(&self, glyph_id: u16) -> Vec<u16> {
        self.variants(glyph_id, true)
    }

    /// Vertical MATH variants of `glyph_id`, including the base glyph first.
    pub fn vertical_variants(&self, glyph_id: u16) -> Vec<u16> {
        self.variants(glyph_id, false)
    }

    /// SHA-256 hex of the raw face bytes.
    #[must_use]
    pub fn sha256_hex(bytes: &[u8]) -> String {
        let d = crate::hash::sha256(bytes);
        let mut s = String::with_capacity(64);
        for b in d {
            s.push_str(&hex_byte(b));
        }
        s
    }

    fn variants(&self, glyph_id: u16, horizontal: bool) -> Vec<u16> {
        let mut out = vec![glyph_id];
        if let Some(found) = self
            .math
            .as_ref()
            .and_then(|math| math.construction(glyph_id, horizontal))
        {
            out.extend(found.variants);
        }
        out
    }

    fn metrics_for(&self, ch: char, gid: u32) -> Result<GlyphMetrics, Error> {
        let gid_u16 = u16::try_from(gid).map_err(|_| FontError::MissingGlyph { ch })?;
        if gid_u16 >= self.glyph_count {
            return Err(FontError::MissingGlyph { ch }.into());
        }
        let advance_fu = self
            .advance_fu(gid_u16)
            .ok_or(FontError::MissingGlyph { ch })?;
        let (height_fu, depth_fu) = self.ink_extents(gid_u16);
        let upem = self.units_per_em;
        Ok(GlyphMetrics {
            ch,
            glyph_id: gid_u16,
            advance_fu,
            advance: Dim::from_font_units(i64::from(advance_fu), upem),
            height: Dim::from_font_units(height_fu, upem),
            depth: Dim::from_font_units(depth_fu, upem),
        })
    }

    fn advance_fu(&self, gid: u16) -> Option<u16> {
        let hmtx = self.face.hmtx().ok()?;
        let metrics = hmtx.h_metrics();
        if metrics.is_empty() {
            return None;
        }
        if usize::from(gid) < metrics.len() {
            Some(metrics[usize::from(gid)].advance())
        } else if gid < self.glyph_count {
            metrics.last().map(|metric| metric.advance())
        } else {
            None
        }
    }

    fn ink_extents(&self, gid: u16) -> (i64, i64) {
        let metrics = self
            .face
            .glyph_metrics(Size::unscaled(), LocationRef::default());
        let Some(bounds) = metrics.bounds(GlyphId::new(u32::from(gid))) else {
            return (0, 0);
        };
        let height = trunc_i64(bounds.y_max).max(0);
        let depth = (-trunc_i64(bounds.y_min)).max(0);
        (height, depth)
    }
}

fn trunc_i64(v: f32) -> i64 {
    if !v.is_finite() {
        0
    } else {
        v as i64
    }
}

fn load_script_alternates(font: &FontRef<'static>) -> Vec<ScriptAlternateEntry> {
    let Ok(gsub) = font.gsub() else {
        return Vec::new();
    };
    let Ok(features) = gsub.feature_list() else {
        return Vec::new();
    };
    let Ok(lookups) = gsub.lookup_list() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for rec in features.feature_records() {
        if rec.feature_tag() != Tag::new(b"ssty") {
            continue;
        }
        let Ok(feature) = rec.feature(features.offset_data()) else {
            continue;
        };
        for idx in feature.lookup_list_indices() {
            let Ok(lookup) = lookups.lookups().get(usize::from(idx.get())) else {
                continue;
            };
            let Ok(subtables) = lookup.subtables() else {
                continue;
            };
            let SubstitutionSubtables::Alternate(tables) = subtables else {
                continue;
            };
            for table in tables.iter().flatten() {
                let Ok(coverage) = table.coverage() else {
                    continue;
                };
                let sets = table.alternate_sets();
                for glyph in coverage.iter() {
                    let Some(index) = coverage.get(glyph) else {
                        continue;
                    };
                    let Ok(set) = sets.get(usize::from(index)) else {
                        continue;
                    };
                    let ids = set.alternate_glyph_ids();
                    let alternates = [
                        ids.first().map(|id| id.get().to_u16()),
                        ids.get(1).map(|id| id.get().to_u16()),
                    ];
                    if alternates.iter().all(Option::is_none) {
                        continue;
                    }
                    insert_alternate(&mut entries, glyph.to_u16(), alternates);
                }
            }
        }
    }
    entries
}

fn insert_alternate(
    entries: &mut Vec<ScriptAlternateEntry>,
    glyph_id: u16,
    alternates: [Option<u16>; 2],
) {
    match entries.binary_search_by_key(&glyph_id, |entry| entry.glyph_id) {
        Ok(_) => {}
        Err(index) => entries.insert(
            index,
            ScriptAlternateEntry {
                glyph_id,
                alternates,
            },
        ),
    }
}

fn hex_byte(b: u8) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let hi = H[usize::from(b >> 4)];
    let lo = H[usize::from(b & 0xf)];
    let mut out = String::with_capacity(2);
    out.push(char::from(hi));
    out.push(char::from(lo));
    out
}

//! OpenType MATH table, read from the font bytes.
//!
//! `read-fonts` 0.36, the last release this crate's toolchain can build, does
//! not expose MATH. The integers below are the values a MathValueRecord
//! stores: the device-table offset is ignored, matching layout's use of the
//! design-unit value only.

use core::cmp::Ordering;

pub(super) struct MathTable {
    data: Vec<u8>,
    constants: Option<usize>,
    glyph_info: Option<usize>,
    variants: Option<usize>,
}

pub(super) struct Construction {
    pub variants: Vec<u16>,
    pub parts: Vec<Part>,
}

pub(super) struct Part {
    pub glyph_id: u16,
    pub start_connector: u16,
    pub end_connector: u16,
    pub full_advance: u16,
    pub extender: bool,
}

pub(crate) struct MathConstants {
    pub script_percent_scale_down: i16,
    pub script_script_percent_scale_down: i16,
    pub display_operator_min_height: u16,
    pub axis_height: i16,
    pub accent_base_height: i16,
    pub flattened_accent_base_height: i16,
    pub subscript_shift_down: i16,
    pub subscript_top_max: i16,
    pub subscript_baseline_drop_min: i16,
    pub superscript_shift_up: i16,
    pub superscript_shift_up_cramped: i16,
    pub superscript_bottom_min: i16,
    pub superscript_baseline_drop_max: i16,
    pub sub_superscript_gap_min: i16,
    pub superscript_bottom_max_with_subscript: i16,
    pub space_after_script: i16,
    pub upper_limit_gap_min: i16,
    pub upper_limit_baseline_rise_min: i16,
    pub lower_limit_gap_min: i16,
    pub lower_limit_baseline_drop_min: i16,
    pub stack_top_shift_up: i16,
    pub stack_bottom_shift_down: i16,
    pub stack_gap_min: i16,
    pub fraction_numerator_shift_up: i16,
    pub fraction_numerator_display_style_shift_up: i16,
    pub fraction_denominator_shift_down: i16,
    pub fraction_denominator_display_style_shift_down: i16,
    pub fraction_numerator_gap_min: i16,
    pub fraction_num_display_style_gap_min: i16,
    pub fraction_rule_thickness: i16,
    pub fraction_denominator_gap_min: i16,
    pub fraction_denom_display_style_gap_min: i16,
    pub overbar_vertical_gap: i16,
    pub overbar_rule_thickness: i16,
    pub overbar_extra_ascender: i16,
    pub underbar_vertical_gap: i16,
    pub underbar_rule_thickness: i16,
    pub underbar_extra_descender: i16,
    pub radical_vertical_gap: i16,
    pub radical_display_style_vertical_gap: i16,
    pub radical_rule_thickness: i16,
    pub radical_extra_ascender: i16,
    pub radical_kern_before_degree: i16,
    pub radical_kern_after_degree: i16,
    pub radical_degree_bottom_raise_percent: i16,
}

impl MathTable {
    pub(super) fn parse(bytes: &[u8]) -> Option<Self> {
        if ru16(bytes, 0)? != 1 {
            return None;
        }
        Some(Self {
            constants: nz_off(bytes, 4),
            glyph_info: nz_off(bytes, 6),
            variants: nz_off(bytes, 8),
            data: bytes.to_vec(),
        })
    }

    pub(super) fn constants(&self) -> Option<MathConstants> {
        let base = self.constants?;
        let v = |rel: usize| ri16(&self.data, base + rel).unwrap_or(0);
        Some(MathConstants {
            script_percent_scale_down: v(0),
            script_script_percent_scale_down: v(2),
            display_operator_min_height: ru16(&self.data, base + 6).unwrap_or(0),
            axis_height: v(12),
            accent_base_height: v(16),
            flattened_accent_base_height: v(20),
            subscript_shift_down: v(24),
            subscript_top_max: v(28),
            subscript_baseline_drop_min: v(32),
            superscript_shift_up: v(36),
            superscript_shift_up_cramped: v(40),
            superscript_bottom_min: v(44),
            superscript_baseline_drop_max: v(48),
            sub_superscript_gap_min: v(52),
            superscript_bottom_max_with_subscript: v(56),
            space_after_script: v(60),
            upper_limit_gap_min: v(64),
            upper_limit_baseline_rise_min: v(68),
            lower_limit_gap_min: v(72),
            lower_limit_baseline_drop_min: v(76),
            stack_top_shift_up: v(80),
            stack_bottom_shift_down: v(88),
            stack_gap_min: v(96),
            fraction_numerator_shift_up: v(120),
            fraction_numerator_display_style_shift_up: v(124),
            fraction_denominator_shift_down: v(128),
            fraction_denominator_display_style_shift_down: v(132),
            fraction_numerator_gap_min: v(136),
            fraction_num_display_style_gap_min: v(140),
            fraction_rule_thickness: v(144),
            fraction_denominator_gap_min: v(148),
            fraction_denom_display_style_gap_min: v(152),
            overbar_vertical_gap: v(164),
            overbar_rule_thickness: v(168),
            overbar_extra_ascender: v(172),
            underbar_vertical_gap: v(176),
            underbar_rule_thickness: v(180),
            underbar_extra_descender: v(184),
            radical_vertical_gap: v(188),
            radical_display_style_vertical_gap: v(192),
            radical_rule_thickness: v(196),
            radical_extra_ascender: v(200),
            radical_kern_before_degree: v(204),
            radical_kern_after_degree: v(208),
            radical_degree_bottom_raise_percent: v(212),
        })
    }

    pub(super) fn italic(&self, glyph: u16) -> Option<i16> {
        self.glyph_value(0, glyph)
    }

    pub(super) fn top_accent(&self, glyph: u16) -> Option<i16> {
        self.glyph_value(2, glyph)
    }

    /// Variant glyph ids and the assembly recipe, when the glyph is covered.
    pub(super) fn construction(&self, glyph: u16, horizontal: bool) -> Option<Construction> {
        let base = self.variants?;
        let data = &self.data;
        let cov_rel = if horizontal {
            nz_off(data, base + 4)?
        } else {
            nz_off(data, base + 2)?
        };
        let vert_count = ru16(data, base + 6)?;
        let horiz_count = ru16(data, base + 8)?;
        let (count, array_at) = if horizontal {
            (horiz_count, base + 10 + usize::from(vert_count) * 2)
        } else {
            (vert_count, base + 10)
        };
        let index = coverage_index(data.get(base + cov_rel..)?, glyph)?;
        if index >= count {
            return None;
        }
        let cons_rel = nz_off(data, array_at + usize::from(index) * 2)?;
        let cons = base + cons_rel;
        let variant_count = ru16(data, cons + 2)?;
        let mut variants = Vec::with_capacity(usize::from(variant_count));
        for i in 0..variant_count {
            let rec = cons + 4 + usize::from(i) * 4;
            variants.push(ru16(data, rec)?);
        }
        let mut parts = Vec::new();
        if let Some(rel) = nz_off(data, cons) {
            let asm = cons + rel;
            let part_count = ru16(data, asm + 4)?;
            parts.reserve(usize::from(part_count));
            for i in 0..part_count {
                let p = asm + 6 + usize::from(i) * 10;
                let flags = ru16(data, p + 8)?;
                parts.push(Part {
                    glyph_id: ru16(data, p)?,
                    start_connector: ru16(data, p + 2)?,
                    end_connector: ru16(data, p + 4)?,
                    full_advance: ru16(data, p + 6)?,
                    extender: flags & 1 != 0,
                });
            }
        }
        Some(Construction { variants, parts })
    }

    fn glyph_value(&self, field: usize, glyph: u16) -> Option<i16> {
        let info = self.glyph_info?;
        let rel = nz_off(&self.data, info + field)?;
        let table = info + rel;
        let cov_rel = nz_off(&self.data, table)?;
        let index = coverage_index(self.data.get(table + cov_rel..)?, glyph)?;
        let count = ru16(&self.data, table + 2)?;
        if index >= count {
            return None;
        }
        ri16(&self.data, table + 4 + usize::from(index) * 4)
    }
}

fn ru16(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn ri16(data: &[u8], off: usize) -> Option<i16> {
    let b = data.get(off..off.checked_add(2)?)?;
    Some(i16::from_be_bytes([b[0], b[1]]))
}

fn nz_off(data: &[u8], off: usize) -> Option<usize> {
    let v = ru16(data, off)?;
    if v == 0 {
        None
    } else {
        Some(usize::from(v))
    }
}

fn coverage_index(data: &[u8], glyph: u16) -> Option<u16> {
    let format = ru16(data, 0)?;
    let count = ru16(data, 2)?;
    if format == 1 {
        let mut lo = 0u16;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let g = ru16(data, 4 + usize::from(mid) * 2)?;
            match g.cmp(&glyph) {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return Some(mid),
            }
        }
        None
    } else if format == 2 {
        let mut lo = 0u16;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let base = 4 + usize::from(mid) * 6;
            let start = ru16(data, base)?;
            let end = ru16(data, base + 2)?;
            if glyph < start {
                hi = mid;
            } else if glyph > end {
                lo = mid + 1;
            } else {
                let cov = ru16(data, base + 4)?;
                return cov.checked_add(glyph - start);
            }
        }
        None
    } else {
        None
    }
}

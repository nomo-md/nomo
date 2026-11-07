//! OpenType MATH constants as [`Dim`](crate::Dim). TeX σ-names are documented on fields.

use crate::dim::Dim;
use crate::error::Error;
use crate::font::MathFont;
use crate::layout::style::MathStyle;

/// MATH-table parameters used by the layout engine.
///
/// # Examples
///
/// ```
/// use latex_rust::{MathFont, MathParams, MathStyle};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let p = MathParams::from_font(&font).unwrap();
/// assert!(!p.axis_height.is_zero());
/// assert!(!p.em(MathStyle::Text).is_zero());
/// ```
#[derive(Clone, Debug)]
pub struct MathParams {
    /// σ1: x-height (em).
    pub x_height: Dim,
    /// σ2: quad / em width.
    pub quad: Dim,
    /// σ6 / σ22: math axis height.
    pub axis_height: Dim,
    /// Accent base height.
    pub accent_base_height: Dim,
    /// Flattened accent base height (cramped / tall bases).
    pub flattened_accent_base_height: Dim,
    /// σ8: default rule thickness (fraction / radical bar).
    pub fraction_rule_thickness: Dim,
    /// σ10 analogue: numerator shift (text).
    pub fraction_numerator_shift_up: Dim,
    /// σ9 analogue: numerator shift (display).
    pub fraction_numerator_display_style_shift_up: Dim,
    /// σ12 analogue: denominator shift (text).
    pub fraction_denominator_shift_down: Dim,
    /// σ11 analogue: denominator shift (display).
    pub fraction_denominator_display_style_shift_down: Dim,
    /// Minimum gap num ↔ rule (text).
    pub fraction_numerator_gap_min: Dim,
    /// Minimum gap num ↔ rule (display).
    pub fraction_num_display_style_gap_min: Dim,
    /// Minimum gap rule ↔ den (text).
    pub fraction_denominator_gap_min: Dim,
    /// Minimum gap rule ↔ den (display).
    pub fraction_denom_display_style_gap_min: Dim,
    /// σ14 analogue: superscript shift (text).
    pub superscript_shift_up: Dim,
    /// σ15 analogue: superscript shift cramped.
    pub superscript_shift_up_cramped: Dim,
    /// σ16 analogue: subscript shift.
    pub subscript_shift_down: Dim,
    /// Maximum height of a subscript that does not force it lower.
    pub subscript_top_max: Dim,
    /// Minimum drop of a subscript baseline below the base.
    pub subscript_baseline_drop_min: Dim,
    /// Minimum height of a superscript bottom that does not force it higher.
    pub superscript_bottom_min: Dim,
    /// Maximum height of a superscript bottom when a subscript is also present.
    pub superscript_bottom_max_with_subscript: Dim,
    /// Maximum drop of a superscript baseline below the top of the base.
    pub superscript_baseline_drop_max: Dim,
    /// Minimum gap between sub and sup.
    pub sub_superscript_gap_min: Dim,
    /// Space after a script.
    pub space_after_script: Dim,
    /// Standard shift of the top element of a stack.
    pub stack_top_shift_up: Dim,
    /// Standard shift of the bottom element of a stack.
    pub stack_bottom_shift_down: Dim,
    /// Minimum gap inside a stack.
    pub stack_gap_min: Dim,
    /// Radical vertical gap (text).
    pub radical_vertical_gap: Dim,
    /// Radical vertical gap (display).
    pub radical_display_style_vertical_gap: Dim,
    /// Radical rule thickness.
    pub radical_rule_thickness: Dim,
    /// Extra ascender above radical rule.
    pub radical_extra_ascender: Dim,
    /// Kern before a radical degree.
    pub radical_kern_before_degree: Dim,
    /// Kern after a radical degree.
    pub radical_kern_after_degree: Dim,
    /// Degree bottom raise percent (integer 0–100).
    pub radical_degree_bottom_raise_percent: i16,
    /// Overbar gap.
    pub overbar_vertical_gap: Dim,
    /// Overbar rule thickness.
    pub overbar_rule_thickness: Dim,
    /// Overbar extra ascender.
    pub overbar_extra_ascender: Dim,
    /// Underbar gap.
    pub underbar_vertical_gap: Dim,
    /// Underbar rule thickness.
    pub underbar_rule_thickness: Dim,
    /// Underbar extra descender.
    pub underbar_extra_descender: Dim,
    /// Upper limit gap min.
    pub upper_limit_gap_min: Dim,
    /// Upper limit baseline rise min.
    pub upper_limit_baseline_rise_min: Dim,
    /// Lower limit gap min.
    pub lower_limit_gap_min: Dim,
    /// Lower limit baseline drop min.
    pub lower_limit_baseline_drop_min: Dim,
    /// Display operator min height (font units, as Dim em).
    pub display_operator_min_height: Dim,
    /// Script scale (percent, e.g. 70).
    pub script_percent_scale_down: i16,
    /// Scriptscript scale (percent, e.g. 55).
    pub script_script_percent_scale_down: i16,
    /// `unitsPerEm`.
    pub units_per_em: u16,
}

impl MathParams {
    /// Load MATH constants from `font`. Missing MATH is [`Error::Unsupported`].
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] if the face has no MATH table or constants.
    pub fn from_font(font: &MathFont) -> Result<Self, Error> {
        let c = font.math_constants().ok_or_else(|| Error::Unsupported {
            what: "OpenType MATH table".into(),
        })?;
        let upem = font.units_per_em();
        let fu = |v: i16| Dim::from_font_units(i64::from(v), upem);
        let fu_u = |v: u16| Dim::from_font_units(i64::from(v), upem);
        let xh = font
            .x_height_fu()
            .map(fu)
            .unwrap_or_else(|| Dim::ratio(1, 2));
        Ok(Self {
            x_height: xh,
            quad: Dim::one(),
            axis_height: fu(c.axis_height),
            accent_base_height: fu(c.accent_base_height),
            flattened_accent_base_height: fu(c.flattened_accent_base_height),
            fraction_rule_thickness: fu(c.fraction_rule_thickness),
            fraction_numerator_shift_up: fu(c.fraction_numerator_shift_up),
            fraction_numerator_display_style_shift_up: fu(
                c.fraction_numerator_display_style_shift_up
            ),
            fraction_denominator_shift_down: fu(c.fraction_denominator_shift_down),
            fraction_denominator_display_style_shift_down: fu(
                c.fraction_denominator_display_style_shift_down
            ),
            fraction_numerator_gap_min: fu(c.fraction_numerator_gap_min),
            fraction_num_display_style_gap_min: fu(c.fraction_num_display_style_gap_min),
            fraction_denominator_gap_min: fu(c.fraction_denominator_gap_min),
            fraction_denom_display_style_gap_min: fu(c.fraction_denom_display_style_gap_min),
            superscript_shift_up: fu(c.superscript_shift_up),
            superscript_shift_up_cramped: fu(c.superscript_shift_up_cramped),
            subscript_shift_down: fu(c.subscript_shift_down),
            subscript_top_max: fu(c.subscript_top_max),
            subscript_baseline_drop_min: fu(c.subscript_baseline_drop_min),
            superscript_bottom_min: fu(c.superscript_bottom_min),
            superscript_bottom_max_with_subscript: fu(c.superscript_bottom_max_with_subscript),
            superscript_baseline_drop_max: fu(c.superscript_baseline_drop_max),
            sub_superscript_gap_min: fu(c.sub_superscript_gap_min),
            space_after_script: fu(c.space_after_script),
            stack_top_shift_up: fu(c.stack_top_shift_up),
            stack_bottom_shift_down: fu(c.stack_bottom_shift_down),
            stack_gap_min: fu(c.stack_gap_min),
            radical_vertical_gap: fu(c.radical_vertical_gap),
            radical_display_style_vertical_gap: fu(c.radical_display_style_vertical_gap),
            radical_rule_thickness: fu(c.radical_rule_thickness),
            radical_extra_ascender: fu(c.radical_extra_ascender),
            radical_kern_before_degree: fu(c.radical_kern_before_degree),
            radical_kern_after_degree: fu(c.radical_kern_after_degree),
            radical_degree_bottom_raise_percent: c.radical_degree_bottom_raise_percent,
            overbar_vertical_gap: fu(c.overbar_vertical_gap),
            overbar_rule_thickness: fu(c.overbar_rule_thickness),
            overbar_extra_ascender: fu(c.overbar_extra_ascender),
            underbar_vertical_gap: fu(c.underbar_vertical_gap),
            underbar_rule_thickness: fu(c.underbar_rule_thickness),
            underbar_extra_descender: fu(c.underbar_extra_descender),
            upper_limit_gap_min: fu(c.upper_limit_gap_min),
            upper_limit_baseline_rise_min: fu(c.upper_limit_baseline_rise_min),
            lower_limit_gap_min: fu(c.lower_limit_gap_min),
            lower_limit_baseline_drop_min: fu(c.lower_limit_baseline_drop_min),
            display_operator_min_height: fu_u(c.display_operator_min_height),
            script_percent_scale_down: c.script_percent_scale_down,
            script_script_percent_scale_down: c.script_script_percent_scale_down,
            units_per_em: upem,
        })
    }

    /// Scale factor for `style` (1, script%, or scriptscript%).
    #[must_use]
    pub fn scale(&self, style: MathStyle) -> Dim {
        match style.script_level() {
            0 => Dim::one(),
            1 => Dim::from_i64(i64::from(self.script_percent_scale_down)) / Dim::from_i64(100),
            _ => {
                Dim::from_i64(i64::from(self.script_script_percent_scale_down)) / Dim::from_i64(100)
            }
        }
    }

    /// Current em (`quad * scale`).
    #[must_use]
    pub fn em(&self, style: MathStyle) -> Dim {
        &self.quad * &self.scale(style)
    }

    /// One mu at `style` (`em / 18`).
    #[must_use]
    pub fn mu(&self, style: MathStyle) -> Dim {
        self.em(style) / Dim::from_i64(18)
    }
}

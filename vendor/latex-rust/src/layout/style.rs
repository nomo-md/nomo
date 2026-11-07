//! TeX math style (Appendix G).

/// Math style, including cramped variants.
///
/// Display and text are script-level 0; `\scriptstyle` is 1; `\scriptscriptstyle`
/// is 2. Cramped variants are used under radicals and in denominators.
///
/// # Examples
///
/// ```
/// use latex_rust::MathStyle;
///
/// assert_eq!(MathStyle::Display.numerator().gold(), "text");
/// assert!(MathStyle::Display.cramp().is_cramped());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathStyle {
    /// Display math (`$$`, `\[`).
    Display,
    /// Cramped display (e.g. under a radical in display).
    DisplayCramped,
    /// Text / inline math (`$`, `\(`).
    Text,
    /// Cramped text.
    TextCramped,
    /// First-level script.
    Script,
    /// Cramped script.
    ScriptCramped,
    /// Second-level script.
    ScriptScript,
    /// Cramped scriptscript.
    ScriptScriptCramped,
}

impl MathStyle {
    /// True for the two display styles.
    #[must_use]
    pub fn is_display(self) -> bool {
        matches!(self, Self::Display | Self::DisplayCramped)
    }

    /// True when cramped.
    #[must_use]
    pub fn is_cramped(self) -> bool {
        matches!(
            self,
            Self::DisplayCramped
                | Self::TextCramped
                | Self::ScriptCramped
                | Self::ScriptScriptCramped
        )
    }

    /// Script nest level: 0 text/display, 1 script, 2 scriptscript.
    #[must_use]
    pub fn script_level(self) -> u8 {
        match self {
            Self::Display | Self::DisplayCramped | Self::Text | Self::TextCramped => 0,
            Self::Script | Self::ScriptCramped => 1,
            Self::ScriptScript | Self::ScriptScriptCramped => 2,
        }
    }

    /// Cramped form of this style.
    #[must_use]
    pub fn cramp(self) -> Self {
        match self {
            Self::Display => Self::DisplayCramped,
            Self::Text => Self::TextCramped,
            Self::Script => Self::ScriptCramped,
            Self::ScriptScript => Self::ScriptScriptCramped,
            other => other,
        }
    }

    /// Style for a superscript or subscript of this style.
    #[must_use]
    pub fn into_script(self) -> Self {
        let cramped = self.is_cramped();
        match self.script_level() {
            0 => {
                if cramped {
                    Self::ScriptCramped
                } else {
                    Self::Script
                }
            }
            _ => {
                if cramped {
                    Self::ScriptScriptCramped
                } else {
                    Self::ScriptScript
                }
            }
        }
    }

    /// Numerator style (TeX: display → text, otherwise one script tighter; not cramped).
    #[must_use]
    pub fn numerator(self) -> Self {
        if self.is_display() {
            Self::Text
        } else {
            self.into_script()
        }
    }

    /// Denominator style (numerator style, cramped).
    #[must_use]
    pub fn denominator(self) -> Self {
        self.numerator().cramp()
    }

    /// Gold name.
    #[must_use]
    pub fn gold(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::DisplayCramped => "display-cramped",
            Self::Text => "text",
            Self::TextCramped => "text-cramped",
            Self::Script => "script",
            Self::ScriptCramped => "script-cramped",
            Self::ScriptScript => "scriptscript",
            Self::ScriptScriptCramped => "scriptscript-cramped",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MathStyle;

    #[test]
    fn style_machine_gold() {
        assert_eq!(MathStyle::Display.numerator().gold(), "text");
        assert_eq!(MathStyle::Display.denominator().gold(), "text-cramped");
        assert_eq!(MathStyle::Text.numerator().gold(), "script");
        assert_eq!(MathStyle::Text.denominator().gold(), "script-cramped");
        assert_eq!(MathStyle::Script.numerator().gold(), "scriptscript");
        assert_eq!(
            MathStyle::Script.denominator().gold(),
            "scriptscript-cramped"
        );
        assert_eq!(MathStyle::Display.into_script().gold(), "script");
        assert_eq!(
            MathStyle::DisplayCramped.into_script().gold(),
            "script-cramped"
        );
        assert_eq!(MathStyle::Text.cramp().gold(), "text-cramped");
        assert_eq!(MathStyle::Script.into_script().gold(), "scriptscript");
        assert!(MathStyle::Display.cramp().is_cramped());
        assert_eq!(MathStyle::Display.script_level(), 0);
        assert_eq!(MathStyle::Script.script_level(), 1);
        assert_eq!(MathStyle::ScriptScript.script_level(), 2);
    }
}

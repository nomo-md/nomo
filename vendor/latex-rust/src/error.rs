//! Public error types. Unsupported input is never rendered.

use core::fmt;

/// Crate-level error.
///
/// Unsupported input is never rendered. [`Error::Parse`] wraps tokenizer/parser
/// failures; [`Error::Font`] wraps face/glyph failures; [`Error::Unsupported`]
/// is a construct or feature out of scope; [`Error::Malformed`] is a bad value
/// that parsed as the wrong shape; [`Error::InvalidOption`] is a render option
/// out of range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Tokenizer or parser rejected the input.
    Parse(ParseError),
    /// Font bytes or a requested glyph could not be used.
    Font(FontError),
    /// A feature listed as out of scope, or not yet implemented.
    ///
    /// Callers must treat this as failure. The renderer does not invent output.
    Unsupported {
        /// Human-readable name of the missing feature or construct.
        what: String,
    },
    /// Syntactically invalid value (for example a color spec with the wrong
    /// number of components). Distinct from [`Self::Unsupported`].
    Malformed {
        /// Human-readable description of what was malformed.
        what: String,
    },
    /// A render option is out of range (for example PNG DPI of 0 or above 2400).
    InvalidOption {
        /// Human-readable name of the invalid option.
        what: String,
    },
}

/// Tokenizer / parser failure.
///
/// The enum is non-exhaustive so a later release can add a variant. A
/// downstream match needs a wildcard.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// Input ended with a stray `\` and no command character.
    TrailingBackslash,
    /// Command is known but this crate will not invent a rendering for it.
    Unsupported(String),
    /// Command is not in the catalog and is not a known math structure.
    Unknown(String),
    /// Syntactically invalid input. Names the construct or position.
    Malformed(String),
    /// `\left` without `\right`, or `\right` without `\left`.
    UnmatchedDelimiter,
    /// `\\` or `\cr` outside a matrix, array, cases, or substack.
    RowBreakOutside,
}

/// Font loader or metric lookup failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    /// Embedded or supplied bytes are not a usable OpenType face.
    InvalidFace,
    /// Character has no glyph in this face.
    MissingGlyph {
        /// Requested character.
        ch: char,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Font(e) => write!(f, "{e}"),
            Self::Unsupported { what } => write!(f, "unsupported: {what}"),
            Self::Malformed { what } => write!(f, "malformed: {what}"),
            Self::InvalidOption { what } => write!(f, "invalid option: {what}"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrailingBackslash => f.write_str("trailing backslash"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Unknown(s) => write!(f, "unknown command: {s}"),
            Self::Malformed(s) => write!(f, "malformed: {s}"),
            Self::UnmatchedDelimiter => f.write_str("unmatched delimiter"),
            Self::RowBreakOutside => f.write_str(r"row break \\ outside an environment"),
        }
    }
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFace => f.write_str("invalid OpenType face"),
            Self::MissingGlyph { ch } => write!(f, "missing glyph for {ch:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<FontError> for Error {
    fn from(e: FontError) -> Self {
        Self::Font(e)
    }
}

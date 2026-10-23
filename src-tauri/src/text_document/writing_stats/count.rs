use super::super::TextDocumentResult;
use super::{check, invalid, source::Source, tape::Mapping, ByteRange, CHUNK};
use std::{
    fs::File,
    path::Path,
    sync::{atomic::AtomicBool, Arc, OnceLock},
};
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

fn han(c: char) -> bool {
    if c < '\u{2e80}' {
        return false;
    }
    static HAN: OnceLock<regex::Regex> = OnceLock::new();
    let mut encoded = [0; 4];
    HAN.get_or_init(|| regex::Regex::new(r"\p{Script=Han}").unwrap())
        .is_match(c.encode_utf8(&mut encoded))
}
fn visible(c: char) -> bool {
    c != '\0'
        && !c.is_whitespace()
        && c != '\u{feff}'
        && c.general_category() != GeneralCategory::Format
}
fn alpha(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    ) && !han(c)
}

struct Graphemes {
    source: Source,
    cursor: GraphemeCursor,
    chunk: String,
    base: u64,
    end: u64,
    previous: u64,
}
impl Graphemes {
    fn emoji_join(&mut self, boundary: u64) -> TextDocumentResult<bool> {
        if boundary >= self.end {
            return Ok(false);
        }
        let next = self.source.char(boundary)?.0;
        if next < '\u{a9}' {
            return Ok(false);
        }
        static PICTOGRAPH: OnceLock<regex::Regex> = OnceLock::new();
        static EXTEND: OnceLock<regex::Regex> = OnceLock::new();
        let pictograph =
            PICTOGRAPH.get_or_init(|| regex::Regex::new(r"\p{Extended_Pictographic}").unwrap());
        let extend = EXTEND.get_or_init(|| regex::Regex::new(r"\p{gcb=Extend}").unwrap());
        let mut encoded = [0; 4];
        if !pictograph.is_match(next.encode_utf8(&mut encoded)) {
            return Ok(false);
        }
        let mut pos = boundary;
        let mut previous = |source: &mut Source| -> TextDocumentResult<Option<char>> {
            if pos == 0 {
                return Ok(None);
            }
            pos -= 1;
            while pos > 0 && source.byte(pos)? & 0xc0 == 0x80 {
                pos -= 1;
            }
            Ok(Some(source.char(pos)?.0))
        };
        if previous(&mut self.source)? != Some('\u{200d}') {
            return Ok(false);
        }
        while let Some(c) = previous(&mut self.source)? {
            let text = c.encode_utf8(&mut encoded);
            if pictograph.is_match(text) {
                return Ok(true);
            }
            if !extend.is_match(text) {
                return Ok(false);
            }
        }
        Ok(false)
    }
    fn new(file: File, from: u64, to: u64, cancel: Arc<AtomicBool>) -> Self {
        Self {
            source: Source::file(file, to, cancel),
            cursor: GraphemeCursor::new(from as usize, to as usize, true),
            chunk: String::new(),
            base: from,
            end: to,
            previous: from,
        }
    }
    fn load(&mut self, from: u64) -> TextDocumentResult<()> {
        let mut end = (from + CHUNK as u64).min(self.end);
        while end < self.end && self.source.byte(end)? & 0xc0 == 0x80 {
            end -= 1;
        }
        self.chunk = self.source.small(from, end)?;
        self.base = from;
        Ok(())
    }
    fn next(&mut self) -> TextDocumentResult<Option<(u64, u64)>> {
        if self.previous >= self.end {
            return Ok(None);
        }
        if self.chunk.is_empty() {
            self.load(self.previous)?;
        }
        loop {
            match self.cursor.next_boundary(&self.chunk, self.base as usize) {
                Ok(Some(end)) => {
                    // 1.13.2 re-checks ZWJ when its Emoji pre-context continues
                    // into another chunk. Validate GB11 using bounded backwards
                    // reads; do not retain an arbitrarily long Extend sequence.
                    if self.emoji_join(end as u64)? {
                        continue;
                    }
                    let from = self.previous;
                    self.previous = end as u64;
                    return Ok(Some((from, end as u64)));
                }
                Ok(None) => return Ok(None),
                Err(GraphemeIncomplete::NextChunk) => {
                    self.load(self.base + self.chunk.len() as u64)?
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let mut from = (end as u64).saturating_sub(CHUNK as u64);
                    while from < end as u64 && self.source.byte(from)? & 0xc0 == 0x80 {
                        from += 1;
                    }
                    let context = self.source.small(from, end as u64)?;
                    self.cursor.provide_context(&context, from as usize);
                }
                Err(other) => return Err(invalid(format!("Unicode 分块边界错误：{other:?}"))),
            }
        }
    }
}

#[derive(Default)]
struct Word {
    from: u64,
    to: u64,
    active: bool,
    selected: bool,
    last_number: bool,
    decimal: bool,
    join_selected: bool,
    join: Option<char>,
}
impl Word {
    fn finish(&mut self, words: &mut u64, selected_words: &mut u64) {
        if self.active {
            *words += 1;
            if self.selected {
                *selected_words += 1;
            }
        }
        *self = Self::default();
    }
}
/// Two bounded cursors read the tape: Unicode segmentation and source-interval
/// reduction. GraphemeCursor asks the file for backwards context, so an unbounded
/// combining sequence / regional-indicator run never grows an in-memory string.
pub(super) fn count(
    root: &Path,
    from: u64,
    to: u64,
    selection: Option<ByteRange>,
    cancel: Arc<AtomicBool>,
) -> TextDocumentResult<(u64, u64, u64, u64)> {
    count_with_boundaries(root, from, to, selection, cancel, &mut |_, _, _| Ok(()))
}
pub(super) fn count_with_boundaries(
    root: &Path,
    from: u64,
    to: u64,
    selection: Option<ByteRange>,
    cancel: Arc<AtomicBool>,
    boundary: &mut dyn FnMut(u64, u64, u64) -> TextDocumentResult<()>,
) -> TextDocumentResult<(u64, u64, u64, u64)> {
    let file = File::open(root.join("body"))?;
    let mut graphemes = Graphemes::new(file.try_clone()?, from, to, cancel.clone());
    let mut source = Source::file(file, to, cancel.clone());
    let mut mapping = Mapping::open(&root.join("spans"), from)?;
    let (mut words, mut chars, mut selected_words, mut selected_chars) = (0, 0, 0, 0);
    let mut word = Word::default();
    let mut batch = 0;
    while let Some((start, end)) = graphemes.next()? {
        batch += 1;
        if batch == 1024 {
            check(&cancel)?;
            batch = 0;
        }
        let mut pos = start;
        let (mut is_visible, mut is_han) = (false, false);
        let mut first = u64::MAX;
        let mut last = 0;
        // Only scalar metadata is accumulated, never the grapheme itself.
        while pos < end {
            let (c, next) = source.char(pos)?;
            let (a, b) = mapping.at(pos, next)?;
            first = first.min(a);
            last = b;
            is_visible |= visible(c);
            is_han |= han(c);
            pos = next;
        }
        let selected = is_visible && selection.is_some_and(|s| s.overlaps(first, last));
        if is_visible {
            chars += 1;
            if selected {
                selected_chars += 1;
            }
        }
        if is_han {
            word.finish(&mut words, &mut selected_words);
            words += 1;
            if selected {
                selected_words += 1;
            }
        }
        pos = start;
        while pos < end {
            let (c, next) = source.char(pos)?;
            let is_alpha = alpha(c);
            let is_mark = c.general_category_group() == GeneralCategoryGroup::Mark;
            let number = c.general_category_group() == GeneralCategoryGroup::Number;
            if word.decimal && word.join.is_none() && ((is_alpha && !number) || is_mark) {
                word.finish(&mut words, &mut selected_words);
            }
            if is_alpha || (is_mark && word.active && word.join.is_none()) {
                if word.join == Some('.') && !number {
                    word.finish(&mut words, &mut selected_words);
                }
                if !word.active {
                    word.from = first;
                    word.active = true;
                }
                if let Some(join) = word.join {
                    word.decimal = join == '.';
                }
                word.to = last;
                word.selected |= selected || word.join_selected;
                word.last_number = number;
                word.join = None;
                word.join_selected = false;
            } else if word.active
                && word.join.is_none()
                && (matches!(c, '\'' | '’' | '_' | '-') || (c == '.' && word.last_number))
            {
                word.join = Some(c);
                word.join_selected = selected;
            } else {
                word.finish(&mut words, &mut selected_words);
            }
            pos = next;
        }
        if source.byte(start)? == 0 {
            boundary(end, words, chars)?;
        }
    }
    word.finish(&mut words, &mut selected_words);
    Ok((words, chars, selected_words, selected_chars))
}

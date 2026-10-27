use super::super::TextDocumentResult;
use super::{source::Source, CHUNK};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// Source intervals, not character offsets. Adjacent literal spans coalesce.
/// Decoded entities and escaped pipes retain the entire consumed source interval.
#[derive(Clone, Copy, Default)]
pub(super) struct Span {
    pub body: u64,
    pub end: u64,
    pub from: u64,
    pub to: u64,
    pub literal: u64,
}
impl Span {
    pub const SIZE: u64 = 40;
    fn write(self, out: &mut impl Write) -> std::io::Result<()> {
        for value in [self.body, self.end, self.from, self.to, self.literal] {
            out.write_all(&value.to_le_bytes())?;
        }
        Ok(())
    }
    pub fn read(input: &mut impl Read) -> std::io::Result<Self> {
        let mut values = [0; 5];
        for value in &mut values {
            let mut bytes = [0; 8];
            input.read_exact(&mut bytes)?;
            *value = u64::from_le_bytes(bytes);
        }
        Ok(Self {
            body: values[0],
            end: values[1],
            from: values[2],
            to: values[3],
            literal: values[4],
        })
    }
    pub fn range(self, from: u64, to: u64) -> (u64, u64) {
        if self.literal & 1 != 0 {
            (self.from + from - self.body, self.from + to - self.body)
        } else {
            (self.from, self.to)
        }
    }
}
pub(super) const HTML_MARKS: &[&str] = &[
    "strong", "b", "em", "i", "code", "a", "s", "del", "strike", "u", "sup", "sub", "mark",
];
#[derive(Default)]
struct HtmlMarks {
    first: [u8; 32],
    file: Option<File>,
    len: u64,
    code: u64,
}
impl HtmlMarks {
    fn get(&mut self, index: u64) -> TextDocumentResult<u8> {
        if index < 32 {
            return Ok(self.first[index as usize]);
        }
        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(index - 32))?;
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        Ok(byte[0])
    }
}
pub(super) struct Tape {
    pub in_code: bool,
    html_marks: HtmlMarks,
    pub root: PathBuf,
    removed: File,
    body: BufWriter<File>,
    spans: BufWriter<File>,
    pending: Option<Span>,
    pub len: u64,
    pub span_count: u64,
}
impl Tape {
    pub fn new(root: &Path) -> TextDocumentResult<Self> {
        Ok(Self {
            in_code: false,
            html_marks: HtmlMarks::default(),
            root: root.to_owned(),
            removed: File::create(root.join("removed"))?,
            body: BufWriter::with_capacity(CHUNK, File::create(root.join("body"))?),
            spans: BufWriter::with_capacity(CHUNK, File::create(root.join("spans"))?),
            pending: None,
            len: 0,
            span_count: 0,
        })
    }
    pub fn marks_empty(&self) -> bool {
        self.html_marks.len == 0
    }
    pub fn open_mark(&mut self, kind: u8) -> TextDocumentResult<()> {
        let marks = &mut self.html_marks;
        if marks.len < 32 {
            marks.first[marks.len as usize] = kind;
        } else {
            if marks.file.is_none() {
                marks.file = Some(
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(self.root.join("inline-html-stack"))?,
                );
            }
            let file = marks.file.as_mut().unwrap();
            file.seek(SeekFrom::Start(marks.len - 32))?;
            file.write_all(&[kind])?;
        }
        marks.len += 1;
        marks.code += (kind == 4) as u64;
        Ok(())
    }
    pub fn close_mark(
        &mut self,
        kind: u8,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> TextDocumentResult<bool> {
        let marks = &mut self.html_marks;
        let mut codes = 0;
        for i in (0..marks.len).rev() {
            if i % 4096 == 0 {
                super::check(cancel)?;
            }
            let current = marks.get(i)?;
            codes += (current == 4) as u64;
            if current == kind {
                marks.len = i;
                marks.code -= codes;
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn span(&mut self, from: u64, to: u64, length: u64, literal: bool) -> TextDocumentResult<()> {
        if length == 0 {
            return Ok(());
        }
        if let Some(previous) = &mut self.pending {
            if literal
                && previous.literal
                    == (1 | (((self.in_code || self.html_marks.code > 0) as u64) << 1))
                && previous.to == from
            {
                previous.end += length;
                previous.to = to;
                self.len += length;
                return Ok(());
            }
        }
        self.flush_span()?;
        self.pending = Some(Span {
            body: self.len,
            end: self.len + length,
            from,
            to,
            literal: literal as u64 | (((self.in_code || self.html_marks.code > 0) as u64) << 1),
        });
        self.len += length;
        Ok(())
    }
    fn flush_span(&mut self) -> TextDocumentResult<()> {
        if let Some(span) = self.pending.take() {
            span.write(&mut self.spans)?;
            self.span_count += 1;
        }
        Ok(())
    }
    pub fn literal(&mut self, src: &mut Source, from: u64, to: u64) -> TextDocumentResult<()> {
        // The writer's buffer is fixed; even a multi-gigabyte literal line is streamed.
        let mut start = from;
        for pos in from..to {
            let byte = src.byte(pos)?;
            if byte == 0 {
                self.span(start, pos, pos - start, true)?;
                self.text("\u{fffd}", pos, pos + 1)?;
                start = pos + 1;
            } else {
                self.body.write_all(&[byte])?;
            }
        }
        self.span(start, to, to - start, true)
    }
    pub fn marker(&mut self, src: &mut Source, from: u64, to: u64) -> TextDocumentResult<u64> {
        self.flush_span()?;
        let index = self.span_count;
        self.literal(src, from, to)?;
        self.flush_span()?;
        Ok(index)
    }
    pub fn remove(&mut self, index: u64) -> TextDocumentResult<()> {
        self.removed.seek(SeekFrom::Start(index))?;
        self.removed.write_all(&[1])?;
        Ok(())
    }
    pub fn text(&mut self, text: &str, from: u64, to: u64) -> TextDocumentResult<()> {
        self.body.write_all(text.as_bytes())?;
        self.span(from, to, text.len() as u64, false)
    }
    pub fn boundary(&mut self, pos: u64) -> TextDocumentResult<()> {
        self.text("\0", pos, pos)
    }
    pub fn copy(
        &mut self,
        reader: &mut CopySource,
        from: u64,
        to: u64,
        delta: i128,
    ) -> TextDocumentResult<()> {
        let CopySource { source, mapping } = reader;
        mapping.seek(from)?;
        let mut pos = from;
        while pos < to {
            while mapping.current.end <= pos {
                mapping.current = Span::read(&mut mapping.reader)?;
            }
            let span = mapping.current;
            let end = span.end.min(to);
            for p in pos..end {
                self.body.write_all(&[source.byte(p)?])?;
            }
            let (a, b) = span.range(pos, end);
            self.in_code = span.literal & 2 != 0;
            self.span(
                (a as i128 + delta) as u64,
                (b as i128 + delta) as u64,
                end - pos,
                span.literal & 1 != 0,
            )?;
            pos = end;
        }
        self.in_code = false;
        Ok(())
    }
    pub fn finish(mut self) -> TextDocumentResult<(u64, u64)> {
        self.flush_span()?;
        self.removed.set_len(self.span_count)?;
        self.body.flush()?;
        self.spans.flush()?;
        if self.html_marks.file.take().is_some() {
            std::fs::remove_file(self.root.join("inline-html-stack"))?;
        }
        Ok((self.len, self.span_count))
    }
}
pub(super) struct CopySource {
    source: Source,
    mapping: Mapping,
}
impl CopySource {
    pub fn open(
        root: &Path,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> TextDocumentResult<Self> {
        let file = File::open(root.join("body"))?;
        let length = file.metadata()?.len();
        Ok(Self {
            source: Source::file(file, length, cancel),
            mapping: Mapping::open(&root.join("spans"), 0)?,
        })
    }
}
pub(super) struct Mapping {
    reader: BufReader<File>,
    current: Span,
    count: u64,
}
impl Mapping {
    pub fn open(path: &Path, body_offset: u64) -> TextDocumentResult<Self> {
        let mut reader = BufReader::with_capacity(CHUNK, File::open(path)?);
        let count = reader.get_ref().metadata()?.len() / Span::SIZE;
        let (mut lo, mut hi) = (0, count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            reader.seek(SeekFrom::Start(mid * Span::SIZE))?;
            if Span::read(&mut reader)?.end <= body_offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        reader.seek(SeekFrom::Start(lo * Span::SIZE))?;
        let current = if lo < count {
            Span::read(&mut reader)?
        } else {
            Span::default()
        };
        Ok(Self {
            reader,
            current,
            count,
        })
    }
    fn seek(&mut self, pos: u64) -> TextDocumentResult<()> {
        if pos >= self.current.body && pos <= self.current.end {
            return Ok(());
        }
        // Consecutive reused checkpoints usually skip only a few spans. Keep
        // their existing reader page instead of performing a full binary seek.
        for _ in 0..32 {
            if pos <= self.current.end || pos < self.current.body {
                break;
            }
            self.current = Span::read(&mut self.reader)?;
        }
        if pos >= self.current.body && pos <= self.current.end {
            return Ok(());
        }
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            self.reader.seek(SeekFrom::Start(mid * Span::SIZE))?;
            if Span::read(&mut self.reader)?.end <= pos {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        self.reader.seek(SeekFrom::Start(lo * Span::SIZE))?;
        if lo < self.count {
            self.current = Span::read(&mut self.reader)?;
        }
        Ok(())
    }
    pub fn at(&mut self, from: u64, to: u64) -> TextDocumentResult<(u64, u64)> {
        while self.current.end <= from {
            self.current = Span::read(&mut self.reader)?;
        }
        Ok(self.current.range(from, to))
    }
}

/// Remove balanced formatting markers before Unicode segmentation, so combining
/// characters and words still join across marks. Every stream has one 64 KiB page.
pub(super) fn compact(
    root: &Path,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> TextDocumentResult<u64> {
    use super::{check, scan::Block};
    let work = root.join("compact");
    std::fs::create_dir(&work)?;
    let mut output = Tape::new(&work)?;
    let mut masks = BufReader::with_capacity(CHUNK, File::open(root.join("removed"))?);
    let mut spans = BufReader::with_capacity(CHUNK, File::open(root.join("spans"))?);
    let span_count = spans.get_ref().metadata()?.len() / Span::SIZE;
    let body = File::open(root.join("body"))?;
    let length = body.metadata()?.len();
    let mut source = Source::file(body, length, cancel.clone());
    let mut blocks = BufReader::with_capacity(CHUNK, File::open(root.join("blocks"))?);
    let mut block_count = blocks.get_ref().metadata()?.len() / Block::SIZE;
    let mut block = if block_count > 0 {
        Some(Block::read(&mut blocks)?)
    } else {
        None
    };
    let mut block_start = 0;
    let mut task_remaining = 0;
    let mut next_block = true;
    let mut prefix_spans = BufReader::with_capacity(CHUNK, File::open(root.join("spans"))?);
    let mut prefix_masks = BufReader::with_capacity(CHUNK, File::open(root.join("removed"))?);
    let mut updated = BufWriter::with_capacity(CHUNK, File::create(work.join("blocks"))?);
    for index in 0..span_count {
        if next_block {
            next_block = false;
            if block.is_some_and(|b| b.task_list()) {
                prefix_spans.seek(SeekFrom::Start(index * Span::SIZE))?;
                prefix_masks.seek(SeekFrom::Start(index))?;
                let mut bytes = [0; 7];
                let mut used = 0;
                let mut starts_code = false;
                for _ in index..span_count {
                    check(&cancel)?;
                    let s = Span::read(&mut prefix_spans)?;
                    let mut skip = [0];
                    prefix_masks.read_exact(&mut skip)?;
                    if skip[0] == 0 {
                        if used == 0 {
                            starts_code = s.literal & 2 != 0;
                        }
                        for p in s.body..s.end {
                            if used == bytes.len() {
                                break;
                            }
                            bytes[used] = source.byte(p)?;
                            used += 1;
                        }
                    }
                    if starts_code || used == bytes.len() || s.end >= block.unwrap().body_end {
                        break;
                    }
                }
                if !starts_code
                    && used >= 3
                    && bytes[0] == b'['
                    && matches!(bytes[1], b' ' | b'x' | b'X')
                    && bytes[2] == b']'
                {
                    task_remaining = 3;
                    if let Ok(rest) = std::str::from_utf8(&bytes[3..used]) {
                        if let Some(c) = rest.chars().next() {
                            if c.is_whitespace() {
                                task_remaining += c.len_utf8() as u64;
                            }
                        }
                    }
                }
            }
        }
        check(&cancel)?;
        let span = Span::read(&mut spans)?;
        let mut skip = [0];
        masks.read_exact(&mut skip)?;
        if skip[0] == 0 {
            let remove = task_remaining.min(span.end - span.body);
            task_remaining -= remove;
            let start = span.body + remove;
            if start < span.end {
                for p in start..span.end {
                    output.body.write_all(&[source.byte(p)?])?;
                }
                let (from, to) = span.range(start, span.end);
                output.in_code = span.literal & 2 != 0;
                output.span(from, to, span.end - start, span.literal & 1 != 0)?;
            }
        }
        if block.as_ref().is_some_and(|b| b.body_end == span.end) {
            let mut b = block.take().unwrap();
            b.body = block_start;
            b.body_end = output.len;
            block_start = output.len;
            b.write(&mut updated)?;
            block_count -= 1;
            if block_count > 0 {
                block = Some(Block::read(&mut blocks)?);
            }
            next_block = true;
            task_remaining = 0;
        }
    }
    updated.flush()?;
    let (length, _) = output.finish()?;
    drop((
        source,
        spans,
        masks,
        blocks,
        updated,
        prefix_spans,
        prefix_masks,
    ));
    for name in ["body", "spans", "blocks"] {
        std::fs::remove_file(root.join(name))?;
        std::fs::rename(work.join(name), root.join(name))?;
    }
    std::fs::remove_file(work.join("removed"))?;
    std::fs::remove_dir_all(work)?;
    std::fs::remove_file(root.join("removed"))?;
    for depth in 0..20 {
        let path = root.join(format!("delimiters-{depth}"));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    let html_stack = root.join("html-stack");
    if html_stack.exists() {
        std::fs::remove_file(html_stack)?;
    }
    Ok(length)
}

use super::super::TextDocumentResult;
use super::{
    check,
    source::{Line, Source},
    tape::{CopySource, Tape},
    CHUNK,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

const PARAGRAPH: u64 = 0;
const HEADING: u64 = 1;
const TABLE: u64 = 2;
const HTML: u64 = 3;
const SKIP: u64 = 4;
const LIST: u64 = 1;
const QUOTE: u64 = 2;
const TABLE_CELL: u64 = 4;
const REUSED: u64 = 1;
const NEUTRAL_HTML: u64 = 2;

#[derive(Clone, Copy, Default)]
pub(super) struct Block {
    pub from: u64,
    pub to: u64,
    content: u64,
    end: u64,
    kind: u64,
    flags: u64,
    pub body: u64,
    pub body_end: u64,
    pub words: u64,
    pub visible: u64,
    pub reused: u64,
    pub digest: [u8; 32],
}
impl Block {
    pub(super) fn task_list(self) -> bool {
        self.kind != TABLE && self.flags & LIST != 0
    }
    pub const SIZE: u64 = 120;
    pub(super) fn write(self, out: &mut impl Write) -> std::io::Result<()> {
        for n in [
            self.from,
            self.to,
            self.content,
            self.end,
            self.kind,
            self.flags,
            self.body,
            self.body_end,
            self.words,
            self.visible,
            self.reused,
        ] {
            out.write_all(&n.to_le_bytes())?;
        }
        out.write_all(&self.digest)?;
        Ok(())
    }
    pub fn read(input: &mut impl Read) -> std::io::Result<Self> {
        let mut n = [0; 11];
        for value in &mut n {
            let mut b = [0; 8];
            input.read_exact(&mut b)?;
            *value = u64::from_le_bytes(b);
        }
        let mut digest = [0; 32];
        input.read_exact(&mut digest)?;
        Ok(Self {
            from: n[0],
            to: n[1],
            content: n[2],
            end: n[3],
            kind: n[4],
            flags: n[5],
            body: n[6],
            body_end: n[7],
            words: n[8],
            visible: n[9],
            reused: n[10],
            digest,
        })
    }
}

/// Disk hash buckets keep reference definitions out of the process heap. Labels
/// are bounded by CommonMark's 999-character limit; first definition wins.
const BUCKETS: usize = 65536;
struct References {
    file: File,
    heads: Vec<u64>,
}
impl References {
    fn new(root: &Path, name: &str) -> TextDocumentResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join(name))?;
        file.set_len((BUCKETS * 8) as u64)?;
        Ok(Self {
            file,
            heads: vec![0; BUCKETS],
        })
    }
    fn open(path: &Path) -> TextDocumentResult<Self> {
        let mut file = File::open(path)?;
        let mut heads = vec![0; BUCKETS];
        let mut buffer = vec![0; CHUNK];
        for page in heads.chunks_mut(CHUNK / 8) {
            file.read_exact(&mut buffer[..page.len() * 8])?;
            for (value, bytes) in page.iter_mut().zip(buffer.chunks_exact(8)) {
                *value = u64::from_le_bytes(bytes.try_into().unwrap());
            }
        }
        Ok(Self { file, heads })
    }
    fn key(text: &str) -> [u8; 32] {
        let normalized = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase();
        Sha256::digest(normalized.as_bytes()).into()
    }
    fn find(&mut self, key: &[u8; 32]) -> TextDocumentResult<Option<u64>> {
        let mut pos = self.heads[u16::from_le_bytes([key[0], key[1]]) as usize];
        while pos != 0 {
            self.file.seek(SeekFrom::Start(pos))?;
            let mut record = [0; 48];
            self.file.read_exact(&mut record)?;
            if &record[8..40] == key {
                return Ok(Some(u64::from_le_bytes(record[40..48].try_into().unwrap())));
            }
            pos = u64::from_le_bytes(record[..8].try_into().unwrap());
        }
        Ok(None)
    }
    fn insert(&mut self, text: &str, valid: bool) -> TextDocumentResult<()> {
        self.insert_value(Self::key(text), if valid { 1 } else { 2 })
    }
    fn insert_value(&mut self, key: [u8; 32], value: u64) -> TextDocumentResult<()> {
        if self.find(&key)?.is_some() {
            return Ok(());
        }
        let slot = u16::from_le_bytes([key[0], key[1]]) as usize;
        let bucket = slot as u64 * 8;
        let previous = self.heads[slot].to_le_bytes();
        let pos = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&previous)?;
        self.file.write_all(&key)?;
        self.file.write_all(&value.to_le_bytes())?;
        self.file.seek(SeekFrom::Start(bucket))?;
        self.file.write_all(&pos.to_le_bytes())?;
        self.heads[slot] = pos;
        Ok(())
    }
    fn contains(&mut self, text: &str) -> TextDocumentResult<bool> {
        Ok(self.find(&Self::key(text))? == Some(1))
    }
}

fn prefix(src: &mut Source, line: Line) -> TextDocumentResult<(u64, u64)> {
    let mut p = line.start;
    let mut flags = 0;
    loop {
        while p < line.end && matches!(src.byte(p)?, b' ' | b'\t') {
            p += 1;
        }
        if src.byte(p)? == b'>' {
            flags |= QUOTE;
            p += 1;
            if src.byte(p)? == b' ' {
                p += 1;
            }
            continue;
        }
        let mut end = p;
        if matches!(src.byte(p)?, b'-' | b'+' | b'*') {
            end += 1;
        } else {
            while end < p + 9 && src.byte(end)?.is_ascii_digit() {
                end += 1;
            }
            if end > p && matches!(src.byte(end)?, b'.' | b')') {
                end += 1;
            } else {
                end = p;
            }
        }
        if end > p && matches!(src.byte(end)?, b' ' | b'\t') {
            flags |= LIST;
            p = end + 1;
            continue;
        }
        break;
    }
    Ok((p.min(line.end), flags))
}
fn repeated(src: &mut Source, mut pos: u64, end: u64, ch: u8) -> TextDocumentResult<u64> {
    let start = pos;
    while pos < end && src.byte(pos)? == ch {
        pos += 1;
    }
    Ok(pos - start)
}
fn separator(src: &mut Source, from: u64, to: u64) -> TextDocumentResult<Option<u64>> {
    let (mut p, end) = src.trim(from, to)?;
    if p == end {
        return Ok(None);
    }
    if src.byte(p)? == b'|' {
        p += 1;
    }
    let mut count = 0;
    while p < end {
        while p < end && src.byte(p)? == b' ' {
            p += 1;
        }
        if src.byte(p)? == b':' {
            p += 1;
        }
        let n = repeated(src, p, end, b'-')?;
        if n == 0 {
            return Ok(None);
        }
        p += n;
        if src.byte(p)? == b':' {
            p += 1;
        }
        while p < end && matches!(src.byte(p)?, b' ' | b'\t') {
            p += 1;
        }
        count += 1;
        if p == end {
            break;
        }
        if src.byte(p)? != b'|' {
            return Ok(None);
        }
        p += 1;
    }
    Ok(Some(count))
}
fn cells(src: &mut Source, from: u64, to: u64) -> TextDocumentResult<u64> {
    let (mut p, mut end) = src.trim(from, to)?;
    if src.byte(p)? == b'|' {
        p += 1;
    }
    if end > p && src.byte(end - 1)? == b'|' && (end < 2 || src.byte(end - 2)? != b'\\') {
        end -= 1;
    }
    let mut count = 1;
    while p < end {
        if src.byte(p)? == b'|' && (p == 0 || src.byte(p - 1)? != b'\\') {
            count += 1;
        }
        p += 1;
    }
    Ok(count)
}
fn valid_link(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    !lower.starts_with("javascript:")
        && !lower.starts_with("vbscript:")
        && !lower.starts_with("data:")
        && !lower.chars().any(|c| c.is_control())
}

fn structure(
    src: &mut Source,
    root: &Path,
    refs: &mut References,
) -> TextDocumentResult<(u64, u64)> {
    let mut output = BufWriter::with_capacity(CHUNK, File::create(root.join("structure"))?);
    let mut pending: Option<Block> = None;
    let mut count = 0;
    let mut headings = 0;
    let mut pos = 0;
    let mut fence: Option<(u8, u64)> = None;
    let mut table_columns = 0;
    let mut list_indent = 0;
    let mut push = |block: Block, pending: &mut Option<Block>| -> TextDocumentResult<()> {
        if let Some(previous) = pending.take() {
            previous.write(&mut output)?;
            count += 1;
        }
        *pending = Some(block);
        Ok(())
    };
    if src.starts(0, b"---\n")? || src.starts(0, b"---\r\n")? {
        let first = src.line(0)?;
        let mut p = first.next;
        while p < src.len {
            let line = src.line(p)?;
            if src.small(line.start, line.end.min(line.start + 4))? == "---" && line.next > line.end
            {
                pos = line.next;
                break;
            }
            p = line.next;
        }
    }
    while pos < src.len {
        check(&src.cancel)?;
        let line = src.line(pos)?;
        let (mut content, mut flags) = prefix(src, line)?;
        let starts_list_item = flags & LIST != 0;
        let (_, end) = src.trim(content, line.end)?;
        let indent = content - line.start;
        let first_byte = src.byte(content)?;
        if flags & LIST != 0 {
            list_indent = indent;
        } else if list_indent > 0 && indent >= list_indent {
            flags |= LIST;
        } else if content < end {
            list_indent = 0;
        }
        let mut block = Block {
            from: pos,
            to: line.next,
            content,
            end,
            kind: PARAGRAPH,
            flags,
            ..Block::default()
        };
        pos = line.next;
        if let Some((marker, length)) = fence {
            block.kind = SKIP;
            let n = repeated(src, content, end, marker)?;
            if n >= length && src.trim(content + n, end)?.0 == end {
                fence = None;
            }
        } else if content == end {
            block.kind = SKIP;
            table_columns = 0;
        } else if matches!(first_byte, b'`' | b'~') && repeated(src, content, end, first_byte)? >= 3
        {
            let marker = src.byte(content)?;
            let n = repeated(src, content, end, marker)?;
            if marker != b'`' || src.find(content + n, end, b"`")?.is_none() {
                fence = Some((marker, n));
                block.kind = SKIP;
                table_columns = 0;
            }
        } else if indent >= 4 && flags == 0 {
            block.kind = SKIP;
            table_columns = 0;
        } else if src.starts(content, b"<!--")? {
            let short = src.small(content, end.min(content + 80))?;
            let closing = if short.trim() == "<!-- toc -->" {
                b"<!-- /toc -->".as_slice()
            } else {
                b"-->".as_slice()
            };
            let close = src.find(content + 4, src.len, closing)?;
            if let Some(close) = close {
                pos = src.line(close + closing.len() as u64)?.next;
            } else {
                pos = src.len;
            }
            block.to = pos;
            block.kind = SKIP;
            table_columns = 0;
        } else if src.starts(content, b"$$")? {
            let mut close = None;
            if end > content + 4 && src.starts(end - 2, b"$$")? {
                close = Some(line.next);
            }
            let mut next = line.next;
            while close.is_none() && next < src.len {
                let other = src.line(next)?;
                let (_, right) = src.trim(other.start, other.end)?;
                if right >= other.start + 2 && src.starts(right - 2, b"$$")? {
                    close = Some(other.next);
                }
                next = other.next;
            }
            if let Some(close) = close {
                pos = close;
                block.to = pos;
                block.kind = SKIP;
                table_columns = 0;
            }
        } else if src.starts(content, b"<details")? {
            if let Some(summary) = src.find(
                content,
                end.max((content + CHUNK as u64).min(src.len)),
                b"</summary>",
            )? {
                if src.find(summary + 10, src.len, b"</details>")?.is_some() {
                    pos = src.line(summary + 10)?.next;
                    block.to = pos;
                    block.kind = SKIP;
                }
            }
        } else if src.starts(content, b"</details>")? {
            block.kind = SKIP;
        } else {
            let hashes = repeated(src, content, end, b'#')?;
            if hashes > 0
                && hashes <= 6
                && (content + hashes == end || matches!(src.byte(content + hashes)?, b' ' | b'\t'))
            {
                content = src.trim(content + hashes, end)?.0;
                block.content = content;
                block.kind = HEADING;
                headings += 1;
                table_columns = 0;
                let mut tail = end;
                while tail > content && src.byte(tail - 1)? == b'#' {
                    tail -= 1;
                }
                if tail < end && tail > content && src.byte(tail - 1)?.is_ascii_whitespace() {
                    block.end = src.trim(content, tail)?.1;
                }
            } else if flags & QUOTE != 0 && src.starts(content, b"[!")? {
                if let Some(close) = src.find(content + 2, end.min(content + 20), b"]")? {
                    let name = src.small(content + 2, close)?;
                    if matches!(
                        name.as_str(),
                        "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
                    ) {
                        block.content = close + 1;
                    }
                }
            } else if src.starts(content, b"[^")? {
                if let Some(close) = src.find(content + 2, end.min(content + 4096), b"]:")? {
                    if !src
                        .small(content + 2, close)?
                        .chars()
                        .any(char::is_whitespace)
                    {
                        block.content = src.trim(close + 2, end)?.0;
                    }
                }
            } else if src.byte(content)? == b'['
                && !pending
                    .as_ref()
                    .is_some_and(|b| b.kind == PARAGRAPH && b.to == block.from && b.flags == flags)
            {
                if let Some(close) = src.find(content + 1, end.min(content + 4096), b"]:")? {
                    let label = src.small(content + 1, close)?;
                    if label.chars().count() <= 999 && !label.trim().is_empty() {
                        let (mut url, mut url_end) = src.trim(close + 2, end)?;
                        let mut reference_end = line.next;
                        if url == url_end && line.next < src.len {
                            let next = src.line(line.next)?;
                            let (start, right) = src.trim(next.start, next.end)?;
                            if start - next.start <= 3 && start < right {
                                url = start;
                                url_end = right;
                                reference_end = next.next;
                            }
                        }
                        if url < url_end {
                            let value = src.small(url, url_end.min(url + CHUNK as u64))?;
                            refs.insert(
                                &label,
                                valid_link(value.split_whitespace().next().unwrap_or("")),
                            )?;
                            block.kind = SKIP;
                            if reference_end < src.len {
                                let title = src.line(reference_end)?;
                                let (left, right) = src.trim(title.start, title.end)?;
                                let quote = src.byte(left)?;
                                if right > left + 1
                                    && matches!(quote, b'\'' | b'"' | b'(')
                                    && src.byte(right - 1)?
                                        == if quote == b'(' { b')' } else { quote }
                                {
                                    reference_end = title.next;
                                }
                            }
                            block.to = reference_end;
                            pos = reference_end;
                        }
                    }
                }
            }
            if block.kind == PARAGRAPH
                && block.content == content
                && table_columns == 0
                && line.next < src.len
            {
                let next = src.line(line.next)?;
                let (left, _) = prefix(src, next)?;
                if let Some(columns) = separator(src, left, next.end)? {
                    if src.find(content, end, b"|")?.is_some()
                        && cells(src, content, end)? == columns
                    {
                        block.kind = TABLE;
                        block.flags = columns;
                        table_columns = columns;
                        block.to = next.next;
                        pos = next.next;
                    } else if src.trim(left, next.end)?.0 < next.end
                        && src.find(left, next.end, b"|")?.is_none()
                        && src.find(left, next.end, b":")?.is_none()
                    {
                        block.kind = HEADING;
                        headings += 1;
                        block.to = next.next;
                        pos = next.next;
                    }
                } else {
                    let (a, b) = src.trim(left, next.end)?;
                    if a < b && repeated(src, a, b, b'=')? == b - a {
                        block.kind = HEADING;
                        headings += 1;
                        block.to = next.next;
                        pos = next.next;
                    }
                }
            }
            if block.kind == PARAGRAPH && table_columns > 0 {
                block.kind = TABLE;
                block.flags = table_columns;
            }
            if block.kind == PARAGRAPH && src.byte(content)? == b'<' {
                if let Some(tag) = tag(src, content, end)? {
                    if !tag.name.is_empty() && (is_block_tag(&tag.name) || tag.end == end) {
                        let mut next = line.next;
                        while next < src.len {
                            let other = src.line(next)?;
                            if src.trim(other.start, other.end)?.0 == other.end {
                                break;
                            }
                            next = other.next;
                        }
                        block.kind = HTML;
                        block.end = next;
                        block.to = next;
                        pos = next;
                    }
                }
            }
            if block.kind == PARAGRAPH {
                let marker = src.byte(content)?;
                if matches!(marker, b'-' | b'*' | b'_') {
                    let mut n = 0;
                    let mut p = content;
                    let mut rule = true;
                    while p < end {
                        let b = src.byte(p)?;
                        if b == marker {
                            n += 1;
                        } else if !b.is_ascii_whitespace() {
                            rule = false;
                            break;
                        }
                        p += 1;
                    }
                    if rule && n >= 3 {
                        block.kind = SKIP;
                    }
                }
            }
        }
        if block.kind == PARAGRAPH && !starts_list_item {
            if let Some(previous) = &mut pending {
                if previous.kind == PARAGRAPH
                    && previous.to == block.from
                    && previous.flags == block.flags
                    && block.content == content
                {
                    previous.to = block.to;
                    previous.end = block.end;
                    continue;
                }
            }
        }
        push(block, &mut pending)?;
    }
    if let Some(block) = pending {
        block.write(&mut output)?;
        count += 1;
    }
    output.flush()?;
    Ok((count, headings))
}

#[derive(Default)]
struct Tag {
    name: String,
    end: u64,
    closing: bool,
    self_closing: bool,
    attrs: Attributes,
}
fn tag(src: &mut Source, start: u64, end: u64) -> TextDocumentResult<Option<Tag>> {
    let mut p = start + 1;
    let closing = src.byte(p)? == b'/';
    if closing {
        p += 1;
    }
    let name_start = p;
    while p < end && (src.byte(p)?.is_ascii_alphanumeric() || matches!(src.byte(p)?, b'-' | b':')) {
        p += 1;
    }
    if p == name_start || p - name_start > 64 {
        return Ok(None);
    }
    let name = src.small(name_start, p)?.to_ascii_lowercase();
    let attrs_start = p;
    let mut quote = 0;
    while p < end {
        let c = src.byte(p)?;
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if matches!(c, b'\'' | b'"') {
            quote = c;
        } else if c == b'>' {
            let self_closing = p > start && src.byte(p - 1)? == b'/';
            let attrs = read_attributes(src, attrs_start, p, name == "a")?;
            return Ok(Some(Tag {
                name,
                end: p + 1,
                closing,
                self_closing,
                attrs,
            }));
        } else if c == b'<' {
            return Ok(None);
        }
        p += 1;
    }
    Ok(None)
}
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "div"
            | "section"
            | "p"
            | "pre"
            | "script"
            | "style"
            | "table"
            | "ul"
            | "ol"
            | "li"
            | "details"
            | "summary"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "form"
            | "iframe"
            | "hr"
    )
}

/// A plain edit inside one established paragraph cannot change block or reference
/// state. Resume that checkpoint and converge at its end. Structural/global
/// changes deliberately use the bounded full pass below.
fn local_structure(
    src: &mut Source,
    root: &Path,
    path: &Path,
    snapshot: &super::super::chunk_reader::DocumentSnapshot,
) -> TextDocumentResult<Option<(u64, u64)>> {
    if !path.join("references").exists() {
        return Ok(None);
    }
    let mut old = Source::snapshot(snapshot.clone(), src.cancel.clone());
    let common = old.len.min(src.len);
    let mut from = 0;
    while from < common && old.byte(from)? == src.byte(from)? {
        from += 1;
    }
    if from == common {
        return Ok(None);
    }
    while from > 0 && src.byte(from)? & 0xc0 == 0x80 {
        from -= 1;
    }
    let (mut old_to, mut new_to) = (old.len, src.len);
    while old_to > from && new_to > from && old.byte(old_to - 1)? == src.byte(new_to - 1)? {
        old_to -= 1;
        new_to -= 1;
    }
    while new_to < src.len && src.byte(new_to)? & 0xc0 == 0x80 {
        old_to += 1;
        new_to += 1;
    }
    // This fast path never has to retain text or infer syntax from an edit alone.
    for (input, end) in [(&mut old, old_to), (&mut *src, new_to)] {
        let mut p = from;
        while p < end {
            let (c, next) = input.char(p)?;
            if !(c.is_alphanumeric()
                || matches!(c, ' ' | '\t')
                || c.general_category_group() == GeneralCategoryGroup::Mark)
            {
                return Ok(None);
            }
            p = next;
        }
    }
    let mut input = BufReader::with_capacity(CHUNK, File::open(path.join("blocks"))?);
    let count = input.get_ref().metadata()?.len() / Block::SIZE;
    let mut target = None;
    for i in 0..count {
        check(&src.cancel)?;
        let block = Block::read(&mut input)?;
        if block.from <= from && from < block.to {
            if !matches!(block.kind, PARAGRAPH | HEADING)
                || block.flags != 0
                || from <= block.content
                || old_to >= block.end
            {
                return Ok(None);
            }
            target = Some(i);
            break;
        }
    }
    let Some(target) = target else {
        return Ok(None);
    };
    input.seek(SeekFrom::Start(0))?;
    let mut output = BufWriter::with_capacity(CHUNK, File::create(root.join("structure"))?);
    let delta = src.len as i128 - old.len as i128;
    let mut headings = 0;
    for i in 0..count {
        check(&src.cancel)?;
        let mut block = Block::read(&mut input)?;
        if block.kind == HEADING {
            headings += 1;
        }
        block.reused = (block.reused & NEUTRAL_HTML) | REUSED;
        if i == target {
            block.to = (block.to as i128 + delta) as u64;
            block.end = (block.end as i128 + delta) as u64;
            block.reused = 0;
        } else if i > target {
            block.from = (block.from as i128 + delta) as u64;
            block.to = (block.to as i128 + delta) as u64;
            block.content = (block.content as i128 + delta) as u64;
            block.end = (block.end as i128 + delta) as u64;
        }
        block.write(&mut output)?;
    }
    output.flush()?;
    std::fs::copy(path.join("references"), root.join("references"))?;
    Ok(Some((count, headings)))
}

pub(super) fn build(
    src: &mut Source,
    root: &Path,
    previous: Option<(&Path, &super::super::chunk_reader::DocumentSnapshot)>,
) -> TextDocumentResult<(u64, u64, u64)> {
    let local = if let Some((path, snapshot)) = previous {
        local_structure(src, root, path, snapshot)?
    } else {
        None
    };
    let mut refs = if local.is_some() {
        References::open(&root.join("references"))?
    } else {
        References::new(root, "references")?
    };
    let (count, headings) = if let Some(local) = local {
        local
    } else {
        structure(src, root, &mut refs)?
    };
    let previous = previous.map(|(path, _)| path);
    let mut hash = Sha256::new();
    refs.file.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0; CHUNK];
    loop {
        let n = refs.file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        check(&src.cancel)?;
        hash.update(&buffer[..n]);
    }
    let signature: [u8; 32] = hash.finalize().into();
    drop(buffer);
    std::fs::write(root.join("reference-signature"), signature)?;
    let previous = previous.filter(|path| {
        std::fs::read(path.join("reference-signature")).is_ok_and(|old| old == signature)
    });
    let mut prior = previous
        .map(|path| -> TextDocumentResult<_> {
            Ok((
                References::open(&path.join("lookup"))?,
                BufReader::with_capacity(CHUNK, File::open(path.join("blocks"))?),
                CopySource::open(path, src.cancel.clone())?,
                0u64,
            ))
        })
        .transpose()?;
    let mut lookup = References::new(root, "lookup")?;
    let mut input = BufReader::with_capacity(CHUNK, File::open(root.join("structure"))?);
    let mut output = BufWriter::with_capacity(CHUNK, File::create(root.join("blocks"))?);
    let mut tape = Tape::new(root)?;
    let mut reused = 0;
    for index in 0..count {
        check(&src.cancel)?;
        let mut block = Block::read(&mut input)?;
        block.body = tape.len;
        let known_index = local.is_some() && block.reused & REUSED != 0;
        if block.reused & REUSED == 0 {
            let mut hash = Sha256::new();
            hash.update(block.kind.to_le_bytes());
            hash.update(block.flags.to_le_bytes());
            hash.update((block.content - block.from).to_le_bytes());
            hash.update((block.end - block.from).to_le_bytes());
            for pos in block.from..block.to {
                hash.update([src.byte(pos)?]);
            }
            block.digest = hash.finalize().into();
        }
        block.reused = 0;
        lookup.insert_value(block.digest, index + 1)?;
        // A checkpoint whose incoming/outgoing HTML state was empty is safe to
        // reuse once the new scan reaches the same empty state, including HTML blocks.
        let reusable = tape.marks_empty();
        if let Some((lookup, file, reader, position)) = prior.as_mut().filter(|_| reusable) {
            // Plain edits preserve block ordinals. Do not resolve repeated cells
            // through the first matching hash and seek backwards for each row.
            let old = if known_index {
                Some(index + 1)
            } else {
                lookup.find(&block.digest)?
            };
            if let Some(old) = old {
                let offset = (old - 1) * Block::SIZE;
                if offset != *position {
                    file.seek_relative(offset as i64 - *position as i64)?;
                }
                let old = Block::read(file)?;
                *position = offset + Block::SIZE;
                if old.reused & NEUTRAL_HTML != 0 {
                    tape.copy(
                        reader,
                        old.body,
                        old.body_end,
                        block.from as i128 - old.from as i128,
                    )?;
                    block.body_end = tape.len;
                    block.words = old.words;
                    block.visible = old.visible;
                    block.reused = REUSED | NEUTRAL_HTML;
                    block.write(&mut output)?;
                    reused += 1;
                    continue;
                }
            }
        }
        let neutral_before = tape.marks_empty();
        match block.kind {
            SKIP => {}
            HTML => html(src, &mut tape, block)?,
            TABLE => {
                let (mut start, mut end) = src.trim(block.content, block.end)?;
                if src.byte(start)? == b'|' {
                    start += 1;
                }
                if end > start && src.byte(end - 1)? == b'|' && src.byte(end - 2)? != b'\\' {
                    end -= 1;
                }
                let mut p = start;
                let mut column = 0;
                while p <= end && column < block.flags {
                    if p == end
                        || (src.byte(p)? == b'|' && (p == start || src.byte(p - 1)? != b'\\'))
                    {
                        let (a, b) = src.trim(start, p)?;
                        inline(src, &mut tape, &mut refs, a, b, TABLE_CELL, 0)?;
                        tape.boundary(p)?;
                        column += 1;
                        start = p + 1;
                    }
                    p += 1;
                }
            }
            _ => inline(
                src,
                &mut tape,
                &mut refs,
                block.content,
                block.end,
                block.flags,
                0,
            )?,
        }
        tape.boundary(block.to)?;
        block.body_end = tape.len;
        if neutral_before && tape.marks_empty() {
            block.reused |= NEUTRAL_HTML;
        }
        block.write(&mut output)?;
    }
    output.flush()?;
    tape.finish()?;
    drop((output, input, refs, lookup, prior));
    let length = super::tape::compact(root, src.cancel.clone())?;
    std::fs::remove_file(root.join("structure"))?;
    let stack = root.join("html-stack");
    if stack.exists() {
        std::fs::remove_file(stack)?;
    }
    Ok((length, headings, reused))
}

/// Initial counts stream once over the complete tape. Incremental scans reuse
/// aggregates for converged block+grammar fingerprints, counting only changed
/// checkpoints. Global reference changes invalidate reuse above.
pub(super) fn summarize(
    root: &Path,
    length: u64,
    reused: u64,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> TextDocumentResult<(u64, u64)> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("blocks"))?;
    let blocks = file.metadata()?.len() / Block::SIZE;
    if reused == 0 && blocks > 0 {
        let mut input = BufReader::with_capacity(CHUNK, file.try_clone()?);
        let mut output =
            BufWriter::with_capacity(CHUNK, File::create(root.join("counted-blocks"))?);
        let mut next = Some(Block::read(&mut input)?);
        let (mut words, mut chars, mut index) = (0, 0, 0);
        let mut boundary =
            |end: u64, total_words: u64, total_chars: u64| -> TextDocumentResult<()> {
                if next.as_ref().is_some_and(|block| block.body_end == end) {
                    let mut block = next.take().unwrap();
                    block.words = total_words - words;
                    block.visible = total_chars - chars;
                    words = total_words;
                    chars = total_chars;
                    block.write(&mut output)?;
                    index += 1;
                    if index < blocks {
                        next = Some(Block::read(&mut input)?);
                    }
                }
                Ok(())
            };
        let (words, chars, _, _) =
            super::count::count_with_boundaries(root, 0, length, None, cancel, &mut boundary)?;
        output.flush()?;
        drop(output);
        drop(input);
        drop(file);
        std::fs::remove_file(root.join("blocks"))?;
        std::fs::rename(root.join("counted-blocks"), root.join("blocks"))?;
        return Ok((words, chars));
    }
    let (mut words, mut chars) = (0, 0);
    // An independent sequential handle keeps cached counts off the random-I/O path.
    let mut reader = BufReader::with_capacity(CHUNK, File::open(root.join("blocks"))?);
    for index in 0..blocks {
        check(&cancel)?;
        let mut block = Block::read(&mut reader)?;
        if block.reused & REUSED == 0 {
            let (w, c, _, _) =
                super::count::count(root, block.body, block.body_end, None, cancel.clone())?;
            block.words = w;
            block.visible = c;
            file.seek(SeekFrom::Start(index * Block::SIZE))?;
            block.write(&mut file)?;
        }
        words += block.words;
        chars += block.visible;
    }
    Ok((words, chars))
}

// Inline and HTML scanners below only hold positions and the current small tag.
// Recursive syntax nesting matches markdown-it's maxNesting (20).
fn inline(
    src: &mut Source,
    tape: &mut Tape,
    refs: &mut References,
    from: u64,
    to: u64,
    flags: u64,
    depth: usize,
) -> TextDocumentResult<()> {
    inline_scan(src, tape, refs, from, to, flags, depth)
}

fn html(src: &mut Source, tape: &mut Tape, block: Block) -> TextDocumentResult<()> {
    html_block(src, tape, block)
}

fn entity(src: &mut Source, from: u64, to: u64) -> TextDocumentResult<Option<(String, u64)>> {
    let Some(end) = src.find(from + 1, to.min(from + 34), b";")? else {
        return Ok(None);
    };
    let name = src.small(from + 1, end + 1)?;
    let mut text = String::new();
    if let Some(number) = name.strip_prefix('#') {
        let number = number.trim_end_matches(';');
        let code = if let Some(hex) = number
            .strip_prefix('x')
            .or_else(|| number.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            number.parse::<u32>().ok()
        };
        if let Some(code) = code {
            let c = if code == 0 {
                '\u{fffd}'
            } else {
                char::from_u32(code).unwrap_or('\u{fffd}')
            };
            text.push(if (0x80..=0x9f).contains(&code) {
                web_atoms::C1_REPLACEMENTS[(code - 0x80) as usize].unwrap_or(c)
            } else {
                c
            });
        }
    } else if let Some(&(a, b)) = web_atoms::NAMED_ENTITIES.get(name.as_str()) {
        if let Some(c) = char::from_u32(a) {
            text.push(c);
        }
        if b != 0 {
            if let Some(c) = char::from_u32(b) {
                text.push(c);
            }
        }
    }
    Ok(if text.is_empty() {
        None
    } else {
        Some((text, end + 1))
    })
}
fn code(
    src: &mut Source,
    tape: &mut Tape,
    mut from: u64,
    mut to: u64,
    flags: u64,
    decode: bool,
) -> TextDocumentResult<()> {
    let previous_code = tape.in_code;
    tape.in_code = true;
    if !decode
        && from < to
        && matches!(src.byte(from)?, b' ' | b'\r' | b'\n')
        && matches!(src.byte(to - 1)?, b' ' | b'\r' | b'\n')
    {
        let mut p = from;
        let mut all_space = true;
        while p < to {
            if !matches!(src.byte(p)?, b' ' | b'\r' | b'\n') {
                all_space = false;
                break;
            }
            p += 1;
        }
        if !all_space {
            from += 1;
            to -= 1;
        }
    }
    let mut p = from;
    while p < to {
        let c = src.byte(p)?;
        if flags & TABLE_CELL != 0 && c == b'\\' && src.byte(p + 1)? == b'|' {
            tape.text("|", p, p + 2)?;
            p += 2;
        } else if decode && c == b'&' {
            if let Some((text, end)) = entity(src, p, to)? {
                tape.text(&text, p, end)?;
                p = end;
            } else {
                tape.literal(src, p, p + 1)?;
                p += 1;
            }
        } else if matches!(c, b'\n' | b'\r') {
            let mut end = p + 1;
            if c == b'\r' && src.byte(end)? == b'\n' {
                end += 1;
            }
            tape.text(if decode { "\n" } else { " " }, p, end)?;
            p = end;
        } else {
            let (_, end) = src.char(p)?;
            tape.literal(src, p, end)?;
            p = end;
        }
    }
    tape.in_code = previous_code;
    Ok(())
}
fn closing(
    src: &mut Source,
    from: u64,
    to: u64,
    open: u8,
    close: u8,
) -> TextDocumentResult<Option<u64>> {
    let mut nesting = 1u64;
    let mut p = from;
    while p < to {
        let c = src.byte(p)?;
        if c == b'\\' {
            p += 2;
            continue;
        }
        if c == open {
            nesting += 1;
        } else if c == close {
            nesting -= 1;
            if nesting == 0 {
                return Ok(Some(p));
            }
        }
        p += 1;
    }
    Ok(None)
}
fn mark_tag(name: &str) -> Option<u8> {
    super::tape::HTML_MARKS
        .iter()
        .position(|tag| *tag == name)
        .map(|i| i as u8)
}

fn delimiter(
    src: &mut Source,
    pos: u64,
    length: u64,
    from: u64,
    to: u64,
) -> TextDocumentResult<(bool, bool)> {
    let mut before = '\n';
    if pos > from {
        let mut p = pos - 1;
        while p > from && src.byte(p)? & 0xc0 == 0x80 {
            p -= 1;
        }
        before = src.char(p)?.0;
    }
    let after = if pos + length < to {
        src.char(pos + length)?.0
    } else {
        '\n'
    };
    let punct = |c: char| {
        matches!(
            c.general_category_group(),
            GeneralCategoryGroup::Punctuation | GeneralCategoryGroup::Symbol
        )
    };
    let left = !after.is_whitespace() && (!punct(after) || before.is_whitespace() || punct(before));
    let right =
        !before.is_whitespace() && (!punct(before) || after.is_whitespace() || punct(after));
    if src.byte(pos)? == b'_' {
        Ok((
            left && (!right || punct(before)),
            right && (!left || punct(after)),
        ))
    } else {
        Ok((left, right))
    }
}
const ATTRIBUTE_NAMES: &[&str] = &[
    "class",
    "id",
    "href",
    "title",
    "name",
    "src",
    "alt",
    "width",
    "align",
    "data-nomo-break",
    "data-tex",
    "data-comment",
    "data-footnote-id",
];
#[derive(Default)]
struct Attributes {
    values: [Option<String>; 13],
    unknown: bool,
    double_href: Option<String>,
}
fn equals_ascii(src: &mut Source, from: u64, to: u64, text: &str) -> TextDocumentResult<bool> {
    if to - from != text.len() as u64 {
        return Ok(false);
    }
    for (i, b) in text.bytes().enumerate() {
        if !src.byte(from + i as u64)?.eq_ignore_ascii_case(&b) {
            return Ok(false);
        }
    }
    Ok(true)
}
// Predicates only inspect small exact values or a URL scheme. Preserve those
// decisions, including controls anywhere in a long URL, without retaining values.
fn attribute_value(
    src: &mut Source,
    mut from: u64,
    to: u64,
    decode: bool,
) -> TextDocumentResult<String> {
    let (mut raw, mut trimmed) = (String::new(), String::new());
    let (mut length, mut seen, mut pending_control, mut control) = (0, false, false, false);
    let mut append = |c: char| {
        length += c.len_utf8();
        if raw.len() < 128 {
            raw.push(c);
        }
        if !c.is_whitespace() {
            control |= pending_control || c.is_control();
            pending_control = false;
            seen = true;
        } else if seen {
            pending_control |= c.is_control();
        }
        if seen && trimmed.len() < 128 {
            trimmed.push(c);
        }
    };
    while from < to {
        if decode && src.byte(from)? == b'&' {
            if let Some((text, end)) = entity(src, from, to)? {
                for c in text.chars() {
                    append(c);
                }
                from = end;
                continue;
            }
        }
        let (c, end) = src.char(from)?;
        append(c);
        from = end;
    }
    if length <= 128 {
        return Ok(raw);
    }
    if !seen {
        return Ok(" ".into());
    }
    trimmed.push('�');
    if control {
        trimmed.push('\0');
    }
    Ok(trimmed)
}
fn read_attributes(
    src: &mut Source,
    from: u64,
    to: u64,
    anchor: bool,
) -> TextDocumentResult<Attributes> {
    let mut result = Attributes::default();
    let mut p = from;
    while p < to {
        while p < to && src.byte(p)?.is_ascii_whitespace() {
            p += 1;
        }
        if p == to || (src.byte(p)? == b'/' && src.trim(p + 1, to)?.0 == to) {
            break;
        }
        let start = p;
        while p < to && !src.byte(p)?.is_ascii_whitespace() && !matches!(src.byte(p)?, b'=' | b'/')
        {
            p += 1;
        }
        if p == start {
            result.unknown = true;
            p += 1;
            continue;
        }
        let mut key = None;
        for (i, name) in ATTRIBUTE_NAMES.iter().enumerate() {
            if equals_ascii(src, start, p, name)? {
                key = Some(i);
                break;
            }
        }
        if key.is_none() {
            result.unknown = true;
        }
        while p < to && src.byte(p)?.is_ascii_whitespace() {
            p += 1;
        }
        let (mut start, mut end) = (p, p);
        if src.byte(p)? == b'=' {
            p += 1;
            while p < to && src.byte(p)?.is_ascii_whitespace() {
                p += 1;
            }
            if matches!(src.byte(p)?, b'\'' | b'"') {
                let quote = src.byte(p)?;
                p += 1;
                start = p;
                while p < to && src.byte(p)? != quote {
                    p += 1;
                }
                end = p;
                p += 1;
            } else {
                start = p;
                while p < to && !src.byte(p)?.is_ascii_whitespace() {
                    p += 1;
                }
                end = p;
            }
        }
        if let Some(key) = key {
            if result.values[key].is_none() {
                result.values[key] = Some(attribute_value(src, start, end, true)?);
            }
        }
    }
    // Match the existing inline HTML adapter's literal double-quoted href rule.
    if anchor {
        p = from;
        while p + 6 <= to {
            if equals_ascii(src, p, p + 6, "href=\"")? {
                if let Some(end) = src.find(p + 6, to, b"\"")? {
                    result.double_href = Some(attribute_value(src, p + 6, end, false)?);
                }
                break;
            }
            p += 1;
        }
    }
    Ok(result)
}
fn attribute<'a>(raw: &'a Attributes, name: &str) -> Option<&'a str> {
    ATTRIBUTE_NAMES
        .iter()
        .position(|n| *n == name)
        .and_then(|i| raw.values[i].as_deref())
}
fn attributes_allowed(raw: &Attributes, names: &[&str]) -> bool {
    !raw.unknown
        && raw
            .values
            .iter()
            .enumerate()
            .all(|(i, value)| value.is_none() || names.contains(&ATTRIBUTE_NAMES[i]))
}
fn double_quoted_href(raw: &Attributes) -> Option<&str> {
    raw.double_href.as_deref()
}

fn inline_scan(
    src: &mut Source,
    tape: &mut Tape,
    refs: &mut References,
    from: u64,
    to: u64,
    flags: u64,
    depth: usize,
) -> TextDocumentResult<()> {
    if depth >= 20 {
        return tape.literal(src, from, to);
    }
    let mut p = from;
    let mut pairs: Option<super::delimiters::Delimiters> = None;
    let mut beginning = true;
    while p < to {
        if p % 4096 == 0 {
            check(&src.cancel)?;
        }
        let c = src.byte(p)?;
        if flags & TABLE_CELL != 0 && c == b'\\' {
            let n = repeated(src, p, to, b'\\')?;
            if src.byte(p + n)? == b'|' {
                for pair in 0..(n - 1) / 2 {
                    tape.text("\\", p + pair * 2, p + pair * 2 + 2)?;
                }
                tape.text("|", p + n - 1 - (n - 1) % 2, p + n + 1)?;
                p += n + 1;
                beginning = false;
                continue;
            }
        }
        if beginning
            && flags & LIST != 0
            && src.starts(p, b"[")?
            && matches!(src.byte(p + 1)?, b' ' | b'x' | b'X')
            && src.byte(p + 2)? == b']'
        {
            p += 3;
            if p < to && src.char(p)?.0.is_whitespace() {
                p = src.char(p)?.1;
            }
            beginning = false;
            continue;
        }
        if c == b'\\'
            && p + 1 < to
            && (src.byte(p + 1)?.is_ascii_punctuation()
                || matches!(src.byte(p + 1)?, b'\r' | b'\n'))
        {
            let next = src.byte(p + 1)?;
            if matches!(next, b'\n' | b'\r') {
                tape.text("\n", p, p + 2)?;
            } else {
                let text = String::from(next as char);
                tape.text(&text, p, p + 2)?;
            }
            p += 2;
            beginning = false;
            continue;
        }
        if c == b'`' {
            let run = repeated(src, p, to, c)?;
            let mut next = p + run;
            let mut found = None;
            while next < to {
                if src.byte(next)? == c {
                    let n = repeated(src, next, to, c)?;
                    if n == run {
                        found = Some(next);
                        break;
                    }
                    next += n;
                } else {
                    next += 1;
                }
            }
            if let Some(end) = found {
                code(src, tape, p + run, end, flags, false)?;
                p = end + run;
                beginning = false;
                continue;
            }
        }
        if c == b'$'
            && src.byte(p + 1)? != b'$'
            && !(p > from
                && src.byte(p - 1)? == b'$'
                && (p == from + 1 || src.byte(p - 2)? == b'$'))
        {
            let mut next = p + 1;
            let mut found = None;
            while next < to {
                let b = src.byte(next)?;
                if b == b'\\' {
                    next += 2;
                } else if b == b'$' {
                    found = Some(next);
                    break;
                } else {
                    next += 1;
                }
            }
            if let Some(end) = found {
                if src.trim(p + 1, end)?.0 < end {
                    p = end + 1;
                    continue;
                }
            }
        }
        if c == b'&' {
            if let Some((text, end)) = entity(src, p, to)? {
                tape.text(&text, p, end)?;
                p = end;
                beginning = false;
                continue;
            }
        }
        if src.starts(p, b"[^")? {
            if let Some(end) = src.find(p + 2, to.min(p + 4096), b"]")? {
                let id = src.small(p + 2, end)?;
                if !id.is_empty() && !id.chars().any(char::is_whitespace) {
                    p = end + 1;
                    continue;
                }
            }
        }
        if c == b'[' || (c == b'!' && src.byte(p + 1)? == b'[') {
            let image = c == b'!';
            let label = p + 1 + image as u64;
            if let Some(end) = closing(src, label, to, b'[', b']')? {
                let mut finish = None;
                if src.byte(end + 1)? == b'(' {
                    if let Some(close) = closing(src, end + 2, to, b'(', b')')? {
                        let value = src.small(end + 2, (end + 2 + 4096).min(close))?;
                        if value.is_empty() || valid_link(value.trim().trim_matches(['<', '>'])) {
                            finish = Some(close + 1);
                        }
                    }
                }
                if finish.is_none() && end - label <= 4096 {
                    let mut reference = src.small(label, end)?;
                    let mut tail = end + 1;
                    if src.byte(tail)? == b'[' {
                        if let Some(close) = src.find(tail + 1, to.min(tail + 4098), b"]")? {
                            let explicit = src.small(tail + 1, close)?;
                            if !explicit.is_empty() {
                                reference = explicit;
                            }
                            tail = close + 1;
                        }
                    }
                    if refs.contains(&reference)? {
                        finish = Some(tail);
                    }
                }
                if let Some(mut finish) = finish {
                    if !image {
                        inline(src, tape, refs, label, end, 0, depth + 1)?;
                    } else if src.byte(finish)? == b'{' {
                        if let Some(close) = src.find(finish + 1, to, b"}")? {
                            if close > finish + 1 {
                                finish = close + 1;
                            }
                        }
                    }
                    p = finish;
                    beginning = false;
                    continue;
                }
            }
        }
        if matches!(c, b'*' | b'_' | b'~' | b'^') {
            let run = repeated(src, p, to, c)?;
            let script = matches!(c, b'~' | b'^') && run == 1;
            if matches!(c, b'*' | b'_') || (c == b'~' && run >= 2) {
                if pairs.is_none() {
                    pairs = Some(super::delimiters::Delimiters::new(
                        tape,
                        depth,
                        src.cancel.clone(),
                    )?);
                }
                let (open, close) = delimiter(src, p, run, from, to)?;
                let mut offset = if c == b'~' { run % 2 } else { 0 };
                if offset != 0 {
                    tape.literal(src, p, p + offset)?;
                }
                let width = if c == b'~' { 2 } else { 1 };
                while offset < run {
                    let span = tape.marker(src, p + offset, p + offset + width)?;
                    pairs.as_mut().unwrap().push(
                        span,
                        p,
                        if c == b'~' { 0 } else { run },
                        width,
                        c,
                        open,
                        close,
                    )?;
                    offset += width;
                }
                p += run;
                beginning = false;
                continue;
            }
            let width = if script {
                1
            } else if c == b'~' {
                2
            } else {
                run
            };
            let mut next = p + width;
            let mut found = None;
            let after = if next < to { src.char(next)?.0 } else { ' ' };
            let can_open = delimiter(src, p, run, from, to)?.0;
            if !after.is_whitespace()
                && can_open
                && (c != b'^' || script)
                && (c != b'~' || run >= 2 || script)
            {
                while next < to {
                    let b = src.byte(next)?;
                    if b == b'\\' {
                        next += 2;
                        continue;
                    }
                    if script && (b.is_ascii_whitespace() || matches!(b, b'\r' | b'\n')) {
                        break;
                    }
                    let closing_run = if b == c {
                        repeated(src, next, to, c)?
                    } else {
                        0
                    };
                    if b == c
                        && closing_run >= width
                        && next > p + width
                        && delimiter(src, next, closing_run, from, to)?.1
                    {
                        if !script || src.byte(next + 1)? != c {
                            found = Some(next);
                            break;
                        }
                    }
                    next += 1;
                }
            }
            if let Some(end) = found {
                inline(src, tape, refs, p + width, end, flags, depth + 1)?;
                p = end + width;
                beginning = false;
                continue;
            }
            tape.literal(src, p, p + run)?;
            p += run;
            beginning = false;
            continue;
        }
        if c == b'<' {
            if src.starts(p, b"<!--")? {
                if let Some(end) = src.find(p + 4, to, b"-->")? {
                    p = end + 3;
                    continue;
                }
            }
            if let Some(token) = tag(src, p, to)? {
                if token.name == "code" && !token.closing && !token.self_closing {
                    if let Some(end) = src.find(token.end, to, b"</code>")? {
                        code(src, tape, token.end, end, flags, true)?;
                        p = end + 7;
                        beginning = false;
                        continue;
                    }
                }
                if token.name == "img" && attribute(&token.attrs, "src").is_some() {
                    p = token.end;
                    continue;
                }
                if token.self_closing && token.name == "br" {
                    tape.text("\n", p, token.end)?;
                    p = token.end;
                    continue;
                }
                if token.closing {
                    if let Some(kind) = mark_tag(&token.name) {
                        if tape.close_mark(kind, &src.cancel)? {
                            p = token.end;
                            continue;
                        }
                    }
                } else if !token.self_closing
                    && mark_tag(&token.name).is_some()
                    && (token.name != "a"
                        || double_quoted_href(&token.attrs)
                            .is_some_and(|href| !href.is_empty() && valid_link(&href)))
                {
                    tape.open_mark(mark_tag(&token.name).unwrap())?;
                    p = token.end;
                    continue;
                }
                tape.literal(src, p, token.end)?;
                p = token.end;
                beginning = false;
                continue;
            }
            if let Some(end) = src.find(p + 1, to.min(p + 4096), b">")? {
                let target = src.small(p + 1, end)?;
                if (target.contains("://") || target.contains('@'))
                    && !target.chars().any(char::is_whitespace)
                    && valid_link(&target)
                {
                    tape.literal(src, p + 1, end)?;
                    p = end + 1;
                    beginning = false;
                    continue;
                }
            }
        }
        if matches!(c, b'\r' | b'\n') {
            let mut next = p + 1;
            if c == b'\r' && src.byte(next)? == b'\n' {
                next += 1;
            }
            tape.text("\n", p, next)?;
            if next < to {
                let line = src.line(next)?;
                p = prefix(src, line)?.0;
            } else {
                p = next;
            }
            beginning = false;
            continue;
        }
        let (_, next) = src.char(p)?;
        tape.literal(src, p, next)?;
        p = next;
        beginning = false;
    }
    if let Some(pairs) = pairs {
        pairs.finish(tape)?;
    }
    Ok(())
}

fn editable_html(
    src: &mut Source,
    from: u64,
    to: u64,
    directory: &Path,
) -> TextDocumentResult<bool> {
    let (from, to) = src.trim(from, to)?;
    let Some(root) = tag(src, from, to)? else {
        return Ok(false);
    };
    if root.closing
        || root.self_closing
        || !matches!(root.name.as_str(), "div" | "section")
        || !attributes_allowed(&root.attrs, &["class", "id"])
    {
        return Ok(false);
    }
    const TAGS: &[&str] = &[
        "div", "section", "span", "strong", "b", "em", "i", "br", "img", "mark", "sup", "sub", "a",
        "code", "s", "del", "strike", "u",
    ];
    let id = |name: &str| TAGS.iter().position(|t| *t == name).map(|n| n as u8);
    // Arbitrarily nested representable HTML uses a disk stack, not an AST.
    let mut stack = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(directory.join("html-stack"))?;
    stack.write_all(&[id(&root.name).unwrap(), 0])?;
    let mut depth = 1;
    let mut anchors = 0;
    let mut p = root.end;
    while p < to {
        check(&src.cancel)?;
        let Some(start) = src.find(p, to, b"<")? else {
            return Ok(false);
        };
        let Some(token) = tag(src, start, to)? else {
            return Ok(false);
        };
        let Some(kind) = id(&token.name) else {
            return Ok(false);
        };
        stack.seek(SeekFrom::Start((depth - 1) * 2))?;
        let mut top = [0; 2];
        stack.read_exact(&mut top)?;
        if token.closing {
            if top[0] != kind {
                return Ok(false);
            }
            depth -= 1;
            if token.name == "a" {
                anchors -= 1;
            }
            if depth == 0 {
                return Ok(token.end == to);
            }
        } else {
            if kind < 2 || top[1] != 0 || (token.name == "a" && anchors > 0) {
                return Ok(false);
            }
            let attr = &token.attrs;
            let atom = match token.name.as_str() {
                "br" => {
                    attributes_allowed(attr, &["data-nomo-break"])
                        && attribute(attr, "data-nomo-break")
                            .is_none_or(|v| matches!(v, "" | "soft" | "hard"))
                }
                "img" => {
                    attributes_allowed(attr, &["src", "alt", "title", "width", "align"])
                        && attribute(attr, "src").is_some_and(|v| {
                            !v.is_empty()
                                && (valid_link(&v)
                                    || v.to_ascii_lowercase().starts_with("data:image/"))
                        })
                        && attribute(attr, "align")
                            .is_none_or(|v| matches!(v, "" | "left" | "center" | "right"))
                }
                "span" => {
                    (attribute(attr, "class").as_deref() == Some("math-inline")
                        && attribute(attr, "data-tex").is_some()
                        && attributes_allowed(attr, &["class", "data-tex"]))
                        || (attribute(attr, "class").as_deref() == Some("comment-inline")
                            && attribute(attr, "data-comment").is_some()
                            && attributes_allowed(attr, &["class", "data-comment"]))
                }
                "sup" => {
                    attribute(attr, "class").as_deref() == Some("footnote-ref")
                        && attribute(attr, "data-footnote-id").is_some()
                        && attributes_allowed(attr, &["class", "data-footnote-id"])
                }
                _ => false,
            };
            let representable = if token.name == "a" {
                attributes_allowed(attr, &["href", "title", "id", "name"])
                    && attribute(attr, "href")
                        .is_none_or(|v| !v.trim().is_empty() && valid_link(&v))
            } else {
                !matches!(token.name.as_str(), "br" | "img") && attributes_allowed(attr, &[])
            };
            if !atom && !representable {
                return Ok(false);
            }
            if !matches!(token.name.as_str(), "br" | "img") {
                if token.self_closing {
                    return Ok(false);
                }
                stack.seek(SeekFrom::Start(depth * 2))?;
                stack.write_all(&[kind, atom as u8])?;
                depth += 1;
                if token.name == "a" {
                    anchors += 1;
                }
            }
        }
        p = token.end;
    }
    Ok(false)
}
fn html_block(src: &mut Source, tape: &mut Tape, block: Block) -> TextDocumentResult<()> {
    let (from, to) = src.trim(block.content, block.end)?;
    let first = tag(src, from, to)?;
    if first
        .as_ref()
        .is_some_and(|t| t.name == "img" && t.end == to && attribute(&t.attrs, "src").is_some())
    {
        return Ok(());
    }
    if first
        .as_ref()
        .is_some_and(|t| t.name == "p" && attribute(&t.attrs, "align").is_some())
    {
        if let Some(image) = src.find(from, to, b"<img ")? {
            if let Some(img) = tag(src, image, to)? {
                if attribute(&img.attrs, "src").is_some()
                    && src.trim(img.end, to)?.0 + 4 == to
                    && src.starts(to - 4, b"</p>")?
                {
                    return Ok(());
                }
            }
        }
    }
    if !editable_html(src, from, to, &tape.root)? {
        // Container prefixes belong to Markdown, even in literal HTML blocks.
        let mut p = block.content;
        while p < block.end {
            let line = src.line(p)?;
            tape.literal(src, p, line.end.min(block.end))?;
            if line.end < block.end {
                tape.text("\n", line.end, line.next)?;
            }
            p = line.next;
            if p < block.end {
                let next = src.line(p)?;
                p = prefix(src, next)?.0;
            }
        }
        return Ok(());
    }
    let mut p = from;
    let mut beginning = true;
    let mut code_depth = 0;
    while p < to {
        if src.byte(p)? == b'<' {
            if let Some(token) = tag(src, p, to)? {
                if token.name == "code" {
                    if token.closing {
                        code_depth = 0;
                    } else {
                        code_depth += 1;
                    }
                }
                tape.in_code = code_depth > 0;
                p = token.end;
                continue;
            }
        }
        if beginning
            && code_depth == 0
            && block.flags & LIST != 0
            && src.byte(p)? == b'['
            && matches!(src.byte(p + 1)?, b' ' | b'x' | b'X')
            && src.byte(p + 2)? == b']'
        {
            p += 3;
            if src.byte(p)?.is_ascii_whitespace() {
                p += 1;
            }
            beginning = false;
            continue;
        }
        if src.byte(p)? == b'&' {
            if let Some((text, end)) = entity(src, p, to)? {
                tape.text(&text, p, end)?;
                p = end;
                beginning = false;
                continue;
            }
        }
        if matches!(src.byte(p)?, b'\r' | b'\n') {
            let line = src.line(p)?;
            tape.text("\n", p, line.next)?;
            p = line.next;
            if p < to {
                let next = src.line(p)?;
                p = prefix(src, next)?.0;
            }
            continue;
        }
        let (_, end) = src.char(p)?;
        tape.literal(src, p, end)?;
        p = end;
        beginning = false;
    }
    Ok(())
}

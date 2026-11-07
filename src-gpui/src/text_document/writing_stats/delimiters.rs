//! markdown-it's delimiter balancing rules, with records/jumps on disk instead
//! of one heap entry per marker. HTML/code/link parsing determines which markers
//! reach this list; each inline/link scope has an independent list.
use super::super::TextDocumentResult;
use super::{check, tape::Tape};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    sync::{atomic::AtomicBool, Arc},
};
#[derive(Clone, Copy, Default)]
struct Record {
    span: u64,
    run: u64,
    length: u64,
    width: u64,
    marker: u64,
    flags: u64,
    end: u64,
    jump: u64,
}
impl Record {
    fn read(file: &mut File, index: u64) -> TextDocumentResult<Self> {
        file.seek(SeekFrom::Start(index * 64))?;
        let mut bytes = [0; 64];
        file.read_exact(&mut bytes)?;
        let mut n = [0; 8];
        for (v, b) in n.iter_mut().zip(bytes.chunks_exact(8)) {
            *v = u64::from_le_bytes(b.try_into().unwrap());
        }
        Ok(Self {
            span: n[0],
            run: n[1],
            length: n[2],
            width: n[3],
            marker: n[4],
            flags: n[5],
            end: n[6],
            jump: n[7],
        })
    }
    fn write(self, file: &mut File, index: u64) -> TextDocumentResult<()> {
        file.seek(SeekFrom::Start(index * 64))?;
        let mut bytes = [0; 64];
        for (b, n) in bytes.chunks_exact_mut(8).zip([
            self.span,
            self.run,
            self.length,
            self.width,
            self.marker,
            self.flags,
            self.end,
            self.jump,
        ]) {
            b.copy_from_slice(&n.to_le_bytes());
        }
        file.write_all(&bytes)?;
        Ok(())
    }
}
pub(super) struct Delimiters {
    file: Option<File>,
    path: std::path::PathBuf,
    // At most 2 KiB per nested scope (20 scopes); spills never grow this cache.
    first: [Record; 32],
    count: u64,
    cancel: Arc<AtomicBool>,
}
impl Delimiters {
    pub fn new(tape: &Tape, depth: usize, cancel: Arc<AtomicBool>) -> TextDocumentResult<Self> {
        Ok(Self {
            file: None,
            path: tape.root.join(format!("delimiters-{depth}")),
            first: [Record::default(); 32],
            count: 0,
            cancel,
        })
    }
    fn get(&mut self, index: u64) -> TextDocumentResult<Record> {
        if index < 32 {
            Ok(self.first[index as usize])
        } else {
            Record::read(self.file.as_mut().unwrap(), index - 32)
        }
    }
    fn put(&mut self, index: u64, record: Record) -> TextDocumentResult<()> {
        if index < 32 {
            self.first[index as usize] = record;
            return Ok(());
        }
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&self.path)?,
            );
        }
        record.write(self.file.as_mut().unwrap(), index - 32)
    }
    pub fn push(
        &mut self,
        span: u64,
        run: u64,
        length: u64,
        width: u64,
        marker: u8,
        open: bool,
        close: bool,
    ) -> TextDocumentResult<()> {
        check(&self.cancel)?;
        self.put(
            self.count,
            Record {
                span,
                run,
                length,
                width,
                marker: marker as u64,
                flags: open as u64 | ((close as u64) << 1),
                ..Record::default()
            },
        )?;
        self.count += 1;
        Ok(())
    }
    pub fn finish(mut self, tape: &mut Tape) -> TextDocumentResult<()> {
        let mut bottom = [[-1i64; 6]; 3];
        let mut header = 0;
        let mut last_run = None;
        for closer_index in 0..self.count {
            check(&self.cancel)?;
            let mut closer = self.get(closer_index)?;
            if last_run != Some((closer.marker, closer.run)) {
                header = closer_index;
            }
            last_run = Some((closer.marker, closer.run));
            if closer.flags & 2 == 0 {
                continue;
            }
            let marker = match closer.marker as u8 {
                b'*' => 0,
                b'_' => 1,
                _ => 2,
            };
            let slot = if closer.flags & 1 != 0 { 3 } else { 0 } + (closer.length % 3) as usize;
            let minimum = bottom[marker][slot];
            let mut opener_index = header as i64 - self.get(header)?.jump as i64 - 1;
            let mut new_minimum = opener_index;
            while opener_index > minimum {
                check(&self.cancel)?;
                let mut opener = self.get(opener_index as u64)?;
                if opener.marker == closer.marker && opener.flags & 1 != 0 && opener.end == 0 {
                    let odd = (opener.flags & 2 != 0 || closer.flags & 1 != 0)
                        && (opener.length + closer.length) % 3 == 0
                        && (opener.length % 3 != 0 || closer.length % 3 != 0);
                    if !odd {
                        let last_jump = if opener_index > 0 {
                            let prev = self.get(opener_index as u64 - 1)?;
                            if prev.flags & 1 == 0 {
                                prev.jump + 1
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        closer.jump = closer_index - opener_index as u64 + last_jump;
                        opener.jump = last_jump;
                        closer.flags = (closer.flags & !1) | 4;
                        opener.end = closer_index + 1;
                        opener.flags = (opener.flags & !2) | 4;
                        self.put(opener_index as u64, opener)?;
                        self.put(closer_index, closer)?;
                        new_minimum = -1;
                        last_run = None;
                        break;
                    }
                }
                opener_index -= opener.jump as i64 + 1;
            }
            if new_minimum != -1 {
                bottom[marker][slot] = new_minimum;
            }
        }
        for i in 0..self.count {
            check(&self.cancel)?;
            let record = self.get(i)?;
            if record.flags & 4 != 0 {
                tape.remove(record.span)?;
            }
        }
        if self.file.take().is_some() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

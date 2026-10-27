//! Markdown 原始字节与 UTF-8 工作正文之间的流式转换。
//! 稀疏检查点只负责坐标映射，原始字节始终保留，保存原文片段无需重新编码。
use super::{TextDocumentError, TextDocumentResult};
use crate::models::MarkdownEncoding;
use encoding_rs::GBK;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

const CHECKPOINT_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SourceCheckpoint {
    pub(crate) utf8: u64,
    pub(crate) raw: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MarkdownSourceMap {
    pub(crate) encoding: MarkdownEncoding,
    pub(crate) bom_len: u64,
    pub(crate) utf8_len: u64,
    pub(crate) checkpoints: Vec<SourceCheckpoint>,
}

fn invalid() -> TextDocumentError {
    TextDocumentError::new(
        "invalid-markdown-encoding",
        "文件不是有效的 UTF-8、带 BOM 的 UTF-16 或 GBK 编码",
    )
}

pub(crate) fn bom(encoding: MarkdownEncoding) -> &'static [u8] {
    match encoding {
        MarkdownEncoding::Utf8Bom => b"\xef\xbb\xbf",
        MarkdownEncoding::Utf16LeBom => b"\xff\xfe",
        MarkdownEncoding::Utf16BeBom => b"\xfe\xff",
        _ => b"",
    }
}

fn read_character(
    reader: &mut impl Read,
    encoding: MarkdownEncoding,
) -> TextDocumentResult<Option<(char, u64)>> {
    let mut first = [0u8; 1];
    if reader.read(&mut first)? == 0 {
        return Ok(None);
    }
    let b = first[0];
    match encoding {
        MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom => {
            let length = match b {
                0..=0x7f => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(invalid()),
            };
            let mut bytes = [0u8; 4];
            bytes[0] = b;
            reader
                .read_exact(&mut bytes[1..length])
                .map_err(|_| invalid())?;
            let text = std::str::from_utf8(&bytes[..length]).map_err(|_| invalid())?;
            Ok(Some((
                text.chars().next().ok_or_else(invalid)?,
                length as u64,
            )))
        }
        MarkdownEncoding::Utf16LeBom | MarkdownEncoding::Utf16BeBom => {
            let mut second = [0u8; 1];
            reader.read_exact(&mut second).map_err(|_| invalid())?;
            let unit = if encoding == MarkdownEncoding::Utf16LeBom {
                u16::from_le_bytes([b, second[0]])
            } else {
                u16::from_be_bytes([b, second[0]])
            };
            let (code, length) = if (0xd800..=0xdbff).contains(&unit) {
                let mut tail = [0u8; 2];
                reader.read_exact(&mut tail).map_err(|_| invalid())?;
                let low = if encoding == MarkdownEncoding::Utf16LeBom {
                    u16::from_le_bytes(tail)
                } else {
                    u16::from_be_bytes(tail)
                };
                if !(0xdc00..=0xdfff).contains(&low) {
                    return Err(invalid());
                }
                (
                    0x10000 + ((unit as u32 - 0xd800) << 10) + low as u32 - 0xdc00,
                    4,
                )
            } else {
                (unit as u32, 2)
            };
            let character = char::from_u32(code).ok_or_else(invalid)?;
            Ok(Some((character, length)))
        }
        MarkdownEncoding::Gbk => {
            let mut bytes = [b, 0];
            let length = if b <= 0x80 {
                1
            } else {
                reader.read_exact(&mut bytes[1..]).map_err(|_| invalid())?;
                2
            };
            let text = GBK
                .decode_without_bom_handling_and_without_replacement(&bytes[..length])
                .ok_or_else(invalid)?;
            let mut characters = text.chars();
            let character = characters.next().ok_or_else(invalid)?;
            if characters.next().is_some() {
                return Err(invalid());
            }
            Ok(Some((character, length as u64)))
        }
    }
}

pub(crate) fn convert(
    raw: &Path,
    output: &Path,
    encoding: MarkdownEncoding,
) -> TextDocumentResult<MarkdownSourceMap> {
    let bom_len = bom(encoding).len() as u64;
    let mut source = File::open(raw)?;
    source.seek(SeekFrom::Start(bom_len))?;
    let mut reader = BufReader::new(source);
    if matches!(encoding, MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom) {
        // 完整校验后直接共享不可变 raw，UTF-8 无需再写一份同内容的 working 文件。
        write_encoded(&mut reader, &mut std::io::sink(), encoding)?;
        return Ok(MarkdownSourceMap {
            encoding,
            bom_len,
            utf8_len: std::fs::metadata(raw)?.len() - bom_len,
            checkpoints: vec![SourceCheckpoint {
                utf8: 0,
                raw: bom_len,
            }],
        });
    }
    let mut writer = BufWriter::new(File::create(output)?);
    let mut map = MarkdownSourceMap {
        encoding,
        bom_len,
        utf8_len: 0,
        checkpoints: vec![SourceCheckpoint {
            utf8: 0,
            raw: bom_len,
        }],
    };
    let mut raw_offset = bom_len;
    while let Some((character, count)) = read_character(&mut reader, encoding)? {
        let mut bytes = [0u8; 4];
        writer.write_all(character.encode_utf8(&mut bytes).as_bytes())?;
        raw_offset += count;
        map.utf8_len += character.len_utf8() as u64;
        if map.utf8_len - map.checkpoints.last().unwrap().utf8 >= CHECKPOINT_BYTES {
            map.checkpoints.push(SourceCheckpoint {
                utf8: map.utf8_len,
                raw: raw_offset,
            });
        }
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(map)
}

/// 只读不可变 raw 快照；无 BOM 时必须完成 UTF-8 校验后才能排除 GBK。
pub(crate) fn prepare_working_source(
    raw: &Path,
    output: &Path,
) -> TextDocumentResult<MarkdownSourceMap> {
    let mut prefix = [0u8; 3];
    let count = File::open(raw)?.read(&mut prefix)?;
    let prefix = &prefix[..count];
    let declared = if prefix.starts_with(b"\xef\xbb\xbf") {
        Some(MarkdownEncoding::Utf8Bom)
    } else if prefix.starts_with(b"\xff\xfe") {
        Some(MarkdownEncoding::Utf16LeBom)
    } else if prefix.starts_with(b"\xfe\xff") {
        Some(MarkdownEncoding::Utf16BeBom)
    } else {
        None
    };
    if let Some(encoding) = declared {
        return convert(raw, output, encoding);
    }
    match convert(raw, output, MarkdownEncoding::Utf8) {
        Ok(map) => Ok(map),
        Err(error) if error.code == "invalid-markdown-encoding" => {
            convert(raw, output, MarkdownEncoding::Gbk)
        }
        Err(error) => Err(error),
    }
}

impl MarkdownSourceMap {
    pub(crate) fn working_source<'a>(&self, raw: &'a Path, converted: &'a Path) -> (&'a Path, u64) {
        if matches!(
            self.encoding,
            MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom
        ) {
            (raw, self.bom_len)
        } else {
            (converted, 0)
        }
    }
    pub(crate) fn raw_offset(&self, path: &Path, utf8_offset: u64) -> TextDocumentResult<u64> {
        if utf8_offset > self.utf8_len {
            return Err(TextDocumentError::new("invalid-range", "原文映射越界"));
        }
        if matches!(
            self.encoding,
            MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom
        ) {
            return Ok(self.bom_len + utf8_offset);
        }
        let index = self
            .checkpoints
            .partition_point(|point| point.utf8 <= utf8_offset)
            .saturating_sub(1);
        let point = &self.checkpoints[index];
        let mut source = File::open(path)?;
        source.seek(SeekFrom::Start(point.raw))?;
        let mut reader = BufReader::new(source);
        let mut logical = point.utf8;
        let mut raw = point.raw;
        while logical < utf8_offset {
            let (character, count) =
                read_character(&mut reader, self.encoding)?.ok_or_else(invalid)?;
            logical += character.len_utf8() as u64;
            raw += count;
        }
        if logical != utf8_offset {
            return Err(TextDocumentError::new(
                "invalid-range",
                "原文映射切开了 Unicode 字符",
            ));
        }
        Ok(raw)
    }
}

/// 不写 BOM。每个新增片段必须从完整字符开始，编码失败时不能替换目标文件。
pub(crate) fn write_encoded(
    reader: impl Read,
    writer: &mut impl Write,
    encoding: MarkdownEncoding,
) -> TextDocumentResult<()> {
    let mut reader = BufReader::new(reader);
    // 按固定字节块校验完整字符，保留 CRLF 和最后一行空白；超长段落不扩大缓冲。
    let mut pending = Vec::with_capacity(64 * 1024 + 4);
    loop {
        let available = reader.fill_buf()?;
        let done = available.is_empty();
        let count = available
            .len()
            .min(64 * 1024 - pending.len().min(64 * 1024));
        pending.extend_from_slice(&available[..count]);
        reader.consume(count);
        let valid = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(error) if error.error_len().is_none() && !done => error.valid_up_to(),
            Err(_) => return Err(invalid()),
        };
        let text = std::str::from_utf8(&pending[..valid]).map_err(|_| invalid())?;
        match encoding {
            MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom => {
                writer.write_all(text.as_bytes())?
            }
            MarkdownEncoding::Utf16LeBom | MarkdownEncoding::Utf16BeBom => {
                for unit in text.encode_utf16() {
                    let bytes = if encoding == MarkdownEncoding::Utf16LeBom {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    };
                    writer.write_all(&bytes)?;
                }
            }
            MarkdownEncoding::Gbk => {
                let (bytes, _, errors) = GBK.encode(text);
                if errors {
                    return Err(TextDocumentError::new(
                        "unrepresentable-character",
                        "文档包含 GBK 无法表示的字符，原文件未被修改",
                    ));
                }
                writer.write_all(&bytes)?;
            }
        }
        pending.drain(..valid);
        if done {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_encodings_without_normalizing_source() {
        let root = std::env::temp_dir().join(format!(
            "nomo-codec-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for encoding in [
            MarkdownEncoding::Utf8,
            MarkdownEncoding::Utf8Bom,
            MarkdownEncoding::Utf16LeBom,
            MarkdownEncoding::Utf16BeBom,
            MarkdownEncoding::Gbk,
        ] {
            let text = "# 中文\r\n\n尾部  \r\n".repeat(4_000);
            let mut bytes = bom(encoding).to_vec();
            write_encoded(text.as_bytes(), &mut bytes, encoding).unwrap();
            let raw = root.join("raw");
            let work = root.join("work");
            std::fs::write(&raw, &bytes).unwrap();
            let map = prepare_working_source(&raw, &work).unwrap();
            assert_eq!(map.encoding, encoding);
            let (working_path, start) = map.working_source(&raw, &work);
            let mut working = File::open(working_path).unwrap();
            working.seek(SeekFrom::Start(start)).unwrap();
            let mut actual = String::new();
            working.read_to_string(&mut actual).unwrap();
            assert_eq!(actual, text);
            let offset = "# 中文\r\n".len() as u64;
            let original_offset = map.raw_offset(&raw, offset).unwrap();
            let mut prefix = bom(encoding).to_vec();
            write_encoded("# 中文\r\n".as_bytes(), &mut prefix, encoding).unwrap();
            assert_eq!(original_offset, prefix.len() as u64);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refuses_lossy_encoding_and_invalid_declared_unicode() {
        assert!(write_encoded("😀".as_bytes(), &mut Vec::new(), MarkdownEncoding::Gbk).is_err());
        let mut bytes = &b"\x00\xd8"[..];
        assert!(read_character(&mut bytes, MarkdownEncoding::Utf16LeBom).is_err());
    }
}

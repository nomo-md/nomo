//! Bounded, read-only search. Nothing in this module retains a complete document.
use encoding_rs::{DecoderResult, Encoding, GBK, UTF_16BE, UTF_16LE, UTF_8};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use unicode_normalization::UnicodeNormalization;

pub(crate) const CHUNK_BYTES: usize = 64 * 1024;
const SNIPPET_CHARS: usize = 160;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snippet {
    pub(crate) text: String,
    pub(crate) highlight_start: usize,
    pub(crate) highlight_end: usize,
    pub(crate) leading_ellipsis: bool,
    pub(crate) trailing_ellipsis: bool,
}

pub(crate) fn normalize(text: &str) -> String {
    text.to_lowercase().nfkd().collect()
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}

/// Encoding detection mirrors document opening: BOM, valid UTF-8, then strict GBK.
/// The UTF-8 validation pass is streamed too, so a late non-UTF-8 byte cannot misclassify a file.
pub(crate) fn scan_file(
    path: &Path,
    query: &str,
    cancel: &AtomicBool,
) -> Result<Option<Snippet>, String> {
    cancelled(cancel)?;
    let mut file = File::open(path).map_err(|_| "read-failed")?;
    let before = file.metadata().map_err(|_| "read-failed")?;
    if !before.is_file() {
        return Err("not-a-file".into());
    }
    let mut prefix = [0; 3];
    let size = file.read(&mut prefix).map_err(|_| "read-failed")?;
    let (encoding, skip) = detect_encoding(&prefix[..size], || {
        file.rewind().map_err(|_| "read-failed")?;
        decode_stream(&mut file, UTF_8, cancel, |_| Ok(false))
    })?;
    file.seek(SeekFrom::Start(skip as u64))
        .map_err(|_| "read-failed")?;
    let result = scan_reader(&mut file, encoding, query, cancel)?;
    let after = std::fs::metadata(path).map_err(|_| "file-changed")?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("file-changed".into());
    }
    Ok(result)
}

fn detect_encoding(
    prefix: &[u8],
    validate_utf8: impl FnOnce() -> Result<(), String>,
) -> Result<(&'static Encoding, usize), String> {
    if prefix.starts_with(&[0xff, 0xfe]) {
        return Ok((UTF_16LE, 2));
    }
    if prefix.starts_with(&[0xfe, 0xff]) {
        return Ok((UTF_16BE, 2));
    }
    if prefix.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Ok((UTF_8, 3));
    }
    match validate_utf8() {
        Ok(()) => Ok((UTF_8, 0)),
        Err(error) if error == "unsupported-encoding" => Ok((GBK, 0)),
        Err(error) => Err(error),
    }
}

pub(crate) fn scan_snapshot(
    readers: impl Fn() -> Box<dyn Read + Send>,
    query: &str,
    cancel: &AtomicBool,
) -> Result<Option<Snippet>, String> {
    cancelled(cancel)?;
    let mut prefix = [0; 3];
    let size = readers().read(&mut prefix).map_err(|_| "read-failed")?;
    let (encoding, skip) = detect_encoding(&prefix[..size], || {
        decode_stream(&mut readers(), UTF_8, cancel, |_| Ok(false))
    })?;
    let mut reader = readers();
    reader
        .read_exact(&mut prefix[..skip])
        .map_err(|_| "read-failed")?;
    scan_reader(&mut reader, encoding, query, cancel)
}

pub(crate) fn scan_reader(
    reader: &mut dyn Read,
    encoding: &'static Encoding,
    query: &str,
    cancel: &AtomicBool,
) -> Result<Option<Snippet>, String> {
    let needle = normalize(query.trim());
    if needle.is_empty() || needle.len() > CHUNK_BYTES {
        return Err("invalid-query".into());
    }
    let keep = needle.chars().count() + SNIPPET_CHARS;
    let mut carry = String::new();
    let mut consumed_chars = 0;
    let mut result = None;
    decode_stream(reader, encoding, cancel, |text| {
        let mut window = std::mem::take(&mut carry);
        window.push_str(text);
        if let Some(snippet) = find_snippet(&window, &needle, consumed_chars > 0, false) {
            result = Some(snippet);
            return Ok(true);
        }
        let chars = window.chars().count();
        let discard = chars.saturating_sub(keep);
        consumed_chars += discard;
        carry = window.chars().skip(discard).collect();
        Ok(false)
    })?;
    if result.is_none() {
        result = find_snippet(&carry, &needle, consumed_chars > 0, true);
    }
    Ok(result)
}

fn decode_stream(
    reader: &mut dyn Read,
    encoding: &'static Encoding,
    cancel: &AtomicBool,
    mut consume: impl FnMut(&str) -> Result<bool, String>,
) -> Result<(), String> {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut input = vec![0; CHUNK_BYTES];
    let mut output = vec![0; CHUNK_BYTES * 4];
    loop {
        cancelled(cancel)?;
        let count = reader.read(&mut input).map_err(|_| "read-failed")?;
        let mut offset = 0;
        loop {
            cancelled(cancel)?;
            let (state, read, written) = decoder.decode_to_utf8_without_replacement(
                &input[offset..count],
                &mut output,
                count == 0,
            );
            offset += read;
            if matches!(state, DecoderResult::Malformed(..)) {
                return Err("unsupported-encoding".into());
            }
            let text =
                std::str::from_utf8(&output[..written]).map_err(|_| "unsupported-encoding")?;
            if !text.is_empty() && consume(text)? {
                return Ok(());
            }
            if !matches!(state, DecoderResult::OutputFull) {
                break;
            }
        }
        if count == 0 {
            return Ok(());
        }
    }
}

/// Match normalized text, but return original spelling and UTF-16 offsets for the WebView.
fn find_snippet(source: &str, needle: &str, has_prefix: bool, eof: bool) -> Option<Snippet> {
    let normalized = normalize(source);
    let start = normalized.find(needle)?;
    let end = start + needle.len();
    let chars: Vec<char> = source.chars().collect();
    let mut position = 0;
    let mut from = None;
    let mut to = 0;
    for (index, character) in chars.iter().enumerate() {
        let length = normalize(&character.to_string()).len();
        if from.is_none() && position + length > start {
            from = Some(index);
        }
        position += length;
        if position >= end {
            to = index + 1;
            break;
        }
    }
    let from = from?;
    // Keep enough trailing context even when the match crosses a read boundary.
    if !eof && chars.len().saturating_sub(to) < 54 {
        return None;
    }
    let snippet_start = from.saturating_sub(34);
    let snippet_end = (snippet_start + SNIPPET_CHARS).min(chars.len());
    let text: String = chars[snippet_start..snippet_end].iter().collect();
    let highlight_start = chars[snippet_start..from.min(snippet_end)]
        .iter()
        .map(|c| c.len_utf16())
        .sum();
    let highlight_end = chars[snippet_start..to.min(snippet_end)]
        .iter()
        .map(|c| c.len_utf16())
        .sum();
    Some(Snippet {
        text,
        highlight_start,
        highlight_end,
        leading_ellipsis: has_prefix || snippet_start > 0,
        trailing_ellipsis: !eof || snippet_end < chars.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn mobile_search_crosses_chunks_and_preserves_unicode_highlights() {
        let source = format!(
            "{}Ａé中😀XYZ{}",
            "a".repeat(CHUNK_BYTES - 2),
            "z".repeat(200)
        );
        let snippet = scan_reader(
            &mut Cursor::new(source.as_bytes()),
            UTF_8,
            "ae\u{301}中😀xyz",
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        assert!(snippet.text.contains("Ａé中😀XYZ"));
        assert!(snippet.text.chars().count() <= SNIPPET_CHARS);
        assert!(snippet.highlight_end > snippet.highlight_start);
    }

    #[test]
    fn mobile_search_decodes_utf16_and_gbk() {
        for encoding in [UTF_16LE, UTF_16BE, GBK] {
            let source = "前文中文目标后文";
            let bytes = if encoding == UTF_16LE {
                source
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>()
            } else if encoding == UTF_16BE {
                source.encode_utf16().flat_map(u16::to_be_bytes).collect()
            } else {
                GBK.encode(source).0.into_owned()
            };
            let snippet = scan_reader(
                &mut Cursor::new(bytes),
                encoding,
                "目标",
                &AtomicBool::new(false),
            )
            .unwrap()
            .unwrap();
            assert!(snippet.text.contains("中文目标"));
        }
    }

    #[test]
    fn mobile_search_scans_100_mib_without_full_file_buffers_and_cancels() {
        struct LargeReader {
            remaining: usize,
            maximum: usize,
        }
        impl Read for LargeReader {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                self.maximum = self.maximum.max(output.len());
                let count = output.len().min(self.remaining);
                output[..count].fill(b'a');
                self.remaining -= count;
                if self.remaining == 0 && count >= 6 {
                    output[count - 6..count].copy_from_slice(b"needle");
                }
                Ok(count)
            }
        }
        let mut reader = LargeReader {
            remaining: 100 * 1024 * 1024,
            maximum: 0,
        };
        let result = scan_reader(&mut reader, UTF_8, "needle", &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert!(result.text.contains("needle"));
        assert!(reader.maximum <= CHUNK_BYTES);
        let cancel = AtomicBool::new(true);
        assert_eq!(
            scan_reader(&mut Cursor::new(b"text"), UTF_8, "text", &cancel).unwrap_err(),
            "cancelled"
        );
    }

    #[test]
    fn mobile_search_stops_reading_after_cancellation_and_reports_read_errors() {
        struct CancelReader<'a> {
            cancel: &'a AtomicBool,
            reads: usize,
        }
        impl Read for CancelReader<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                assert_eq!(
                    self.reads, 1,
                    "must not schedule a second read after cancellation"
                );
                output.fill(b'a');
                self.cancel.store(true, Ordering::Release);
                Ok(output.len())
            }
        }
        let cancel = AtomicBool::new(false);
        let mut reader = CancelReader {
            cancel: &cancel,
            reads: 0,
        };
        assert_eq!(
            scan_reader(&mut reader, UTF_8, "tail", &cancel).unwrap_err(),
            "cancelled"
        );
        assert_eq!(reader.reads, 1);
        struct FailedReader;
        impl Read for FailedReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "unavailable",
                ))
            }
        }
        assert_eq!(
            scan_reader(&mut FailedReader, UTF_8, "tail", &AtomicBool::new(false)).unwrap_err(),
            "read-failed"
        );
        assert_eq!(
            scan_reader(
                &mut Cursor::new(b"no match"),
                UTF_8,
                "tail",
                &AtomicBool::new(false)
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn mobile_search_detects_file_encodings_and_finds_real_100_mib_tail() {
        use std::{
            fs,
            io::Write,
            time::{SystemTime, UNIX_EPOCH},
        };
        let root = std::env::temp_dir().join(format!(
            "nomo-mobile-encoding-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("large.md");
        let mut file = File::create(&path).unwrap();
        file.set_len(100 * 1024 * 1024).unwrap();
        file.seek(SeekFrom::End(-6)).unwrap();
        file.write_all(b"needle").unwrap();
        drop(file);
        let cancel = AtomicBool::new(false);
        assert!(scan_file(&path, "needle", &cancel).unwrap().is_some());
        for (bom, encoding) in [
            (&[0xff, 0xfe][..], UTF_16LE),
            (&[0xfe, 0xff][..], UTF_16BE),
            (&[][..], GBK),
        ] {
            let text = "中文目标";
            let bytes: Vec<u8> = if encoding == UTF_16LE {
                text.encode_utf16().flat_map(u16::to_le_bytes).collect()
            } else if encoding == UTF_16BE {
                text.encode_utf16().flat_map(u16::to_be_bytes).collect()
            } else {
                GBK.encode(text).0.into_owned()
            };
            fs::write(&path, [bom, &bytes].concat()).unwrap();
            assert!(scan_file(&path, "目标", &cancel).unwrap().is_some());
        }
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

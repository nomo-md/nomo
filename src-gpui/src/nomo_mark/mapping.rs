//! 坐标转换只针对宿主给出的局部原文。严格拒绝越界和字符中间的写入。
use super::protocol::ControlError;
use std::ops::Range;

pub fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

pub fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].encode_utf16().count()
}

pub fn local_range(
    text: &str,
    from_byte: u64,
    global: Range<u64>,
) -> Result<Range<usize>, ControlError> {
    if global.start > global.end
        || global.start < from_byte
        || global.end > from_byte.saturating_add(text.len() as u64)
    {
        return Err(ControlError::InvalidRange);
    }
    let range = (global.start - from_byte) as usize..(global.end - from_byte) as usize;
    if !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end) {
        return Err(ControlError::InvalidUtf8Boundary);
    }
    Ok(range)
}

/// 一个可见片段对应的真实源码区间。实体、转义、原子对象不是线性字节映射。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSegment {
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub exact: bool,
}
impl SourceSegment {
    pub fn to_source(&self, offset: usize, trailing: bool) -> usize {
        if self.exact {
            self.source.start + offset.clamp(self.display.start, self.display.end)
                - self.display.start
        } else if trailing || offset >= self.display.end {
            self.source.end
        } else {
            self.source.start
        }
    }
    pub fn to_display(&self, offset: usize, trailing: bool) -> usize {
        if self.exact {
            self.display.start + offset.clamp(self.source.start, self.source.end)
                - self.source.start
        } else if trailing || offset >= self.source.end {
            self.display.end
        } else {
            self.display.start
        }
    }
}

/// 解析器规范化 CRLF 时逐段保留原始位置；其他转换保留完整 token 范围。
/// 不通过累加规范化后的长度推算源码（那会把第二行之后的光标移到 CR 中）。
pub fn text_segments(display: &str, raw: &str, source_start: usize) -> Vec<SourceSegment> {
    if display == raw {
        return vec![SourceSegment {
            display: 0..display.len(),
            source: source_start..source_start + raw.len(),
            exact: true,
        }];
    }
    if raw.contains("\r\n") && raw.replace("\r\n", "\n") == display {
        let mut result = Vec::new();
        let mut from = 0;
        let mut shown = 0;
        for part in raw.split_inclusive("\r\n") {
            let text = part.strip_suffix("\r\n").unwrap_or(part);
            if !text.is_empty() {
                result.push(SourceSegment {
                    display: shown..shown + text.len(),
                    source: source_start + from..source_start + from + text.len(),
                    exact: true,
                });
            }
            shown += text.len();
            from += text.len();
            if part.ends_with("\r\n") {
                result.push(SourceSegment {
                    display: shown..shown + 1,
                    source: source_start + from..source_start + from + 2,
                    exact: false,
                });
                shown += 1;
                from += 2;
            }
        }
        return result;
    }
    vec![SourceSegment {
        display: 0..display.len(),
        source: source_start..source_start + raw.len(),
        exact: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_and_window_origins_are_not_interchangeable() {
        let text = "中😀e\u{301}\r\n";
        assert_eq!(utf16_to_utf8(text, 2), 3);
        assert_eq!(utf16_to_utf8(text, 3), 7);
        assert_eq!(utf8_to_utf16(text, 7), 3);
        assert_eq!(local_range(text, 100, 103..107).unwrap(), 3..7);
        assert_eq!(
            local_range(text, 100, 104..107),
            Err(ControlError::InvalidUtf8Boundary)
        );
        assert_eq!(
            local_range(text, 100, 0..3),
            Err(ControlError::InvalidRange)
        );
    }
    #[test]
    fn transformed_text_maps_to_whole_source_token() {
        let entity = SourceSegment {
            display: 0..1,
            source: 10..15,
            exact: false,
        };
        assert_eq!(entity.to_source(0, false), 10);
        assert_eq!(entity.to_source(1, false), 15);
        assert_eq!(entity.to_display(12, true), 1);
    }
    #[test]
    fn code_normalization_keeps_unicode_and_mixed_line_positions() {
        let raw = "中😀\r\nx\ny\r\n";
        let display = raw.replace("\r\n", "\n");
        for segment in text_segments(&display, raw, 100) {
            assert!(raw.is_char_boundary(segment.source.start - 100));
            assert!(raw.is_char_boundary(segment.source.end - 100));
            if segment.exact {
                assert_eq!(
                    &display[segment.display.clone()],
                    &raw[segment.source.start - 100..segment.source.end - 100]
                );
            }
        }
        let at = display.find('y').unwrap();
        let segment = text_segments(&display, raw, 100)
            .into_iter()
            .find(|segment| segment.display.contains(&at))
            .unwrap();
        assert_eq!(segment.to_source(at, false), 100 + raw.find('y').unwrap());
    }
}

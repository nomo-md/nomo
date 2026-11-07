//! 表格列宽沿 string-width 8.2.1 的字素、Emoji、Hangul 与 East Asian Width 规则。
//! East Asian W/F 数据对应 get-east-asian-width 1.4.0；仅静态区间，无新增运行依赖。
// Sources: https://github.com/sindresorhus/string-width/tree/v8.2.1
// https://github.com/sindresorhus/get-east-asian-width/tree/v1.4.0
// MIT License
//
// Copyright (c) Sindre Sorhus <sindresorhus@gmail.com> (https://sindresorhus.com)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use regex::Regex;
use std::sync::OnceLock;
use unicode_properties::{UnicodeEmoji, emoji::EmojiStatus};
use unicode_segmentation::UnicodeSegmentation;

const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115f),
    (0x231a, 0x231b),
    (0x2329, 0x232a),
    (0x23e9, 0x23ec),
    (0x23f0, 0x23f0),
    (0x23f3, 0x23f3),
    (0x25fd, 0x25fe),
    (0x2614, 0x2615),
    (0x2630, 0x2637),
    (0x2648, 0x2653),
    (0x267f, 0x267f),
    (0x268a, 0x268f),
    (0x2693, 0x2693),
    (0x26a1, 0x26a1),
    (0x26aa, 0x26ab),
    (0x26bd, 0x26be),
    (0x26c4, 0x26c5),
    (0x26ce, 0x26ce),
    (0x26d4, 0x26d4),
    (0x26ea, 0x26ea),
    (0x26f2, 0x26f3),
    (0x26f5, 0x26f5),
    (0x26fa, 0x26fa),
    (0x26fd, 0x26fd),
    (0x2705, 0x2705),
    (0x270a, 0x270b),
    (0x2728, 0x2728),
    (0x274c, 0x274c),
    (0x274e, 0x274e),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27b0, 0x27b0),
    (0x27bf, 0x27bf),
    (0x2b1b, 0x2b1c),
    (0x2b50, 0x2b50),
    (0x2b55, 0x2b55),
    (0x2e80, 0x2e99),
    (0x2e9b, 0x2ef3),
    (0x2f00, 0x2fd5),
    (0x2ff0, 0x303e),
    (0x3041, 0x3096),
    (0x3099, 0x30ff),
    (0x3105, 0x312f),
    (0x3131, 0x318e),
    (0x3190, 0x31e5),
    (0x31ef, 0x321e),
    (0x3220, 0x3247),
    (0x3250, 0xa48c),
    (0xa490, 0xa4c6),
    (0xa960, 0xa97c),
    (0xac00, 0xd7a3),
    (0xf900, 0xfaff),
    (0xfe10, 0xfe19),
    (0xfe30, 0xfe52),
    (0xfe54, 0xfe66),
    (0xfe68, 0xfe6b),
    (0xff01, 0xff60),
    (0xffe0, 0xffe6),
    (0x16fe0, 0x16fe4),
    (0x16ff0, 0x16ff6),
    (0x17000, 0x18cd5),
    (0x18cff, 0x18d1e),
    (0x18d80, 0x18df2),
    (0x1aff0, 0x1aff3),
    (0x1aff5, 0x1affb),
    (0x1affd, 0x1affe),
    (0x1b000, 0x1b122),
    (0x1b132, 0x1b132),
    (0x1b150, 0x1b152),
    (0x1b155, 0x1b155),
    (0x1b164, 0x1b167),
    (0x1b170, 0x1b2fb),
    (0x1d300, 0x1d356),
    (0x1d360, 0x1d376),
    (0x1f004, 0x1f004),
    (0x1f0cf, 0x1f0cf),
    (0x1f18e, 0x1f18e),
    (0x1f191, 0x1f19a),
    (0x1f200, 0x1f202),
    (0x1f210, 0x1f23b),
    (0x1f240, 0x1f248),
    (0x1f250, 0x1f251),
    (0x1f260, 0x1f265),
    (0x1f300, 0x1f320),
    (0x1f32d, 0x1f335),
    (0x1f337, 0x1f37c),
    (0x1f37e, 0x1f393),
    (0x1f3a0, 0x1f3ca),
    (0x1f3cf, 0x1f3d3),
    (0x1f3e0, 0x1f3f0),
    (0x1f3f4, 0x1f3f4),
    (0x1f3f8, 0x1f43e),
    (0x1f440, 0x1f440),
    (0x1f442, 0x1f4fc),
    (0x1f4ff, 0x1f53d),
    (0x1f54b, 0x1f54e),
    (0x1f550, 0x1f567),
    (0x1f57a, 0x1f57a),
    (0x1f595, 0x1f596),
    (0x1f5a4, 0x1f5a4),
    (0x1f5fb, 0x1f64f),
    (0x1f680, 0x1f6c5),
    (0x1f6cc, 0x1f6cc),
    (0x1f6d0, 0x1f6d2),
    (0x1f6d5, 0x1f6d8),
    (0x1f6dc, 0x1f6df),
    (0x1f6eb, 0x1f6ec),
    (0x1f6f4, 0x1f6fc),
    (0x1f7e0, 0x1f7eb),
    (0x1f7f0, 0x1f7f0),
    (0x1f90c, 0x1f93a),
    (0x1f93c, 0x1f945),
    (0x1f947, 0x1f9ff),
    (0x1fa70, 0x1fa7c),
    (0x1fa80, 0x1fa8a),
    (0x1fa8e, 0x1fac6),
    (0x1fac8, 0x1fac8),
    (0x1facd, 0x1fadc),
    (0x1fadf, 0x1faea),
    (0x1faef, 0x1faf8),
    (0x20000, 0x2fffd),
    (0x30000, 0x3fffd),
];
fn east_asian_width(code: u32) -> usize {
    let at = WIDE.partition_point(|(start, _)| *start <= code);
    if at > 0 && code <= WIDE[at - 1].1 {
        2
    } else {
        1
    }
}
fn leading(code: u32) -> bool {
    matches!(code, 0x1100..=0x115f | 0xa960..=0xa97c)
}
fn vowel(code: u32) -> bool {
    matches!(code, 0x1160..=0x11a7 | 0xd7b0..=0xd7c6)
}
fn trailing(code: u32) -> bool {
    matches!(code, 0x11a8..=0x11ff | 0xd7cb..=0xd7fb)
}
pub(super) fn display_width(text: &str) -> usize {
    if text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return text.len();
    }
    static INVISIBLE: OnceLock<Regex> = OnceLock::new();
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let invisible = INVISIBLE.get_or_init(|| {
        Regex::new(r"^[\p{Default_Ignorable_Code_Point}\p{Cc}\p{Cf}\p{M}]+$").unwrap()
    });
    let ansi = ANSI.get_or_init(|| {
        Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))|\x{9b}[0-?]*[ -/]*[@-~]")
            .unwrap()
    });
    let clean = ansi.replace_all(text, "");
    clean
        .graphemes(true)
        .map(|cluster| {
            let chars = cluster
                .chars()
                .filter(|ch| {
                    let mut bytes = [0; 4];
                    !invisible.is_match(ch.encode_utf8(&mut bytes))
                })
                .collect::<Vec<_>>();
            let Some(first) = chars.first().copied() else {
                return 0;
            };
            let presentation = matches!(
                first.emoji_status(),
                EmojiStatus::EmojiPresentation
                    | EmojiStatus::EmojiPresentationAndModifierBase
                    | EmojiStatus::EmojiPresentationAndEmojiComponent
                    | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
            );
            let keycap = (first.is_ascii_digit() || first == '#' || first == '*')
                && cluster.contains('\u{20e3}');
            let flag = chars.len() == 2
                && chars
                    .iter()
                    .all(|ch| matches!(*ch as u32, 0x1f1e6..=0x1f1ff));
            let joined = cluster.len() <= 200
                && cluster.contains('\u{200d}')
                && chars
                    .iter()
                    .filter(|ch| ch.is_emoji_char() && !ch.is_ascii())
                    .count()
                    >= 2;
            if keycap
                || flag
                || joined
                || (first.is_emoji_char() && cluster.contains('\u{fe0f}'))
                || (presentation && !matches!(first as u32, 0x1f1e6..=0x1f1ff))
            {
                return 2;
            }
            let code = first as u32;
            if leading(code) || vowel(code) || trailing(code) {
                let mut width = 0;
                let mut i = 0;
                while i < chars.len() {
                    let code = chars[i] as u32;
                    if leading(code) && chars.get(i + 1).is_some_and(|ch| vowel(*ch as u32)) {
                        width += 2;
                        i += if chars.get(i + 2).is_some_and(|ch| trailing(*ch as u32)) {
                            3
                        } else {
                            2
                        };
                    } else {
                        width += east_asian_width(code);
                        i += 1;
                    }
                }
                width
            } else {
                east_asian_width(code)
                    + chars
                        .iter()
                        .skip(1)
                        .filter(|ch| matches!(**ch as u32, 0xff00..=0xffef))
                        .map(|ch| east_asian_width(*ch as u32))
                        .sum::<usize>()
            }
        })
        .sum()
}

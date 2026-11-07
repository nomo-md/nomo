//! 编辑、剪贴板及导出共用的受限行内 HTML 样式；不加载字体、不保留正文。
use super::nomo_syntax::HtmlTag;
use crate::layout::ColorRole;

#[derive(Clone)]
pub(crate) struct HtmlStyle {
    pub tag: &'static str,
    pub foreground: Option<ColorRole>,
    pub background: Option<ColorRole>,
    pub font_family: Option<gpui::SharedString>,
    pub font_scale: u16,
}
impl HtmlStyle {
    pub fn parse(tag: &HtmlTag) -> Self {
        let style = tag.attrs.get("style").map(String::as_str).unwrap_or_default();
        let declaration = |property: &str| style.split(';').rev().find_map(|part| {
            let (key, value) = part.split_once(':')?;
            key.trim().eq_ignore_ascii_case(property).then(|| value.trim())
        });
        let attr = |name: &str| tag.attrs.get(name).map(String::as_str);
        let class_color = |prefix: &str| attr("class").unwrap_or_default().split_whitespace()
            .find_map(|class| class.strip_prefix(prefix)).and_then(palette_index).map(ColorRole::Palette);
        Self {
            tag: if tag.name == "font" { "font" } else { "span" },
            foreground: class_color("nomo-fg-").or_else(|| declaration("color").or_else(|| attr("color")).and_then(css_color).map(ColorRole::Rgba)),
            background: class_color("nomo-bg-").or_else(|| declaration("background-color").or_else(|| declaration("background")).and_then(css_color).map(ColorRole::Rgba)),
            font_family: declaration("font-family").or_else(|| attr("face")).and_then(font_family).map(Into::into),
            font_scale: if tag.name == "font" { attr("size").and_then(font_scale).unwrap_or(0) } else { 0 },
        }
    }
}

pub(crate) fn css_color(value: &str) -> Option<u32> {
    if value.len() > 128 { return None; }
    let value = value.trim().to_ascii_lowercase();
    let value = value.trim_end_matches("!important").trim();
    // SVG 的兼容解析会把非法十六进制字符当作 0；这里只接受完整有效色值。
    if let Some(hex) = value.strip_prefix('#') {
        if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    }
    let color: svgtypes::Color = value.parse().ok()?;
    Some(u32::from_be_bytes([color.red, color.green, color.blue, color.alpha]))
}

pub(crate) fn font_family(value: &str) -> Option<String> {
    let family = value.split(',').next()?.trim().trim_matches(['\'', '"']).trim();
    (!family.is_empty() && family.len() <= 128 && !family.chars().any(char::is_control)).then(|| family.to_owned())
}

fn font_size(value: &str) -> Option<i16> {
    let value = value.trim();
    let number = value.parse::<i16>().ok()?;
    Some((if value.starts_with(['+', '-']) { number.saturating_add(3) } else { number }).clamp(1, 7))
}
pub(crate) fn font_scale(value: &str) -> Option<u16> {
    // HTML 的 1–7 档对应 x-small 到 xxx-large；3 继承段落基准字号。
    Some([625, 813, 1000, 1125, 1500, 2000, 3000][font_size(value)? as usize - 1])
}

fn palette_index(value: &str) -> Option<u8> {
    match value.to_ascii_lowercase().as_str() {
        "red" | "crimson" | "tomato" => Some(0),
        "orange" | "darkorange" | "coral" => Some(1),
        "yellow" | "gold" | "khaki" => Some(2),
        "green" | "lime" | "seagreen" | "limegreen" => Some(3),
        "blue" | "royalblue" | "dodgerblue" | "skyblue" | "lightblue" => Some(4),
        "purple" | "violet" | "magenta" | "orchid" => Some(5),
        "gray" | "grey" | "silver" | "darkgray" | "lightgray" => Some(6),
        _ => None,
    }
}

pub(crate) fn safe_style(input: &str) -> String {
    input.split(';').filter_map(|entry| {
        let (key, value) = entry.split_once(':')?;
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let value = match key.as_str() {
            "color" | "background-color" | "background" => format!("#{:08x}", css_color(value)?),
            "font-family" => {
                let family = font_family(value)?;
                let mut escaped = String::from("\"");
                for ch in family.chars() {
                    if ch.is_alphanumeric() || matches!(ch, ' ' | '-') { escaped.push(ch); }
                    else { escaped.push_str(&format!("\\{:x} ", ch as u32)); }
                }
                escaped.push('"'); escaped
            },
            "text-align" if matches!(value, "left" | "center" | "right" | "justify") => value.to_owned(),
            "width" | "max-width" => crate::image_import::normalize_width(value)?,
            _ => return None,
        };
        Some(format!("{key}:{value}"))
    }).collect::<Vec<_>>().join(";")
}

/// 返回经过校验的属性值，调用方仍负责 HTML 属性转义。
pub(crate) fn safe_attribute(tag: &str, name: &str, value: &str) -> Option<String> {
    match name {
        "style" => Some(safe_style(value)),
        "color" if tag == "font" => css_color(value).map(|color| format!("#{:06x}", color >> 8)),
        "face" if tag == "font" => font_family(value),
        "size" if tag == "font" => font_size(value).map(|size| size.to_string()),
        "class" => {
            let classes = value.split_whitespace().filter(|class| ["nomo-fg-", "nomo-bg-"].iter()
                .any(|prefix| class.strip_prefix(prefix).and_then(palette_index).is_some())).collect::<Vec<_>>().join(" ");
            (!classes.is_empty()).then_some(classes)
        },
        _ => None,
    }
}

/// 输出只含白名单 CSS；legacy font 的透明色通过 CSS 保留，色板在导出时固定为当前主题。
pub(crate) fn safe_tag_style(tag: &HtmlTag, theme: Option<&crate::theme::Theme>) -> String {
    let safe = safe_style(tag.attrs.get("style").map(String::as_str).unwrap_or_default());
    // 颜色在下方按有效作用域输出一次，避免反复复制时追加重复声明。
    let mut css = safe.split(';').filter(|entry| !entry.split_once(':').is_some_and(|(key, _)|
        matches!(key, "color" | "background" | "background-color"))).collect::<Vec<_>>().join(";");
    let style = HtmlStyle::parse(tag);
    for (key, color) in [("color", style.foreground), ("background-color", style.background)] {
        let Some(color) = color else { continue; };
        let rgba = match color {
            ColorRole::Rgba(value) => value,
            ColorRole::Palette(index) => {
                let Some(theme) = theme else { continue; };
                let color = gpui::Rgba::from(if key == "color" { theme.palette_fg[usize::from(index).min(6)] } else { theme.palette_bg[usize::from(index).min(6)] });
                u32::from_be_bytes([color.r, color.g, color.b, color.a].map(|value| (value * 255.0).round() as u8))
            },
            _ => continue,
        };
        if !css.is_empty() { css.push(';'); }
        css.push_str(&format!("{key}:#{rgba:08x}"));
    }
    css
}

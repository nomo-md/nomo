//! 图片属性的局部往返。未修改时返回原字节；标题/表格保持单行 Markdown 属性写法。
use super::{nomo_syntax::html_tag, protocol::ControlError};
use pulldown_cmark::{Event, Parser, Tag};

#[derive(Clone,Copy,Debug)]
pub enum ImageWidth {Pixels(f32),Percent(f32)}
impl ImageWidth {
    pub fn parse(value:&str)->Option<Self> {
        let value=value.trim();let percent=value.ends_with('%');
        let number=if percent {value.strip_suffix('%')?}else{value.strip_suffix("px").unwrap_or(value)}.parse::<f32>().ok()?;
        if !number.is_finite() || number<=0.0 || (percent && number>100.0){return None;}
        Some(if percent{Self::Percent(number)}else{Self::Pixels(number)})
    }
    pub fn resolve(self,available:f32,zoom:f32)->f32 {match self{Self::Pixels(value)=>value*zoom,Self::Percent(value)=>available*value/100.0}}
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageProperties {
    pub src: String,
    pub alt: String,
    pub title: String,
    pub width: String,
    pub align: String,
}
impl ImageProperties {
    pub fn read(raw: &str) -> Option<Self> {
        if let Some(at) = raw.find("<img") {
            let tag = html_tag(raw, at)?;
            if tag.name != "img"
                || tag
                    .attrs
                    .keys()
                    .any(|k| !["src", "alt", "title", "width", "align"].contains(&k.as_str()))
            {
                return None;
            }
            let value = |name: &str| {
                tag.attrs
                    .get(name)
                    .map(|v| html_escape::decode_html_entities(v).into_owned())
                    .unwrap_or_default()
            };
            let mut align = value("align");
            if let Some(p) = html_tag(raw, 0).filter(|p| p.name == "p") {
                if p.attrs.keys().any(|k| k != "align") {
                    return None;
                }
                if let Some(a) = p.attrs.get("align") {
                    align = a.clone();
                }
            }
            return Some(Self {
                src: value("src"),
                alt: value("alt"),
                title: value("title"),
                width: value("width"),
                align,
            });
        }
        let mut events = Parser::new(raw).into_offset_iter();
        while let Some((event, range)) = events.next() {
            if let Event::Start(Tag::Image {
                dest_url, title, ..
            }) = event
            {
                let mut alt = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        Event::End(pulldown_cmark::TagEnd::Image) => break,
                        _ => {}
                    }
                }
                let mut width = String::new();
                let mut align = String::new();
                if let Some(attrs) = raw[range.end..]
                    .strip_prefix('{')
                    .and_then(|r| r.split_once('}').map(|(s, _)| s))
                {
                    for attr in attrs.split_whitespace() {
                        let (key, value) = attr.split_once('=')?;
                        match key {
                            "width" => width = value.into(),
                            "align" => align = value.into(),
                            _ => return None,
                        }
                    }
                }
                return Some(Self {
                    src: dest_url.into_string(),
                    alt,
                    title: title.into_string(),
                    width,
                    align,
                });
            }
        }
        None
    }
    pub fn write(&self, original: &str, inline: bool, eol: &str) -> Result<String, ControlError> {
        if Self::read(original).as_ref() == Some(self) {
            return Ok(original.into());
        }
        if self.src.is_empty() || self.src.chars().any(char::is_control) {
            return Err(ControlError::InvalidRange);
        }
        if !matches!(self.align.as_str(), "" | "left" | "center" | "right") {
            return Err(ControlError::InvalidRange);
        }
        if !self.width.is_empty() {
            if ImageWidth::parse(&self.width).is_none() {
                return Err(ControlError::InvalidRange);
            }
        }
        let text = if inline && !original.trim_start().starts_with('<') {
            let alt = self
                .alt
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]")
                .replace('\n', "&#10;")
                .replace('\r', "&#13;");
            let target = self
                .src
                .replace('&', "&amp;")
                .replace('<', "%3C")
                .replace('>', "%3E");
            let title = if self.title.is_empty() {
                String::new()
            } else {
                format!(
                    " \"{}\"",
                    self.title.replace('\\', "\\\\").replace('"', "\\\"")
                )
            };
            let mut text = format!("![{alt}](<{target}>{title})");
            let mut attrs = Vec::new();
            if !self.width.is_empty() {
                attrs.push(format!("width={}", self.width));
            }
            if !self.align.is_empty() {
                attrs.push(format!("align={}", self.align));
            }
            if !attrs.is_empty() {
                text.push_str(&format!("{{{}}}", attrs.join(" ")));
            }
            text
        } else {
            let img = self.html(inline);
            if !inline && !self.align.is_empty() {
                format!("<p align=\"{}\">{eol}  {img}{eol}</p>", self.align)
            } else {
                img
            }
        };
        // 对象范围可能含原块结束换行。不能让图片属性变化粘连下一段。
        let suffix = &original[original.trim_end_matches(['\r', '\n']).len()..];
        Ok(format!("{text}{suffix}"))
    }

    pub(crate) fn html(&self, inline: bool) -> String {
        let attr = |s: &str| html_escape::encode_double_quoted_attribute(s).into_owned();
        let mut img = format!("<img src=\"{}\" alt=\"{}\"", attr(&self.src), attr(&self.alt));
        if !self.title.is_empty() { img.push_str(&format!(" title=\"{}\"", attr(&self.title))); }
        if !self.width.is_empty() { img.push_str(&format!(" width=\"{}\"", attr(&self.width))); }
        if inline && !self.align.is_empty() { img.push_str(&format!(" align=\"{}\"", attr(&self.align))); }
        img.push('>');
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untouched_images_keep_bytes_and_changed_properties_keep_nomo_forms() {
        let raw = "![说明](a.png \"原题\"){width=320 align=center}";
        let mut p = ImageProperties::read(raw).unwrap();
        assert_eq!(p.write(raw, true, "\r\n").unwrap(), raw);
        p.width = "480".into();
        p.title = "引号\"&".into();
        let inline = p.write(raw, true, "\r\n").unwrap();
        assert_eq!(ImageProperties::read(&inline).unwrap(), p);
        let block = p.write(raw, false, "\r\n").unwrap();
        assert!(block.starts_with("<p align=\"center\">\r\n"));
        assert!(block.contains("&quot;&amp;"));
        assert_eq!(ImageProperties::read(&block).unwrap(), p);
        p.width = "NaN".into();
        assert!(p.write(raw, false, "\n").is_err());
    }
}

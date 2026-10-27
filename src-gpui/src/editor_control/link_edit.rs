//! 链接弹层的局部源码事务。只使用当前解析片段，未改显示文字时保留原有内联结构。
use super::{commands::EditResult, mapping::local_range, nomo_syntax::html_tag, parse::{InlineFormat, ParsedWindow}, protocol::*};
use pulldown_cmark::{Event, Tag};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone)]
pub(crate) struct LinkDraft {
    pub text: String,
    pub href: String,
    pub active: bool,
    range: Range<usize>,
    content: Range<usize>,
    title: String,
    html: bool,
    attrs: BTreeMap<String,String>,
}
impl LinkDraft {
    pub fn begin(parsed:&ParsedWindow, selection:DocumentSelection) -> Result<Self,ControlError> {
        let selected=local_range(&parsed.source,parsed.identity.from_byte,selection.range())?;
        let contains=|range:&Range<usize>|range.start<=selected.start && selected.end<=range.end;
        for (event,range) in &parsed.events {
            let Event::Start(Tag::Link{dest_url,title,..})=event else {continue;};
            if !contains(range) {continue;}
            let raw=&parsed.source[range.clone()];
            let content=if raw.starts_with('<') && raw.ends_with('>') {range.start+1..range.end-1}
                else if raw.starts_with('[') {
                    let end=label_end(raw).ok_or_else(||ControlError::NeedsContext("链接标签边界无效".into()))?;
                    range.start+1..range.start+end
                } else {range.clone()};
            // 在链接内部选中一段时保留准确选区，不能偷偷扩大到整条链接。
            if !selected.is_empty() && selected!=content && selected!=*range {
                return Err(ControlError::Unsupported("部分链接选区编辑尚待接入；请将光标放入链接后编辑整条链接".into()));
            }
            return Ok(Self {text:plain_text(parsed,&content),href:dest_url.to_string(),active:true,
                range:range.clone(),content,title:title.to_string(),html:false,attrs:BTreeMap::new()});
        }
        let html_cell=parsed.html_tables.iter().flat_map(|table|&table.cells).find(|cell|cell.content.start<=selected.start && selected.end<=cell.content.end);
        let html_span=parsed.inline.iter().chain(html_cell.into_iter().flat_map(|cell|&cell.spans))
            .find(|span|span.format==InlineFormat::Link && contains(&span.range()));
        if let Some(span)=html_span {
            if let Some(tag)=html_tag(&parsed.source,span.open.start).filter(|tag|tag.name=="a") {
                if !selected.is_empty() && selected!=span.content && selected!=span.range() {
                    return Err(ControlError::Unsupported("部分链接选区编辑尚待接入；请将光标放入链接后编辑整条链接".into()));
                }
                return Ok(Self {text:html_text(&parsed.source[span.content.clone()]),href:tag.attrs.get("href").cloned().unwrap_or_default(),
                    active:true,range:span.range(),content:span.content.clone(),title:tag.attrs.get("title").cloned().unwrap_or_default(),html:true,attrs:tag.attrs});
            }
        }
        if parsed.inline.iter().any(|span|span.format==InlineFormat::Link && span.intersects(&selected)) {
            return Err(ControlError::Unsupported("跨链接选区的链接合并尚待接入".into()));
        }
        // 不把半个格式定界符带进链接标签；完整内联标记或同一格式内部的选区均可保留。
        if parsed.inline.iter().any(|span|span.intersects(&selected)
            && !(selected.start<=span.open.start && span.close.end<=selected.end)
            && !(span.content.start<=selected.start && selected.end<=span.content.end)) {
            return Err(ControlError::Unsupported("跨行内格式边界的链接编辑尚待接入".into()));
        }
        if parsed.nodes.iter().any(|node|node.kind=="code" && node.source.start<=selected.start && selected.end<=node.source.end) {
            return Err(ControlError::Unsupported("代码块内不能应用链接格式".into()));
        }
        if parsed.source[selected.clone()].contains('\n') {
            return Err(ControlError::Unsupported("跨行或跨段链接格式尚待接入".into()));
        }
        Ok(Self {text:if html_cell.is_some() {html_text(&parsed.source[selected.clone()])}else{plain_text(parsed,&selected)},href:String::new(),active:false,
            range:selected.clone(),content:selected,title:String::new(),html:html_cell.is_some(),attrs:BTreeMap::new()})
    }
    pub fn edit(&self, parsed:&ParsedWindow, text:&str, href:Option<&str>) -> Result<EditResult,ControlError> {
        let raw=parsed.source.get(self.content.clone()).ok_or(ControlError::InvalidRange)?;
        let remove=href.is_none();
        if remove && !self.active {return Err(ControlError::InvalidRange);}
        let href=href.map(normalize_href).transpose()?;
        let shown=if text.trim().is_empty() {
            if self.text.is_empty() {href.as_deref().unwrap_or("")}else{&self.text}
        }else{text};
        let table_cell=parsed.events.iter().any(|(event,range)|matches!(event,Event::Start(Tag::TableCell)) && range.start<=self.range.start && self.range.end<=range.end);
        let body=if shown==self.text {raw.to_owned()}else if self.html {html_escape::encode_text(shown).to_string()}else{escape_label(shown)};
        let body=if table_cell && !self.html {escape_table_pipes(&body)}else{body};
        let replacement=if let Some(href)=href {
            if self.html {
                let mut attrs=self.attrs.clone();attrs.insert("href".into(),href);
                let attrs=attrs.into_iter().map(|(name,value)|format!(" {name}=\"{}\"",html_escape::encode_double_quoted_attribute(&value))).collect::<String>();
                format!("<a{attrs}>{body}</a>")
            }else {
                let destination=serialize_destination(&href);
                let destination=if table_cell {escape_table_pipes(&destination)}else{destination};
                let title=if self.title.is_empty() {String::new()}else{format!(" \"{}\"",self.title.replace('&',"&amp;").replace('\\',"\\\\").replace('"',"\\\"").replace('\r',"&#13;").replace('\n',"&#10;"))};
                let title=if table_cell {escape_table_pipes(&title)}else{title};
                format!("[{body}]({destination}{title})")
            }
        }else{raw.to_string()};
        let base=parsed.identity.from_byte+self.range.start as u64;
        let content_start=if remove {0}else if self.html {replacement.find('>').map_or(0,|at|at+1)}else{1};
        let selected_bytes=if remove {raw.len()}else{body.len()};
        Ok(EditResult {edits:vec![SourceEdit {from_byte:base,to_byte:parsed.identity.from_byte+self.range.end as u64,inserted_text:replacement.clone()}],
            selection:DocumentSelection {anchor_byte:base+content_start as u64,head_byte:base+(content_start+selected_bytes) as u64},template_update:None})
    }
}

fn normalize_href(value:&str)->Result<String,ControlError> {
    let href=value.trim();
    let invalid=||ControlError::Unsupported("请输入有效的网页、邮件、锚点或文件路径链接".into());
    if href.is_empty() || href.chars().any(|c|c<='\u{1f}' || c=='\u{7f}') {return Err(invalid());}
    if let Some(colon)=href.find(':') {
        let prefix=&href[..colon];
        let lower=prefix.trim_end().to_ascii_lowercase();
        if matches!(lower.as_str(),"javascript"|"vbscript"|"data") {return Err(invalid());}
        let protocol=prefix.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && prefix.bytes().all(|c|c.is_ascii_alphanumeric() || b"+.-".contains(&c));
        let drive=prefix.len()==1 && href.as_bytes().get(colon+1).is_some_and(|c|*c==b'/' || *c==b'\\');
        if protocol && !drive && !matches!(lower.as_str(),"http"|"https"|"mailto") {return Err(invalid());}
    }
    Ok(href.to_owned())
}
fn serialize_destination(href:&str)->String {
    let mut escaped=String::new();
    for c in href.chars() {match c {'&'=>escaped.push_str("&amp;"),'\\'|'('|')'|'<'|'>'=>{escaped.push('\\');escaped.push(c);},_=>escaped.push(c)}}
    if href.chars().any(char::is_whitespace) {format!("<{escaped}>")}else{escaped}
}
fn escape_label(text:&str)->String {
    let mut result=String::new();
    for c in text.chars() {if c=='&' {result.push_str("&amp;");continue;}if "\\`*_{}[]<>~^".contains(c) {result.push('\\');}result.push(c);}
    result
}
fn escape_table_pipes(text:&str)->String {
    let mut result=String::new();let mut slashes=0;
    for ch in text.chars() {
        if ch=='|' && slashes%2==0 {result.push('\\');}
        result.push(ch);
        slashes=if ch=='\\' {slashes+1}else{0};
    }
    result
}
fn label_end(raw:&str)->Option<usize> {
    let bytes=raw.as_bytes();let mut depth=0usize;let mut at=0;
    while at<bytes.len() {
        match bytes[at] {
            b'\\'=>{at+=2;continue;},
            b'`'=>{
                let count=bytes[at..].iter().take_while(|b|**b==b'`').count();
                let mut scan=at+count;let mut close=None;
                while scan<bytes.len() {
                    if bytes[scan]!=b'`' {scan+=1;continue;}
                    let run=bytes[scan..].iter().take_while(|b|**b==b'`').count();
                    if run==count {close=Some(scan+run);break;}
                    scan+=run;
                }
                at=close.unwrap_or(at+count);continue;
            },
            b'['=>depth+=1,b']'=>{depth=depth.checked_sub(1)?;if depth==0{return Some(at);}},_=>{}
        }at+=1;
    }None
}
fn plain_text(parsed:&ParsedWindow, range:&Range<usize>)->String {
    let mut result=String::new();
    for (event,source) in &parsed.events {
        if source.end<=range.start || range.end<=source.start {continue;}
        if source.start<range.start || source.end>range.end {
            if let Event::Text(text)=event {
                if parsed.source.get(source.clone())==Some(text.as_ref()) {
                    result.push_str(&parsed.source[source.start.max(range.start)..source.end.min(range.end)]);
                }else {
                    result.push_str(&html_text(&parsed.source[source.start.max(range.start)..source.end.min(range.end)]));
                }
            }
            continue;
        }
        match event {Event::Text(text)|Event::Code(text)=>result.push_str(text),Event::SoftBreak|Event::HardBreak=>result.push('\n'),_=>{}}
    }
    if result.is_empty() {html_text(&parsed.source[range.clone()])}else{result}
}
fn html_text(raw:&str)->String {
    let mut text=String::new();let mut at=0;
    while at<raw.len() {
        if let Some(tag)=html_tag(raw,at) {if tag.name=="br" {text.push('\n');}at=tag.end;continue;}
        let c=raw[at..].chars().next().unwrap();text.push(c);at+=c.len_utf8();
    }
    html_escape::decode_html_entities(&text).into_owned()
}

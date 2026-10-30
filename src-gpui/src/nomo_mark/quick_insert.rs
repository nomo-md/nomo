//! 快捷插入的静态目录和局部事务。只处理当前窗口，不保存正文或文档历史。
use super::{commands::{self, EditIntent, EditResult}, mapping::local_range, parse::ParsedWindow, protocol::*};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuickInsertAction {
    Heading(u8), List(u8), Table(usize, usize), Code(&'static str), Quote,
    Callout(&'static str), Image, Details, Link, Math(bool), Diagram(usize),
    Comment(bool), Divider, Emoji,
}

pub(crate) struct Category {
    pub key: &'static str,
    pub icon: &'static str,
    pub variants: &'static [(&'static str, QuickInsertAction)],
    pub default: usize,
}
use QuickInsertAction as A;
pub(crate) const CATEGORIES: &[Category] = &[
    Category { key:"quick_heading", icon:"heading", default:0, variants:&[("H1",A::Heading(1)),("H2",A::Heading(2)),("H3",A::Heading(3)),("H4",A::Heading(4)),("H5",A::Heading(5)),("H6",A::Heading(6))] },
    Category { key:"quick_list", icon:"list", default:0, variants:&[("quick_bullet",A::List(0)),("quick_ordered",A::List(1)),("quick_task",A::List(2))] },
    Category { key:"quick_table", icon:"table", default:1, variants:&[("2×2",A::Table(2,2)),("3×2",A::Table(3,2)),("3×3",A::Table(3,3)),("4×3",A::Table(4,3))] },
    Category { key:"quick_code", icon:"code", default:0, variants:&[("quick_plain",A::Code("")),("JS",A::Code("js")),("TS",A::Code("ts")),("Python",A::Code("python")),("JSON",A::Code("json")),("Bash",A::Code("bash"))] },
    Category { key:"quick_quote", icon:"quote", default:0, variants:&[("",A::Quote)] },
    Category { key:"quick_callout", icon:"callout", default:0, variants:&[("callout_tip",A::Callout("TIP")),("callout_note",A::Callout("NOTE")),("callout_important",A::Callout("IMPORTANT")),("callout_warning",A::Callout("WARNING")),("callout_caution",A::Callout("CAUTION"))] },
    Category { key:"quick_image", icon:"image", default:0, variants:&[("",A::Image)] },
    Category { key:"quick_details", icon:"quote", default:0, variants:&[("",A::Details)] },
    Category { key:"quick_link", icon:"link", default:0, variants:&[("",A::Link)] },
    Category { key:"quick_math", icon:"math", default:0, variants:&[("math_block",A::Math(false)),("math_inline",A::Math(true))] },
    Category { key:"quick_diagram", icon:"diagram", default:0, variants:&[("diagram_flowchart",A::Diagram(0)),("diagram_sequenceDiagram",A::Diagram(1)),("diagram_classDiagram",A::Diagram(2)),("diagram_stateDiagram",A::Diagram(3)),("diagram_pie",A::Diagram(4)),("diagram_gantt",A::Diagram(5)),("diagram_erDiagram",A::Diagram(6))] },
    Category { key:"quick_comment", icon:"comment", default:0, variants:&[("quick_inline",A::Comment(true)),("quick_block",A::Comment(false))] },
    Category { key:"quick_divider", icon:"divider", default:0, variants:&[("",A::Divider)] },
    Category { key:"quick_emoji", icon:"emoji", default:0, variants:&[("",A::Emoji)] },
];

/// 与旧版相同的七种模板；工具栏也复用这里的原文。
pub(crate) const DIAGRAMS: &[(&str, &str, &str)] = &[
    ("流程图","flowchart","flowchart TD\n  A[Start] --> B{Continue?}\n  B -- Yes --> C[Process]\n  B -- No --> D[End]\n  C --> D"),
    ("时序图","sequenceDiagram","sequenceDiagram\n  participant User\n  participant App\n  User->>App: Request action\n  App-->>User: Return result"),
    ("类图","classDiagram","classDiagram\n  class Document {\n    +string title\n    +save()\n  }\n  class Editor {\n    +render()\n  }\n  Document --> Editor"),
    ("状态图","stateDiagram","stateDiagram-v2\n  [*] --> Draft\n  Draft --> Review\n  Review --> Published\n  Published --> [*]"),
    ("饼图","pie","pie title Content share\n  \"Writing\" : 45\n  \"Editing\" : 30\n  \"Organizing\" : 25"),
    ("甘特图","gantt","gantt\n  title Project plan\n  dateFormat  YYYY-MM-DD\n  section Phase one\n  Design :a1, 2026-06-01, 3d\n  Build :after a1, 5d"),
    ("实体关系图","erDiagram","erDiagram\n  USER ||--o{ DOCUMENT : owns\n  DOCUMENT ||--o{ ASSET : references\n  USER {\n    string id\n    string name\n  }\n  DOCUMENT {\n    string id\n    string title\n  }"),
];

/// 未识别旧配置沿用流程图；不会改写存储值。
pub(crate) fn default_diagram_index(kind: &str) -> usize {
    DIAGRAMS.iter().position(|(_, key, _)| *key == kind).unwrap_or(0)
}

#[derive(Clone)]
pub(crate) struct QuickInsertSession {
    pub identity: WindowIdentity,
    pub range: Range<usize>,
    pub trigger: String,
    pub category: usize,
    pub variants: [usize; 14],
    pub error: Option<String>,
    pub scroll: gpui::ScrollHandle,
}
impl QuickInsertSession {
    pub fn new(parsed: &ParsedWindow, range: Range<usize>, trigger: String) -> Self {
        Self { identity:parsed.identity.clone(), range, trigger, category:0,
            variants:std::array::from_fn(|index| CATEGORIES[index].default),
            error:None, scroll:gpui::ScrollHandle::new() }
    }
    pub fn valid(&self, parsed: &ParsedWindow, selection: DocumentSelection) -> bool {
        self.identity == parsed.identity && self.matches(parsed, selection)
    }
    pub fn matches(&self, parsed: &ParsedWindow, selection: DocumentSelection) -> bool {
        parsed.source.get(self.range.clone()) == Some(self.trigger.as_str())
            && selection == DocumentSelection::caret(parsed.identity.from_byte + self.range.end as u64)
    }
    pub fn action(&self) -> QuickInsertAction { CATEGORIES[self.category].variants[self.variants[self.category]].1 }
    pub fn move_category(&mut self, down: bool) {
        self.category=(self.category+CATEGORIES.len()+if down {1}else{CATEGORIES.len()-1})%CATEGORIES.len();
        self.error=None;
        self.scroll.scroll_to_item(self.category);
    }
    pub fn move_variant(&mut self, category: usize, forward: bool) {
        self.category=category;
        let count=CATEGORIES[category].variants.len();
        self.variants[category]=(self.variants[category]+if forward {1}else{count-1})%count;
        self.error=None;
        self.scroll.scroll_to_item(category);
    }
}

pub(crate) fn valid_trigger(value: &str) -> bool {
    use unicode_properties::{UnicodeGeneralCategory, GeneralCategoryGroup};
    let mut chars=value.chars();
    chars.next().is_some_and(|ch| matches!(ch.general_category_group(), GeneralCategoryGroup::Punctuation|GeneralCategoryGroup::Symbol)) && chars.next().is_none()
}

pub(crate) fn can_trigger(parsed: &ParsedWindow, at: usize) -> bool {
    use super::parse::InlineFormat;
    if parsed.context.source_only || !parsed.source.is_char_boundary(at) { return false; }
    if parsed.nodes.iter().any(|node| matches!(node.kind,"code"|"math") && node.source.start<=at && at<node.source.end)
        || parsed.inline.iter().any(|span| matches!(span.format,InlineFormat::Code|InlineFormat::Link) && span.open.start<=at && at<=span.close.end)
        || parsed.events.iter().any(|(event, range)| matches!(event,pulldown_cmark::Event::InlineMath(_)|pulldown_cmark::Event::DisplayMath(_)) && range.start<=at && at<range.end)
    { return false; }
    let before=&parsed.source[..at];
    let previous=before.chars().next_back();
    let line=&parsed.source[commands::line_start(&parsed.source,at)..at];
    let mut prefix=0;
    while let Some(length)=commands::structural_prefix_len(&line[prefix..]) {if length==0{break;}prefix+=length;}
    let structural_start=prefix>0 && prefix==line.len();
    let start=structural_start || parsed.nodes.iter().any(|node| matches!(node.kind,"paragraph"|"heading"|"cell"|"table-cell") && node.source.start==at);
    (start || previous.is_none() || matches!(previous,Some(' '| '\u{3000}'|'\n')))
        && (structural_start || parsed.nodes.iter().any(|node| matches!(node.kind,"paragraph"|"heading"|"cell"|"table-cell") && node.source.start<=at && at<=node.source.end)
            || parsed.source[commands::line_start(&parsed.source,at)..at].trim().is_empty())
}

fn no_change(selection: DocumentSelection) -> EditResult { EditResult { edits:Vec::new(), selection, template_update:None } }

fn line_style(source: &str, at: usize, kind: u8) -> bool {
    let line=&source[commands::line_start(source,at)..at];
    let line=line.trim_start_matches([' ', '\t', '>']);
    let bullet=line.starts_with(['-','*','+']) && line.as_bytes().get(1).is_some_and(|b|b.is_ascii_whitespace());
    let digits=line.bytes().take_while(u8::is_ascii_digit).count();
    let ordered=digits>0 && line.as_bytes().get(digits).is_some_and(|b|matches!(b,b'.'|b')')) && line.as_bytes().get(digits+1).is_some_and(|b|b.is_ascii_whitespace());
    let rest=if bullet {line.get(2..)}else if ordered{line.get(digits+2..)}else{None};
    let task=rest.is_some_and(|rest| rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] "));
    match kind { 2=>task, 1=>!task && ordered, _=>!task && bullet }
}

/// 在去掉触发符的局部投影里执行命令，再折成一笔最小替换。
pub(crate) fn apply(parsed: &ParsedWindow, range: Range<usize>, action: QuickInsertAction) -> Result<EditResult,ControlError> {
    apply_with(parsed,range,|projected,selection| {
        let at=local_range(&projected.source,projected.identity.from_byte,selection.range())?.start;
        let intent=match action {
            A::Heading(level)=>EditIntent::Heading(level),
            A::List(kind) if line_style(&projected.source,at,kind)=>return Ok(no_change(selection)),
            A::List(kind)=>EditIntent::LinePrefix(match kind {1=>"1. ",2=>"- [ ] ",_=>"- "}.into()),
            A::Quote if projected.context.ancestors.iter().any(|c|matches!(c,AncestorContainer::Quote))
                || projected.source[commands::line_start(&projected.source,at)..at].trim_start().starts_with('>')=>return Ok(no_change(selection)),
            A::Quote=>EditIntent::LinePrefix("> ".into()),
            A::Callout(kind)=>return callout(projected,selection,kind),
            A::Table(columns,rows)=>{
                let header=format!("|{}", "  |".repeat(columns));
                let separator=format!("|{}", " --- |".repeat(columns));
                let block=format!("{header}\n{separator}{}",format!("\n{header}").repeat(rows));
                EditIntent::InsertBlockAt{block,caret:2}
            },
            A::Code(language)=>EditIntent::InsertCodeBlock(language.into()),
            A::Details=>EditIntent::InsertDetails,
            A::Diagram(index)=>EditIntent::InsertDiagram(DIAGRAMS[index].2.into()),
            A::Divider=>EditIntent::ToggleRule,
            _=>return Err(ControlError::Unsupported("此项目需要选择器确认".into())),
        };
        let block=matches!(action,A::Table(..)|A::Code(_)|A::Details|A::Diagram(_)|A::Divider);
        if block {return apply_block(projected,selection,intent,matches!(action,A::Diagram(_)|A::Divider));}
        commands::apply_semantic(projected,selection,intent)
    })
}

/// 块保持在当前列表／引用内；单元格与不完整的外层容器明确返回不可用。
pub(crate) fn apply_block(parsed:&ParsedWindow,selection:DocumentSelection,intent:EditIntent,continue_after:bool)->Result<EditResult,ControlError> {
    let at=local_range(&parsed.source,parsed.identity.from_byte,selection.range())?.start;
    let range=at..at;
    if super::table_inline::markdown_cell(parsed,&range) || super::table_inline::html_cell(parsed,&range).is_some()
        || parsed.nodes.iter().any(|node|node.kind=="html" && node.source.start<=at && at<node.source.end)
        || (!matches!(intent,EditIntent::InsertCodeBlock(_)|EditIntent::InsertDiagram(_)) && parsed.inline.iter().any(|span|span.open.start<at && at<span.close.end))
        || !parsed.context.ancestors.is_empty() {
        return Err(ControlError::NeedsContext("当前格式或容器内不能安全插入此块；原文已保留".into()));
    }
    let line_start=commands::line_start(&parsed.source,at);
    let line=&parsed.source[line_start..at];
    let mut length=0;
    while let Some(next)=commands::structural_prefix_len(&line[length..]) {if next==0{break;}length+=next;}
    let prefix=line[..length].chars().map(|ch|if ch=='>' || ch.is_whitespace(){ch}else{' '}).collect::<String>();
    let mut result=commands::apply_semantic(parsed,selection,intent)?;
    if continue_after {continue_after_block(parsed,&mut result)?;}
    if !prefix.is_empty() {
        for edit in &mut result.edits {
            let original=std::mem::take(&mut edit.inserted_text);
            let additions:Vec<_>=original.match_indices('\n').map(|(offset,_)|offset+1).collect();
            let map=|position:u64|position+if position>=edit.from_byte {additions.iter().filter(|offset|**offset as u64<=position-edit.from_byte).count() as u64*prefix.len() as u64}else{0};
            result.selection=DocumentSelection{anchor_byte:map(result.selection.anchor_byte),head_byte:map(result.selection.head_byte)};
            edit.inserted_text=original.replace('\n',&format!("\n{prefix}"));
        }
    }
    Ok(result)
}

/// 块后保留可继续输入的段落，不让后续文字粘回分隔线、围栏或 HTML 结束标签。
pub(crate) fn continue_after_block(parsed: &ParsedWindow, result: &mut EditResult) -> Result<(), ControlError> {
    if result.edits.len() != 1 {
        return Err(ControlError::NeedsContext("当前位置的块后段落需要完整结构；原文已保留".into()));
    }
    let edit = &mut result.edits[0];
    let offset = (edit.to_byte - parsed.identity.from_byte) as usize;
    let suffix = parsed.source.get(offset..).ok_or(ControlError::InvalidRange)?;
    let trailing = edit.inserted_text.chars().rev().take_while(|ch| matches!(ch, '\r' | '\n')).filter(|ch| *ch == '\n').count().min(2);
    let mut leading = 0;
    let mut count = 0;
    while count < 2 - trailing {
        let rest = &suffix[leading..];
        if rest.starts_with("\r\n") { leading += 2; }
        else if rest.starts_with('\n') { leading += 1; }
        else { break; }
        count += 1;
    }
    let eol = commands::line_ending(&parsed.source, offset);
    edit.inserted_text.push_str(&eol.repeat(2 - trailing - count));
    result.selection = DocumentSelection::caret(edit.from_byte + edit.inserted_text.len() as u64 + leading as u64);
    Ok(())
}

fn callout(parsed:&ParsedWindow, selection:DocumentSelection, kind:&str)->Result<EditResult,ControlError> {
    let at=local_range(&parsed.source,parsed.identity.from_byte,selection.range())?.start;
    let paragraph=parsed.nodes.iter().rev().find(|node|node.kind=="paragraph" && node.source.start<=at && at<=node.source.end);
    let quote_node=parsed.nodes.iter().filter(|node|node.kind=="quote" && node.source.start<=at && at<=node.source.end).min_by_key(|node|node.source.end-node.source.start);
    let start=quote_node.or(paragraph).map_or(commands::line_start(&parsed.source,at),|p|commands::line_start(&parsed.source,p.source.start));
    let end=quote_node.or(paragraph).map_or(at,|p|p.source.end);
    let raw=&parsed.source[start..end];
    let quote=raw.trim_start().starts_with('>');
    let indent=&raw[..raw.len()-raw.trim_start().len()];
    if !parsed.context.ancestors.is_empty() || (!quote && paragraph.is_some_and(|p|p.parent.is_some())) {
        return Err(ControlError::NeedsContext("当前嵌套容器内的提示块需要完整结构；原文已保留".into()));
    }
    let eol=commands::line_ending(&parsed.source,at);
    let body=raw.trim_end_matches(['\r','\n']);
    let text=if quote {
        let first=body.lines().next().unwrap_or("");
        if let Some(marker)=first.find("[!") { if let Some(close)=first[marker..].find(']') {
            let mut result=body.to_string();result.replace_range(marker..marker+close+1,&format!("[!{kind}]"));result
        } else {return Err(ControlError::InvalidRange);} }
        else {format!("{indent}> [!{kind}]{eol}{body}")}
    } else {format!("> [!{kind}]{eol}{}",if body.is_empty(){"> ".into()}else{body.lines().map(|line|format!("> {line}")).collect::<Vec<_>>().join(eol)})};
    let text=if text.ends_with(eol) {text}else{format!("{text}{}",&raw[body.len()..])};
    let caret=parsed.identity.from_byte+start as u64+text.trim_end_matches(['\r','\n']).len() as u64;
    Ok(EditResult { edits:vec![SourceEdit{from_byte:parsed.identity.from_byte+start as u64,to_byte:parsed.identity.from_byte+end as u64,inserted_text:text}],selection:DocumentSelection::caret(caret),template_update:None })
}

pub(crate) fn apply_with(parsed:&ParsedWindow, range:Range<usize>, build:impl FnOnce(&ParsedWindow,DocumentSelection)->Result<EditResult,ControlError>)->Result<EditResult,ControlError> {
    if parsed.source.get(range.clone()).is_none() {return Err(ControlError::InvalidRange);}
    let base=parsed.identity.from_byte;
    let mut source=parsed.source.to_string();source.replace_range(range.clone(),"");
    let mut identity=parsed.identity.clone();identity.to_byte=base+source.len() as u64;
    let mut context=parsed.context.clone();
    crate::markdown_templates::map_templates(&mut context.inline_templates,[(base+range.start as u64,base+range.end as u64,0)],false);
    let projected=ParsedWindow::from_source_with_context(identity,source.clone().into(),context);
    let result=build(&projected,DocumentSelection::caret(base+range.start as u64))?;
    for edit in result.edits.into_iter().rev() {source.replace_range((edit.from_byte-base) as usize..(edit.to_byte-base) as usize,&edit.inserted_text);}
    let mut from=0;let mut old=parsed.source.len();let mut new=source.len();
    while from<old && from<new && parsed.source.as_bytes()[from]==source.as_bytes()[from] {from+=1;}
    while !source.is_char_boundary(from) || !parsed.source.is_char_boundary(from) {from-=1;}
    while old>from && new>from && parsed.source.as_bytes()[old-1]==source.as_bytes()[new-1] {old-=1;new-=1;}
    while !source.is_char_boundary(new) || !parsed.source.is_char_boundary(old) {new+=1;old+=1;}
    Ok(EditResult { edits:vec![SourceEdit{from_byte:base+from as u64,to_byte:base+old as u64,inserted_text:source[from..new].into()}],selection:result.selection,template_update:result.template_update })
}

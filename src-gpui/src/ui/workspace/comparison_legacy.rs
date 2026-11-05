//! 旧工作区仅转换路径、标签关系和阅读锚点，正文由恢复目录按需导入。
use super::*;
use crate::legacy_records::{self, Draft};
use serde_json::Value;

fn mode(value: &Value) -> EditorMode {
    match value.as_str() {Some("source")=>EditorMode::Source,Some("split")=>EditorMode::Split,_=>EditorMode::Semantic}
}

pub(super) fn load(root: &Path) -> Result<(Vec<SavedLayout>, Option<String>), String> {
    let config = legacy_records::read_config(root)?;
    let records = legacy_records::workspaces(&config)?;
    if records.is_empty() {return Ok((Vec::new(),None));}
    let default_mode = mode(&legacy_records::setting(&config,"editorMode")?);
    let positions = legacy_records::setting(&config,"readingPositions")?;
    let mut output = Vec::new();
    let mut signatures = HashSet::new();
    let mut warning = None;
    // 文件夹缓存和窗口记录常指向同一份工作区；从较新的记录开始去重。
    for record in records.into_iter().rev() {
        let value = &record.value;
        let signature = serde_json::to_string(value).map_err(|e|e.to_string())?;
        if !signatures.insert(legacy_records::session_id(&signature)) {continue;}
        let mut keys = HashMap::new();
        let mut documents = BTreeMap::new();
        let mut order = Vec::new();
        for (index,tab) in value["tabs"].as_array().unwrap().iter().enumerate() {
            let key = index as u64 + 1;
            let path = legacy_records::absolute_path(&tab["nativePath"]);
            let draft = match Draft::for_tab(root,&record.key,index,tab) {
                Ok(draft)=>draft,
                Err(error)=>{warning.get_or_insert(error);None},
            };
            if draft.as_ref().is_some_and(|draft|draft.handled()) && path.is_none() {continue;}
            let tab_id = tab["id"].as_str().map(str::to_owned).unwrap_or_else(||format!("tab-{index}"));
            let session = draft.as_ref().map(|draft|draft.id.clone()).unwrap_or_else(||legacy_records::session_id(&format!("{}:{tab_id}",record.key)));
            let dirty = draft.as_ref().map_or(tab["dirty"].as_bool().unwrap_or(false)||tab["draftId"].as_str().is_some_and(|id|!id.is_empty()),|draft|!draft.handled());
            let reading = tab["nativePath"].as_str().and_then(|path|positions.get(path.trim().replace('\\',"/").trim_end_matches('/').to_lowercase()));
            let anchor = reading.and_then(|reading|reading.get("anchor").filter(|v|!v.is_null()).or_else(||reading.get("semanticAnchor").filter(|v|!v.is_null())).or_else(||reading.get("sourceAnchor")));
            let legacy_line = anchor.and_then(|anchor|anchor["sourceLine"].as_u64()).filter(|line|*line>1);
            let byte = tab["scrollAnchor"]["byteOffset"].as_u64().unwrap_or(0);
            let selection = DocumentSelection {anchor_byte:tab["selection"]["anchorByte"].as_u64().unwrap_or(byte),head_byte:tab["selection"]["headByte"].as_u64().unwrap_or(byte)};
            let tab_mode = if matches!(tab["documentKind"].as_str(),Some("text"|"json")) {EditorMode::Source} else {default_mode};
            documents.insert(key,SavedDocument {path,session,mode:tab_mode,dirty,revision:0,byte,fraction:0.0,selection,legacy_line});
            keys.insert(tab_id,key);
            order.push(key);
        }
        let mut items = Vec::new();
        let mut active = value["activeTabId"].as_str().and_then(|id|keys.get(id)).copied();
        if let Some(saved_items) = value["items"].as_array() {
            for item in saved_items {
                let lookup = |field:&str|item[field].as_str().and_then(|id|keys.get(id)).copied();
                let next = match item["kind"].as_str() {
                    Some("comparison") => match (lookup("leftTabId"),lookup("rightTabId")) {
                        (Some(left),Some(right)) if left!=right => {
                            let focused = lookup("focusedTabId").filter(|key|*key==left||*key==right).unwrap_or(left);
                            let pair_mode = mode(&item["mode"]);
                            for key in [left,right] {documents.get_mut(&key).unwrap().mode=pair_mode;}
                            Some(TabItem::Pair(Pair{left,right,focused,ratio:(item["leftPercent"].as_f64().unwrap_or(50.0)/100.0).clamp(0.2,0.8) as f32}))
                        },
                        (Some(key),_)|(_,Some(key))=>Some(TabItem::Single(key)),
                        _=>None,
                    },
                    Some("single") => lookup("tabId").map(TabItem::Single),
                    _=>None,
                };
                if let Some(next) = next {
                    if item["id"].as_str().is_some() && item["id"]==value["activeItemId"] {active=Some(next.focused());}
                    items.push(next);
                }
            }
        }
        if items.is_empty() {items.extend(order.into_iter().map(TabItem::Single));}
        let id = u64::from_str_radix(&legacy_records::session_id(&record.key)[10..26],16).map_err(|e|e.to_string())?;
        let mut layout = SavedLayout {id,items,documents,active,folder:legacy_records::absolute_path(&value["currentFolderPath"])};
        layout.normalize();
        if !layout.items.is_empty() {output.push(layout);}
    }
    output.reverse();
    Ok((output,warning))
}

use super::*;

/// 每个构建仅保存有界差异和续扫位置；每一批后让出执行线程。
#[derive(Default)]
pub struct ChangeScan {
    phase: u8,
    byte: u64,
    key: Option<(String, String)>,
    dependency: usize,
    ranges: Vec<Value>,
    mappings: Vec<Value>,
    keys: Vec<Value>,
    full: bool,
    outline_changed: bool,
    references_changed: bool,
}
const CHANGE_BATCH: usize = 64;
impl ChangeScan {
    fn advance(&mut self) { self.phase += 1; self.byte = 0; self.key = None; }
    fn bound(&mut self) {
        if self.full || self.ranges.len() > PAGE_ITEMS || self.mappings.len() > PAGE_ITEMS
            || self.keys.len() > PAGE_ITEMS
            || serde_json::to_vec(&(&self.ranges, &self.mappings, &self.keys))
                .map_or(true, |bytes| bytes.len() > PAGE_BYTES - 4096) {
            self.full = true;
            self.ranges.clear(); self.mappings.clear(); self.keys.clear();
        }
    }
}
impl Store {
    /// 分页事实查询与身份查找代替全文 EXCEPT；SQL 内部排序由 progress handler 响应取消。
    pub fn record_changes_step(&self, current: &Identity, previous: Option<&Identity>, scan: &mut ChangeScan) -> TextDocumentResult<bool> {
        self.check_cancelled()?;
        let Some(previous) = previous else { return Ok(true); };
        let db = self.db();
        let generation = current.index_generation;
        let old = previous.index_generation;
        match scan.phase {
            0 => {
                let mut statement = db.prepare("SELECT start,end,id,json_extract(payload,'$.sourceHash'),json_extract(payload,'$.kind') FROM blocks WHERE generation=?1 AND start>=?2 ORDER BY start LIMIT 64").map_err(sql)?;
                let rows = statement.query_map(params![generation,scan.byte], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<u32>>(3)?,r.get::<_,String>(4)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
                for (start,end,id,hash,kind) in &rows {
                    let same: bool = hash.is_some() && db.query_row("SELECT EXISTS(SELECT 1 FROM projected_blocks WHERE generation=?1 AND id=?2 AND json_extract(payload,'$.sourceHash') IS ?3 AND json_extract(payload,'$.kind')=?4)",params![old,id,hash,kind],|r|r.get(0)).map_err(sql)?;
                    if !same { scan.ranges.push(json!({"fromByte":start,"toByte":end})); }
                    scan.byte = (*end).max(start+1);
                }
                scan.bound();
                if rows.len()<CHANGE_BATCH || scan.full { scan.advance(); }
            }
            1 => {
                let mut statement = db.prepare("SELECT MAX(n.source_from,o.source_from)+o.byte_delta,MIN(n.source_to,o.source_to)+o.byte_delta,n.byte_delta-o.byte_delta,n.line_delta-o.line_delta FROM mappings n JOIN mappings o ON o.generation=?2 AND o.source_generation=n.source_generation WHERE n.generation=?1 AND n.source_from<o.source_to AND n.source_to>o.source_from LIMIT 257").map_err(sql)?;
                for row in statement.query_map(params![generation,old], |r| Ok(json!({"fromByte":r.get::<_,u64>(0)?,"toByte":r.get::<_,u64>(1)?,"byteDelta":r.get::<_,i64>(2)?,"lineDelta":r.get::<_,i64>(3)?}))).map_err(sql)? { scan.mappings.push(row.map_err(sql)?); }
                scan.bound(); scan.advance();
            }
            2 | 3 => {
                let (source,other) = if scan.phase==2 {(generation,old)} else {(old,generation)};
                let (namespace,key) = scan.key.clone().unwrap_or_default();
                let mut statement = db.prepare("SELECT namespace,key FROM projected_definitions WHERE generation=?1 AND (?2=0 OR (namespace,key)>(?3,?4)) GROUP BY namespace,key ORDER BY namespace,key LIMIT 64").map_err(sql)?;
                let rows = statement.query_map(params![source,scan.key.is_some(),namespace,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
                for (namespace,key) in &rows {
                    let effective = |version| -> TextDocumentResult<Option<String>> { db.query_row("SELECT payload FROM projected_definitions WHERE generation=?1 AND namespace=?2 AND key=?3 ORDER BY byte LIMIT 1",params![version,namespace,key],|r|r.get(0)).optional().map_err(sql) };
                    if effective(source)? != effective(other)? {
                        scan.references_changed = true;
                        let item = json!({"namespace":namespace,"key":key});
                        if !scan.full && !scan.keys.contains(&item) { scan.keys.push(item); scan.bound(); }
                    }
                    scan.key = Some((namespace.clone(),key.clone()));
                }
                if rows.len()<CHANGE_BATCH || (scan.full && scan.references_changed) { scan.advance(); }
            }
            4 | 5 => {
                let (source,other) = if scan.phase==4 {(generation,old)} else {(old,generation)};
                let mut statement = db.prepare("SELECT byte,block_id,heading_index,level,title,slug FROM projected_headings WHERE generation=?1 AND byte>=?2 ORDER BY byte LIMIT 64").map_err(sql)?;
                let rows = statement.query_map(params![source,scan.byte],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?,r.get::<_,u8>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
                for (byte,id,index,level,title,slug) in &rows {
                    let same: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM projected_headings WHERE generation=?1 AND block_id=?2 AND heading_index=?3 AND level=?4 AND title=?5 AND slug=?6)",params![other,id,index,level,title,slug],|r|r.get(0)).map_err(sql)?;
                    scan.outline_changed |= !same; scan.byte=byte+1;
                    if scan.outline_changed { break; }
                }
                if rows.len()<CHANGE_BATCH || scan.outline_changed { scan.advance(); }
            }
            6 => {
                if !scan.full && scan.dependency<scan.keys.len() {
                    let key=&scan.keys[scan.dependency];
                    let mut statement=db.prepare("SELECT b.start,b.end FROM projected_blocks b JOIN dependencies d ON d.generation=b.source_generation AND d.block_id=b.id WHERE b.generation=?1 AND d.namespace=?2 AND d.key=?3 LIMIT 257").map_err(sql)?;
                    for row in statement.query_map(params![generation,key["namespace"].as_str(),key["key"].as_str()],|r|Ok(json!({"fromByte":r.get::<_,u64>(0)?,"toByte":r.get::<_,u64>(1)?}))).map_err(sql)? { scan.ranges.push(row.map_err(sql)?); }
                    scan.dependency+=1; scan.bound();
                }
                if scan.full || scan.dependency==scan.keys.len() { scan.advance(); }
            }
            _ => {
                let reference_version: u64 = if scan.references_changed {generation} else {db.query_row("SELECT json_extract(payload,'$.referenceVersion') FROM structure_changes WHERE generation=?1",[old],|r|r.get(0)).optional().map_err(sql)?.unwrap_or(old)};
                let payload=json!({"fromGeneration":old,"invalidateAll":scan.full,"ranges":scan.ranges,"mappings":scan.mappings,"references":scan.keys,"referenceVersion":reference_version,"outlineChanged":scan.outline_changed});
                db.execute("INSERT OR REPLACE INTO structure_changes VALUES(?1,?2)",params![generation,payload.to_string()]).map_err(sql)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn changes(&self, generation:u64, observed:u64) -> TextDocumentResult<Value> {
        let payload:Option<String> = self.db().query_row("SELECT payload FROM structure_changes WHERE generation=?1",[generation],|r|r.get(0)).optional().map_err(sql)?;
        if let Some(payload)=payload {
            let value:Value=serde_json::from_str(&payload).map_err(sql)?;
            if value["fromGeneration"].as_u64()==Some(observed) {return Ok(value);}
        }
        Ok(json!({"fromGeneration":observed,"invalidateAll":true,"ranges":[],"mappings":[],"references":[],"referenceVersion":generation,"outlineChanged":true}))
    }
}

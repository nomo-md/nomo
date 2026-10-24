use super::*;

/// 共享已发布 slug 的不可变区间，仅重算编辑范围和受碰撞影响的名称。
/// dirty 名称存入分页库；不在每个文档常驻全量名称集合。
pub struct SlugScan {
    previous: Identity,
    start: u64,
    old_end: u64,
    new_end: u64,
    cursor: u64,
    phase: u8,
    pub recalculated: u64,
}
impl SlugScan {
    pub fn new(previous: Identity, start: u64, old_end: u64, new_end: u64) -> Self {
        Self { previous, start, old_end, new_end, cursor:start, phase:0, recalculated:0 }
    }
}
impl Store {
    fn share_slugs(&self, generation:u64, old:u64, from:u64, to:u64, delta:i64) -> TextDocumentResult<()> {
        if from>=to { return Ok(()); }
        self.db().execute("INSERT OR IGNORE INTO slug_reuse SELECT ?1,source_generation,MAX(source_from,?3-byte_delta),MIN(source_to,?4-byte_delta),byte_delta+?5 FROM slug_mappings WHERE generation=?2 AND source_from+byte_delta<?4 AND source_to+byte_delta>?3",params![generation,old,from,to,delta]).map_err(sql)?;
        Ok(())
    }
    fn dirty_slug(&self, generation:u64, previous:u64, base:&str) -> TextDocumentResult<()> {
        self.db().execute("INSERT OR IGNORE INTO slug_dirty VALUES(?1,?2)",params![generation,base]).map_err(sql)?;
        // 新增 a-2 也可能占用后续重复 a 原来的输出，反向传播到该输出的 base。
        self.db().execute("INSERT OR IGNORE INTO slug_dirty SELECT ?1,h.base FROM projected_slugs s JOIN raw_headings h ON h.generation=s.generation AND h.byte=s.byte WHERE s.generation=?2 AND s.slug=?3",params![generation,previous,base]).map_err(sql)?;
        // 空标题按全文序号生成 heading-N；显式同名前缀变化也会改变它的碰撞结果。
        if base.starts_with("heading-") {
            self.db().execute("INSERT OR IGNORE INTO slug_dirty VALUES(?1,'')",[generation]).map_err(sql)?;
        }
        Ok(())
    }
    fn allocate_incremental_slug(&self, generation:u64, byte:u64, base:&str, level:u8) -> TextDocumentResult<String> {
        let db=self.db();
        let base=if base.is_empty() {
            let ordinal:u64=db.query_row("SELECT COUNT(*) FROM raw_headings WHERE generation=?1 AND byte<=?2",params![generation,byte],|r|r.get(0)).map_err(sql)?;
            format!("heading-{ordinal}")
        } else {base.to_owned()};
        let mut suffix:u64=db.query_row("SELECT next FROM suffixes WHERE generation=?1 AND base=?2",params![generation,base],|r|r.get(0)).optional().map_err(sql)?.unwrap_or(2);
        let mut slug=base.clone();
        while db.query_row("SELECT EXISTS(SELECT 1 FROM projected_slugs WHERE generation=?1 AND slug=?2)",params![generation,slug],|r|r.get::<_,bool>(0)).map_err(sql)? {
            self.check_cancelled()?;
            slug=format!("{base}-{suffix}"); suffix+=1;
        }
        db.execute("INSERT OR REPLACE INTO suffixes VALUES(?1,?2,?3)",params![generation,base,suffix]).map_err(sql)?;
        db.execute("INSERT INTO heading_slugs(generation,byte,slug,level) VALUES(?1,?2,?3,?4)",params![generation,byte,slug,level]).map_err(sql)?;
        Ok(slug)
    }
    pub fn calibrate_incremental(&self, identity:&Identity, scan:&mut SlugScan) -> TextDocumentResult<bool> {
        self.check_cancelled()?;
        let db=self.db();
        let generation=identity.index_generation;
        let old=scan.previous.index_generation;
        // 单批事务只包含最多 64 个标题；取消时整批回滚，旧发布代保持可读。
        let tx=db.unchecked_transaction().map_err(sql)?;
        match scan.phase {
            0 | 1 => {
                let (source,end)=if scan.phase==0 {(old,scan.old_end)} else {(generation,scan.new_end)};
                let mut statement=db.prepare("SELECT h.byte,h.base,h.slug FROM raw_headings h WHERE h.generation=?1 AND h.byte>=?2 AND h.byte<?3 ORDER BY h.byte LIMIT 64").map_err(sql)?;
                let rows=statement.query_map(params![source,scan.cursor,end],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
                for (byte,base,_) in &rows {
                    self.dirty_slug(generation,old,base)?;
                    if scan.phase==0 {
                        let slug:String=db.query_row("SELECT slug FROM projected_slugs WHERE generation=?1 AND byte=?2",params![old,byte],|r|r.get(0)).map_err(sql)?;
                        self.dirty_slug(generation,old,&slug)?;
                    }
                    scan.cursor=byte+1;
                }
                if rows.len()<64 {scan.phase+=1;scan.cursor=scan.start;}
            }
            2 => {
                self.share_slugs(generation,old,0,scan.start,0)?;
                db.execute("DELETE FROM suffixes WHERE generation=?1",[generation]).map_err(sql)?;
                let counts_equal:bool=db.query_row("SELECT (SELECT COUNT(*) FROM raw_headings WHERE generation=?1 AND byte>=?3 AND byte<?4)=(SELECT COUNT(*) FROM raw_headings WHERE generation=?2 AND byte>=?3 AND byte<?5)",params![generation,old,scan.start,scan.new_end,scan.old_end],|r|r.get(0)).map_err(sql)?;
                if !counts_equal {self.dirty_slug(generation,old,"")?;}
                scan.phase+=1;
            }
            3 => {
                let mut statement=db.prepare("SELECT byte,base,level FROM raw_headings WHERE generation=?1 AND byte>=?2 AND byte<?3 ORDER BY byte LIMIT 64").map_err(sql)?;
                let rows=statement.query_map(params![generation,scan.cursor,scan.new_end],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,u8>(2)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
                for (byte,base,level) in &rows {
                    let slug=self.allocate_incremental_slug(generation,*byte,base,*level)?;
                    self.dirty_slug(generation,old,&slug)?;
                    scan.recalculated+=1;scan.cursor=byte+1;
                }
                if rows.len()<64 {scan.phase+=1;scan.cursor=scan.new_end;}
            }
            _ => {
                let delta=scan.new_end as i64-scan.old_end as i64;
                // 每个标题后重新选取，刚改变的输出 slug 可能影响后续同名 base。
                for _ in 0..64 {
                    self.check_cancelled()?;
                    let next:Option<(u64,String,u8)>=db.query_row("SELECT byte,base,level FROM raw_headings WHERE generation=?1 AND byte>=?2 AND base IN (SELECT base FROM slug_dirty WHERE generation=?1) ORDER BY byte LIMIT 1",params![generation,scan.cursor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
                    let Some((byte,base,level))=next else {
                        let total:u64=db.query_row("SELECT total FROM generations WHERE generation=?1",[generation],|r|r.get(0)).map_err(sql)?;
                        self.share_slugs(generation,old,scan.cursor.saturating_add_signed(-delta),total.saturating_add_signed(-delta)+1,delta)?;
                        tx.commit().map_err(sql)?;
                        return Ok(true);
                    };
                    self.share_slugs(generation,old,scan.cursor.saturating_add_signed(-delta),byte.saturating_add_signed(-delta),delta)?;
                    let before:String=db.query_row("SELECT slug FROM projected_slugs WHERE generation=?1 AND byte=?2",params![old,byte.saturating_add_signed(-delta)],|r|r.get(0)).map_err(sql)?;
                    let slug=self.allocate_incremental_slug(generation,byte,&base,level)?;
                    if slug!=before {self.dirty_slug(generation,old,&before)?;self.dirty_slug(generation,old,&slug)?;}
                    scan.recalculated+=1;scan.cursor=byte+1;
                }
            }
        }
        tx.commit().map_err(sql)?;
        Ok(false)
    }
}

use super::*;

impl Store {
    pub fn close_outline(&self, document: &str, view: &str) -> TextDocumentResult<()> {
        if self.connection.is_none() {
            return Ok(());
        }
        let db = self.db();
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outline_views WHERE document=?1 AND view_id=?2)",
                params![document, view],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if exists {
            for table in ["outline_rows", "outline_folds", "outline_views"] {
                db.execute(&format!("DELETE FROM {table} WHERE view_id=?1"), [view])
                    .map_err(sql)?;
            }
        }
        Ok(())
    }
    pub fn release_views(&self, document: &str) -> TextDocumentResult<()> {
        if self.connection.is_none() {
            return Ok(());
        }
        let db = self.db();
        db.execute("DELETE FROM toc_baselines WHERE document=?1", [document])
            .map_err(sql)?;
        for table in ["outline_rows", "outline_folds"] {
            db.execute(&format!("DELETE FROM {table} WHERE view_id IN (SELECT view_id FROM outline_views WHERE document=?1)"),[document]).map_err(sql)?;
        }
        db.execute("DELETE FROM outline_views WHERE document=?1", [document])
            .map_err(sql)?;
        Ok(())
    }
    fn reset_outline(&self, view: &str, generation: u64) -> TextDocumentResult<()> {
        self.db()
            .execute("DELETE FROM outline_rows WHERE view_id=?1", [view])
            .map_err(sql)?;
        self.db().execute("UPDATE outline_views SET generation=?2,view_generation=view_generation+1,scan_byte=0,visible_count=0,complete=0 WHERE view_id=?1",params![view,generation]).map_err(sql)?;
        Ok(())
    }
    pub(super) fn outline_query(
        &self,
        identity: &Identity,
        query: Query,
        response: &mut Response,
    ) -> TextDocumentResult<()> {
        let db = self.db();
        let generation = identity.index_generation;
        if let Query::OpenOutline { default_level } = query {
            let view: String = db
                .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
                .map_err(sql)?;
            db.execute("INSERT INTO outline_views(view_id,document,default_level,generation,view_generation) VALUES(?1,?2,?3,?4,0)",params![view,identity.document_id,default_level.clamp(1,6),generation]).map_err(sql)?;
            response
                .items
                .push(json!({"viewId":view,"viewGeneration":0}));
            return Ok(());
        }
        let view = match &query {
            Query::OutlineView { view_id, .. }
            | Query::SetOutlineFold { view_id, .. }
            | Query::SetOutlineLevel { view_id, .. }
            | Query::CloseOutline { view_id } => view_id,
            _ => unreachable!(),
        };
        let valid: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outline_views WHERE view_id=?1 AND document=?2)",
                params![view, identity.document_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if !valid {
            return Err(TextDocumentError::new(
                "outline-view-expired",
                "大纲视图已释放或不属于当前文档",
            ));
        }
        match query {
            Query::SetOutlineFold {
                view_id,
                heading_id,
                collapsed,
            } => {
                db.execute(
                    "INSERT OR REPLACE INTO outline_folds VALUES(?1,?2,?3)",
                    params![view_id, heading_id, collapsed],
                )
                .map_err(sql)?;
                self.reset_outline(&view_id, generation)?;
            }
            Query::SetOutlineLevel { view_id, level } => {
                db.execute("DELETE FROM outline_folds WHERE view_id=?1", [&view_id])
                    .map_err(sql)?;
                db.execute(
                    "UPDATE outline_views SET default_level=?2 WHERE view_id=?1",
                    params![view_id, level.clamp(1, 6)],
                )
                .map_err(sql)?;
                self.reset_outline(&view_id, generation)?;
            }
            Query::CloseOutline { view_id } => {
                for table in ["outline_rows", "outline_folds", "outline_views"] {
                    db.execute(&format!("DELETE FROM {table} WHERE view_id=?1"), [&view_id])
                        .map_err(sql)?;
                }
            }
            Query::OutlineView {
                view_id,
                from_index,
                view_generation,
            } => {
                let current: u64 = db
                    .query_row(
                        "SELECT generation FROM outline_views WHERE view_id=?1",
                        [&view_id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if current != generation {
                    self.reset_outline(&view_id, generation)?;
                }
                let (epoch,mut scan,mut count,mut complete,level):(u64,u64,u64,bool,u8)=db.query_row("SELECT view_generation,scan_byte,visible_count,complete,default_level FROM outline_views WHERE view_id=?1",[&view_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(sql)?;
                if view_generation.is_some_and(|expected| expected != epoch) {
                    return Err(TextDocumentError::new(
                        "outline-cursor-expired",
                        "折叠投影已变化，请重新定位当前视图",
                    ));
                }
                // 一次查询最多推进 256 个可见标题，折叠子树直接按章节末尾跳过。
                // 结果只有局部行；视图顺序、父子关系和折叠计算不进入 WebView。
                if !complete && response.state == "ready" {
                    let tx = db.unchecked_transaction().map_err(sql)?;
                    for _ in 0..PAGE_ITEMS {
                        let next:Option<(u64,u8,bool)>=db.query_row("SELECT h.byte,h.level,COALESCE(f.collapsed,h.level>=?4) FROM raw_headings h LEFT JOIN outline_folds f ON f.view_id=?3 AND f.heading_id=h.block_id||':'||h.heading_index WHERE h.generation=?1 AND h.byte>=?2 ORDER BY h.byte LIMIT 1",params![generation,scan,view_id,level],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
                        let Some((byte, heading_level, collapsed)) = next else {
                            complete = true;
                            break;
                        };
                        db.execute(
                            "INSERT INTO outline_rows VALUES(?1,?2,?3)",
                            params![view_id, count, byte],
                        )
                        .map_err(sql)?;
                        count += 1;
                        scan = if collapsed {
                            let end:u64=db.query_row("SELECT COALESCE(MIN(byte),?4) FROM raw_headings WHERE generation=?1 AND byte>?2 AND level<=?3",params![generation,byte,heading_level,response.total_bytes],|r|r.get(0)).map_err(sql)?;
                            end.max(byte + 1)
                        } else {
                            byte + 1
                        };
                    }
                    db.execute("UPDATE outline_views SET scan_byte=?2,visible_count=?3,complete=?4 WHERE view_id=?1",params![view_id,scan,count,complete]).map_err(sql)?;
                    tx.commit().map_err(sql)?;
                }
                // 是否有子项只取紧随其后的标题级别，避免对整篇标题逐项展开章节视图。
                let mut statement=db.prepare("SELECT r.position,h.block_id,h.heading_index,h.level,h.title,h.slug,h.byte,h.line,COALESCE(f.collapsed,h.level>=?4),(SELECT MAX(p.byte) FROM raw_headings p WHERE p.generation=h.generation AND p.byte<h.byte AND p.level<h.level),COALESCE((SELECT n.level>h.level FROM raw_headings n WHERE n.generation=h.generation AND n.byte>h.byte ORDER BY n.byte LIMIT 1),0),COALESCE((SELECT MIN(n.byte) FROM raw_headings n WHERE n.generation=h.generation AND n.byte>h.byte AND n.level<=h.level),?5) FROM outline_rows r JOIN projected_headings h ON h.generation=?1 AND h.byte=r.byte LEFT JOIN outline_folds f ON f.view_id=r.view_id AND f.heading_id=h.block_id||':'||h.heading_index WHERE r.view_id=?2 AND r.position>=?3 ORDER BY r.position LIMIT 256").map_err(sql)?;
                let mapped=statement.query_map(params![generation,view_id,from_index,level,response.total_bytes],|r|Ok(json!({"position":r.get::<_,u64>(0)?,"blockId":r.get::<_,String>(1)?,"blockHeadingIndex":r.get::<_,u64>(2)?,"level":r.get::<_,u8>(3)?,"title":r.get::<_,String>(4)?,"id":r.get::<_,String>(5)?,"byte":r.get::<_,u64>(6)?,"line":r.get::<_,u64>(7)?,"collapsed":r.get::<_,bool>(8)?,"parentByte":r.get::<_,Option<u64>>(9)?,"hasChildren":r.get::<_,bool>(10)?,"toByte":r.get::<_,u64>(11)?}))).map_err(sql)?;
                let mut rows = vec![];
                let mut bytes = 4096;
                for row in mapped {
                    let row = row.map_err(sql)?;
                    let size = serde_json::to_vec(&row).map_err(sql)?.len();
                    if bytes + size > PAGE_BYTES - 2048 {
                        if rows.is_empty() {
                            return Err(TextDocumentError::new("outline-item-too-large", "大纲标题超过单页预算，无法安全显示此项"));
                        }
                        break;
                    }
                    bytes += size;
                    rows.push(row);
                }
                let has_collapsed: Option<bool> = if from_index == 0 { Some(db.query_row("SELECT EXISTS(SELECT 1 FROM raw_headings h LEFT JOIN outline_folds f ON f.view_id=?2 AND f.heading_id=h.block_id||':'||h.heading_index WHERE h.generation=?1 AND COALESCE(f.collapsed,h.level>=?3) AND COALESCE((SELECT n.level>h.level FROM raw_headings n WHERE n.generation=h.generation AND n.byte>h.byte ORDER BY n.byte LIMIT 1),0))",params![generation,view_id,level],|r|r.get(0)).map_err(sql)?) } else { None };
                let has_expandable: Option<bool> = if from_index == 0 { Some(db.query_row("SELECT EXISTS(SELECT 1 FROM raw_headings h WHERE h.generation=?1 AND COALESCE((SELECT n.level>h.level FROM raw_headings n WHERE n.generation=h.generation AND n.byte>h.byte ORDER BY n.byte LIMIT 1),0))",[generation],|r|r.get(0)).map_err(sql)?) } else { None };
                response.items.push(json!({"viewId":view_id,"viewGeneration":epoch,"visibleCount":count,"projectionComplete":complete,"fromIndex":from_index,"hasCollapsed":has_collapsed,"hasExpandable":has_expandable,"rows":rows}));
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

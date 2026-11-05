use super::*;
use std::time::{Duration, Instant};

// 复用一次事务，避免 Windows 为每一小组行反复创建回滚日志；遇到大行或慢磁盘时提早让锁。
fn reclaim_rows(
    db: &Connection,
    table: &str,
    key: &str,
    value: &dyn rusqlite::ToSql,
    started: Instant,
) -> TextDocumentResult<bool> {
    let mut delete = db.prepare(&format!(
        "DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE {key}=?1 LIMIT 128)"
    )).map_err(sql)?;
    let mut total = 0;
    loop {
        let rows = delete.execute([value]).map_err(sql)?;
        total += rows;
        if rows < 128 || total >= 1024 || started.elapsed() >= Duration::from_millis(4) {
            return Ok(total != 0);
        }
    }
}

impl Store {
    /// 关闭/移交必须立即使旧大纲、TOC 和代数不可见。只摘除身份和映射，
    /// 将实际页的来源登记在派生库里；新 owner 或重新打开的标签不会被迟到的清理误删。
    pub fn retire_document(&self, document: &str, generations: &[u64]) -> TextDocumentResult<bool> {
        if self.connection.is_none() {
            return Ok(false);
        }
        let tx = self.db().unchecked_transaction().map_err(sql)?;
        let views = tx.execute(
            "INSERT OR IGNORE INTO retired_views SELECT view_id FROM outline_views WHERE document=?1",
            [document],
        ).map_err(sql)?;
        tx.execute("DELETE FROM outline_views WHERE document=?1", [document]).map_err(sql)?;
        tx.execute("DELETE FROM toc_baselines WHERE document=?1", [document]).map_err(sql)?;
        for &generation in generations {
            tx.execute("INSERT OR IGNORE INTO retired_sources VALUES(?1)", [generation]).map_err(sql)?;
            for table in ["reuse", "slug_reuse"] {
                tx.execute(
                    &format!("INSERT OR IGNORE INTO retired_sources SELECT source_generation FROM {table} WHERE generation=?1"),
                    [generation],
                ).map_err(sql)?;
                tx.execute(&format!("DELETE FROM {table} WHERE generation=?1"), [generation]).map_err(sql)?;
            }
            tx.execute("DELETE FROM structure_changes WHERE generation=?1", [generation]).map_err(sql)?;
            tx.execute("DELETE FROM generations WHERE generation=?1", [generation]).map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        Ok(views != 0 || !generations.is_empty())
    }

    /// 每次至多删除一批行，让其它文档的查询在两批之间即可取得数据库锁。
    /// 页仍被其它代数复用时保留，最终引用退出时会再次登记同一来源。
    pub fn reclaim_batch(&self) -> TextDocumentResult<bool> {
        let started = Instant::now();
        let db = self.db();
        let tx = db.unchecked_transaction().map_err(sql)?;
        if let Some(view) = tx.query_row("SELECT view_id FROM retired_views LIMIT 1", [], |r| r.get::<_, String>(0)).optional().map_err(sql)? {
            for table in ["outline_rows", "outline_folds"] {
                if reclaim_rows(&tx, table, "view_id", &view, started)? {
                    tx.commit().map_err(sql)?;
                    return Ok(true);
                }
            }
            tx.execute("DELETE FROM retired_views WHERE view_id=?1", [view]).map_err(sql)?;
        } else if let Some(generation) = tx.query_row("SELECT generation FROM retired_sources LIMIT 1", [], |r| r.get::<_, u64>(0)).optional().map_err(sql)? {
            let (live, shared_blocks, shared_slugs): (bool, bool, bool) = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM generations WHERE generation=?1), EXISTS(SELECT 1 FROM reuse WHERE source_generation=?1), EXISTS(SELECT 1 FROM slug_reuse WHERE source_generation=?1)",
                [generation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).map_err(sql)?;
            if !live {
                for (table, keep) in [
                    ("suffixes", false), ("slug_dirty", false), ("heading_slugs", shared_slugs),
                    ("blocks", shared_blocks), ("headings", shared_blocks),
                    ("definitions", shared_blocks), ("dependencies", shared_blocks), ("geometry", shared_blocks),
                ] {
                    if keep {
                        continue;
                    }
                    if reclaim_rows(&tx, table, "generation", &generation, started)? {
                        tx.commit().map_err(sql)?;
                        return Ok(true);
                    }
                }
            }
            tx.execute("DELETE FROM retired_sources WHERE generation=?1", [generation]).map_err(sql)?;
        } else {
            tx.commit().map_err(sql)?;
            db.execute_batch("PRAGMA incremental_vacuum(256)").map_err(sql)?;
            return Ok(false);
        }
        tx.commit().map_err(sql)?;
        Ok(true)
    }
}

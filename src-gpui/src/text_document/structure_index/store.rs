use super::parser::{Block, Enhancements, SYNTAX_VERSION};
use crate::text_document::{TextDocumentError, TextDocumentResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
#[path = "outline.rs"]
mod outline;
#[path = "changes.rs"]
mod changes;
pub use changes::ChangeScan;
#[path = "slugs.rs"]
mod slugs;
pub use slugs::SlugScan;

/// 只在执行构建片段期间安装；清理 SQL 不受已取消任务的回调影响。
pub struct SqlCancellation<'a>(&'a Store);
impl Drop for SqlCancellation<'_> {
    fn drop(&mut self) {
        let _ = self.0.db().progress_handler(0, None::<fn() -> bool>);
        self.0.cancellation.borrow_mut().take();
    }
}

pub const PAGE_ITEMS: usize = 256;
pub const PAGE_BYTES: usize = 256 * 1024;
pub const DISK_BYTES: u64 = 512 * 1024 * 1024;
fn sql(error: impl std::fmt::Display) -> TextDocumentError {
    TextDocumentError::new("structure-storage", error.to_string())
}
fn project(mut block: Block, revision: u64, byte_delta: i64, line_delta: i64) -> Block {
    block.version = revision;
    block.from_byte = block.from_byte.saturating_add_signed(byte_delta);
    block.to_byte = block.to_byte.saturating_add_signed(byte_delta);
    block.from_line = block.from_line.saturating_add_signed(line_delta);
    block.to_line = block.to_line.saturating_add_signed(line_delta);
    for heading in &mut block.headings {
        heading.byte = heading.byte.saturating_add_signed(byte_delta);
        heading.line = heading.line.saturating_add_signed(line_delta);
    }
    for note in &mut block.footnotes {
        note.from_byte = note.from_byte.saturating_add_signed(byte_delta);
        note.to_byte = note.to_byte.saturating_add_signed(byte_delta);
    }
    block
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub document_id: String,
    pub revision: u64,
    pub index_generation: u64,
    pub syntax_version: String,
}
impl Identity {
    pub fn new(document_id: String, revision: u64, generation: u64) -> Self {
        Self {
            document_id,
            revision,
            index_generation: generation,
            syntax_version: SYNTAX_VERSION.into(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub generation: u64,
    pub ordinal: u64,
    #[serde(default)]
    pub offset: usize,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Query {
    OpenOutline {
        default_level: u8,
    },
    OutlineView {
        view_id: String,
        from_index: u64,
        view_generation: Option<u64>,
    },
    SetOutlineFold {
        view_id: String,
        heading_id: String,
        collapsed: bool,
    },
    SetOutlineLevel {
        view_id: String,
        level: u8,
    },
    CloseOutline {
        view_id: String,
    },
    Progress,
    LocateOutline { view_id: String, heading_id: String, byte: u64, parent: bool },
    /// 阅读锚点对应的真实标题；不把折叠后可见的父项误认为活动标题。
    HeadingAtByte { byte: u64 },
    Changes { observed_generation: u64 },
    TocBlocks {
        cursor: Option<Cursor>,
    },
    Blocks {
        from_byte: u64,
        to_byte: u64,
        cursor: Option<Cursor>,
    },
    Outline {
        cursor: Option<Cursor>,
    },
    Headings {
        from_byte: u64,
        to_byte: u64,
        cursor: Option<Cursor>,
    },
    Geometry {
        from_byte: u64,
        to_byte: u64,
        #[serde(default = "default_group")]
        group_size: u64,
        cursor: Option<Cursor>,
    },
    Resolve {
        namespace: String,
        key: String,
    },
    References {
        keys: Vec<String>,
    },
}
fn default_group() -> u64 {
    256
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    #[serde(flatten)]
    pub identity: Identity,
    pub state: String,
    pub indexed_bytes: u64,
    pub total_bytes: u64,
    pub semantic_complete: bool,
    pub failure: Option<String>,
    pub items: Vec<Value>,
    pub next: Option<Cursor>,
    /// 超长单项按 UTF-8 边界续取 JSON；没有静默裁剪标题或定义。
    pub fragment: Option<String>,
}

/// 所有文档共用一个 SQLite pager，容量按字节限制；不保留全量 Rust AST/正文。
/// 派生库可重建，关闭时删除；主库至多 255 MiB，预留等量回滚日志及页头，
/// 新旧版本和中间事务合计不超过 512 MiB。容量失败仍能回滚并保留已发布视图。
pub struct Store {
    connection: Option<Connection>,
    path: PathBuf,
    cancellation: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}
impl Store {
    pub fn cancellable(&self, cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>) -> TextDocumentResult<SqlCancellation<'_>> {
        *self.cancellation.borrow_mut() = Some(cancelled.clone());
        self.db().progress_handler(1000, Some(move || cancelled.load(std::sync::atomic::Ordering::Acquire))).map_err(sql)?;
        Ok(SqlCancellation(self))
    }
    fn check_cancelled(&self) -> TextDocumentResult<()> {
        if self.cancellation.borrow().as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(TextDocumentError::new("structure-cancelled", "结构构建已取消"));
        }
        Ok(())
    }
    pub fn toc_baseline(
        &self,
        document: &str,
        data: Option<&str>,
    ) -> TextDocumentResult<Option<String>> {
        if let Some(data) = data {
            self.db()
                .execute(
                    "INSERT OR REPLACE INTO toc_baselines VALUES(?1,?2)",
                    params![document, data],
                )
                .map_err(sql)?;
        }
        self.db()
            .query_row(
                "SELECT payload FROM toc_baselines WHERE document=?1",
                [document],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)
    }
    pub fn lazy(path: PathBuf) -> Self {
        Self {
            connection: None,
            path,
            cancellation: Default::default(),
        }
    }
    pub fn open(path: PathBuf) -> TextDocumentResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // 派生库使用进程唯一文件且从不跨进程恢复；保留回滚日志保障运行时事务，
        // 不为可重建结构页逐批执行断电持久化 fsync，用户正文仍由文档会话保存。
        let db = Connection::open(&path).map_err(sql)?;
        db.execute_batch("PRAGMA page_size=4096; PRAGMA cache_size=-24576; PRAGMA hard_heap_limit=33554432; PRAGMA cache_spill=1; PRAGMA mmap_size=0; PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY; PRAGMA auto_vacuum=INCREMENTAL;
            CREATE TABLE generations(generation INTEGER PRIMARY KEY, document TEXT NOT NULL, revision INTEGER NOT NULL, state TEXT NOT NULL, indexed INTEGER NOT NULL, total INTEGER NOT NULL, semantic INTEGER NOT NULL DEFAULT 1, failure TEXT);
            CREATE INDEX generations_document ON generations(document, generation);
            CREATE TABLE blocks(generation INTEGER NOT NULL, ordinal INTEGER NOT NULL, id TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(generation, ordinal));
            CREATE UNIQUE INDEX blocks_identity ON blocks(generation,id);
            CREATE INDEX blocks_source ON blocks(generation,start,end);
            CREATE TABLE headings(generation INTEGER NOT NULL, ordinal INTEGER NOT NULL, block_id TEXT NOT NULL, heading_index INTEGER NOT NULL, level INTEGER NOT NULL, title TEXT NOT NULL, slug TEXT NOT NULL, byte INTEGER NOT NULL, line INTEGER NOT NULL, PRIMARY KEY(generation,ordinal));
            CREATE UNIQUE INDEX headings_slug ON headings(generation,slug);
            CREATE TABLE suffixes(generation INTEGER NOT NULL, base TEXT NOT NULL, next INTEGER NOT NULL, PRIMARY KEY(generation,base));
            CREATE TABLE definitions(generation INTEGER NOT NULL, namespace TEXT NOT NULL, key TEXT NOT NULL, byte INTEGER NOT NULL, end INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(generation,namespace,key,byte));
            CREATE TABLE dependencies(generation INTEGER NOT NULL, namespace TEXT NOT NULL, key TEXT NOT NULL, block_id TEXT NOT NULL, byte INTEGER NOT NULL, end INTEGER NOT NULL);
            CREATE INDEX dependencies_key ON dependencies(generation,namespace,key);
            CREATE TABLE geometry(generation INTEGER NOT NULL, ordinal INTEGER NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL, layout INTEGER NOT NULL, enhancements TEXT NOT NULL, complete INTEGER NOT NULL, PRIMARY KEY(generation,ordinal));").map_err(sql)?;
        db.pragma_update(
            None,
            "max_page_count",
            (DISK_BYTES / 2 - 1024 * 1024) / 4096,
        )
        .map_err(sql)?;
        db.execute_batch(include_str!("mapping.sql")).map_err(sql)?;
        Ok(Self {
            connection: Some(db),
            path,
            cancellation: Default::default(),
        })
    }
    fn db(&self) -> &Connection {
        self.connection.as_ref().unwrap()
    }
    pub fn create(&mut self, identity: &Identity, total: u64) -> TextDocumentResult<()> {
        if self.connection.is_none() {
            let mut opened = Self::open(self.path.clone())?;
            self.connection = opened.connection.take();
        }
        self.db().execute("INSERT INTO generations(generation,document,revision,state,indexed,total) VALUES(?1,?2,?3,'building',0,?4)", params![identity.index_generation, identity.document_id, identity.revision, total]).map_err(sql)?;
        Ok(())
    }
    pub fn remove(&self, generation: u64) -> TextDocumentResult<()> {
        if self.connection.is_none() {
            return Ok(());
        }
        for table in ["reuse", "slug_reuse", "slug_dirty", "suffixes", "structure_changes", "generations"] {
            self.db()
                .execute(
                    &format!("DELETE FROM {table} WHERE generation=?1"),
                    [generation],
                )
                .map_err(sql)?;
        }
        self.db().execute("DELETE FROM heading_slugs WHERE generation NOT IN (SELECT generation FROM generations UNION SELECT source_generation FROM slug_reuse)", []).map_err(sql)?;
        for table in [
            "blocks",
            "headings",
            "definitions",
            "dependencies",
            "geometry",
        ] {
            self.db().execute(&format!("DELETE FROM {table} WHERE generation NOT IN (SELECT generation FROM generations UNION SELECT source_generation FROM reuse)"),[]).map_err(sql)?;
        }
        self.db()
            .execute_batch("PRAGMA incremental_vacuum(256)")
            .map_err(sql)?;
        Ok(())
    }
    pub fn fail(&self, generation: u64, reason: &str) -> TextDocumentResult<()> {
        self.db()
            .execute(
                "UPDATE generations SET state='failed',failure=?2 WHERE generation=?1",
                params![generation, reason],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub fn block_at(&self, identity: &Identity, byte: u64) -> TextDocumentResult<Option<Block>> {
        let row:Option<(String,i64,i64)>=self.db().query_row("SELECT payload,byte_delta,line_delta FROM projected_blocks WHERE generation=?1 AND start<=?2 ORDER BY start DESC LIMIT 1",params![identity.index_generation,byte],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
        row.map(|(payload, bytes, lines)| {
            Ok(project(
                serde_json::from_str(&payload).map_err(sql)?,
                identity.revision,
                bytes,
                lines,
            ))
        })
        .transpose()
    }
    /// 展平映射到实际存储片段；私有页可共享，但不形成修订链或复制整个后缀。
    pub fn reuse_range(
        &self,
        identity: &Identity,
        previous: &Identity,
        from: u64,
        to: u64,
        byte_delta: i64,
        line_delta: i64,
    ) -> TextDocumentResult<()> {
        self.db().execute("INSERT OR IGNORE INTO reuse SELECT ?1,m.source_generation,MAX(m.source_from,?3-m.byte_delta),MIN(m.source_to,?4-m.byte_delta),m.byte_delta+?5,m.line_delta+?6 FROM mappings m WHERE m.generation=?2 AND m.source_from+m.byte_delta<?4 AND m.source_to+m.byte_delta>?3 AND EXISTS(SELECT 1 FROM blocks b WHERE b.generation=m.source_generation AND b.start>=MAX(m.source_from,?3-m.byte_delta) AND b.end<=MIN(m.source_to,?4-m.byte_delta))",params![identity.index_generation,previous.index_generation,from,to,byte_delta,line_delta]).map_err(sql)?;
        self.db().execute("UPDATE generations SET semantic=semantic AND NOT EXISTS(SELECT 1 FROM projected_blocks WHERE generation=?1 AND json_extract(payload,'$.semanticComplete')=0) WHERE generation=?1",[identity.index_generation]).map_err(sql)?;
        Ok(())
    }
    /// 分批校准全局 slug；未完成校准前不会声明生成完成。
    pub fn calibrate(&self, identity: &Identity, from: u64) -> TextDocumentResult<Option<u64>> {
        let db = self.db();
        let generation = identity.index_generation;
        // 全量扫描已按文档顺序分配全局 slug。没有共享旧片段时，逐页发布这些事实即可；
        // 不再逐标题重复去重。章节关系由 heading_sections_current 按当前位置查询。
        let has_reuse: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM reuse WHERE generation=?1)",
            [generation], |r| r.get(0),
        ).map_err(sql)?;
        if !has_reuse {
            self.check_cancelled()?;
            let (count, last): (u64, Option<u64>) = db.query_row(
                "SELECT COUNT(*),MAX(byte) FROM (SELECT byte FROM headings WHERE generation=?1 AND byte>=?2 ORDER BY byte LIMIT 256)",
                params![generation, from], |r| Ok((r.get(0)?, r.get(1)?)),
            ).map_err(sql)?;
            db.execute(
                "INSERT INTO heading_slugs(generation,byte,slug,level) SELECT generation,byte,slug,level FROM headings WHERE generation=?1 AND byte>=?2 ORDER BY byte LIMIT 256",
                params![generation, from],
            ).map_err(sql)?;
            return Ok(if count == 256 { last.map(|byte| byte + 1) } else { None });
        }
        if from == 0 {
            db.execute("DELETE FROM suffixes WHERE generation=?1", [generation])
                .map_err(sql)?;
        }
        let tx = db.unchecked_transaction().map_err(sql)?;
        let mut statement=db.prepare("SELECT byte,title,level FROM projected_headings WHERE generation=?1 AND byte>=?2 ORDER BY byte LIMIT 256").map_err(sql)?;
        let rows = statement
            .query_map(params![generation, from], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u8>(2)?,
                ))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut ordinal: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM heading_slugs WHERE generation=?1",
                [generation],
                |r| r.get(0),
            )
            .map_err(sql)?;
        for (byte, title, level) in &rows {
            ordinal += 1;
            let filtered: String = title
                .to_lowercase()
                .chars()
                .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '-')
                .collect();
            let base = filtered.split_whitespace().collect::<Vec<_>>().join("-");
            let base = if base.is_empty() {
                format!("heading-{ordinal}")
            } else {
                base
            };
            let mut suffix: u64 = db
                .query_row(
                    "SELECT next FROM suffixes WHERE generation=?1 AND base=?2",
                    params![generation, base],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?
                .unwrap_or(2);
            let mut slug = base.clone();
            while db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM heading_slugs WHERE generation=?1 AND slug=?2)",
                    params![generation, slug],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(sql)?
            {
                self.check_cancelled()?;
                slug = format!("{base}-{suffix}");
                suffix += 1;
            }
            db.execute(
                "INSERT OR REPLACE INTO suffixes VALUES(?1,?2,?3)",
                params![generation, base, suffix],
            )
            .map_err(sql)?;
            db.execute(
                "UPDATE heading_slugs SET end_byte=?2 WHERE generation=?1 AND end_byte IS NULL AND level>=?3",
                params![generation,byte,level],
            )
            .map_err(sql)?;
            let parent:Option<u64>=db.query_row("SELECT byte FROM heading_slugs WHERE generation=?1 AND end_byte IS NULL AND level<?2 ORDER BY byte DESC LIMIT 1",params![generation,level],|r|r.get(0)).optional().map_err(sql)?;
            db.execute("INSERT INTO heading_slugs(generation,byte,slug,level,parent_byte) VALUES(?1,?2,?3,?4,?5)",params![generation,byte,slug,level,parent]).map_err(sql)?;
        }
        if rows.len() < 256 {
            db.execute("UPDATE heading_slugs SET end_byte=(SELECT total FROM generations WHERE generation=?1) WHERE generation=?1 AND end_byte IS NULL",[generation]).map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        Ok(if rows.len() == 256 {
            rows.last().map(|(byte, _, _)| byte + 1)
        } else {
            None
        })
    }
    pub fn publish(
        &self,
        identity: &Identity,
        blocks: &[Block],
        consumed: u64,
        complete: bool,
    ) -> TextDocumentResult<()> {
        let db = self.db();
        let generation = identity.index_generation;
        // 单片段事务的行数与源码上限绑定；调用方在片段之间释放数据库锁。
        let transaction = db.unchecked_transaction().map_err(sql)?;
        let mut ordinal: u64 = db
            .query_row(
                "SELECT COALESCE(MAX(ordinal)+1,0) FROM blocks WHERE generation=?1",
                [generation],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let mut heading_ordinal: u64 = db
            .query_row(
                "SELECT COALESCE(MAX(ordinal)+1,0) FROM headings WHERE generation=?1",
                [generation],
                |r| r.get(0),
            )
            .map_err(sql)?;
        // 每批复用热路径语句，避免对每个正文块重复编译相同 SQL。
        let mut find_block = db.prepare_cached("SELECT ordinal FROM blocks WHERE generation=?1 AND id=?2").map_err(sql)?;
        let mut write_block = db.prepare_cached("INSERT OR REPLACE INTO blocks VALUES(?1,?2,?3,?4,?5,?6)").map_err(sql)?;
        let mut write_geometry = db.prepare_cached("INSERT OR REPLACE INTO geometry VALUES(?1,?2,?3,?4,?5,?6,?7)").map_err(sql)?;
        for block in blocks {
            let existing: Option<u64> = find_block
                .query_row(
                    params![generation, block.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?;
            let index = existing.unwrap_or(ordinal);
            if existing.is_none() {
                ordinal += 1;
            }
            let payload = serde_json::to_string(block).map_err(sql)?;
            write_block.execute(
                params![
                    generation,
                    index,
                    block.id,
                    block.from_byte,
                    block.to_byte,
                    payload
                ],
            )
            .map_err(sql)?;
            write_geometry.execute(
                params![
                    generation,
                    index,
                    block.from_byte,
                    block.to_byte,
                    block.layout_units,
                    serde_json::to_string(&block.enhancements).map_err(sql)?,
                    block.metrics_complete
                ],
            )
            .map_err(sql)?;
            if existing.is_some() {
                continue;
            } // 只有末尾 raw 区域允许延长；没有可重复发布的语义事实。
            for (i, heading) in block.headings.iter().enumerate() {
                let filtered: String = heading
                    .title
                    .to_lowercase()
                    .chars()
                    .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '-')
                    .collect();
                let base = filtered.split_whitespace().collect::<Vec<_>>().join("-");
                let base = if base.is_empty() {
                    format!("heading-{}", heading_ordinal + 1)
                } else {
                    base
                };
                let mut suffix: u64 = db
                    .query_row(
                        "SELECT next FROM suffixes WHERE generation=?1 AND base=?2",
                        params![generation, base],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(sql)?
                    .unwrap_or(2);
                let mut slug = base.clone();
                while db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM headings WHERE generation=?1 AND slug=?2)",
                        params![generation, slug],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(sql)?
                {
                    self.check_cancelled()?;
                    slug = format!("{base}-{suffix}");
                    suffix += 1;
                }
                db.execute(
                    "INSERT OR REPLACE INTO suffixes VALUES(?1,?2,?3)",
                    params![generation, base, suffix],
                )
                .map_err(sql)?;
                db.execute(
                    "INSERT INTO headings VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        generation,
                        heading_ordinal,
                        block.id,
                        i,
                        heading.level,
                        heading.title,
                        slug,
                        heading.byte,
                        heading.line,
                        filtered.split_whitespace().collect::<Vec<_>>().join("-")
                    ],
                )
                .map_err(sql)?;
                heading_ordinal += 1;
            }
            for (key, definition) in &block.definitions {
                db.execute(
                    "INSERT OR IGNORE INTO definitions VALUES(?1,'link',?2,?3,?4,?5,?6)",
                    params![
                        generation,
                        key,
                        block.from_byte,
                        block.to_byte,
                        serde_json::to_string(definition).map_err(sql)?,
                        block.id
                    ],
                )
                .map_err(sql)?;
            }
            for key in &block.references {
                db.execute(
                    "INSERT INTO dependencies VALUES(?1,'link',?2,?3,?4,?5)",
                    params![generation, key, block.id, block.from_byte, block.to_byte],
                )
                .map_err(sql)?;
            }
            for note in &block.footnotes {
                if note.definition {
                    db.execute(
                        "INSERT OR IGNORE INTO definitions VALUES(?1,'footnote',?2,?3,?4,'{}',?5)",
                        params![
                            generation,
                            note.label,
                            note.from_byte,
                            note.to_byte,
                            block.id
                        ],
                    )
                    .map_err(sql)?;
                } else {
                    db.execute(
                        "INSERT INTO dependencies VALUES(?1,'footnote',?2,?3,?4,?5)",
                        params![
                            generation,
                            note.label,
                            block.id,
                            note.from_byte,
                            note.to_byte
                        ],
                    )
                    .map_err(sql)?;
                }
            }
        }
        db.execute("UPDATE generations SET indexed=?2,state=?3,semantic=semantic AND ?4 WHERE generation=?1", params![generation,consumed,if complete { "ready" } else { "building" },blocks.iter().all(|b| b.semantic_complete)]).map_err(sql)?;
        transaction.commit().map_err(sql)?;
        Ok(())
    }
    pub fn query(&self, identity: &Identity, query: Query) -> TextDocumentResult<Response> {
        if self.connection.is_none() {
            return Err(TextDocumentError::new(
                "structure-generation-expired",
                "结构索引尚未创建",
            ));
        }
        let db = self.db();
        let generation = identity.index_generation;
        let mut response = db.query_row("SELECT document,revision,state,indexed,total,semantic,failure FROM generations WHERE generation=?1", [generation], |r| {
            Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,Response { identity: identity.clone(),state:r.get(2)?,indexed_bytes:r.get(3)?,total_bytes:r.get(4)?,semantic_complete:r.get(5)?,failure:r.get(6)?,items:vec![],next:None,fragment:None }))
        }).optional().map_err(sql)?.ok_or_else(|| TextDocumentError::new("structure-generation-expired", "结构生成版本已释放"))?;
        if response.0 != identity.document_id
            || response.1 != identity.revision
            || identity.syntax_version != SYNTAX_VERSION
        {
            return Err(TextDocumentError::new(
                "structure-generation-conflict",
                "结构版本身份不匹配",
            ));
        }
        let response = &mut response.2;
        let cursor = match &query {
            Query::Blocks { cursor, .. }
            | Query::TocBlocks { cursor }
            | Query::Outline { cursor }
            | Query::Headings { cursor, .. }
            | Query::Geometry { cursor, .. } => cursor.clone(),
            _ => None,
        };
        if cursor.as_ref().is_some_and(|c| c.generation != generation) {
            return Err(TextDocumentError::new(
                "structure-cursor-expired",
                "分页游标属于另一生成版本",
            ));
        }
        let ordinal = cursor.as_ref().map(|c| c.ordinal).unwrap_or(0);
        let offset = cursor.as_ref().map(|c| c.offset).unwrap_or(0);
        match query {
            query @ (Query::OpenOutline { .. }
            | Query::OutlineView { .. }
            | Query::SetOutlineFold { .. }
            | Query::SetOutlineLevel { .. }
            | Query::CloseOutline { .. }) => self.outline_query(identity, query, response)?,
            Query::Progress => {}
            Query::HeadingAtByte { byte } => {
                if response.state == "ready" {
                    // 每个映射段只查索引中的最近一个候选，滚动时不遍历整篇标题。
                    let mut heading: Option<Value> = db.query_row(
                        "SELECT h.block_id,h.heading_index,h.byte+m.byte_delta,h.level FROM mappings m JOIN headings h ON h.generation=m.source_generation AND h.byte=(SELECT p.byte FROM headings p WHERE p.generation=m.source_generation AND p.byte>=m.source_from AND p.byte<m.source_to AND p.byte<=?2-m.byte_delta ORDER BY p.byte DESC LIMIT 1) WHERE m.generation=?1 ORDER BY h.byte+m.byte_delta DESC LIMIT 1",
                        params![generation, byte],
                        |r| Ok(json!({"blockId":r.get::<_,String>(0)?,"blockHeadingIndex":r.get::<_,u64>(1)?,"byte":r.get::<_,u64>(2)?,"level":r.get::<_,u8>(3)?})),
                    ).optional().map_err(sql)?;
                    if heading.is_none() {
                        heading = db.query_row("SELECT h.block_id,h.heading_index,h.byte+m.byte_delta,h.level FROM mappings m JOIN headings h ON h.generation=m.source_generation AND h.byte=(SELECT p.byte FROM headings p WHERE p.generation=m.source_generation AND p.byte>=m.source_from AND p.byte<m.source_to ORDER BY p.byte LIMIT 1) WHERE m.generation=?1 ORDER BY h.byte+m.byte_delta LIMIT 1",[generation],
                            |r| Ok(json!({"blockId":r.get::<_,String>(0)?,"blockHeadingIndex":r.get::<_,u64>(1)?,"byte":r.get::<_,u64>(2)?,"level":r.get::<_,u8>(3)?}))).optional().map_err(sql)?;
                    }
                    if let Some(heading) = heading { response.items.push(heading); }
                }
            }
            Query::LocateOutline { view_id, heading_id, byte, parent } => {
                self.outline_query(identity, Query::OutlineView {view_id: view_id.clone(),from_index:0,view_generation:None}, response)?;
                response.items.clear();
                let heading: Option<(u64,u8)> = db.query_row("SELECT byte,level FROM raw_headings WHERE generation=?1 AND block_id||':'||heading_index=?2 LIMIT 1",params![generation,heading_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
                let target = match heading {
                    Some((at,level)) if parent => db.query_row("SELECT MAX(byte) FROM raw_headings WHERE generation=?1 AND byte<?2 AND level<?3",params![generation,at,level],|r|r.get::<_,Option<u64>>(0)).map_err(sql)?,
                    Some((at,_)) => Some(at), None => None,
                };
                let target = target.unwrap_or(byte);
                let (scan,complete):(u64,bool)=db.query_row("SELECT scan_byte,complete FROM outline_views WHERE view_id=?1",[&view_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
                let position:Option<u64>= if scan>target || complete { db.query_row("SELECT position FROM outline_rows WHERE view_id=?1 AND byte<=?2 ORDER BY byte DESC LIMIT 1",params![view_id,target],|r|r.get(0)).optional().map_err(sql)? } else {None};
                response.items.push(json!({"position":position,"complete":complete || scan>target}));
            }
            Query::Changes { observed_generation } => {
                if response.state == "ready" { response.items.push(self.changes(generation, observed_generation)?); }
            }
            query @ (Query::Blocks { .. } | Query::TocBlocks { .. }) => {
                let (from_byte, to_byte, toc) = match query {
                    Query::Blocks {
                        from_byte, to_byte, ..
                    } => (from_byte, to_byte, false),
                    _ => (0, response.total_bytes, true),
                };
                let mut statement = db.prepare("SELECT start,payload,byte_delta,line_delta FROM projected_blocks WHERE generation=?1 AND start>=?2 AND end>?3 AND start<?4 AND (?5=0 OR json_extract(payload,'$.kind') IN ('toc_block','html_block')) ORDER BY start LIMIT 257").map_err(sql)?;
                let rows = statement
                    .query_map(params![generation, ordinal, from_byte, to_byte, toc], |r| {
                        Ok((
                            r.get::<_, u64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    })
                    .map_err(sql)?;
                let mut bytes = 2048;
                for row in rows {
                    let (index, payload, byte_delta, line_delta) = row.map_err(sql)?;
                    let payload = serde_json::to_string(&project(
                        serde_json::from_str(&payload).map_err(sql)?,
                        identity.revision,
                        byte_delta,
                        line_delta,
                    ))
                    .map_err(sql)?;
                    if response.items.len() == PAGE_ITEMS
                        || (!response.items.is_empty() && bytes + payload.len() > PAGE_BYTES - 2048)
                    {
                        response.next = Some(Cursor {
                            generation,
                            ordinal: index,
                            offset: 0,
                        });
                        break;
                    }
                    if payload.len() > PAGE_BYTES - 4096 || offset > 0 {
                        if !response.items.is_empty() {
                            response.next = Some(Cursor {
                                generation,
                                ordinal: index,
                                offset: 0,
                            });
                            break;
                        }
                        if offset >= payload.len() || !payload.is_char_boundary(offset) {
                            return Err(TextDocumentError::new(
                                "structure-cursor-invalid",
                                "结构续取偏移无效",
                            ));
                        }
                        // JSON string 转义最坏六倍，给响应身份与字段留足空间。
                        let mut end = (offset + (PAGE_BYTES - 4096) / 6).min(payload.len());
                        while !payload.is_char_boundary(end) {
                            end -= 1;
                        }
                        response.fragment = Some(payload[offset..end].to_owned());
                        response.next = Some(Cursor {
                            generation,
                            ordinal: if end == payload.len() {
                                index + 1
                            } else {
                                index
                            },
                            offset: if end == payload.len() { 0 } else { end },
                        });
                        break;
                    }
                    bytes += payload.len();
                    response
                        .items
                        .push(serde_json::from_str(&payload).map_err(sql)?);
                }
                if response.next.is_none()
                    && response.state == "building"
                    && response.indexed_bytes < to_byte
                {
                    let (last,raw): (u64,bool) = db.query_row("SELECT start,json_extract(payload,'$.mode')='raw' FROM projected_blocks WHERE generation=?1 ORDER BY start DESC LIMIT 1",[generation],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?.unwrap_or((0,false));
                    response.next = Some(Cursor {
                        generation,
                        ordinal: if raw {
                            last
                        } else if response.indexed_bytes == 0 {
                            0
                        } else {
                            last + 1
                        },
                        offset: 0,
                    });
                }
            }
            query @ (Query::Outline { .. } | Query::Headings { .. }) => {
                let (from, to) = match query {
                    Query::Headings {
                        from_byte, to_byte, ..
                    } => (from_byte, to_byte),
                    _ => (0, response.total_bytes),
                };
                // 全局 slug 校准结束前不提供可用于导航或写入的临时 slug。
                if response.state != "ready" {
                    return Ok(std::mem::replace(
                        response,
                        Response {
                            identity: identity.clone(),
                            state: String::new(),
                            indexed_bytes: 0,
                            total_bytes: 0,
                            semantic_complete: false,
                            failure: None,
                            items: vec![],
                            next: None,
                            fragment: None,
                        },
                    ));
                }
                let mut statement = db.prepare("SELECT byte,block_id,heading_index,level,title,slug,byte,line FROM projected_headings WHERE generation=?1 AND byte>=?2 AND byte<?3 ORDER BY byte LIMIT 257").map_err(sql)?;
                let rows = statement.query_map(params![generation,ordinal.max(from),to], |r| Ok((r.get::<_,u64>(0)?,json!({"blockId":r.get::<_,String>(1)?,"blockHeadingIndex":r.get::<_,u64>(2)?,"level":r.get::<_,u8>(3)?,"title":r.get::<_,String>(4)?,"id":r.get::<_,String>(5)?,"byte":r.get::<_,u64>(6)?,"line":r.get::<_,u64>(7)?})))).map_err(sql)?;
                let mut bytes = 2048;
                for row in rows {
                    let (index, item) = row.map_err(sql)?;
                    let size = serde_json::to_vec(&item).map_err(sql)?.len();
                    if response.items.len() == PAGE_ITEMS || bytes + size > PAGE_BYTES - 2048 {
                        response.next = Some(Cursor {
                            generation,
                            ordinal: index,
                            offset: 0,
                        });
                        break;
                    }
                    bytes += size;
                    response.items.push(item);
                }
            }
            Query::Geometry {
                from_byte,
                to_byte,
                group_size,
                ..
            } => {
                let size = group_size.clamp(1, 1 << 30);
                let mut statement=db.prepare("SELECT start,start,end,layout,enhancements,complete FROM projected_geometry WHERE generation=?1 AND start>=?2 AND end>?3 AND start<?4 ORDER BY start LIMIT ?5").map_err(sql)?;
                let rows = statement
                    .query_map(
                        params![
                            generation,
                            ordinal,
                            from_byte,
                            to_byte,
                            (size * PAGE_ITEMS as u64).min(65536)
                        ],
                        |r| {
                            Ok((
                                r.get::<_, u64>(0)?,
                                r.get::<_, u64>(1)?,
                                r.get::<_, u64>(2)?,
                                r.get::<_, u64>(3)?,
                                r.get::<_, String>(4)?,
                                r.get::<_, bool>(5)?,
                            ))
                        },
                    )
                    .map_err(sql)?;
                let mut group: Option<(u64, u64, u64, u64, u64, Enhancements, bool)> = None;
                for row in rows {
                    let (i, from, to, layout, enhancements, complete) = row.map_err(sql)?;
                    let current =
                        group.get_or_insert((i, from, to, 0, 0, Enhancements::default(), true));
                    current.2 = to;
                    current.3 += 1;
                    current.4 += layout;
                    current
                        .5
                        .add(&serde_json::from_str(&enhancements).map_err(sql)?);
                    current.6 &= complete;
                    response.next = Some(Cursor {
                        generation,
                        ordinal: i + 1,
                        offset: 0,
                    });
                    if current.3 == size {
                        let g = group.take().unwrap();
                        response.items.push(json!({"ordinal":g.0,"fromByte":g.1,"toByte":g.2,"blockCount":g.3,"layoutUnits":g.4,"enhancements":g.5,"metricsComplete":g.6}));
                    }
                }
                if let Some(g) = group {
                    response.items.push(json!({"ordinal":g.0,"fromByte":g.1,"toByte":g.2,"blockCount":g.3,"layoutUnits":g.4,"enhancements":g.5,"metricsComplete":g.6}));
                }
            }
            Query::Resolve { namespace, key } => {
                let target: Option<Value> = if response.state != "ready" {
                    None
                } else if namespace == "heading" || namespace == "slug" {
                    let predicate = if namespace == "heading" {
                        "h.block_id||':'||h.heading_index=?2"
                    } else {
                        "h.slug=?2"
                    };
                    let heading=db.query_row(&format!("SELECT h.byte,h.block_id,h.slug,h.level,h.line,h.heading_index FROM projected_headings h WHERE h.generation=?1 AND {predicate}"),params![generation,key],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u8>(3)?,r.get::<_,u64>(4)?,r.get::<_,u64>(5)?))).optional().map_err(sql)?;
                    if let Some((byte,block,slug,level,line,heading_index))=heading {
                        let end:u64=db.query_row("SELECT COALESCE(MIN(byte),?4) FROM raw_headings WHERE generation=?1 AND byte>?2 AND level<=?3",params![generation,byte,level,response.total_bytes],|r|r.get(0)).map_err(sql)?;
                        Some(json!({"fromByte":byte,"blockId":block,"slug":slug,"toByte":end,"line":line,"level":level,"blockHeadingIndex":heading_index}))
                    } else {None}
                } else if namespace == "footnoteReference" {
                    db.query_row("SELECT d.byte+b.byte_delta,d.end+b.byte_delta FROM projected_blocks b JOIN dependencies d ON d.generation=b.source_generation AND d.block_id=b.id WHERE b.generation=?1 AND d.namespace='footnote' AND d.key=?2 ORDER BY d.byte+b.byte_delta LIMIT 1",params![generation,key],|r|Ok(json!({"fromByte":r.get::<_,u64>(0)?,"toByte":r.get::<_,u64>(1)?}))).optional().map_err(sql)?
                } else {
                    if namespace != "link" && namespace != "footnote" {
                        return Err(TextDocumentError::new(
                            "structure-namespace",
                            "未知结构引用命名空间",
                        ));
                    }
                    let key = if namespace == "link" {
                        super::parser::normalize_label(&key)
                    } else {
                        key
                    };
                    db.query_row("SELECT byte,end,payload FROM projected_definitions WHERE generation=?1 AND namespace=?2 AND key=?3 ORDER BY byte LIMIT 1",params![generation,namespace,key],|r|Ok(json!({"fromByte":r.get::<_,u64>(0)?,"toByte":r.get::<_,u64>(1)?,"definition":r.get::<_,String>(2)?}))).optional().map_err(sql)?
                };
                response.items.push(json!({"status": if target.is_some(){"found"} else if response.state=="ready" && response.semantic_complete {"missing"} else {"incomplete"},"target":target}));
            }
            Query::References { keys } => {
                if keys.len() > PAGE_ITEMS {
                    return Err(TextDocumentError::new(
                        "structure-page-limit",
                        "引用查询超过 256 项",
                    ));
                }
                let mut bytes = 2048;
                for key in keys {
                    let key = super::parser::normalize_label(&key);
                    let value:Option<String>=db.query_row("SELECT payload FROM projected_definitions WHERE generation=?1 AND namespace='link' AND key=?2 ORDER BY byte LIMIT 1",params![generation,key],|r|r.get(0)).optional().map_err(sql)?;
                    if let Some(value) = value {
                        let item = json!({"key":key,"definition":serde_json::from_str::<Value>(&value).map_err(sql)?});
                        bytes += serde_json::to_vec(&item).map_err(sql)?.len();
                        if bytes > PAGE_BYTES - 2048 {
                            return Err(TextDocumentError::new(
                                "structure-page-limit",
                                "引用结果超过单页字节预算，请缩小查询范围",
                            ));
                        }
                        response.items.push(item);
                    }
                }
            }
        }
        Ok(std::mem::replace(
            response,
            Response {
                identity: identity.clone(),
                state: String::new(),
                indexed_bytes: 0,
                total_bytes: 0,
                semantic_complete: false,
                failure: None,
                items: vec![],
                next: None,
                fragment: None,
            },
        ))
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        if self.connection.is_none() {
            return;
        }
        drop(self.connection.take());
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::app_logger::warn("StructureIndex", &format!("清理派生索引失败：{error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{parser::parse_fragment, scanner::Scanner};
    use super::*;
    fn store() -> Store {
        let path = std::env::temp_dir().join(format!(
            "nomo-structure-store-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Store::open(path).unwrap()
    }
    #[test]
    fn sql_cancellation_interrupts_inside_statement_and_releases_callback() {
        use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
        let store=store();
        let cancelled=Arc::new(AtomicBool::new(false));
        let signal=cancelled.clone();
        let thread=std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            signal.store(true,Ordering::Release);
        });
        let started=std::time::Instant::now();
        {
            let _guard=store.cancellable(cancelled).unwrap();
            let result=store.db().query_row("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT SUM(x) FROM n",[],|r|r.get::<_,u64>(0));
            assert!(matches!(result,Err(rusqlite::Error::SqliteFailure(error,_)) if error.code==rusqlite::ErrorCode::OperationInterrupted));
            assert_eq!(store.check_cancelled().unwrap_err().code, "structure-cancelled");
        }
        thread.join().unwrap();
        store.check_cancelled().unwrap();
        assert!(started.elapsed()<std::time::Duration::from_secs(2));
        assert_eq!(store.db().query_row("SELECT 1",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    }
    #[test]
    #[ignore = "手动分解临时派生库构建耗时；NOMO_STRUCTURE_BENCH 指向固定 Markdown 样本"]
    fn profile_derived_store_build() {
        let source=std::fs::read(std::env::var("NOMO_STRUCTURE_BENCH").unwrap()).unwrap();
        for sync in ["NORMAL","OFF"] {
            let mut store=store();store.db().pragma_update(None,"synchronous",sync).unwrap();
            let identity=Identity::new("bench".into(),0,1);store.create(&identity,source.len() as u64).unwrap();
            let mut scanner=super::super::scanner::Scanner::new(0,0,1);
            let (mut parse,mut publish)=(std::time::Duration::ZERO,std::time::Duration::ZERO);
            let mut at=0;while at<source.len() {
                let mut end=(at+16384).min(source.len());while std::str::from_utf8(&source[at..end]).is_err(){end-=1;}
                let started=std::time::Instant::now();let batch=scanner.push(std::str::from_utf8(&source[at..end]).unwrap(),end==source.len()).unwrap();parse+=started.elapsed();
                let started=std::time::Instant::now();store.publish(&identity,&batch.blocks,batch.consumed_bytes,false).unwrap();publish+=started.elapsed();at=end;
            }
            eprintln!("derived-store sync={sync} bytes={} parse_ms={} publish_ms={}",source.len(),parse.as_millis(),publish.as_millis());
        }
    }
    #[test]
    fn incremental_slugs_converge_without_recalculating_unrelated_suffix() {
        let mut store=store();
        let tail=(0..1000).map(|i|format!("## unique {i}\n\n")).collect::<String>();
        let original=format!("# first\n\n{tail}");
        let old=Identity::new("doc".into(),0,1);
        store.create(&old,original.len() as u64).unwrap();
        store.publish(&old,&parse_fragment(&original,0,true).unwrap(),original.len() as u64,true).unwrap();
        let mut cursor=0; while let Some(next)=store.calibrate(&old,cursor).unwrap(){cursor=next;}
        let changed=format!("# renamed\n\n{tail}");
        let new=Identity::new("doc".into(),1,2);
        store.create(&new,changed.len() as u64).unwrap();
        store.publish(&new,&parse_fragment(&changed,1,true).unwrap(),changed.len() as u64,true).unwrap();
        let mut scan=SlugScan::new(old.clone(),0,9,11);
        while !store.calibrate_incremental(&new,&mut scan).unwrap() {}
        assert_eq!(scan.recalculated,1);
        let opened=store.query(&new,Query::OpenOutline{default_level:6}).unwrap();
        let view=opened.items[0]["viewId"].as_str().unwrap().to_owned();
        store.publish(&new,&[],changed.len() as u64,true).unwrap();
        let page=store.query(&new,Query::OutlineView{view_id:view,from_index:0,view_generation:None}).unwrap();
        let rows=page.items[0]["rows"].as_array().unwrap();
        assert_eq!(rows.len(),256);
        assert_eq!(rows[0]["hasChildren"],true);
        assert_eq!(rows[1]["hasChildren"],false);
        assert_eq!(rows[1]["parentByte"],0);
        store.remove(old.index_generation).unwrap();
        let target=store.query(&new,Query::Resolve{namespace:"slug".into(),key:"unique-999".into()}).unwrap();
        assert_eq!(target.items[0]["status"],"found");
        assert_eq!(target.items[0]["target"]["toByte"],changed.len() as u64);
        store.remove(new.index_generation).unwrap();
        assert_eq!(store.db().query_row("SELECT COUNT(*) FROM heading_slugs",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    }
    #[test]
    fn incremental_slug_collisions_empty_ordinals_and_sections_match_rebuild() {
      for prefix in ["", "# prefix\n\n## a\n\n", "# a\n\n"] {
       for middle in ["# a\n\n", "body\n\n"] {
        for replacement in ["# b\n\n","", "# a\n\n# a\n\n", "## a-2\n\n", "# !!!\n\n", "# heading-4\n\n"] {
            let mut store=store();
            let tail="# a\n\n## a-2\n\n### !!!\n\n# heading-4\n\n## a\n\n# a-2-2\n\n";
            let original=format!("{prefix}{middle}{tail}");
            let changed=format!("{prefix}{replacement}{tail}");
            let old=Identity::new("doc".into(),0,1);
            let new=Identity::new("doc".into(),1,2);
            let rebuilt=Identity::new("rebuilt".into(),0,3);
            for (identity,text) in [(&old,&original),(&new,&changed),(&rebuilt,&changed)] {
                store.create(identity,text.len() as u64).unwrap();
                store.publish(identity,&parse_fragment(text,identity.revision,true).unwrap(),text.len() as u64,true).unwrap();
                if identity!=&new { let mut cursor=0;while let Some(next)=store.calibrate(identity,cursor).unwrap(){cursor=next;} }
            }
            let mut scan=SlugScan::new(old,prefix.len() as u64,(prefix.len()+middle.len()) as u64,(prefix.len()+replacement.len()) as u64);
            while !store.calibrate_incremental(&new,&mut scan).unwrap() {}
            let facts=|id:u64| {
                let mut statement=store.db().prepare("SELECT byte,slug,level,end_byte,parent_byte FROM heading_sections_current WHERE generation=?1 ORDER BY byte").unwrap();
                statement.query_map([id],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,u8>(2)?,r.get::<_,u64>(3)?,r.get::<_,Option<u64>>(4)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
            };
            assert_eq!(facts(new.index_generation),facts(rebuilt.index_generation),"replacement={replacement:?}");
        }
       }
      }
    }
    #[test]
    fn pages_are_bounded_versioned_and_preserve_all_source_ranges() {
        let mut store = store();
        let identity = Identity::new("doc".into(), 4, 7);
        let text = "# repeated\n\n".repeat(600);
        store.create(&identity, text.len() as u64).unwrap();
        let blocks = parse_fragment(&text, 4, true).unwrap();
        store
            .publish(&identity, &blocks, text.len() as u64, true)
            .unwrap();
        let mut from = 0;
        while let Some(next) = store.calibrate(&identity, from).unwrap() {
            from = next;
        }
        let mut cursor = None;
        let mut count = 0;
        let mut byte = 0;
        loop {
            let page = store
                .query(
                    &identity,
                    Query::Blocks {
                        from_byte: 0,
                        to_byte: text.len() as u64,
                        cursor,
                    },
                )
                .unwrap();
            assert!(page.items.len() <= PAGE_ITEMS);
            assert!(serde_json::to_vec(&page).unwrap().len() <= PAGE_BYTES);
            for block in page.items {
                assert_eq!(block["fromByte"], byte);
                byte = block["toByte"].as_u64().unwrap();
                count += 1;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(count, 600);
        assert_eq!(byte, text.len() as u64);
        assert!(store
            .query(
                &identity,
                Query::Outline {
                    cursor: Some(Cursor {
                        generation: 8,
                        ordinal: 0,
                        offset: 0
                    })
                }
            )
            .is_err());
        let target = store
            .query(
                &identity,
                Query::Resolve {
                    namespace: "slug".into(),
                    key: "repeated-600".into(),
                },
            )
            .unwrap();
        assert_eq!(target.items[0]["status"], "found");
        let second = store
            .query(
                &identity,
                Query::Resolve {
                    namespace: "slug".into(),
                    key: "repeated-2".into(),
                },
            )
            .unwrap();
        assert_eq!(second.items[0]["target"]["fromByte"], blocks[1].from_byte);
        let stable = store
            .query(
                &identity,
                Query::Resolve {
                    namespace: "heading".into(),
                    key: format!("{}:0", blocks[1].id),
                },
            )
            .unwrap();
        assert_eq!(stable.items[0], second.items[0]);
        let range = store
            .query(
                &identity,
                Query::Headings {
                    from_byte: blocks[1].from_byte,
                    to_byte: blocks[2].from_byte,
                    cursor: None,
                },
            )
            .unwrap();
        assert_eq!(range.items.len(), 1);
        assert_eq!(range.items[0]["id"], "repeated-2");
        store.remove(7).unwrap();
        assert!(store.query(&identity, Query::Progress).is_err());
    }
    #[test]
    fn oversized_single_item_is_losslessly_continued() {
        let mut store = store();
        let identity = Identity::new("doc".into(), 1, 1);
        let source = "- # x\n".repeat(5000);
        let blocks = parse_fragment(&source, 1, true).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(serde_json::to_vec(&blocks[0]).unwrap().len() > PAGE_BYTES);
        store.create(&identity, source.len() as u64).unwrap();
        store
            .publish(&identity, &blocks, source.len() as u64, true)
            .unwrap();
        let mut cursor = None;
        let mut joined = String::new();
        loop {
            let response = store
                .query(
                    &identity,
                    Query::Blocks {
                        from_byte: 0,
                        to_byte: source.len() as u64,
                        cursor,
                    },
                )
                .unwrap();
            assert!(serde_json::to_vec(&response).unwrap().len() <= PAGE_BYTES);
            if let Some(fragment) = response.fragment {
                joined.push_str(&fragment);
            } else {
                break;
            }
            cursor = response.next;
            if cursor.as_ref().is_some_and(|cursor| cursor.offset == 0) {
                break;
            }
        }
        let decoded: Block = serde_json::from_str(&joined).unwrap();
        assert_eq!(decoded, blocks[0]);
    }
    #[test]
    fn capacity_failure_rolls_back_build_pages_and_keeps_published_generation() {
        let mut store = store();
        let published = Identity::new("doc".into(), 0, 1);
        store.create(&published, 5).unwrap();
        let original = parse_fragment("# A\n\n", 0, true).unwrap();
        store.publish(&published, &original, 5, true).unwrap();
        store.calibrate(&published, 0).unwrap();
        let building = Identity::new("doc".into(), 1, 2);
        let source = "- # title\n".repeat(3000);
        store.create(&building, source.len() as u64).unwrap();
        let pages: u64 = store
            .db()
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        store
            .db()
            .pragma_update(None, "max_page_count", pages + 1)
            .unwrap();
        let blocks = parse_fragment(&source, 1, true).unwrap();
        assert!(store
            .publish(&building, &blocks, source.len() as u64, true)
            .is_err());
        let old = store
            .query(&published, Query::Outline { cursor: None })
            .unwrap();
        assert_eq!(old.state, "ready");
        assert_eq!(old.items[0]["title"], "A");
        let failed = store
            .query(
                &building,
                Query::Blocks {
                    from_byte: 0,
                    to_byte: source.len() as u64,
                    cursor: None,
                },
            )
            .unwrap();
        assert!(failed.items.is_empty());
        assert_eq!(failed.indexed_bytes, 0);
    }
    #[test]
    fn unresolved_targets_are_not_missing_and_duplicate_definitions_use_document_order() {
        let mut store = store();
        let identity = Identity::new("doc".into(), 1, 1);
        let source = "[id]: /first\n\n# A\n\n[id]: /second\n\n# B\n";
        store.create(&identity, source.len() as u64).unwrap();
        assert_eq!(
            store
                .query(
                    &identity,
                    Query::Resolve {
                        namespace: "link".into(),
                        key: "ID".into()
                    }
                )
                .unwrap()
                .items[0]["status"],
            "incomplete"
        );
        let mut scanner = Scanner::new(1, 0, 1);
        let batch = scanner.push(source, true).unwrap();
        store
            .publish(&identity, &batch.blocks, batch.consumed_bytes, true)
            .unwrap();
        let refs = store
            .query(
                &identity,
                Query::References {
                    keys: vec!["id".into()],
                },
            )
            .unwrap();
        assert_eq!(refs.items[0]["definition"]["href"], "/first");
        assert_eq!(
            store
                .query(
                    &identity,
                    Query::Resolve {
                        namespace: "footnote".into(),
                        key: "absent".into()
                    }
                )
                .unwrap()
                .items[0]["status"],
            "missing"
        );
    }
}

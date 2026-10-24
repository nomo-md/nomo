-- 原生块页不可变；新修订仅保存编辑片段及分段偏移，不重写未变后缀。
CREATE TABLE reuse(generation INTEGER NOT NULL, source_generation INTEGER NOT NULL,
  source_from INTEGER NOT NULL, source_to INTEGER NOT NULL, byte_delta INTEGER NOT NULL,
  line_delta INTEGER NOT NULL,
  PRIMARY KEY(generation,source_generation,source_from,source_to,byte_delta,line_delta));
CREATE INDEX reuse_source ON reuse(source_generation);
CREATE TABLE toc_baselines(document TEXT PRIMARY KEY, payload TEXT NOT NULL);
CREATE TABLE structure_changes(generation INTEGER PRIMARY KEY,payload TEXT NOT NULL);
CREATE VIEW mappings AS
  SELECT generation,generation AS source_generation,0 AS source_from,total AS source_to,
    0 AS byte_delta,0 AS line_delta FROM generations
  UNION ALL SELECT * FROM reuse;
CREATE VIEW projected_blocks AS
  SELECT m.generation,b.ordinal AS source_ordinal,b.generation AS source_generation,b.id,
    b.start+m.byte_delta AS start,b.end+m.byte_delta AS end,b.payload,m.byte_delta,m.line_delta
  FROM mappings m JOIN blocks b ON b.generation=m.source_generation
    AND b.start>=m.source_from AND b.end<=m.source_to;
ALTER TABLE definitions ADD COLUMN block_id TEXT NOT NULL DEFAULT '';
CREATE TABLE heading_slugs(generation INTEGER NOT NULL,byte INTEGER NOT NULL,slug TEXT NOT NULL,
  PRIMARY KEY(generation,byte));
CREATE UNIQUE INDEX heading_slugs_unique ON heading_slugs(generation,slug);
ALTER TABLE heading_slugs ADD COLUMN level INTEGER NOT NULL DEFAULT 1;
ALTER TABLE heading_slugs ADD COLUMN end_byte INTEGER;
ALTER TABLE heading_slugs ADD COLUMN parent_byte INTEGER;
CREATE INDEX heading_sections ON heading_slugs(generation,end_byte,level);
CREATE INDEX heading_parents ON heading_slugs(generation,parent_byte);
ALTER TABLE headings ADD COLUMN base TEXT NOT NULL DEFAULT '';
CREATE INDEX headings_block ON headings(generation,block_id,heading_index);
CREATE INDEX headings_byte ON headings(generation,byte,level);
CREATE INDEX headings_base ON headings(generation,base,byte);
CREATE TABLE slug_reuse(generation INTEGER NOT NULL,source_generation INTEGER NOT NULL,
 source_from INTEGER NOT NULL,source_to INTEGER NOT NULL,byte_delta INTEGER NOT NULL,
 PRIMARY KEY(generation,source_generation,source_from,source_to));
CREATE TABLE slug_dirty(generation INTEGER NOT NULL,base TEXT NOT NULL,PRIMARY KEY(generation,base));
CREATE VIEW slug_mappings AS
 SELECT generation,generation AS source_generation,0 AS source_from,total+1 AS source_to,0 AS byte_delta FROM generations
 UNION ALL SELECT * FROM slug_reuse;
CREATE VIEW projected_slugs AS
 SELECT m.generation,s.byte+m.byte_delta AS byte,s.slug FROM slug_mappings m JOIN heading_slugs s
 ON s.generation=m.source_generation AND s.byte>=m.source_from AND s.byte<m.source_to;
CREATE TABLE outline_views(view_id TEXT PRIMARY KEY,document TEXT NOT NULL,default_level INTEGER NOT NULL,
  generation INTEGER NOT NULL,view_generation INTEGER NOT NULL,scan_byte INTEGER NOT NULL DEFAULT 0,
  visible_count INTEGER NOT NULL DEFAULT 0,complete INTEGER NOT NULL DEFAULT 0);
CREATE TABLE outline_folds(view_id TEXT NOT NULL,heading_id TEXT NOT NULL,collapsed INTEGER NOT NULL,
  PRIMARY KEY(view_id,heading_id));
CREATE TABLE outline_rows(view_id TEXT NOT NULL,position INTEGER NOT NULL,byte INTEGER NOT NULL,
  PRIMARY KEY(view_id,position));
CREATE INDEX outline_row_bytes ON outline_rows(view_id,byte);
CREATE VIEW raw_headings AS
  SELECT m.generation,h.ordinal AS source_ordinal,h.block_id,h.heading_index,h.level,h.title,
    h.slug,h.byte+m.byte_delta AS byte,h.line+m.line_delta AS line,h.base
  FROM mappings m JOIN headings h ON h.generation=m.source_generation
    AND h.byte>=m.source_from AND h.byte<m.source_to;
CREATE VIEW projected_headings AS
 SELECT h.generation,h.source_ordinal,h.block_id,h.heading_index,h.level,h.title,
 COALESCE(s.slug,h.slug) AS slug,h.byte,h.line FROM raw_headings h
 LEFT JOIN projected_slugs s ON s.generation=h.generation AND s.byte=h.byte;
-- 层级关系按当前映射解析，不复制旧字节位置；标题重命名/增删可独立复用 slug 页。
CREATE VIEW heading_sections_current AS
 SELECT h.generation,h.byte,s.slug,h.level,
 COALESCE((SELECT MIN(n.byte) FROM raw_headings n WHERE n.generation=h.generation AND n.byte>h.byte AND n.level<=h.level),g.total) AS end_byte,
 (SELECT MAX(p.byte) FROM raw_headings p WHERE p.generation=h.generation AND p.byte<h.byte AND p.level<h.level) AS parent_byte
 FROM raw_headings h JOIN projected_slugs s ON s.generation=h.generation AND s.byte=h.byte
 JOIN generations g ON g.generation=h.generation;
CREATE VIEW projected_definitions AS
  SELECT b.generation,d.namespace,d.key,d.byte+b.byte_delta AS byte,d.end+b.byte_delta AS end,d.payload
  FROM projected_blocks b JOIN definitions d ON d.generation=b.source_generation AND d.block_id=b.id;
CREATE VIEW projected_geometry AS
  SELECT b.generation,b.start,b.end,g.layout,g.enhancements,g.complete
  FROM projected_blocks b JOIN geometry g ON g.generation=b.source_generation AND g.ordinal=b.source_ordinal;

//! 导出期间独占的字体与字形数据；TTC 文件按路径共用，任务结束即释放。
use super::{Appearance, Job, model::Style};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use resvg::usvg::fontdb::{self, Family, Query, Source, Weight};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct Face { pub font: Font, bytes: Arc<Vec<u8>>, index: u32 }
pub(super) struct Fonts {
    database: Arc<fontdb::Database>, pub faces: Vec<Face>, ids: HashMap<fontdb::ID, usize>,
    files: HashMap<PathBuf, Arc<Vec<u8>>>, choices: HashMap<(u8, String), usize>, bytes: usize,
    body: String, mono: String,
}
impl Fonts {
    pub fn new(appearance: &Appearance) -> Self {
        Self { database: crate::embeds::font_database(), faces: Vec::new(), ids: HashMap::new(), files: HashMap::new(), choices: HashMap::new(), bytes: 0,
            body: appearance.font.clone(), mono: appearance.mono.clone() }
    }
    pub fn segments(&mut self, text: &str, style: &Style, job: &Job) -> Result<Vec<(usize, String)>, String> {
        let mut parts: Vec<(usize, String)> = Vec::new();
        for grapheme in text.graphemes(true) {
            job.check()?;
            let id = self.select(grapheme, style, job)?;
            if let Some(last) = parts.last_mut().filter(|last| last.0 == id) { last.1.push_str(grapheme); }
            else { parts.push((id, grapheme.to_owned())); }
        }
        Ok(parts)
    }
    fn select(&mut self, text: &str, style: &Style, job: &Job) -> Result<usize, String> {
        let key = (u8::from(style.bold) | u8::from(style.italic) << 1 | u8::from(style.mono) << 2, text.to_owned());
        if let Some(id) = self.choices.get(&key) { return Ok(*id); }
        let requested = if style.mono { &self.mono } else { &self.body };
        let names = [requested.as_str(), "Microsoft YaHei", "SimSun", "Noto Sans CJK SC", "PingFang SC", "Noto Sans", "DejaVu Sans", "Segoe UI Symbol", "Segoe UI Emoji"];
        let mut candidates = Vec::new();
        for name in names {
            if let Some(id) = self.database.query(&Query { families: &[Family::Name(name)], weight: if style.bold { Weight::BOLD } else { Weight::NORMAL },
                style: if style.italic { fontdb::Style::Italic } else { fontdb::Style::Normal }, ..Default::default() }) { if !candidates.contains(&id) { candidates.push(id); } }
        }
        // 显式首选字体失败后才扫描字体库；每个字素的结果仅在本任务缓存。
        candidates.extend(self.database.faces().map(|face| face.id).filter(|id| !candidates.contains(id)).collect::<Vec<_>>());
        for id in candidates {
            job.check()?;
            let supported = self.database.with_face_data(id, |bytes, index| {
                ttf_parser::Face::parse(bytes, index).is_ok_and(|face| {
                    face.permissions().is_some_and(|p| p != ttf_parser::Permissions::Restricted)
                        && face.is_subsetting_allowed() && face.is_outline_embedding_allowed()
                        && text.chars().all(|ch| ignorable(ch) || face.glyph_index(ch).is_some_and(|id| id.0 != 0))
                })
            }).unwrap_or(false);
            if !supported { continue; }
            if let Some(index) = self.load(id)? {
                if self.choices.len() < 16_384 { self.choices.insert(key, index); }
                return Ok(index);
            }
        }
        let codepoints = text.chars().take(6).map(|ch| format!("U+{:04X}", ch as u32)).collect::<Vec<_>>().join(" ");
        Err(format!("没有可嵌入并覆盖 {codepoints} 的字体；请安装允许嵌入的对应字体后重试。"))
    }
    fn load(&mut self, id: fontdb::ID) -> Result<Option<usize>, String> {
        if let Some(index) = self.ids.get(&id) { return Ok(Some(*index)); }
        if self.faces.len() >= 24 { return Err("PDF 字体超过 24 个字面，已有输出文件未改动。".into()); }
        let (source, face_index) = self.database.face_source(id).ok_or("字体已不可用")?;
        let bytes = match source {
            Source::File(path) | Source::SharedFile(path, _) => {
                if let Some(bytes) = self.files.get(&path) { bytes.clone() }
                else {
                    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                    if size > 96 * 1024 * 1024 || self.bytes as u64 + size > 128 * 1024 * 1024 { return Err("PDF 字体数据超过 128 MiB 预算。".into()); }
                    use std::io::Read;
                    let mut bytes = Vec::new();
                    std::fs::File::open(&path).map_err(|e| e.to_string())?.take(96 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    if bytes.len() > 96 * 1024 * 1024 || self.bytes + bytes.len() > 128 * 1024 * 1024 { return Err("PDF 字体数据超过预算。".into()); }
                    self.bytes += bytes.len(); let bytes = Arc::new(bytes); self.files.insert(path, bytes.clone()); bytes
                }
            },
            Source::Binary(bytes) => {
                let data = bytes.as_ref().as_ref();
                if self.bytes + data.len() > 128 * 1024 * 1024 { return Err("PDF 字体数据超过预算。".into()); }
                self.bytes += data.len(); Arc::new(data.to_vec())
            },
        };
        let Some(font) = Font::new(bytes.clone().into(), face_index) else { return Ok(None); };
        let index = self.faces.len(); self.faces.push(Face { font, bytes, index: face_index }); self.ids.insert(id, index); Ok(Some(index))
    }
    pub fn shape(&self, index: usize, text: &str, size: f32, rtl: bool) -> Result<(Vec<KrillaGlyph>, f32), String> {
        let face = &self.faces[index];
        let font = rustybuzz::Face::from_slice(&face.bytes, face.index).ok_or("PDF 字体整形失败")?;
        let em = font.units_per_em() as f32;
        let mut buffer = rustybuzz::UnicodeBuffer::new(); buffer.push_str(text); buffer.guess_segment_properties();
        buffer.set_direction(if rtl { rustybuzz::Direction::RightToLeft } else { rustybuzz::Direction::LeftToRight });
        let output = rustybuzz::shape(&font, &[], buffer);
        let infos = output.glyph_infos();
        let mut clusters = infos.iter().map(|i| i.cluster as usize).collect::<Vec<_>>(); clusters.push(text.len()); clusters.sort_unstable(); clusters.dedup();
        let mut glyphs = Vec::with_capacity(infos.len()); let mut width = 0.0;
        for (info, pos) in infos.iter().zip(output.glyph_positions()) {
            if info.glyph_id == 0 { return Err("PDF 整形产生缺失字形，已取消导出。".into()); }
            let start = info.cluster as usize;
            let end = clusters.get(clusters.partition_point(|c| *c <= start)).copied().unwrap_or(text.len());
            glyphs.push(KrillaGlyph::new(GlyphId::new(info.glyph_id), pos.x_advance as f32 / em, pos.x_offset as f32 / em,
                pos.y_offset as f32 / em, pos.y_advance as f32 / em, start..end, None));
            width += pos.x_advance as f32 / em * size;
        }
        Ok((glyphs, width.abs()))
    }
}
fn ignorable(ch: char) -> bool { ch.is_control() || matches!(ch, '\u{200c}' | '\u{200d}' | '\u{fe0e}' | '\u{fe0f}') }

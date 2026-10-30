//! 数学字体缺字时借用系统字体的真实度量和轮廓；只在本次公式排版期间保存结果。
use latex_rust::{Dim, Error, FallbackGlyph, TextStyle};
use resvg::usvg::fontdb::{Database, Family, Query, Style, Weight};
use std::{cell::{OnceCell, RefCell}, collections::HashMap, fmt::Write, sync::Arc};

const MAX_GLYPHS: usize = 1024;
const MAX_OUTLINE_BYTES: usize = 2 * 1024 * 1024;
const MAX_GLYPH_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct MathFallback {
    database: OnceCell<Arc<Database>>,
    cache: RefCell<HashMap<(char, u8), Option<FallbackGlyph>>>,
    bytes: std::cell::Cell<usize>,
    emitted_bytes: std::cell::Cell<usize>,
}

impl MathFallback {
    pub fn glyph(&self, ch: char, style: TextStyle) -> Result<Option<FallbackGlyph>, Error> {
        let key = (ch, style as u8);
        if let Some(cached) = self.cache.borrow().get(&key) { self.account_output(cached.as_ref())?; return Ok(cached.clone()); }
        if self.cache.borrow().len() >= MAX_GLYPHS { return Err(limit()); }
        let database = self.database.get_or_init(super::font_database);
        let weight = if matches!(style, TextStyle::Bf | TextStyle::Boldsymbol | TextStyle::Pmb) { Weight::BOLD } else { Weight::NORMAL };
        let font_style = if style == TextStyle::It { Style::Italic } else { Style::Normal };
        let family = match style { TextStyle::Sf => Family::SansSerif, TextStyle::Tt => Family::Monospace, _ => Family::Serif };
        let names = [family, Family::Name("SimSun"), Family::Name("Songti SC"), Family::Name("Noto Serif CJK SC"),
            Family::Name("Microsoft YaHei"), Family::Name("Hiragino Sans GB"), Family::Name("Noto Sans CJK SC"), Family::SansSerif];
        let mut candidates = Vec::new();
        for family in names {
            if let Some(id) = database.query(&Query { families: &[family], weight, style: font_style, ..Default::default() }) {
                if !candidates.contains(&id) { candidates.push(id); }
            }
        }
        // 所选平台字体缺字时有限地探测其它系统字体；不读取文档提供的字体路径。
        for face in database.faces().filter(|face| face.style == font_style && face.weight == weight).take(64) {
            if !candidates.contains(&face.id) { candidates.push(face.id); }
        }
        // 中文宋体常只有常规字重；优先已有粗体／斜体字形，再接受字体族的近似匹配。
        candidates.sort_by_key(|id| database.face(*id).map_or((true, true), |face| (face.weight != weight, face.style != font_style)));
        let mut result = None;
        for id in candidates {
            if let Some(glyph) = database.with_face_data(id, |data, index| outline(data, index, ch)).flatten() {
                let bytes = glyph.outline.len() + std::mem::size_of::<FallbackGlyph>() + 64;
                if self.bytes.get().saturating_add(bytes) > MAX_OUTLINE_BYTES { return Err(limit()); }
                self.bytes.set(self.bytes.get() + bytes);
                result = Some(glyph);
                break;
            }
        }
        self.cache.borrow_mut().insert(key, result.clone());
        self.account_output(result.as_ref())?;
        Ok(result)
    }

    fn account_output(&self, glyph: Option<&FallbackGlyph>) -> Result<(), Error> {
        let bytes = self.emitted_bytes.get().saturating_add(glyph.map_or(0, |glyph| glyph.outline.len() + 128));
        if bytes > 4 * 1024 * 1024 { return Err(limit()); }
        self.emitted_bytes.set(bytes);
        Ok(())
    }
}

fn limit() -> Error { Error::Unsupported { what: "公式字体回退超过本次排版预算".into() } }

fn outline(data: &[u8], index: u32, ch: char) -> Option<FallbackGlyph> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let glyph = face.glyph_index(ch)?;
    if glyph.0 == 0 { return None; }
    let units = face.units_per_em();
    let advance = face.glyph_hor_advance(glyph)?;
    let mut path = Outline { text: String::new(), valid: true };
    let bounds = face.outline_glyph(glyph, &mut path).or_else(|| ch.is_whitespace().then_some(ttf_parser::Rect { x_min: 0, x_max: 0, y_min: 0, y_max: 0 }))?;
    if !path.valid || (path.text.is_empty() && !ch.is_whitespace()) { return None; }
    let em = i64::from(units);
    Some(FallbackGlyph {
        advance: Dim::ratio(i64::from(advance), em),
        height: Dim::ratio(i64::from(bounds.y_max.max(0)), em),
        depth: Dim::ratio(i64::from((-i32::from(bounds.y_min)).max(0)), em),
        units_per_em: units, outline: path.text.into(),
    })
}

struct Outline { text: String, valid: bool }
impl Outline {
    fn segment(&mut self, command: char, points: &[f32]) {
        if !self.valid { return; }
        if points.iter().any(|v| !v.is_finite()) || self.text.len() > MAX_GLYPH_BYTES.saturating_sub(256) {
            self.valid = false; return;
        }
        self.text.push(command);
        for coordinate in points { let _ = write!(self.text, "{coordinate:.3} "); }
    }
}
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) { self.segment('M', &[x,y]); }
    fn line_to(&mut self, x: f32, y: f32) { self.segment('L', &[x,y]); }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) { self.segment('Q', &[x1,y1,x,y]); }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) { self.segment('C', &[x1,y1,x2,y2,x,y]); }
    fn close(&mut self) { self.segment('Z', &[]); }
}

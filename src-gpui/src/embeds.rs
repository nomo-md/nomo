//! 公式与 Mermaid 图表的后台渲染和有界缓存（GPUI 全局状态）。
//!
//! 渲染链路全部为 Rust：公式由 latex-rust 按 TeX 规则排版成 SVG（字形已转为路径），
//! Mermaid 由 merman 解析、布局并输出 resvg 兼容的 SVG；两者再用 resvg 按 2 倍分辨率栅格化，
//! 交给 GPUI 作为位图绘制。
//!
//! 生命周期：布局阶段查询状态，缺失的条目登记后由 `request` 统一在后台线程渲染，完成后递增
//! `generation` 并刷新窗口；含公式或图表的块把 generation 纳入布局缓存键，因而在结果就绪后重排。
//! 位图淘汰保留尺寸，只有可见绘制才恢复位图；布局查询不能反复请求屏外资源。

use gpui::{App, Global, RenderImage};
use image::{Frame, RgbaImage};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[path = "embeds_native.rs"]
mod native;
pub use native::MermaidPalette;
pub(crate) use native::{math_svg, thumbnail_mermaid, export_mermaid, font_database, mermaid_font_resolver};
use native::mermaid_svg_with_palette;
#[cfg(test)] use native::{mermaid_svg, MERMAID_FONT_FAMILY};

/// 位图缓存上限（BGRA 字节）。800 个块级公式在 2 倍分辨率下约占 60–90 MiB，留出余量。
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_SPEC_BYTES: usize = 8 * 1024 * 1024;
// 只在绘制后尝试恢复，不设置定时器；保护刚刚画过的位图，避免同屏资源互相驱逐。
const VISIBLE_GRACE: Duration = Duration::from_secs(1);
const BUDGET_MESSAGE: &str = "图表／公式超过缓存预算；原文已保留";
/// 单张位图的边长上限（设备像素），防止超大图表占用过多内存；超出时降低栅格倍率。
const MAX_RASTER_SIDE: f32 = 4096.0;
/// 栅格倍率：按 2 倍（视网膜屏）渲染，逻辑尺寸不变。
const RASTER_SCALE: f32 = 2.0;
/// latex-rust 以 12pt 作为 em 渲染，对应 16 个逻辑像素。
const BASE_EM_PX: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EmbedKind {
    /// `display` 为真时使用显示样式（块级公式），否则为行内样式。
    Math { display: bool },
    Mermaid,
    /// 只有少量颜色值参与键，继续使用同一份有界位图缓存。
    MermaidThemed(MermaidPalette),
}

impl EmbedKind {
    pub fn with_palette(self, palette: MermaidPalette) -> Self {
        if matches!(self, Self::Mermaid) { Self::MermaidThemed(palette) } else { self }
    }
}

/// 一次渲染请求。`em_px` 为正文字号（逻辑像素），`color` 为公式前景色，`dark` 决定图表配色。
#[derive(Clone, Debug)]
pub struct EmbedSpec {
    pub kind: EmbedKind,
    pub source: String,
    pub em_px: f32,
    pub font_family: String,
    pub color: [u8; 3],
    pub dark: bool,
}

impl EmbedSpec {
    fn estimated_bytes(&self) -> usize { self.source.capacity() + self.font_family.capacity() }
    // 只有尺寸不同的同一张 Mermaid 可临时替代；源码、字体或配色变化不能串图。
    fn scale_family(&self) -> Option<u64> {
        if !matches!(self.kind, EmbedKind::Mermaid | EmbedKind::MermaidThemed(_)) { return None; }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.kind.hash(&mut hasher);
        self.font_family.hash(&mut hasher);
        self.source.trim().hash(&mut hasher);
        self.color.hash(&mut hasher);
        self.dark.hash(&mut hasher);
        Some(hasher.finish())
    }

    pub fn key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.kind.hash(&mut hasher);
        if matches!(self.kind, EmbedKind::Mermaid | EmbedKind::MermaidThemed(_)) { self.font_family.hash(&mut hasher); }
        // 围栏解析和局部草稿保留的首尾换行不同，但它们使用同一份渲染结果。
        self.source.trim().hash(&mut hasher);
        self.em_px.to_bits().hash(&mut hasher);
        self.color.hash(&mut hasher);
        self.dark.hash(&mut hasher);
        hasher.finish()
    }
}

/// 渲染结果的逻辑尺寸（像素）；`depth` 为基线以下的部分，用于行内公式对齐基线。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmbedInfo {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

#[derive(Clone, Debug)]
pub enum EmbedState {
    Pending,
    Ready(EmbedInfo),
    Failed(Arc<str>),
}

enum Entry {
    Loading(u64),
    Ready(Ready),
    Failed(Arc<str>),
}

struct Ready {
    info: EmbedInfo,
    scale_family: Option<(u64, f32)>,
    image: Option<Arc<RenderImage>>,
    bytes: usize,
    used: Cell<u64>,
    visible: Cell<Option<Instant>>,
    restoring: Option<u64>,
    blocked: Cell<bool>,
}
impl Ready {
    fn resident_bytes(&self) -> usize {
        if self.image.is_some() || self.restoring.is_some() { self.bytes } else { 0 }
    }
    fn evictable(&self, now: Instant) -> bool {
        self.image.is_some() && self.visible.get().is_none_or(|at| now.saturating_duration_since(at) > VISIBLE_GRACE)
    }
}

#[derive(Default)]
pub struct EmbedStore {
    entries: HashMap<u64, Entry>,
    specs: HashMap<u64, EmbedSpec>,
    spec_bytes: usize,
    visible_requests: RefCell<BTreeSet<u64>>,
    restore_scheduled: Cell<bool>,
    bytes: usize,
    clock: Cell<u64>,
    next_request: u64,
    pub generation: u64,
    renders: u64,
    evictions: u64,
    restores: u64,
}

impl Global for EmbedStore {}

impl EmbedStore {
    #[cfg(feature = "automation")]
    pub(crate) fn automation_state(&self) -> serde_json::Value {
        serde_json::json!({"entries":self.entries.len(),"decodedBytes":self.bytes,
            "specBytes":self.spec_bytes,"renders":self.renders,"evictions":self.evictions,"restores":self.restores,
            "pending":self.entries.values().filter(|e|matches!(e,Entry::Loading(_)) || matches!(e,Entry::Ready(r) if r.restoring.is_some())).count(),
            "evicted":self.entries.values().filter(|e|matches!(e,Entry::Ready(r) if r.image.is_none())).count(),
            "failed":self.entries.values().filter(|e|matches!(e,Entry::Failed(_))).count(),
            "scope":"process cache; requested resources only"})
    }
    /// 查询状态；返回 None 表示尚未请求，调用方应登记到 `request`。
    pub fn state(&self, key: u64) -> Option<EmbedState> {
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_ENTRIES {
            return Some(EmbedState::Failed(BUDGET_MESSAGE.into()));
        }
        Some(match self.entries.get(&key)? {
            Entry::Loading(_) => EmbedState::Pending,
            Entry::Ready(ready) => EmbedState::Ready(ready.info),
            Entry::Failed(message) => EmbedState::Failed(message.clone()),
        })
    }

    pub fn image(&self, key: u64) -> Option<Arc<RenderImage>> {
        self.clock.set(self.clock.get() + 1);
        match self.entries.get(&key) {
            Some(Entry::Ready(ready)) => {
                ready.used.set(self.clock.get());
                ready.image.clone()
            }
            _ => None,
        }
    }

    /// 新缩放版本等待期间沿用已有位图，逻辑尺寸立即跟随目标字号；不增加位图副本。
    pub fn scaled_fallback(&self, spec: &EmbedSpec) -> Option<(u64, EmbedInfo)> {
        let family = spec.scale_family()?;
        if !spec.em_px.is_finite() || spec.em_px <= 0.0 { return None; }
        self.entries.iter().filter_map(|(key, entry)| {
            let Entry::Ready(ready) = entry else { return None; };
            let (stored_family, em_px) = ready.scale_family?;
            if stored_family != family || ready.image.is_none() || !em_px.is_finite() || em_px <= 0.0 { return None; }
            let scale = spec.em_px / em_px;
            let info = EmbedInfo { width: ready.info.width * scale, height: ready.info.height * scale, depth: ready.info.depth * scale };
            Some((*key, info, scale.ln().abs()))
        }).min_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.0.cmp(&b.0)))
            .map(|(key, info, _)| (key, info))
    }

    /// 正文裁切之后／对象预览使用。恢复发生在本帧绘制结束后，尺寸查询不发起恢复。
    pub fn visible_image(&self, key: u64, cx: &App) -> Option<Arc<RenderImage>> {
        if let Some(Entry::Ready(ready)) = self.entries.get(&key) {
            ready.visible.set(Some(Instant::now()));
            if ready.image.is_none() && ready.restoring.is_none() && self.specs.contains_key(&key) {
                self.visible_requests.borrow_mut().insert(key);
                if !self.restore_scheduled.replace(true) {
                    cx.spawn(async move |cx| {
                        cx.update(|cx| {
                            let store = cx.global_mut::<EmbedStore>();
                            store.restore_scheduled.set(false);
                            let keys = std::mem::take(store.visible_requests.get_mut());
                            let mut retired = Vec::new();
                            let mut work = Vec::new();
                            let mut notice_changed = false;
                            for key in keys {
                                let blocked = matches!(store.entries.get(&key), Some(Entry::Ready(ready)) if ready.blocked.get());
                                if let Some((request, spec)) = store.begin_restore(key, &mut retired) {
                                    work.push((key, request, spec));
                                }
                                notice_changed |= blocked != matches!(store.entries.get(&key), Some(Entry::Ready(ready)) if ready.blocked.get());
                            }
                            for image in retired { cx.drop_image(image, None); }
                            for (key, request, spec) in work { spawn_render(key, request, spec, cx); }
                            if notice_changed { cx.refresh_windows(); }
                        });
                    }).detach();
                }
            }
        }
        self.image(key).or_else(|| {
            let (fallback, _) = self.scaled_fallback(self.specs.get(&key)?)?;
            if let Some(Entry::Ready(ready)) = self.entries.get(&fallback) { ready.visible.set(Some(Instant::now())); }
            self.image(fallback)
        })
    }

    pub fn notice(&self, key: u64) -> &'static str {
        if matches!(self.entries.get(&key), Some(Entry::Ready(ready)) if ready.blocked.get()) {
            "图表／公式位图预算不足，暂缓显示"
        } else { "正在加载图表／公式…" }
    }

    fn begin_restore(&mut self, key: u64, retired: &mut Vec<Arc<RenderImage>>) -> Option<(u64, EmbedSpec)> {
        let Entry::Ready(ready) = self.entries.get(&key)? else { return None; };
        if ready.image.is_some() || ready.restoring.is_some() || !self.specs.contains_key(&key) { return None; }
        let bytes = ready.bytes;
        // 先预留再启动任务；同屏总量超过预算时保持尺寸和提示，不相互淘汰或忙重试。
        if !self.make_room(bytes, retired, Instant::now()) {
            if let Some(Entry::Ready(ready)) = self.entries.get(&key) { ready.blocked.set(true); }
            return None;
        }
        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        let Some(Entry::Ready(ready)) = self.entries.get_mut(&key) else { return None; };
        ready.restoring = Some(request);
        ready.blocked.set(false);
        self.bytes += bytes;
        self.restores += 1;
        Some((request, self.specs[&key].clone()))
    }

    fn make_room(&mut self, bytes: usize, retired: &mut Vec<Arc<RenderImage>>, now: Instant) -> bool {
        if self.bytes.saturating_add(bytes) <= MAX_CACHE_BYTES { return true; }
        let reclaimable: usize = self.entries.values().filter_map(|entry| match entry {
            Entry::Ready(ready) if ready.evictable(now) => Some(ready.bytes), _ => None,
        }).sum();
        if self.bytes.saturating_sub(reclaimable).saturating_add(bytes) > MAX_CACHE_BYTES { return false; }
        while self.bytes + bytes > MAX_CACHE_BYTES {
            let oldest = self.entries.iter().filter_map(|(key, entry)| match entry {
                Entry::Ready(ready) if ready.evictable(now) => Some((*key, ready.used.get())), _ => None,
            }).min_by_key(|(_, used)| *used).map(|(key, _)| key);
            let Some(Entry::Ready(ready)) = oldest.and_then(|key| self.entries.get_mut(&key)) else { return false; };
            if let Some(image) = ready.image.take() {
                self.bytes -= ready.bytes;
                retired.push(image);
                self.evictions += 1;
            }
        }
        true
    }

    /// 关闭文档后不继续保留位图、失败结果或异步任务的旧登记。
    pub(crate) fn retain(&mut self, keep: impl Fn(u64) -> bool) -> Vec<Arc<RenderImage>> {
        let mut retired = Vec::new();
        let old_len = self.entries.len();
        self.entries.retain(|key, entry| {
            if keep(*key) { return true; }
            if let Entry::Ready(ready) = entry {
                self.bytes -= ready.resident_bytes();
                if let Some(image) = ready.image.take() { retired.push(image); }
            }
            false
        });
        self.specs.retain(|key, spec| {
            if self.entries.contains_key(key) { return true; }
            self.spec_bytes -= spec.estimated_bytes();
            false
        });
        self.visible_requests.get_mut().retain(|key| self.entries.contains_key(key));
        if old_len != self.entries.len() { self.generation = self.generation.wrapping_add(1); }
        retired
    }
    fn complete(&mut self, key: u64, request: u64, result: Result<(EmbedInfo, RenderImage, usize), String>) -> Vec<Arc<RenderImage>> {
        let current = matches!(self.entries.get(&key), Some(Entry::Loading(current)) if *current == request)
            || matches!(self.entries.get(&key), Some(Entry::Ready(ready)) if ready.restoring == Some(request));
        if !current { return Vec::new(); }
        self.insert(key, result)
    }
    /// CPU 缓存和窗口 atlas 必须一起淘汰，避免公式／图表纹理持续累积。
    fn insert(&mut self, key: u64, result: Result<(EmbedInfo, RenderImage, usize), String>) -> Vec<Arc<RenderImage>> {
        let mut retired = Vec::new();
        let previous = self.entries.remove(&key);
        let mut visible = None;
        let mut previous_info = None;
        if let Some(Entry::Ready(ready)) = previous {
            self.bytes -= ready.resident_bytes();
            visible = ready.visible.get();
            previous_info = Some(ready.info);
            if let Some(image) = ready.image { retired.push(image); }
        }
        self.clock.set(self.clock.get() + 1);
        match result {
            Ok((info, image, bytes)) => {
                let room = self.make_room(bytes, &mut retired, Instant::now());
                let image = Arc::new(image);
                let image = if room { self.bytes += bytes; Some(image) } else { retired.push(image); None };
                let scale_family = self.specs.get(&key).and_then(|spec| spec.scale_family().map(|family| (family, spec.em_px)));
                self.entries.insert(key, Entry::Ready(Ready { info: previous_info.unwrap_or(info), scale_family, image, bytes,
                    used: Cell::new(self.clock.get()), visible: Cell::new(visible), restoring: None, blocked: Cell::new(!room) }));
            }
            Err(message) => {
                let end = message.char_indices().map(|(at, _)| at).take_while(|at| *at <= 1024).last().unwrap_or(0);
                let message = if message.len() > 1024 { &message[..end] } else { &message };
                self.entries.insert(key, Entry::Failed(message.into()));
            }
        }
        self.generation += 1;
        retired
    }
}

/// 在后台渲染一批请求；已缓存或正在渲染的条目会被跳过。
pub fn request(specs: Vec<EmbedSpec>, cx: &mut App) {
    for spec in specs {
        let key = spec.key();
        let store = cx.global_mut::<EmbedStore>();
        if store.entries.contains_key(&key) {
            continue;
        }
        if store.entries.len() >= MAX_ENTRIES { continue; }
        if store.spec_bytes + spec.estimated_bytes() > MAX_SPEC_BYTES {
            store.entries.insert(key, Entry::Failed(BUDGET_MESSAGE.into()));
            store.generation = store.generation.wrapping_add(1);
            cx.refresh_windows();
            continue;
        }
        let saved = spec.clone();
        store.spec_bytes += saved.estimated_bytes();
        store.specs.insert(key, saved);
        store.next_request = store.next_request.wrapping_add(1);
        let request = store.next_request;
        store.entries.insert(key, Entry::Loading(request));
        spawn_render(key, request, spec, cx);
    }
}

fn spawn_render(key: u64, request: u64, spec: EmbedSpec, cx: &mut App) {
    cx.global_mut::<EmbedStore>().renders += 1;
    let task = cx.background_executor().spawn(async move { render(&spec) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let store = cx.global_mut::<EmbedStore>();
            let generation = store.generation;
            let retired = store.complete(key, request, result);
            let changed = generation != store.generation;
            for image in retired { cx.drop_image(image, None); }
            if changed { cx.refresh_windows(); }
        });
    }).detach();
}

fn render(spec: &EmbedSpec) -> Result<(EmbedInfo, RenderImage, usize), String> {
    let scale_to_em = spec.em_px / BASE_EM_PX;
    let source = spec.source.trim();
    match spec.kind {
        EmbedKind::Math { display } => {
            let (svg, depth_ratio) = math_svg(source, display, spec.color)?;
            let (image, width, height, bytes) = rasterize(&svg, scale_to_em, None)?;
            Ok((EmbedInfo { width, height, depth: height * depth_ratio }, image, bytes))
        }
        EmbedKind::Mermaid | EmbedKind::MermaidThemed(_) => {
            let palette = if let EmbedKind::MermaidThemed(palette) = spec.kind { Some(palette) } else { None };
            let svg = mermaid_svg_with_palette(source, spec.dark, palette, &spec.font_family, merman::OperationControl::new())?;
            let (image, width, height, bytes) = rasterize(&svg, scale_to_em, Some(&spec.font_family))?;
            Ok((EmbedInfo { width, height, depth: 0.0 }, image, bytes))
        }
    }
}

/// 栅格化 SVG：`scale` 为逻辑缩放（相对 SVG 自身尺寸），实际按 2 倍分辨率出图并转为 BGRA。
fn rasterize(svg: &str, scale: f32, font_family: Option<&str>) -> Result<(RenderImage, f32, f32, usize), String> {
    let mut options = resvg::usvg::Options { fontdb: font_database(), ..Default::default() };
    if let Some(font_family) = font_family {
        options.font_family = font_family.into();
        options.font_resolver = mermaid_font_resolver();
    }
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|error| format!("SVG 解析失败：{error}"))?;
    let size = tree.size();
    let (width, height) = (size.width() * scale, size.height() * scale);
    if width < 1.0 || height < 1.0 {
        return Err("渲染结果为空".into());
    }
    let mut raster = RASTER_SCALE;
    let longest = width.max(height) * raster;
    if longest > MAX_RASTER_SIDE {
        raster *= MAX_RASTER_SIDE / longest;
    }
    let (pixel_width, pixel_height) = ((width * raster).ceil() as u32, (height * raster).ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_width, pixel_height).ok_or("位图尺寸无效")?;
    let factor = scale * raster;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(factor, factor), &mut pixmap.as_mut());
    // tiny-skia 输出预乘 RGBA；GPUI 期望 BGRA（与其 img 元素的转换一致）。
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bytes = data.len();
    let buffer = RgbaImage::from_raw(pixel_width, pixel_height, data).ok_or("位图数据长度不匹配")?;
    Ok((RenderImage::new(vec![Frame::new(buffer)]), width, height, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_resources_release_textures_and_ignore_late_render() {
        let mut store = EmbedStore::default();
        let decoded = || Ok((EmbedInfo { width: 1.0, height: 1.0, depth: 0.0 },
            RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), 4));
        store.insert(1, decoded());
        store.insert(2, decoded());
        store.entries.insert(3, Entry::Loading(1));
        store.insert(4, Err("failed".into()));
        assert_eq!(store.retain(|key| key == 1).len(), 1);
        assert_eq!((store.entries.len(), store.bytes), (1, 4));
        store.complete(3, 1, decoded());
        assert!(store.state(3).is_none());
        store.entries.insert(3, Entry::Loading(2));
        store.complete(3, 1, Err("late failure".into()));
        assert!(matches!(store.state(3), Some(EmbedState::Pending)));
        store.complete(3, 2, decoded());
        assert_eq!(store.bytes, 8);
        assert_eq!(store.retain(|_| false).len(), 2);
        assert_eq!((store.entries.len(), store.bytes), (0, 0));
    }

    #[test]
    fn cache_eviction_returns_embed_texture_and_preserves_budget() {
        let mut store = EmbedStore::default();
        let decoded = || Ok((EmbedInfo { width: 1.0, height: 1.0, depth: 0.0 },
            RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), MAX_CACHE_BYTES / 2));
        assert!(store.insert(1, decoded()).is_empty());
        assert!(store.insert(2, decoded()).is_empty());
        let oldest = store.image(1).unwrap();
        store.image(2);
        let retired = store.insert(3, decoded());
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &oldest));
        assert!(matches!(store.state(1), Some(EmbedState::Ready(_))));
        assert!(store.image(1).is_none());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
        assert_eq!(store.insert(2, Err("failed".into())).len(), 1);
        assert_eq!(store.bytes, MAX_CACHE_BYTES / 2);
        assert!(store.insert(2, decoded()).is_empty());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
    }

    #[test]
    fn evicted_mermaid_layout_stays_ready_without_offscreen_demands() {
        use crate::nomo_mark::resources::ResourceAdapter;
        fn adapter<'a>(store: &'a EmbedStore, images: &'a crate::images::ImageStore, spec: &'a EmbedSpec, palette: MermaidPalette) -> ResourceAdapter<'a> {
            ResourceAdapter { images, embeds: store, em_px: spec.em_px, font_family: &spec.font_family, dark: false,
                mermaid: palette, color: [0; 3], demands: RefCell::new(Vec::new()), dependencies: Default::default() }
        }
        for zoom in [1.0, 1.4] {
            let mut store = EmbedStore::default();
            let images = crate::images::ImageStore::default();
            let palette = MermaidPalette { mode: 0, colors: [0; 9] };
            let spec = EmbedSpec { kind: EmbedKind::MermaidThemed(palette), source: "flowchart TD\n A --> B".into(),
                em_px: 16.0 * zoom, font_family: MERMAID_FONT_FAMILY.into(), color: [0; 3], dark: false };
            let key = spec.key();
            let info = EmbedInfo { width: 800.0 * zoom, height: 1200.0 * zoom, depth: 0.0 };
            let decoded = || Ok((info, RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), MAX_CACHE_BYTES / 2));
            store.insert(key, decoded());
            let resources = adapter(&store, &images, &spec, palette);
            resources.embed(EmbedKind::Mermaid, &spec.source);
            let dependencies = resources.dependencies.into_inner();
            let generation = dependencies.generation(&images, &store);
            store.insert(key.wrapping_add(1), decoded());
            store.insert(key.wrapping_add(2), decoded());
            assert!(store.image(key).is_none());
            for _ in 0..20 {
                let resources = adapter(&store, &images, &spec, palette);
                assert!(matches!(resources.embed(EmbedKind::Mermaid, &spec.source).1, EmbedState::Ready(actual) if actual == info));
                assert!(resources.demands.borrow().is_empty());
                assert_eq!(dependencies.generation(&images, &store), generation);
            }
        }
    }

    #[test]
    fn visible_restore_reserves_budget_and_does_not_cycle_under_pressure() {
        let mut store = EmbedStore::default();
        let info = EmbedInfo { width: 800.0, height: 1200.0, depth: 0.0 };
        let decoded = || Ok((info, RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), MAX_CACHE_BYTES / 2));
        for key in 1..=3 {
            let spec = EmbedSpec { kind: EmbedKind::Mermaid, source: format!("flowchart TD\n A --> B{key}"), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0; 3], dark: false };
            store.spec_bytes += spec.estimated_bytes();
            store.specs.insert(key, spec);
            store.insert(key, decoded());
        }
        // 三张共需 192 MiB：两张已绘制，第三张仍保留几何但不得驱逐同屏位图。
        for key in [2, 3] {
            if let Some(Entry::Ready(ready)) = store.entries.get(&key) { ready.visible.set(Some(Instant::now())); }
        }
        let mut retired = Vec::new();
        for _ in 0..20 { assert!(store.begin_restore(1, &mut retired).is_none()); }
        assert_eq!((store.restores, retired.len(), store.bytes), (0, 0, MAX_CACHE_BYTES));
        assert!(store.notice(1).contains("预算不足"));
        // 旧图离开可见区后才允许恢复；预留期间的重复请求和迟到结果均不能重复计费。
        if let Some(Entry::Ready(ready)) = store.entries.get(&2) { ready.visible.set(Some(Instant::now() - VISIBLE_GRACE - Duration::from_secs(1))); }
        let (request, _) = store.begin_restore(1, &mut retired).unwrap();
        assert_eq!(retired.len(), 1);
        assert!(store.begin_restore(1, &mut retired).is_none());
        assert!(matches!(store.state(1), Some(EmbedState::Ready(actual)) if actual == info));
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
        store.complete(1, request.wrapping_add(1), decoded());
        assert!(store.image(1).is_none());
        store.complete(1, request, decoded());
        assert!(store.image(1).is_some());
        assert_eq!((store.restores, store.bytes), (1, MAX_CACHE_BYTES));
        store.retain(|_| false);
        assert_eq!((store.spec_bytes, store.bytes, store.specs.len()), (0, 0, 0));
    }

    #[test]
    fn closing_during_restore_releases_reservation_and_ignores_completion() {
        let mut store = EmbedStore::default();
        let spec = EmbedSpec { kind: EmbedKind::Mermaid, source: "flowchart TD\n A --> B".into(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0; 3], dark: false };
        let info = EmbedInfo { width: 800.0, height: 1200.0, depth: 0.0 };
        let decoded = || Ok((info, RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), MAX_CACHE_BYTES));
        store.spec_bytes = spec.estimated_bytes();
        store.specs.insert(1, spec);
        store.insert(1, decoded());
        store.insert(2, decoded());
        let (request, _) = store.begin_restore(1, &mut Vec::new()).unwrap();
        store.retain(|_| false);
        let generation = store.generation;
        store.complete(1, request, decoded());
        assert_eq!((store.bytes, store.spec_bytes, store.entries.len()), (0, 0, 0));
        assert_eq!(store.generation, generation);
    }

    #[test]
    fn renders_display_math_with_depth() {
        let spec = EmbedSpec { kind: EmbedKind::Math { display: true }, source: r"\frac{a}{b} + \sum_{i=1}^n x_i".into(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0, 0, 0], dark: false };
        let (info, _, bytes) = render(&spec).expect("render");
        assert!(info.width > 30.0 && info.height > 20.0, "{info:?}");
        assert!(info.depth > 0.0 && info.depth < info.height);
        assert!(bytes > 0);
    }

    #[test]
    fn renders_mermaid_flowchart_with_text() {
        let svg = mermaid_svg("flowchart TD\n  A[开始] --> B{判断}\n  B --> C[结束]", false).expect("svg");
        assert!(!svg.contains("<foreignObject"), "resvg 管线不应保留 foreignObject");
        assert!(svg.contains("开始"));
        let spec = EmbedSpec { kind: EmbedKind::Mermaid, source: "flowchart TD\n  A[开始] --> B[结束]".into(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0, 0, 0], dark: true };
        assert!(render(&spec).is_ok());
    }

    #[test]
    fn mermaid_zoom_keeps_existing_image_until_target_resolution_is_ready() {
        use crate::nomo_mark::resources::ResourceAdapter;
        let palette = MermaidPalette { mode: 0, colors: [0; 9] };
        let base = EmbedSpec { kind: EmbedKind::MermaidThemed(palette), source: "flowchart TD\n A[开始] --> B[结束]".into(),
            em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0; 3], dark: false };
        let mut store = EmbedStore::default();
        let images = crate::images::ImageStore::default();
        let base_key = base.key();
        store.specs.insert(base_key, base.clone());
        let rendered = render(&base).unwrap();
        let original_info = rendered.0;
        store.insert(base_key, Ok(rendered));
        let original_image = store.image(base_key).unwrap();
        for em_px in [9.6, 22.4, 32.0, 12.8, 16.0] {
            let spec = EmbedSpec { em_px, ..base.clone() };
            let resources = ResourceAdapter { images: &images, embeds: &store, em_px, font_family: &base.font_family, dark: false,
                mermaid: palette, color: [0; 3], demands: RefCell::new(Vec::new()), dependencies: Default::default() };
            let (key, state) = resources.embed(EmbedKind::Mermaid, &base.source);
            assert_eq!(key, base_key);
            let EmbedState::Ready(info) = state else { panic!("缩放不应退回加载占位"); };
            assert!((info.height - original_info.height * em_px / base.em_px).abs() < 0.01);
            assert!(Arc::ptr_eq(&original_image, &store.image(key).unwrap()));
            assert_eq!(resources.demands.borrow().len(), usize::from(spec.key() != base_key));
        }
        let target = EmbedSpec { em_px: 22.4, ..base.clone() };
        let key = target.key();
        store.specs.insert(key, target.clone());
        store.entries.insert(key, Entry::Loading(7));
        let resources = ResourceAdapter { images: &images, embeds: &store, em_px: target.em_px, font_family: &target.font_family, dark: false,
            mermaid: palette, color: [0; 3], demands: RefCell::new(Vec::new()), dependencies: Default::default() };
        assert_eq!(resources.embed(EmbedKind::Mermaid, &base.source).0, base_key);
        assert!(resources.demands.borrow().is_empty());
        let dependencies = resources.dependencies.into_inner();
        let generation = dependencies.generation(&images, &store);
        store.complete(key, 7, render(&target));
        assert_ne!(dependencies.generation(&images, &store), generation);
        let resources = ResourceAdapter { images: &images, embeds: &store, em_px: target.em_px, font_family: &target.font_family, dark: false,
            mermaid: palette, color: [0; 3], demands: RefCell::new(Vec::new()), dependencies: Default::default() };
        assert_eq!(resources.embed(EmbedKind::Mermaid, &base.source).0, key);
        for different in [EmbedSpec { source: "flowchart TD\n A --> C".into(), ..target.clone() },
            EmbedSpec { dark: true, ..target.clone() }, EmbedSpec { color: [255; 3], ..target.clone() },
            EmbedSpec { kind: EmbedKind::MermaidThemed(MermaidPalette { mode: 1, colors: [0; 9] }), ..target.clone() },
            EmbedSpec { kind: EmbedKind::Math { display: true }, ..target }] {
            assert!(store.scaled_fallback(&different).is_none());
        }
    }

    /// 目视检查用：`NOMO_EMBED_DUMP=<目录> NOMO_CORPUS=<测试集> cargo test --release dump_ -- --ignored`
    /// 把七类 Mermaid 样例与若干公式栅格化为 PNG。
    #[test]
    #[ignore]
    fn dump_samples_for_review() {
        let (Some(out), Some(corpus)) = (std::env::var_os("NOMO_EMBED_DUMP"), std::env::var_os("NOMO_CORPUS")) else { return };
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        let text = std::fs::read_to_string(std::path::Path::new(&corpus).join("04_代码公式与图表/07_七类Mermaid图表.md")).unwrap();
        let mut sources = Vec::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            match (&mut current, line.trim_start()) {
                (None, trimmed) if trimmed.starts_with("```mermaid") => current = Some(String::new()),
                (Some(block), trimmed) if trimmed.starts_with("```") => {
                    sources.push((EmbedKind::Mermaid, std::mem::take(block)));
                    current = None;
                }
                (Some(block), _) => {
                    block.push_str(line);
                    block.push('\n');
                }
                _ => {}
            }
        }
        sources.push((EmbedKind::Math { display: true }, r"\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}".into()));
        sources.push((EmbedKind::Math { display: true }, r"\begin{bmatrix}1&2\\3&4\end{bmatrix}".into()));
        for (index, (kind, source)) in sources.into_iter().enumerate() {
            for dark in [false, true] {
                let spec = EmbedSpec { kind, source: source.clone(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: if dark { [215, 220, 225] } else { [32, 36, 40] }, dark };
                let (_, image, _) = render(&spec).unwrap_or_else(|error| panic!("{index}: {error}"));
                let size = image.size(0);
                let mut data = image.as_bytes(0).unwrap().to_vec();
                for pixel in data.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let buffer = RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, data).unwrap();
                buffer.save(out.join(format!("{index}-{}.png", if dark { "dark" } else { "light" }))).unwrap();
            }
        }
    }

    #[test]
    fn renders_cjk_text_with_system_font_outlines() {
        let spec = EmbedSpec { kind: EmbedKind::Math { display: false }, source: r"E = mc^2 \text{（质能方程）}".into(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [0, 0, 0], dark: false };
        let (info, image, bytes) = render(&spec).expect("中文公式应使用可用系统字体");
        assert!(info.width > 100.0 && info.height > 10.0 && bytes > 0, "{info:?}");
        assert!(info.depth >= 0.0 && info.depth < info.height);
        assert!(image.as_bytes(0).unwrap().chunks_exact(4).any(|pixel| pixel[3] != 0));
        let (svg, _) = math_svg(&spec.source, false, [0, 0, 0]).unwrap();
        assert!(svg.contains("<path") && !svg.contains("<text"));
    }

    #[test]
    fn cjk_fraction_scripts_and_spaces_keep_real_metrics() {
        let measure = |source: &str| {
            render(&EmbedSpec { kind: EmbedKind::Math { display: true }, source: source.into(), em_px: 16.0, font_family: MERMAID_FONT_FAMILY.into(), color: [32, 36, 40], dark: false }).unwrap().0
        };
        let base = measure(r"\text{中文}");
        let fraction = measure(r"\frac{\text{中文}}{\text{分母}}");
        let scripts = measure(r"\text{中文}_{\text{下}}^{\text{上}}");
        let spaced = measure("\\text{中　文}");
        assert!(fraction.height > base.height && fraction.depth > 0.0);
        assert!(scripts.height > base.height && scripts.width > base.width);
        assert!(spaced.width > base.width);
        assert!(measure(r"\pmb{中文}").width >= base.width);
        assert!(measure(r"\hat{\text{中}}").height > measure(r"\text{中}").height);
    }

    #[test]
    fn math_still_reports_missing_fonts_and_unsupported_commands() {
        let error = math_svg("\\text{\u{10ffff}}", false, [0, 0, 0]).unwrap_err();
        assert!(error.contains("缺少可用字形"), "{error}");
        assert!(math_svg(r"\nomoUnknownCommand{x}", false, [0, 0, 0]).is_err());
        assert!(math_svg(&"x".repeat(64 * 1024 + 1), false, [0, 0, 0]).unwrap_err().contains("预算"));
    }
}

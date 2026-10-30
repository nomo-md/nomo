//! 仅查询宿主共享资源和报告需求，不进行文件访问、解码或持有独立大型缓存。
use crate::{
    embeds::{EmbedKind, EmbedSpec, EmbedState, EmbedStore},
    images::ImageStore,
};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub enum ResourceRequest {
    Image(PathBuf),
    Embed(EmbedSpec),
}
impl PartialEq for ResourceRequest {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Image(a), Self::Image(b)) => a == b,
            (Self::Embed(a), Self::Embed(b)) => a.key() == b.key(),
            _ => false,
        }
    }
}
pub struct ResourceAdapter<'a> {
    pub images: &'a ImageStore,
    pub embeds: &'a EmbedStore,
    pub em_px: f32,
    pub dark: bool,
    pub mermaid: crate::embeds::MermaidPalette,
    pub color: [u8; 3],
    pub demands: RefCell<Vec<ResourceRequest>>,
    pub dependencies: RefCell<ResourceDependencies>,
}

#[derive(Default)]
pub struct ResourceDependencies { images: BTreeSet<PathBuf>, embeds: BTreeSet<u64>, bytes: usize, overflow: bool }
impl ResourceDependencies {
    fn image(&mut self, path: &Path) -> bool {
        if self.images.contains(path) { return false; }
        let bytes = path.as_os_str().len() + 128;
        if self.bytes + bytes > 256 * 1024 || self.images.len() + self.embeds.len() >= 512 { self.overflow = true; return true; }
        self.bytes += bytes;
        self.images.insert(path.into());
        true
    }
    fn embed(&mut self, key: u64) -> bool {
        if self.embeds.contains(&key) { return false; }
        if self.bytes + 48 > 256 * 1024 || self.images.len() + self.embeds.len() >= 512 { self.overflow = true; return true; }
        self.bytes += 48;
        self.embeds.insert(key);
        true
    }
    pub fn estimated_bytes(&self) -> usize {
        self.bytes
    }
    pub fn generation(&self, images: &ImageStore, embeds: &EmbedStore) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        if self.overflow { (images.generation,embeds.generation).hash(&mut hash); }
        for path in &self.images { path.hash(&mut hash); images.version(path).hash(&mut hash); }
        for key in &self.embeds {
            key.hash(&mut hash);
            match embeds.state(*key) {
                Some(EmbedState::Ready(info)) => { 1u8.hash(&mut hash); (info.width.to_bits(),info.height.to_bits(),info.depth.to_bits()).hash(&mut hash); }
                Some(EmbedState::Failed(message)) => { 2u8.hash(&mut hash); message.hash(&mut hash); }
                Some(EmbedState::Pending) => 0u8.hash(&mut hash),
                None => { 3u8.hash(&mut hash); embeds.generation.hash(&mut hash); }
            }
        }
        hash.finish()
    }
}
impl ResourceAdapter<'_> {
    pub fn image_size(&self, path: &Path) -> Option<(u32, u32)> {
        let first = self.dependencies.borrow_mut().image(path);
        let size = self.images.size(path);
        if first && size.is_none() && !self.images.is_pending_or_known(path) {
            self.demands.borrow_mut().push(ResourceRequest::Image(path.into()));
        }
        size
    }
    pub fn embed(&self, kind: EmbedKind, source: &str) -> (u64, EmbedState) {
        let spec = EmbedSpec {
            kind: kind.with_palette(self.mermaid),
            source: source.into(),
            em_px: self.em_px,
            dark: self.dark,
            color: self.color,
        };
        let key = spec.key();
        let first = self.dependencies.borrow_mut().embed(key);
        if let Some(state) = self.embeds.state(key) {
            (key, state)
        } else {
            if first { self.demands.borrow_mut().push(ResourceRequest::Embed(spec)); }
            (key, EmbedState::Pending)
        }
    }
}

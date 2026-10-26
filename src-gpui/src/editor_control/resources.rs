//! 仅查询宿主共享资源和报告需求，不进行文件访问、解码或持有独立大型缓存。
use crate::{
    embeds::{EmbedKind, EmbedSpec, EmbedState, EmbedStore},
    images::ImageStore,
};
use std::{
    cell::RefCell,
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
    pub color: [u8; 3],
    pub demands: RefCell<Vec<ResourceRequest>>,
}
impl ResourceAdapter<'_> {
    pub fn image_size(&self, path: &Path) -> Option<(u32, u32)> {
        let size = self.images.size(path);
        if size.is_none() && !self.images.is_pending_or_known(path) {
            self.demands
                .borrow_mut()
                .push(ResourceRequest::Image(path.into()));
        }
        size
    }
    pub fn embed(&self, kind: EmbedKind, source: &str) -> (u64, EmbedState) {
        let spec = EmbedSpec {
            kind,
            source: source.into(),
            em_px: self.em_px,
            dark: self.dark,
            color: self.color,
        };
        let key = spec.key();
        if let Some(state) = self.embeds.state(key) {
            (key, state)
        } else {
            self.demands.borrow_mut().push(ResourceRequest::Embed(spec));
            (key, EmbedState::Pending)
        }
    }
}

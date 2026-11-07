//! 关闭文档时回收无主位图；只登记弱宿主和有界资源键，不持有正文、视图回调或定时器。
use super::DocumentHost;
use crate::nomo_mark::resources::ResourceDependencies;
use gpui::{App, Context, Global, WeakEntity};

#[derive(Default)]
struct ResourceOwners { hosts: Vec<WeakEntity<DocumentHost>>, scheduled: bool }
impl Global for ResourceOwners {}

impl DocumentHost {
    pub(super) fn register_resources(cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        cx.default_global::<ResourceOwners>().hosts.push(weak);
        cx.on_release(|_, cx| schedule_collection(cx)).detach();
    }

    fn collect_resources(&self, into: &mut ResourceDependencies, cx: &App) {
        into.extend(&self.retained_resources);
        for pane in &self.panes {
            pane.control.read(cx).retain_resources(into, cx);
            for mount in &pane.viewport.mounts { mount.view.read(cx).retain_resources(into, cx); }
        }
    }

    pub(super) fn remember_resources(&mut self, cx: &App) {
        // 隐藏会清掉布局，但后台标签仍保有这些键；再次激活不必重新解码。
        for pane in &self.panes {
            pane.control.read(cx).retain_resources(&mut self.retained_resources, cx);
            for mount in &pane.viewport.mounts { mount.view.read(cx).retain_resources(&mut self.retained_resources, cx); }
        }
    }
}

pub(super) fn schedule_collection(cx: &mut App) {
    let owners = cx.default_global::<ResourceOwners>();
    if owners.scheduled { return; }
    owners.scheduled = true;
    // 等宿主关闭/移交事务结束，避免读取正在更新的 Entity；同一批关闭只扫描一次。
    cx.defer(|cx| {
        let hosts = std::mem::take(&mut cx.global_mut::<ResourceOwners>().hosts);
        let mut live = Vec::new();
        let mut resources = ResourceDependencies::default();
        for weak in hosts {
            if let Some(host) = weak.upgrade() {
                let host = host.read(cx);
                if !host.closed { host.collect_resources(&mut resources, cx); live.push(weak); }
            }
        }
        let owners = cx.global_mut::<ResourceOwners>();
        owners.hosts = live;
        owners.scheduled = false;
        // 超过 512 键 / 256 KiB 时保守保留，仍受原有 256/128 MiB 位图预算限制。
        let images = cx.global_mut::<crate::images::ImageStore>().retain(|path| resources.keeps_image(path));
        let embeds = cx.global_mut::<crate::embeds::EmbedStore>().retain(|key| resources.keeps_embed(key));
        for image in images.into_iter().chain(embeds) { cx.drop_image(image, None); }
    });
}

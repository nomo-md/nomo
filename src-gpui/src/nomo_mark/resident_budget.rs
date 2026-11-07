//! 原生宿主共享的驻留登记。只持有标量，不保留正文、控件或回调。
//! 复用原版窗口数/源文字节预算；原生塑形结果额外按估算字节限制可选预取。
use std::{collections::BTreeMap, sync::{Arc,Mutex,OnceLock}, sync::atomic::{AtomicU64,Ordering}};
#[derive(Clone,Copy,Default)]
pub struct Cost { pub views:usize, pub source:usize, pub layout:usize }
type Registry=Arc<Mutex<BTreeMap<u64,Cost>>>;
pub struct ResidentLease { registry:Registry, id:u64 }
fn limits()->Cost {
    static LIMITS:OnceLock<Cost>=OnceLock::new();
    *LIMITS.get_or_init(|| {
        let data:serde_json::Value=serde_json::from_str(include_str!("../../resources/windowingLimits.json")).expect("shared limits");
        Cost { views:data["resident"]["views"].as_u64().unwrap() as usize, source:data["resident"]["sourceBytes"].as_u64().unwrap() as usize, layout:64*1024*1024 }
    })
}
fn total(registry:&BTreeMap<u64,Cost>)->Cost { registry.values().fold(Cost::default(),|a,b| Cost { views:a.views+b.views,source:a.source+b.source,layout:a.layout+b.layout }) }
fn exceeds(cost:Cost)->bool { let limit=limits();cost.views>limit.views || cost.source>limit.source || cost.layout>limit.layout }
impl Default for ResidentLease {
    fn default()->Self {
        static REGISTRY:OnceLock<Registry>=OnceLock::new();
        static NEXT:AtomicU64=AtomicU64::new(1);
        Self{registry:REGISTRY.get_or_init(Default::default).clone(),id:NEXT.fetch_add(1,Ordering::Relaxed)}
    }
}
impl ResidentLease {
    pub fn update(&self,cost:Cost) { self.registry.lock().unwrap().insert(self.id,cost); }
    pub fn pressured(&self)->bool { exceeds(total(&self.registry.lock().unwrap())) }
    pub fn reserve(&self,extra:Cost,required:bool)->bool {
        let mut registry=self.registry.lock().unwrap();
        let current=total(&registry);
        if !required && exceeds(Cost { views:current.views+extra.views,source:current.source+extra.source,layout:current.layout+extra.layout }) {return false;}
        let own=registry.entry(self.id).or_default();
        own.views+=extra.views;own.source+=extra.source;own.layout+=extra.layout;
        true
    }
}
impl Drop for ResidentLease { fn drop(&mut self) { self.registry.lock().unwrap().remove(&self.id); } }
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_budget_releases_registration_without_retaining_views() {
        let registry=Registry::default();
        let first=ResidentLease{registry:registry.clone(),id:1};
        let second=ResidentLease{registry:registry.clone(),id:2};
        first.update(Cost{views:20,source:700_000,layout:0});
        assert!(!second.reserve(Cost{views:13,source:400_000,layout:0},false));
        assert!(second.reserve(Cost{views:1,source:20_000,layout:0},false));
        assert!(!first.pressured());
        drop(first);
        assert_eq!(total(&registry.lock().unwrap()).views,1);
        drop(second);assert!(registry.lock().unwrap().is_empty());
    }
}

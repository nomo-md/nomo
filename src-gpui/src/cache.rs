//! 有界 LRU 缓存：同时限制条数与估算字节，超限时淘汰最久未用的条目。
//!
//! 淘汰时线性扫描最小访问时钟；条数上限在千级，扫描成本远小于一次塑形，换来实现简单、无额外索引。

use std::collections::HashMap;
use std::rc::Rc;

pub struct LruCache<V> {
    entries: HashMap<u64, (Rc<V>, usize, u64)>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl<V> LruCache<V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self { entries: HashMap::new(), bytes: 0, clock: 0, max_entries, max_bytes }
    }

    pub fn get(&mut self, key: u64) -> Option<Rc<V>> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(&key)?;
        entry.2 = clock;
        Some(entry.0.clone())
    }

    /// 插入并返回共享引用；`bytes` 由调用方估算。
    pub fn insert(&mut self, key: u64, value: V, bytes: usize) -> Rc<V> {
        self.clock += 1;
        let value = Rc::new(value);
        if let Some((_, old_bytes, _)) = self.entries.insert(key, (value.clone(), bytes, self.clock)) {
            self.bytes -= old_bytes;
        }
        self.bytes += bytes;
        while (self.entries.len() > self.max_entries || self.bytes > self.max_bytes) && self.entries.len() > 1 {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, entry)| entry.2).map(|(key, _)| *key) else {
                break;
            };
            if let Some((_, removed, _)) = self.entries.remove(&oldest) {
                self.bytes -= removed;
            }
        }
        value
    }

    pub fn get_or_insert_with(&mut self, key: u64, build: impl FnOnce() -> (V, usize)) -> Rc<V> {
        if let Some(value) = self.get(key) {
            return value;
        }
        let (value, bytes) = build();
        self.insert(key, value, bytes)
    }
}

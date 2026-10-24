//! 单线程实验进程的分配计数，不是应用 RSS/提交量，也不包括内核或 JS。
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

fn add(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
    ALLOCATED.fetch_add(bytes, Relaxed);
}

unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = System.alloc(layout);
        if !result.is_null() { add(layout.size()); }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let result = System.alloc_zeroed(layout);
        if !result.is_null() { add(layout.size()); }
        result
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = System.realloc(ptr, layout, new_size);
        if !result.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            add(new_size);
        }
        result
    }
}

pub fn start() -> (usize, usize) {
    let live = LIVE.load(Relaxed);
    PEAK.store(live, Relaxed);
    (live, ALLOCATED.load(Relaxed))
}

pub fn finish(start: (usize, usize)) -> (usize, usize) {
    (PEAK.load(Relaxed).saturating_sub(start.0), ALLOCATED.load(Relaxed) - start.1)
}

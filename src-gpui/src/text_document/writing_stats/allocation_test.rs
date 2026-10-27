//! Test-only allocation instrumentation. No allocator hook ships in Nomo.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! {
    static ENABLED:Cell<bool>=const{Cell::new(false)};
    static LIVE:Cell<isize>=const{Cell::new(0)};
    static PEAK:Cell<usize>=const{Cell::new(0)};
}
struct MeasuredSystem;
fn delta(bytes: isize) {
    let _ = ENABLED.try_with(|enabled| {
        if enabled.get() {
            let _ = LIVE.try_with(|live| {
                let value = live.get() + bytes;
                live.set(value);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(value.max(0) as usize)));
            });
        }
    });
}
unsafe impl GlobalAlloc for MeasuredSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            delta(layout.size() as isize);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            delta(layout.size() as isize);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        delta(-(layout.size() as isize));
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(ptr, layout, size);
        if !next.is_null() {
            delta(size as isize - layout.size() as isize);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredSystem = MeasuredSystem;
pub(super) struct Scope;
impl Scope {
    pub fn start() -> Self {
        LIVE.with(|v| v.set(0));
        PEAK.with(|v| v.set(0));
        ENABLED.with(|v| v.set(true));
        Self
    }
    pub fn peak(&self) -> usize {
        PEAK.with(Cell::get)
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ENABLED.with(|v| v.set(false));
    }
}

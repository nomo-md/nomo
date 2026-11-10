//! 可见绘制驱动的有界播放器。任务只持有资源键和解码状态，不持有文档或视图。
use super::{Entry, ImageStore, gif::GifStream};
use gpui::{AnyWindowHandle, App, RenderImage, Task, Window};
use image::Frame;
use std::{cell::RefCell, collections::VecDeque, path::Path, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}, time::{Duration, Instant}};

const PREFETCH: usize = 2;
const MAX_WINDOWS: usize = 32;
const RETRY: Duration = Duration::from_millis(20);

/// 关闭资源后，在途解码／待回收帧仍持有额度，直到最后一个持有者退出。
struct Reservation {
    total: Arc<AtomicUsize>,
    bytes: usize,
}
impl Reservation {
    fn new(total: Arc<AtomicUsize>, bytes: usize) -> Arc<Self> {
        total.fetch_add(bytes, Ordering::Relaxed);
        Arc::new(Self { total, bytes })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) { self.total.fetch_sub(self.bytes, Ordering::Relaxed); }
}

pub(super) struct Playback {
    state: RefCell<State>,
    reservation: Arc<Reservation>,
    cancelled: Arc<AtomicBool>,
}
struct State {
    stream: Option<GifStream>,
    current: Arc<RenderImage>,
    next: VecDeque<Frame>,
    deadline: Option<Instant>,
    timer: Option<Task<()>>,
    windows: Vec<AnyWindowHandle>,
    all_windows: bool,
    queued: bool,
    decoding: bool,
    retiring: bool,
    ended: bool,
}

impl Drop for Playback {
    fn drop(&mut self) { self.cancelled.store(true, Ordering::Relaxed); }
}

impl Playback {
    pub fn new(stream: GifStream, first: Arc<RenderImage>, total: Arc<AtomicUsize>) -> Self {
        // Entry 已计入首帧；其余工作集和压缩源按保守峰值预留，共用进程缓存上限。
        let bytes = stream.reserved_bytes().saturating_sub(first.as_bytes(0).map_or(0, <[u8]>::len));
        Self {
            state: RefCell::new(State { stream: Some(stream), current: first, next: VecDeque::new(), deadline: None,
                timer: None, windows: Vec::new(), all_windows: false, queued: false, decoding: false, retiring: false, ended: false }),
            reservation: Reservation::new(total, bytes), cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn current(&self) -> Arc<RenderImage> { self.state.borrow().current.clone() }

    pub fn image(&self, store: &ImageStore, path: &Path, version: u64, poster: &Arc<RenderImage>, window: &Window, cx: &App) -> Arc<RenderImage> {
        let mut state = self.state.borrow_mut();
        if !window.is_visible() { return poster.clone(); }
        if crate::ui::popup::reduced_motion(cx) {
            state.timer = None;
            state.windows.clear();
            state.all_windows = false;
            return poster.clone();
        }
        let handle = window.window_handle();
        if !state.windows.contains(&handle) {
            if state.windows.len() < MAX_WINDOWS { state.windows.push(handle); } else { state.all_windows = true; }
        }
        let now = Instant::now();
        if state.deadline.is_none() { state.deadline = Some(now + delay(&state.current)); }
        if state.deadline.is_some_and(|deadline| deadline <= now) && !state.retiring {
            if let Some(next) = state.next.pop_front() {
                let previous = std::mem::replace(&mut state.current, Arc::new(RenderImage::new(vec![next])));
                state.deadline = Some(now + delay(&state.current));
                if !Arc::ptr_eq(&previous, poster) {
                    state.retiring = true;
                    let path = path.to_owned();
                    let reservation = self.reservation.clone();
                    // 在本次绘制退出后清理所有窗口，不能只释放 CPU Arc 或在 paint 中改 atlas。
                    cx.spawn(async move |cx| {
                        cx.update(|cx| {
                            cx.drop_image(previous, None);
                            let store = cx.global::<ImageStore>();
                            if store.version(&path) == (1, version) {
                                if let Some(playback) = store.animations.get(&path) { playback.state.borrow_mut().retiring = false; }
                            }
                        });
                        drop(reservation);
                    }).detach();
                }
            }
        }
        if !state.ended && !state.decoding && !state.queued && state.next.len() < PREFETCH {
            let mut queue = store.animation_queue.borrow_mut();
            if queue.len() + store.queued.len() < super::MAX_QUEUED {
                queue.push_back((path.to_owned(), version));
                state.queued = true;
                schedule(store, cx);
            }
        }
        // 定时器只唤醒一次绘制。若图片已离开视口，新的 paint 不会到这里，也不会续订。
        // 到期还没解出下一帧时等待任务完成唤醒，避免在慢解码期间轮询刷新。
        let deadline = state.deadline.unwrap();
        let waiting = state.next.is_empty() && (state.decoding || state.queued);
        if state.timer.is_none() && (!state.ended || !state.next.is_empty()) && (deadline > now || !waiting) {
            let wait = if deadline > now { deadline - now } else { RETRY };
            let path = path.to_owned();
            state.timer = Some(cx.spawn(async move |cx| {
                cx.background_executor().timer(wait).await;
                cx.update(|cx| wake(&path, version, cx));
            }));
        }
        state.current.clone()
    }
}

fn delay(image: &RenderImage) -> Duration {
    let (n, d) = image.delay(0).numer_denom_ms();
    Duration::from_millis((u64::from(n) / u64::from(d.max(1))).max(20))
}

fn schedule(store: &ImageStore, cx: &App) {
    if store.animation_scheduled.replace(true) { return; }
    cx.spawn(async |cx| {
        cx.update(|cx| {
            cx.global::<ImageStore>().animation_scheduled.set(false);
            super::start_pending(cx);
        });
    }).detach();
}

fn wake(path: &Path, version: u64, cx: &mut App) {
    let store = cx.global::<ImageStore>();
    if store.version(path) != (1, version) { return; }
    let Some(playback) = store.animations.get(path) else { return; };
    let (windows, all_windows) = {
        let mut state = playback.state.borrow_mut();
        state.timer = None;
        (std::mem::take(&mut state.windows), std::mem::take(&mut state.all_windows))
    };
    if all_windows { cx.refresh_windows(); return; }
    for handle in windows {
        let _ = handle.update(cx, |_, window, _| { if window.is_visible() { window.refresh(); } });
    }
}

/// 与首次图片解码共用 MAX_DECODERS，不为每张 GIF 启动常驻线程。
pub(super) fn start_pending(cx: &mut App) -> bool {
    loop {
        let reduced = crate::ui::popup::reduced_motion(cx);
        let store = cx.global_mut::<ImageStore>();
        let Some((path, version)) = store.animation_queue.get_mut().pop_front() else { return false; };
        if store.version(&path) != (1, version) { continue; }
        let Some(playback) = store.animations.get(&path) else { continue; };
        let (mut stream, count, cancelled, reservation) = {
            let mut state = playback.state.borrow_mut();
            state.queued = false;
            if reduced || state.decoding || state.ended { continue; }
            let Some(stream) = state.stream.take() else { continue; };
            state.decoding = true;
            (stream, PREFETCH.saturating_sub(state.next.len()), playback.cancelled.clone(), playback.reservation.clone())
        };
        store.running += 1;
        let task = cx.background_executor().spawn(async move {
            let mut frames = VecDeque::new();
            let mut ended = false;
            for _ in 0..count {
                if cancelled.load(Ordering::Relaxed) { return None; }
                match stream.next_frame() {
                    Ok(Some(frame)) => frames.push_back(frame),
                    Ok(None) => { ended = true; break; },
                    Err(error) => return Some(Err(error)),
                }
            }
            if cancelled.load(Ordering::Relaxed) { return None; }
            Some(Ok(((!ended).then_some(stream), frames, ended)))
        });
        cx.spawn(async move |cx| {
            let decoded = task.await;
            cx.update(|cx| {
                let store = cx.global_mut::<ImageStore>();
                store.running -= 1;
                let mut retired = None;
                let mut refresh = false;
                if store.version(&path) == (1, version) {
                    if let (Some(playback), Some(decoded)) = (store.animations.get(&path), decoded) {
                        match decoded {
                            Ok((stream, frames, ended)) => {
                                let mut state = playback.state.borrow_mut();
                                state.stream = stream;
                                state.next.extend(frames);
                                state.decoding = false;
                                state.ended = ended;
                                refresh = state.deadline.is_some_and(|deadline| deadline <= Instant::now());
                                if ended && state.next.is_empty() && matches!(store.entries.get(&path), Some(Entry::Ready { image, .. }) if Arc::ptr_eq(image, &state.current)) {
                                    retired = Some(state.current.clone());
                                }
                            },
                            Err(reason) => {
                                // 保留可解码首帧和原因，不能因后续坏帧将整图替换成失败占位。
                                retired = Some(playback.current());
                                if let Some(Entry::Ready { notice, .. }) = store.entries.get_mut(&path) { *notice = Some(reason); }
                                refresh = true;
                            },
                        }
                    }
                }
                if refresh { wake(&path, version, cx); }
                if let Some(image) = retired {
                    let store = cx.global_mut::<ImageStore>();
                    store.animations.remove(&path);
                    let poster = store.image(&path);
                    if !poster.is_some_and(|poster| Arc::ptr_eq(&poster, &image)) { cx.drop_image(image, None); }
                }
                super::start_pending(cx);
            });
            drop(reservation);
        }).detach();
        return true;
    }
}

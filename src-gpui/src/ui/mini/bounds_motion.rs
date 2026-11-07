//! 同一个 HWND 的两端都是屏幕物理坐标；折返沿上次已显示的位置继续。
use super::native::MiniTarget;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::RECT;

const DURATION: Duration = Duration::from_millis(180);

pub(super) fn same_rect(a: RECT, b: RECT) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

pub(super) struct BoundsMotion {
    main: RECT,
    mini: RECT,
    pub(super) last: RECT,
    pub(super) target: MiniTarget,
    pub(super) drag_epoch: usize,
    phase: f32,
    tick: Option<Instant>,
}

impl BoundsMotion {
    pub(super) fn new(main: RECT, mini: RECT, target: MiniTarget, drag_epoch: usize) -> Self {
        let entering = target == MiniTarget::Mini;
        Self { main, mini, last: if entering { main } else { mini }, target, drag_epoch,
            phase: if entering { 0.0 } else { 1.0 }, tick: None }
    }

    pub(super) fn redirect(&mut self, target: MiniTarget) {
        if self.target != target {
            self.target = target;
            self.tick = Some(Instant::now());
        }
    }

    // 窗框和纸面共用同一个已显示帧的进度，折返时两者一起换向。
    pub(super) fn progress(&self) -> f32 { self.phase * self.phase * (3.0 - 2.0 * self.phase) }

    pub(super) fn resume_after_handoff(&mut self) { self.tick = Some(Instant::now()); }

    pub(super) fn next(&mut self, reduced: bool, handoff: bool) -> (RECT, bool) {
        let now = Instant::now();
        // 第一帧开始计时，首次排版和 Shell 调用的耗时不占用动效时长。
        let elapsed = self.tick.replace(now).map_or(Duration::ZERO, |last| now - last);
        let step = if reduced { 1.0 } else { elapsed.as_secs_f32() / DURATION.as_secs_f32() };
        let entering = self.target == MiniTarget::Mini;
        let next = (self.phase + if entering { step } else { -step }).clamp(0.0, 1.0);
        // 慢帧也要画到换版的交接点，避免把另一种表示直接画成完全不透明。
        self.phase = if handoff && !reduced && ((self.phase < 0.5 && next > 0.5) || (self.phase > 0.5 && next < 0.5)) { 0.5 } else { next };
        if self.phase == 0.0 { return (self.main, !entering); }
        if self.phase == 1.0 { return (self.mini, entering); }
        // 对称缓动：进入和返回共享曲线，慢帧按经过的时间直接赶到当前位置。
        let t = self.progress();
        let edge = |a: i32, b: i32| (f64::from(a) + (f64::from(b) - f64::from(a)) * f64::from(t)).round() as i32;
        (RECT { left: edge(self.main.left, self.mini.left), top: edge(self.main.top, self.mini.top),
            right: edge(self.main.right, self.mini.right), bottom: edge(self.main.bottom, self.mini.bottom) }, false)
    }
}

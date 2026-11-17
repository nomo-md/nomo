//! 外壳动效共用的系统偏好与短时插值，不持有文档或视图。
use gpui::{Div, Global, Hsla, ParentElement, Rgba, Styled, div, px};
use std::time::{Duration, Instant};

/// 拖拽和纸片保留速度，只改变目标；四个子步限制掉帧后的积分误差。
/// 参数与积分方式参考 Quiet.md；MIT 声明见 assets/licenses/LICENSE-Rendering。
#[derive(Clone, Copy)]
pub(crate) struct Spring<const N: usize> {
    pub p: [f32; N],
    pub v: [f32; N],
    pub target: [f32; N],
    stiffness: f32,
    damping: f32,
    precision: (f32, f32),
}

impl<const N: usize> Spring<N> {
    pub fn new(p: [f32; N], stiffness: f32, damping: f32) -> Self {
        Self { p, v: [0.0; N], target: p, stiffness, damping, precision: (0.002, 0.02) }
    }

    pub fn layout(p: [f32; N]) -> Self {
        Self { precision: (0.05, 0.1), ..Self::new(p, 420.0, 31.0) }
    }

    pub fn snap(&mut self) { self.p = self.target; self.v = [0.0; N]; }
    pub fn running(&self) -> bool { self.p != self.target || self.v != [0.0; N] }

    pub fn step(&mut self, dt: f32) -> bool {
        let step = dt.clamp(0.0, 0.032) / 4.0;
        for i in 0..N {
            for _ in 0..4 {
                self.v[i] += (self.stiffness * (self.target[i] - self.p[i]) - self.damping * self.v[i]) * step;
                self.p[i] += self.v[i] * step;
            }
            if (self.target[i] - self.p[i]).abs() < self.precision.0 && self.v[i].abs() < self.precision.1 {
                self.p[i] = self.target[i]; self.v[i] = 0.0;
            }
        }
        self.running()
    }
}

/// 两页侧栏使用相同的裁剪几何。回弹只移动栏内内容，避免网格跨过已到达的宽度阈值。
#[derive(Clone, Copy, Default)]
pub(super) struct SidebarFrame {
    pub full_width: f32,
    pub progress: f32,
    pub target_visible: bool,
}

impl SidebarFrame {
    pub fn occupied_width(self) -> f32 {
        self.full_width * self.progress.clamp(0.0, 1.0)
    }

    pub fn clip(self, panel: Div) -> Div {
        div().relative().w(px(self.occupied_width())).h_full().flex_none().overflow_hidden()
            .child(panel.absolute().top_0().left(px(self.full_width * (self.progress - 1.0)))
                .w(px(self.full_width)).h_full())
    }
}

#[derive(Default)]
pub struct MotionPreference {
    pub reduced: bool,
}

impl Global for MotionPreference {}

/// 仅在主题过渡期间要求当前视图重绘，不注册常驻定时器。
pub fn theme_frame() -> impl gpui::IntoElement {
    gpui::canvas(|_, _, _| (), |_, _, window, _| window.request_animation_frame())
        .absolute().top_0().left_0().size_full()
}

/// CSS `ease`：在 x 轴反解 cubic-bezier(0.25, 0.1, 0.25, 1)。
pub fn ease(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    if progress == 0.0 || progress == 1.0 { return progress; }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..12 {
        let t = (low + high) * 0.5;
        let x = 3.0 * (1.0 - t) * (1.0 - t) * t * 0.25
            + 3.0 * (1.0 - t) * t * t * 0.25 + t * t * t;
        if x < progress { low = t; } else { high = t; }
    }
    let t = (low + high) * 0.5;
    3.0 * (1.0 - t) * (1.0 - t) * t * 0.1
        + 3.0 * (1.0 - t) * t * t + t * t * t
}

/// 按 sRGB 预乘透明度插值，避免透明颜色切换时出现色边。
pub fn color(from: Hsla, to: Hsla, progress: f32) -> Hsla {
    let from: Rgba = from.into();
    let to: Rgba = to.into();
    let progress = progress.clamp(0.0, 1.0);
    let alpha = from.a + (to.a - from.a) * progress;
    if alpha <= f32::EPSILON { return Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }.into(); }
    let channel = |a: f32, b: f32| (a * from.a * (1.0 - progress) + b * to.a * progress) / alpha;
    Rgba { r: channel(from.r, to.r), g: channel(from.g, to.g), b: channel(from.b, to.b), a: alpha }.into()
}

#[derive(Clone, Copy)]
pub struct ScalarMotion {
    from: f32,
    target: f32,
    started: Option<Instant>,
    duration: Duration,
}

impl Default for ScalarMotion {
    fn default() -> Self {
        Self { from: 0.0, target: 0.0, started: None, duration: Duration::from_millis(150) }
    }
}

impl ScalarMotion {
    pub fn settled(active: bool) -> Self {
        let value = if active { 1.0 } else { 0.0 };
        Self { from: value, target: value, ..Self::default() }
    }

    pub fn set(&mut self, active: bool, reduced: bool, duration: Duration) {
        let target = if active { 1.0 } else { 0.0 };
        let duration = if reduced { duration.min(Duration::from_millis(90)) } else { duration };
        if self.target != target || (self.running() && self.duration != duration) {
            self.from = self.value();
            self.target = target;
            self.duration = duration;
            self.started = Some(Instant::now());
        }
    }

    pub fn retime(&mut self, reduced: bool, duration: Duration) {
        self.set(self.target > 0.0, reduced, duration);
    }

    pub fn reset(&mut self) {
        self.from = 0.0;
        self.target = 0.0;
        self.started = None;
    }

    pub fn value(&self) -> f32 {
        self.started.map(|started| {
            let progress = (started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0);
            self.from + (self.target - self.from) * ease(progress)
        }).unwrap_or(self.target)
    }

    pub fn running(&self) -> bool {
        self.started.is_some_and(|started| started.elapsed() < self.duration)
    }
}

#[cfg(target_os = "macos")]
pub fn macos_reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(target_os = "linux")]
pub async fn linux_reduce_motion() -> Result<bool, String> {
    let settings = ashpd::desktop::settings::Settings::new().await
        .map_err(|error| format!("无法读取系统动态效果偏好：{error}"))?;
    match settings.read::<u32>("org.freedesktop.appearance", "reduced-motion").await {
        Ok(value) => Ok(value == 1),
        Err(portal_error) => settings.read::<bool>("org.gnome.desktop.interface", "enable-animations").await
            .map(|enabled| !enabled)
            .map_err(|_| format!("系统未提供动态效果偏好：{portal_error}")),
    }
}

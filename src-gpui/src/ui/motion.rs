//! 外壳动效共用的系统偏好与短时插值，不持有文档或视图。
use gpui::{Global, Hsla, Rgba, Styled};
use std::time::{Duration, Instant};

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

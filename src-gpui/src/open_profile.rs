//! 显式启用的打开耗时采样；只记录阶段与几何，不记录正文或文件路径。
use std::{collections::BTreeMap, io::Write, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicU8, Ordering}}, time::Instant};

#[derive(Clone)]
pub(crate) struct OpenProfile(Arc<Sample>);
struct Sample {
    started: Instant,
    output: PathBuf,
    values: Mutex<BTreeMap<&'static str, serde_json::Value>>,
    frame: AtomicU8,
}
impl OpenProfile {
    pub(crate) fn begin() -> Option<Self> {
        let output = std::env::var_os("NOMO_OPEN_PROFILE")?;
        Some(Self(Arc::new(Sample {
            started: Instant::now(), output: output.into(), values: Mutex::new(BTreeMap::new()), frame: AtomicU8::new(0),
        })))
    }
    pub(crate) fn stage(&self, name: &'static str) {
        if let Ok(mut values) = self.0.values.lock() {
            values.insert(name, (self.0.started.elapsed().as_secs_f64() * 1000.0).into());
        }
    }
    pub(crate) fn value(&self, name: &'static str, value: u64) {
        if let Ok(mut values) = self.0.values.lock() { values.insert(name, value.into()); }
    }
    pub(crate) fn frame(&self, source: bool) -> Option<&'static str> {
        let previous = self.0.frame.load(Ordering::Relaxed);
        if previous == 2 || (previous == 1 && source) { return None; }
        self.0.frame.compare_exchange(previous, if source { 1 } else { 2 }, Ordering::Relaxed, Ordering::Relaxed).ok()?;
        Some(if previous == 0 { "first_frame_ms" } else { "semantic_frame_ms" })
    }
    pub(crate) fn finish(self, stage: &'static str, bytes: usize, source: bool, window: &gpui::Window, cx: &gpui::App) {
        self.stage(stage);
        let Ok(mut values) = self.0.values.lock() else { return; };
        values.insert("window_bytes", bytes.into());
        values.insert("source_only", source.into());
        values.insert("width", f32::from(window.viewport_size().width).into());
        values.insert("height", f32::from(window.viewport_size().height).into());
        values.insert("scale_factor", window.scale_factor().into());
        values.insert("pid", std::process::id().into());
        values.insert("frame", stage.into());
        let encoded = serde_json::to_vec(&*values);
        drop(values);
        let output = self.0.output.clone();
        cx.background_executor().spawn(async move {
            let result = (|| -> std::io::Result<()> {
                let mut bytes = encoded?;
                bytes.push(b'\n');
                std::fs::OpenOptions::new().create(true).append(true).open(output)?.write_all(&bytes)
            })();
            if let Err(error) = result { crate::app_logger::warn("OpenProfile", &error.to_string()); }
        }).detach();
    }
}

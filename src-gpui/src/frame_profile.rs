//! 显式启用的帧耗时采样（`NOMO_FRAME_PROFILE=<输出 JSON 路径>`）：记录工作区每帧 CPU 侧
//! 渲染到绘制结束的耗时，以及按键到首个随后完成绘制的帧的延迟。不记录正文或路径。
//!
//! 未设置环境变量时所有入口立即返回，不分配、不加锁。启用后样本放在有界环形缓冲区，
//! 每累计 `FLUSH_EVERY` 个新样本覆盖写一次汇总（中位数、p95、最大值），进程退出无需额外钩子。
//!
//! 另设 `NOMO_FRAME_PROFILE_PAGES=<次数>` 时，打开后等待 `AUTOSCROLL_DELAY` 让首屏和索引稳定，
//! 再由工作区每帧向焦点文档派发一次 `PageDown`，用于无人值守的同场景滚动对照；
//! 最后一次翻页上屏后写最终汇总并停止采样。
use std::{io::Write, path::PathBuf, sync::{Mutex, OnceLock}, time::Instant};

/// 每类样本最多保留的条数：约 2 分钟 120Hz 连续滚动，超出后覆盖最旧样本。
const CAPACITY: usize = 16_384;
/// 每新增多少帧样本刷新一次汇总文件；写盘只发生在采样模式下。
const FLUSH_EVERY: usize = 240;
/// 自动翻页开始前的等待时间：覆盖百万字样本的首屏与语义恢复（经验值，样本更大时应调大）。
const AUTOSCROLL_DELAY: std::time::Duration = std::time::Duration::from_secs(4);

struct Samples {
    output: PathBuf,
    frames: Vec<f32>,
    frame_cursor: usize,
    inputs: Vec<f32>,
    input_cursor: usize,
    since_flush: usize,
    frame_started: Option<Instant>,
    pending_input: Option<Instant>,
    /// 自动翻页：采样启用时刻与剩余次数；未设置 `NOMO_FRAME_PROFILE_PAGES` 时为 0。
    created: Instant,
    pages_left: u32,
    /// 自动翻页的最后一次已派发：其上屏帧结算后写最终汇总并置 `stopped`。
    finishing: bool,
    /// 自动翻页完成后停止采样，避免进程结束前的其余帧覆盖最终汇总。
    stopped: bool,
}

fn state() -> Option<&'static Mutex<Samples>> {
    static STATE: OnceLock<Option<Mutex<Samples>>> = OnceLock::new();
    STATE.get_or_init(|| {
        let output = std::env::var_os("NOMO_FRAME_PROFILE")?;
        Some(Mutex::new(Samples {
            output: output.into(), frames: Vec::new(), frame_cursor: 0, inputs: Vec::new(), input_cursor: 0,
            since_flush: 0, frame_started: None, pending_input: None, created: Instant::now(), finishing: false, stopped: false,
            pages_left: std::env::var("NOMO_FRAME_PROFILE_PAGES").ok().and_then(|value| value.parse().ok()).unwrap_or(0),
        }))
    }).as_ref()
}

fn push(buffer: &mut Vec<f32>, cursor: &mut usize, value: f32) {
    if buffer.len() < CAPACITY { buffer.push(value); } else { buffer[*cursor] = value; }
    *cursor = (*cursor + 1) % CAPACITY;
}

/// 工作区 `render` 开始时调用；同一帧内重复调用只保留最早的起点。
pub(crate) fn frame_begin() {
    let Some(state) = state() else { return };
    if let Ok(mut samples) = state.lock() { if !samples.stopped { samples.frame_started.get_or_insert_with(Instant::now); } }
}

/// 记录一次按键；只保留尚未上屏的最早一次，连续按键按首键计延迟。
pub(crate) fn input() {
    let Some(state) = state() else { return };
    if let Ok(mut samples) = state.lock() { if !samples.stopped { samples.pending_input.get_or_insert_with(Instant::now); } }
}

/// 工作区元素树中最后绘制的画布调用：结束本帧计时并结算待上屏的按键。
pub(crate) fn frame_end() {
    let Some(state) = state() else { return };
    let Ok(mut samples) = state.lock() else { return };
    let now = Instant::now();
    let Some(started) = samples.frame_started.take() else { return };
    let samples = &mut *samples;
    push(&mut samples.frames, &mut samples.frame_cursor, (now - started).as_secs_f32() * 1000.0);
    if let Some(input) = samples.pending_input.take() {
        push(&mut samples.inputs, &mut samples.input_cursor, (now - input).as_secs_f32() * 1000.0);
        if samples.finishing {
            samples.stopped = true;
            write_summary(samples);
            return;
        }
    }
    samples.since_flush += 1;
    if samples.since_flush >= FLUSH_EVERY {
        samples.since_flush = 0;
        write_summary(samples);
    }
}

/// 自动翻页的下一步：`Some(true)` 表示应在下一帧回调里派发一次翻页（派发时由调用方记为输入），
/// `Some(false)` 表示仍在等待期、需要继续请求帧，`None` 表示未启用或已完成。
pub(crate) fn autoscroll_step() -> Option<bool> {
    let state = state()?;
    let mut samples = state.lock().ok()?;
    if samples.pages_left == 0 { return None; }
    if samples.created.elapsed() < AUTOSCROLL_DELAY {
        // 等待期为维持计时而主动请求的空帧不计入样本，避免拉低帧耗时中位数。
        samples.frame_started = None;
        return Some(false);
    }
    samples.pages_left -= 1;
    samples.finishing = samples.pages_left == 0;
    Some(true)
}

fn summary(values: &[f32]) -> serde_json::Value {
    if values.is_empty() { return serde_json::json!({ "count": 0 }); }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |fraction: f32| sorted[((sorted.len() - 1) as f32 * fraction).round() as usize];
    serde_json::json!({ "count": sorted.len(), "median_ms": at(0.5), "p95_ms": at(0.95), "max_ms": sorted[sorted.len() - 1] })
}

fn write_summary(samples: &Samples) {
    let report = serde_json::json!({ "frame": summary(&samples.frames), "input_to_paint": summary(&samples.inputs) });
    let result = std::fs::File::create(&samples.output).and_then(|mut file| file.write_all(report.to_string().as_bytes()));
    if let Err(error) = result { eprintln!("帧耗时采样写入失败: {error}"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_overwrites_oldest_after_capacity() {
        let (mut buffer, mut cursor) = (Vec::new(), 0);
        for value in 0..CAPACITY + 2 { push(&mut buffer, &mut cursor, value as f32); }
        assert_eq!(buffer.len(), CAPACITY);
        assert_eq!(buffer[0], CAPACITY as f32);
        assert_eq!(buffer[1], (CAPACITY + 1) as f32);
    }

    #[test]
    fn summary_reports_median_and_p95() {
        let values: Vec<f32> = (1..=100).map(|value| value as f32).collect();
        let report = summary(&values);
        assert_eq!(report["count"], 100);
        assert_eq!(report["median_ms"].as_f64(), Some(51.0));
        assert_eq!(report["p95_ms"].as_f64(), Some(95.0));
        assert_eq!(report["max_ms"].as_f64(), Some(100.0));
    }
}

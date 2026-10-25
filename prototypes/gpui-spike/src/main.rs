//! Nomo GPUI 原型入口。
//!
//! 用法：`nomo-gpui-spike <file.md> [--greedy] [--bench] [--ime-selftest] [--exit-after <秒>]`
//! - `--bench`：逐帧执行跳转、匀速滚动、快速甩动和中部连续输入，结束后输出 JSON 统计并退出。
//! - `--ime-selftest`：按 macOS 拼音输入法的调用顺序直接驱动 `EntityInputHandler`，校验组字与撤销。
//! - `--exit-after`：供外部内存采样使用，到时自动退出。

mod editor;
mod kp;
mod text_document;

use editor::{BreakMode, Editor, Quit};
use gpui::{
    App, AppContext, Application, Bounds, Entity, EntityInputHandler, Focusable, TitlebarOptions,
    Window, WindowBounds, WindowOptions, px, size,
};
use std::time::{Duration, Instant};

struct Options {
    path: String,
    mode: BreakMode,
    bench: bool,
    ime_selftest: bool,
    exit_after: Option<u64>,
}

fn parse_options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut options = Options { path: String::new(), mode: BreakMode::KnuthPlass, bench: false, ime_selftest: false, exit_after: None };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--greedy" => options.mode = BreakMode::Greedy,
            "--bench" => options.bench = true,
            "--ime-selftest" => options.ime_selftest = true,
            "--exit-after" => options.exit_after = args.next().and_then(|value| value.parse().ok()),
            _ => options.path = arg,
        }
    }
    options
}

fn main() {
    let started = Instant::now();
    let options = parse_options();
    let bytes = if options.path.is_empty() { Vec::new() } else { std::fs::read(&options.path).expect("读取文件失败") };
    let read_ms = started.elapsed().as_secs_f64() * 1000.0;
    let indexed = Instant::now();
    let document = text_document::Document::from_bytes(bytes);
    let index_ms = indexed.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "{{\"event\":\"open\",\"bytes\":{},\"lines\":{},\"read_ms\":{read_ms:.1},\"index_ms\":{index_ms:.1},\"doc_resident_bytes\":{}}}",
        document.len(),
        document.line_count(),
        document.resident_bytes()
    );

    Application::new().run(move |cx: &mut App| {
        eprintln!("{{\"event\":\"app_ready\",\"since_process_start_ms\":{:.1}}}", started.elapsed().as_secs_f64() * 1000.0);
        editor::bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let mode = options.mode;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Nomo GPUI Spike".into()), ..Default::default() }),
                    ..Default::default()
                },
                move |_, cx| cx.new(|cx| Editor::new(document, mode, cx)),
            )
            .expect("打开窗口失败");
        eprintln!("{{\"event\":\"window_opened\",\"since_process_start_ms\":{:.1}}}", started.elapsed().as_secs_f64() * 1000.0);
        window
            .update(cx, |editor, window, cx| {
                window.focus(&editor.focus_handle(cx));
                cx.activate(true);
                let entity = cx.entity();
                window.on_next_frame(move |window, cx| {
                    report_first_frame(&entity, started, cx);
                    if options.ime_selftest {
                        ime_selftest(&entity, window, cx);
                    }
                    if options.bench {
                        let script = bench_script();
                        entity.update(cx, |editor, _| editor.stats_enabled = true);
                        run_bench(entity.clone(), script, 0, Phase::new("warmup"), window, cx);
                    }
                });
            })
            .expect("初始化窗口失败");
        if let Some(seconds) = options.exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(seconds)).await;
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}

fn report_first_frame(entity: &Entity<Editor>, started: Instant, cx: &mut App) {
    let editor = entity.read(cx);
    let first = editor.first_frame_at.map_or(0.0, |at| (at - started).as_secs_f64() * 1000.0);
    eprintln!("{{\"event\":\"first_frame\",\"since_process_start_ms\":{first:.1}}}");
}

// ---- 输入法协议自测 ----

/// 按 macOS 拼音输入法实际的调用顺序驱动输入处理器：逐步 setMarkedText、清空组字、
/// 以显式范围替换已有组字（gpui 示例在此越界），最后 insertText 提交并检查撤销。
fn ime_selftest(entity: &Entity<Editor>, window: &mut Window, cx: &mut App) {
    entity.update(cx, |editor, cx| {
        let original = editor.text();
        let mut failures = Vec::new();
        let mut check = |name: &str, ok: bool| {
            if !ok {
                failures.push(name.to_string());
            }
        };
        for (text, caret) in [("n", 1), ("ni", 2), ("ni h", 4), ("ni hao", 6)] {
            editor.replace_and_mark_text_in_range(None, text, Some(caret..caret), window, cx);
        }
        check("marked=0..6", editor.marked_text_range(window, cx) == Some(0..6));
        check("selected=6", editor.selected_text_range(false, window, cx).map(|s| s.range) == Some(6..6));
        check("bounds_for_range", editor.bounds_for_range(0..6, window.bounds(), window, cx).is_some());
        editor.replace_text_in_range(None, "你好", window, cx);
        check("commit 你好", editor.text() == format!("你好{original}"));
        check("marked cleared", editor.marked_text_range(window, cx).is_none());

        editor.replace_and_mark_text_in_range(None, "z", Some(1..1), window, cx);
        editor.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
        check("cancel restores", editor.text() == format!("你好{original}"));
        editor.replace_and_mark_text_in_range(None, "zh", Some(2..2), window, cx);
        let marked = editor.marked_text_range(window, cx);
        check("marked=2..4", marked == Some(2..4));
        // 显式传入已有组字范围并给出新选区：gpui 示例在这里会把选区终点算到文本之外。
        editor.replace_and_mark_text_in_range(marked, "zhong", Some(5..5), window, cx);
        check("selected after explicit replace", editor.selected_text_range(false, window, cx).map(|s| s.range) == Some(7..7));
        editor.replace_text_in_range(None, "中", window, cx);
        check("commit 中", editor.text() == format!("你好中{original}"));
        let mut actual = None;
        check(
            "text_for_range",
            editor.text_for_range(0..3, &mut actual, window, cx).as_deref() == Some("你好中") && actual == Some(0..3),
        );
        editor.undo_once(cx);
        check("undo 中", editor.text() == format!("你好{original}"));
        editor.undo_once(cx);
        check("undo 你好", editor.text() == original);
        eprintln!(
            "{{\"event\":\"ime_selftest\",\"pass\":{},\"failures\":{:?}}}",
            failures.is_empty(),
            failures
        );
    });
}

// ---- 逐帧基准 ----

#[derive(Clone, Copy)]
enum Step {
    Phase(&'static str),
    Jump(f64),
    Scroll(f32),
    Idle,
    CheckEnd,
    PlaceCursor,
    Type,
    UndoAll,
}

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn bench_script() -> Vec<Step> {
    let mut steps = vec![Step::Idle; 10];
    steps.push(Step::Phase("jump"));
    for fraction in [0.25, 0.5, 0.75, 1.0] {
        steps.push(Step::Jump(fraction));
        steps.extend([Step::Idle; 15]);
    }
    steps.push(Step::CheckEnd);
    steps.push(Step::Phase("scroll_40px"));
    steps.push(Step::Jump(0.0));
    steps.extend([Step::Scroll(40.0); 900]);
    steps.push(Step::Phase("fling_4000px"));
    steps.extend([Step::Scroll(4000.0); 150]);
    steps.push(Step::Phase("type_mid"));
    steps.push(Step::Jump(0.5));
    steps.extend([Step::Idle; 5]);
    steps.push(Step::PlaceCursor);
    steps.push(Step::Idle);
    steps.extend([Step::Type; 60]);
    steps.push(Step::Phase("undo"));
    steps.push(Step::UndoAll);
    steps.extend([Step::Idle; 5]);
    steps.push(Step::Phase("done"));
    steps
}

struct Phase {
    name: &'static str,
    frame_start: usize,
    end_reached: Option<bool>,
}

impl Phase {
    fn new(name: &'static str) -> Self {
        Self { name, frame_start: 0, end_reached: None }
    }
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

fn report_phase(editor: &Editor, phase: &Phase) {
    let to_ms = |values: &[Duration]| values.iter().map(|d| d.as_secs_f64() * 1000.0).collect::<Vec<_>>();
    let start = phase.frame_start;
    let mut frame = to_ms(&editor.stats.frame[start.min(editor.stats.frame.len())..]);
    let mut layout = to_ms(&editor.stats.layout[start.min(editor.stats.layout.len())..]);
    let mut interval = to_ms(&editor.stats.interval[start.saturating_sub(1).min(editor.stats.interval.len())..]);
    let median_interval = percentile(&mut interval.clone(), 0.5);
    let janky = interval.iter().filter(|value| **value > median_interval * 1.5 + 1.0).count();
    let (entries, bytes, hits, misses) = editor.cache_summary();
    eprintln!(
        "{{\"event\":\"phase\",\"name\":\"{}\",\"frames\":{},\"element_ms\":{{\"p50\":{:.2},\"p95\":{:.2},\"max\":{:.2}}},\"layout_ms\":{{\"p50\":{:.2},\"p95\":{:.2},\"max\":{:.2}}},\"interval_ms\":{{\"p50\":{:.2},\"p95\":{:.2},\"max\":{:.2}}},\"janky_frames\":{janky},\"cache\":{{\"entries\":{entries},\"bytes\":{bytes},\"hits\":{hits},\"misses\":{misses}}},\"kp\":{{\"paragraphs\":{},\"total_ms\":{:.1},\"max_ms\":{:.2}}},\"end_reached\":{}}}",
        phase.name,
        frame.len(),
        percentile(&mut frame, 0.5), percentile(&mut frame, 0.95), percentile(&mut frame, 1.0),
        percentile(&mut layout, 0.5), percentile(&mut layout, 0.95), percentile(&mut layout, 1.0),
        median_interval, percentile(&mut interval, 0.95), percentile(&mut interval, 1.0),
        editor.stats.kp_paragraphs,
        editor.stats.kp_time.as_secs_f64() * 1000.0,
        editor.stats.kp_max.as_secs_f64() * 1000.0,
        phase.end_reached.map_or("null".to_string(), |value| value.to_string()),
    );
}

/// 每帧执行一步脚本；步骤修改编辑器状态并请求重绘，下一帧回调再推进。
fn run_bench(entity: Entity<Editor>, script: Vec<Step>, index: usize, mut phase: Phase, window: &mut Window, cx: &mut App) {
    let Some(step) = script.get(index).copied() else {
        entity.update(cx, |editor, _| report_phase(editor, &phase));
        cx.quit();
        return;
    };
    entity.update(cx, |editor, cx| match step {
        Step::Phase(name) => {
            if phase.name != "warmup" {
                report_phase(editor, &phase);
            }
            phase = Phase { name, frame_start: editor.stats.frame.len(), end_reached: None };
            cx.notify();
        }
        Step::Jump(fraction) => editor.jump_to_fraction(fraction, cx),
        Step::Scroll(dy) => editor.scroll_by(dy, cx),
        Step::Idle => cx.notify(),
        Step::CheckEnd => {
            phase.end_reached = Some(editor.visible_text_contains("NOMO_END"));
            cx.notify();
        }
        Step::Type => {
            // 光标放在中部可见区第一段段首后连续输入，模拟逐字键入（不经过输入法组字）。
            editor.replace_text_in_range(None, "测", window, cx);
        }
        Step::PlaceCursor => {
            editor.place_cursor_at_visible_top();
            editor.bench_text_hash = Some(text_hash(&editor.text()));
            cx.notify();
        }
        Step::UndoAll => {
            for _ in 0..60 {
                editor.undo_once(cx);
            }
            // 撤销阶段的 end_reached 表示全文是否与输入前逐字节一致。
            phase.end_reached = Some(editor.bench_text_hash == Some(text_hash(&editor.text())));
            cx.notify();
        }
    });
    window.on_next_frame(move |window, cx| run_bench(entity, script, index + 1, phase, window, cx));
}

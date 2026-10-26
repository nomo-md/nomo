//! 复用 test-lab 语料的定向验收，不把解析检查冒充桌面排版或加载调度验收。
use super::{
    commands::{self, EditIntent},
    native_port::NativeWindowPort,
    parse::ParsedWindow,
    protocol::*,
};
use pulldown_cmark::{Event, Tag};
use std::{path::PathBuf, time::Instant};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-lab/fixtures/main")
}

#[test]
#[ignore = "显式运行：依赖已有 test-lab 大型语料，仅验收解析和补丁"]
fn supplied_giant_structures_keep_source_and_positions() {
    for (path, kind, minimum) in [
        ("08_边界与兼容/01_单段落1MiB.md", "paragraph", 1),
        ("04_代码公式与图表/03_单个25000行代码块.md", "code", 1),
        (
            "02_表格专项/04_极限长表格_10000行12列.md",
            "table-cell",
            120012,
        ),
        ("02_表格专项/02_超宽表格_120行64列.md", "table-cell", 7744),
        ("01_基础与节点/06_200折叠块与12层嵌套.md", "details", 212),
        ("04_代码公式与图表/08_120张不同Mermaid图.md", "code", 120),
        ("03_图片专项/07_图文瀑布流120段.md", "image", 120),
    ] {
        let text = std::fs::read_to_string(corpus().join(path)).expect("fixture must exist");
        let started = Instant::now();
        let parsed = ParsedWindow::parse(
            DocumentWindow {
                inline_templates: Vec::new(),
                document_id: "corpus".into(),
                revision: 0,
                from_byte: 0,
                to_byte: text.len() as u64,
                text: text.clone(),
            },
            path,
            0,
            WindowContext::Complete,
        )
        .unwrap();
        assert_eq!(parsed.source.as_ref(), text, "{path}");
        let count = if kind == "details" {
            parsed.details.len()
        } else {
            parsed.nodes.iter().filter(|n| n.kind == kind).count()
        };
        assert!(count >= minimum, "{path}: expected {minimum}, got {count}");
        for (_, range) in &parsed.events {
            assert!(
                range.start <= range.end && range.end <= text.len(),
                "{path}: {range:?}"
            );
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        }
        for span in &parsed.inline {
            assert!(span.open.end <= span.content.start && span.content.end <= span.close.start);
        }
        if path.contains("25000") {
            let mut in_code = false;
            let mut lines = 0;
            for (event, _) in &parsed.events {
                match event {
                    Event::Start(Tag::CodeBlock(_)) => in_code = true,
                    Event::End(pulldown_cmark::TagEnd::CodeBlock) => in_code = false,
                    Event::Text(s) if in_code => lines += s.lines().count(),
                    _ => {}
                }
            }
            assert_eq!(lines, 25000);
        }
        let at = parsed
            .events
            .iter()
            .find_map(|(event, range)| matches!(event, Event::Text(_)).then_some(range.start))
            .unwrap();
        let edit = commands::apply(
            &parsed,
            DocumentSelection::caret(at as u64),
            EditIntent::Replace("验收😀".into()),
        )
        .unwrap();
        assert_eq!(edit.edits.len(), 1);
        assert_eq!(edit.edits[0].from_byte, at as u64);
        assert_eq!(edit.edits[0].to_byte, at as u64);
        println!(
            "{path}: bytes={}, nodes={}, estimated_bytes={}, parse_and_patch_ms={}",
            text.len(),
            parsed.nodes.len(),
            parsed.estimated_bytes(),
            started.elapsed().as_millis()
        );
    }
}

#[test]
#[ignore = "显式运行：50 MiB/10 MiB 会话窗口和完整快照，非滚动加载验收"]
fn large_documents_use_host_windows_and_full_snapshots() {
    let root = std::env::temp_dir().join(format!(
        "nomo-control-corpus-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for path in [
        "05_纯文本体量阶梯/50MiB_纯文本.md",
        "06_综合混合压力/10MiB_全节点混合.md",
    ] {
        let started = Instant::now();
        let input = corpus().join(path).canonicalize().unwrap();
        let port = NativeWindowPort::open(
            root.join("sessions"),
            "corpus".into(),
            Some(input.clone()),
            None,
        )
        .unwrap();
        let status = port.status().unwrap();
        // 测试给定明确字节范围；不在适配层增加窗口选择、结构索引或预取策略。
        let window = port.read_window(status.revision, 0..0).unwrap();
        assert!(window.text.is_empty());
        let tx = DocumentTransaction {
            template_update: None,
            base_revision: status.revision,
            operation_id: "insert".into(),
            history_group_id: "corpus-input".into(),
            edits: vec![SourceEdit {
                from_byte: 0,
                to_byte: 0,
                inserted_text: "控件验收\n\n".into(),
            }],
            selection_before: DocumentSelection::caret(0),
            selection_after: DocumentSelection::caret(14),
        };
        let revision = port.submit(tx.clone()).unwrap();
        assert_eq!(port.submit(tx).unwrap(), revision);
        assert_eq!(
            port.status().unwrap().byte_length,
            status.byte_length + "控件验收\n\n".len() as u64
        );
        let (revision, _) = port.history(revision, false).unwrap();
        let sample = port.read_window(revision, 0..1024).unwrap();
        assert!(sample.text.len() <= 1027 && sample.text.len() > 1000);
        let output = root.join(input.file_name().unwrap());
        port.save_to(output.clone()).unwrap();
        assert_eq!(
            std::fs::metadata(&output).unwrap().len(),
            std::fs::metadata(&input).unwrap().len()
        );
        // 逐块比较，测试也不常驻两份完整正文。
        use std::io::Read;
        let mut expected = std::fs::File::open(&input).unwrap();
        let mut actual = std::fs::File::open(&output).unwrap();
        let mut a = vec![0; 64 * 1024];
        let mut b = vec![0; 64 * 1024];
        loop {
            let n = expected.read(&mut a).unwrap();
            let m = actual.read(&mut b).unwrap();
            assert_eq!(n, m);
            assert_eq!(a[..n], b[..m]);
            if n == 0 {
                break;
            }
        }
        drop(expected);
        drop(actual);
        port.close(false).unwrap();
        println!(
            "{path}: host_bytes={}, local_window_bytes={}, snapshot_and_history_ms={}",
            status.byte_length,
            sample.text.len(),
            started.elapsed().as_millis()
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

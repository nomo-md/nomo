use super::*;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc},
    time::Instant,
};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nomo-stats-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn measure(
    path: &Path,
    root: &Path,
    range: Option<ByteRange>,
) -> TextDocumentResult<(DocumentStats, Option<DocumentStats>)> {
    let cancel = Arc::new(AtomicBool::new(false));
    let file = File::open(path)?;
    let mut source =
        source::Source::file(file.try_clone()?, file.metadata()?.len(), cancel.clone());
    let (len, headings, reused) = scan::build(&mut source, root, None)?;
    let totals = scan::summarize(root, len, reused, cancel.clone())?;
    let (words, visible_chars) = totals;
    let (sw, sc) = if range.is_some() {
        let (w, v, sw, sc) = count::count(root, 0, len, range, cancel)?;
        assert_eq!((w, v), totals);
        (sw, sc)
    } else {
        (0, 0)
    };
    let (chars, lines) = source.raw_counts(0, source.len)?;
    let full = DocumentStats {
        words,
        visible_chars,
        chars,
        lines,
        headings,
        reading_minutes: 0,
    }
    .finish();
    let selected = range.map(|r| {
        let (chars, lines) = source.raw_counts(r.from, r.to).unwrap();
        DocumentStats {
            words: sw,
            visible_chars: sc,
            chars,
            lines,
            headings: 0,
            reading_minutes: 0,
        }
        .finish()
    });
    Ok((full, selected))
}
#[test]
fn source_semantics_match_typescript_oracle() {
    let cases: serde_json::Value = serde_json::from_str(include_str!("golden.json")).unwrap();
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        let fixture = Fixture::new();
        let markdown = if let Some(path) = case["path"].as_str() {
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()
        } else {
            case["markdown"].as_str().unwrap().to_owned()
        };
        let path = fixture.0.join("input");
        fs::write(&path, &markdown).unwrap();
        let (actual, _) = measure(&path, &fixture.0, None).unwrap();
        let expected: DocumentStats = serde_json::from_value(case["expected"].clone()).unwrap();
        if actual != expected {
            failures.push(format!(
                "{:?}\nactual={actual:?}\nexpected={expected:?}",
                case["path"].as_str().unwrap_or(&markdown)
            ));
        }
        let length = fs::metadata(fixture.0.join("body")).unwrap().len();
        for selection in case["selections"].as_array().into_iter().flatten() {
            let range = ByteRange {
                from: selection["from"].as_u64().unwrap(),
                to: selection["to"].as_u64().unwrap(),
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let (_, _, words, visible_chars) =
                count::count(&fixture.0, 0, length, Some(range), cancel.clone()).unwrap();
            let mut source =
                source::Source::file(File::open(&path).unwrap(), markdown.len() as u64, cancel);
            let (chars, lines) = source.raw_counts(range.from, range.to).unwrap();
            let selected = DocumentStats {
                words,
                visible_chars,
                chars,
                lines,
                headings: if range.from == 0 && range.to == markdown.len() as u64 {
                    actual.headings
                } else {
                    0
                },
                reading_minutes: 0,
            }
            .finish();
            let expected: DocumentStats =
                serde_json::from_value(selection["expected"].clone()).unwrap();
            if selected != expected {
                failures.push(format!(
                    "selection {:?} {range:?}\nactual={selected:?}\nexpected={expected:?}",
                    case["path"].as_str().unwrap_or(&markdown)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn selection_keeps_source_context() {
    for (markdown, text, words, visible) in [
        ("[链接](https://example.com)", "example.com", 0, 0),
        ("hel**lo**", "lo", 1, 2),
        ("hel**lo**", "**", 0, 0),
        (
            "| A | A |\r\n| --- | --- |\r\n| a\\|b | a\\|b |",
            "a\\|b",
            2,
            3,
        ),
        ("<div>e&#769; &amp;</div>", "&#769;", 1, 1),
    ] {
        let fixture = Fixture::new();
        let path = fixture.0.join("input");
        fs::write(&path, markdown).unwrap();
        let from = markdown.find(text).unwrap() as u64;
        let (_, selected) = measure(
            &path,
            &fixture.0,
            Some(ByteRange {
                from,
                to: from + text.len() as u64,
            }),
        )
        .unwrap();
        let selected = selected.unwrap();
        assert_eq!(
            (selected.words, selected.visible_chars),
            (words, visible),
            "{markdown}"
        );
    }
}
#[test]
#[ignore = "explicit 10 MiB fixture validation"]
fn original_mixed_fixture() {
    let path = PathBuf::from(std::env::var("NOMO_STATS_FIXTURE").unwrap());
    let fixture = Fixture::new();
    let start = Instant::now();
    let allocations = allocation_test::Scope::start();
    let result = measure(&path, &fixture.0, None).unwrap().0;
    let peak = allocations.peak();
    drop(allocations);
    println!(
        "{} elapsedMs={} allocatedPeakBytes={peak}",
        serde_json::to_string(&result).unwrap(),
        start.elapsed().as_millis()
    );
    assert!(peak <= MEMORY_BUDGET, "statistics heap peak {peak}");
    assert_eq!(
        result,
        DocumentStats {
            chars: 7_874_154,
            words: 1_534_780,
            visible_chars: 4_973_320,
            lines: 195_522,
            headings: 4_225,
            reading_minutes: 5_482
        }
    );
}

#[test]
#[ignore = "explicit 10 MiB incremental fixture validation"]
fn original_mixed_incremental_fixture() {
    use super::super::markdown_session::{
        MarkdownDocumentSessionManager, MarkdownEdit, MarkdownTransaction,
    };
    use std::io::Read;
    crate::async_runtime::block_on(async {
        let fixture = Fixture::new();
        let path = fixture.0.join("copy.md");
        fs::copy(std::env::var("NOMO_STATS_FIXTURE").unwrap(), &path).unwrap();
        let mut prefix = [0; 4096];
        File::open(&path).unwrap().read_exact(&mut prefix).unwrap();
        let marker = "NOMO_START — ".as_bytes();
        let position = prefix
            .windows(marker.len())
            .position(|s| s == marker)
            .unwrap()
            + marker.len();
        let manager = MarkdownDocumentSessionManager::new(fixture.0.join("sessions")).unwrap();
        let doc = manager.open("main", Some(path), None).unwrap();
        let query = StatsQuery {
            document_id: doc.document_id.clone(),
            revision: 0,
            request_id: 1,
            selection_version: 0,
            selection: None,
        };
        let baseline = manager
            .writing_stats("main".into(), query.clone())
            .await
            .unwrap();
        manager
            .apply(
                "main",
                MarkdownTransaction {
                    template_update: None,
                    derived: false,
                    document_id: doc.document_id.clone(),
                    base_revision: 0,
                    operation_id: "insert".into(),
                    history_group_id: "insert".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: position as u64,
                        to_byte: position as u64,
                        inserted_text: "新增 ".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let result = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: 1,
                    request_id: 2,
                    ..query
                },
            )
            .await
            .unwrap();
        assert_eq!(result.full.words, baseline.full.words + 2);
        assert_eq!(result.full.visible_chars, baseline.full.visible_chars + 2);
        assert_eq!(result.full.chars, baseline.full.chars + 3);
        assert!(result.metrics.reused_blocks > 100_000);
        println!(
            "incremental elapsedMs={} reusedBlocks={}",
            result.metrics.elapsed_ms, result.metrics.reused_blocks
        );
        manager.close("main", &doc.document_id, true).unwrap();
        drop(manager);
    });
}

#[test]
fn chunk_boundaries_and_unbounded_graphemes_keep_counts_and_memory() {
    for padding in [CHUNK - 11, CHUNK - 3, CHUNK - 1, CHUNK, CHUNK + 2] {
        let fixture = Fixture::new();
        let path = fixture.0.join("input");
        let markdown = format!("{} **中e\u{301}👨‍👩‍👧‍👦**\r\n\n", "x".repeat(padding));
        fs::write(&path, &markdown).unwrap();
        let allocations = allocation_test::Scope::start();
        let (actual, _) = measure(&path, &fixture.0, None).unwrap();
        let peak = allocations.peak();
        drop(allocations);
        assert_eq!(
            (
                actual.words,
                actual.visible_chars,
                actual.chars,
                actual.lines
            ),
            (
                3,
                padding as u64 + 3,
                markdown.encode_utf16().count() as u64,
                3
            )
        );
        assert!(peak <= MEMORY_BUDGET, "{peak}");
    }
    let fixture = Fixture::new();
    let path = fixture.0.join("input");
    let text = format!("e{}末", "\u{301}".repeat(CHUNK));
    fs::write(&path, text).unwrap();
    let allocations = allocation_test::Scope::start();
    let (actual, _) = measure(&path, &fixture.0, None).unwrap();
    let peak = allocations.peak();
    drop(allocations);
    assert_eq!((actual.words, actual.visible_chars), (2, 2));
    assert!(peak <= MEMORY_BUDGET, "{peak}");
    let fixture = Fixture::new();
    let path = fixture.0.join("input");
    let text = format!("👨{}\u{200d}👩", "\u{301}".repeat(CHUNK));
    fs::write(&path, text).unwrap();
    let allocations = allocation_test::Scope::start();
    let (actual, _) = measure(&path, &fixture.0, None).unwrap();
    let peak = allocations.peak();
    drop(allocations);
    assert_eq!((actual.words, actual.visible_chars), (0, 1));
    assert!(peak <= MEMORY_BUDGET, "{peak}");
}

#[test]
fn html_attributes_and_mark_stack_spill_without_changing_counts() {
    for (markdown, words, visible) in [
        (
            format!("<div class=\"{}\">hello</div>", "x".repeat(CHUNK * 2)),
            1,
            5,
        ),
        (
            format!(
                "before {}hello{} after",
                "<em>".repeat(5000),
                "</em>".repeat(5000)
            ),
            3,
            16,
        ),
    ] {
        let fixture = Fixture::new();
        let path = fixture.0.join("input");
        fs::write(&path, markdown).unwrap();
        let allocations = allocation_test::Scope::start();
        let (actual, _) = measure(&path, &fixture.0, None).unwrap();
        let peak = allocations.peak();
        drop(allocations);
        assert_eq!((actual.words, actual.visible_chars), (words, visible));
        assert!(peak <= MEMORY_BUDGET, "{peak}");
        assert!(!fixture.0.join("inline-html-stack").exists());
        assert!(!fixture.0.join("html-stack").exists());
    }
}

#[test]
fn huge_table_and_forward_references_are_disk_backed() {
    use std::io::Write;
    let fixture = Fixture::new();
    let path = fixture.0.join("input");
    let mut file = File::create(&path).unwrap();
    write!(file, "| a | b |\n| --- | --- |\n").unwrap();
    for _ in 0..10000 {
        writeln!(file, "| a\\|b | `é` |").unwrap();
    }
    writeln!(file).unwrap();
    for i in 0..10000 {
        writeln!(file, "[word][ref{i}]\n").unwrap();
    }
    for i in 0..10000 {
        writeln!(file, "[ref{i}]: https://example.com").unwrap();
    }
    drop(file);
    let allocations = allocation_test::Scope::start();
    let (actual, _) = measure(&path, &fixture.0, None).unwrap();
    let peak = allocations.peak();
    drop(allocations);
    assert_eq!((actual.words, actual.visible_chars), (40002, 80002));
    assert!(peak <= MEMORY_BUDGET, "{peak}");
}

#[test]
fn native_revision_ownership_unsaved_history_and_incremental_cache() {
    use super::super::markdown_session::{
        MarkdownDocumentSessionManager, MarkdownEdit, MarkdownTransaction,
    };
    crate::async_runtime::block_on(async {
        let fixture = Fixture::new();
        let manager = MarkdownDocumentSessionManager::new(fixture.0.join("sessions")).unwrap();
        let doc = manager
            .open(
                "main",
                None,
                Some("# 标题\n\nhello **world**\n\n[link][ref]\n\n[ref]: /ok\n".into()),
            )
            .unwrap();
        let query = StatsQuery {
            document_id: doc.document_id.clone(),
            revision: 0,
            request_id: 1,
            selection_version: 0,
            selection: None,
        };
        assert_eq!(
            manager
                .writing_stats("other".into(), query.clone())
                .await
                .unwrap_err()
                .code,
            "document-owner-conflict"
        );
        let first = manager
            .writing_stats("main".into(), query.clone())
            .await
            .unwrap();
        assert_eq!(first.full.words, 5);
        let again = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    request_id: 2,
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert!(again.metrics.cache_hit);
        let edited = manager
            .apply(
                "main",
                MarkdownTransaction {
                    template_update: None,
                    derived: false,
                    document_id: doc.document_id.clone(),
                    base_revision: 0,
                    operation_id: "edit-stats".into(),
                    history_group_id: "edit-stats".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: 0,
                        to_byte: 0,
                        inserted_text: "新增\n\n".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        assert_eq!(
            manager
                .writing_stats("main".into(), query.clone())
                .await
                .unwrap_err()
                .code,
            "revision-conflict"
        );
        let next = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: edited.revision,
                    request_id: 3,
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(next.full.words, first.full.words + 2);
        assert!(next.metrics.reused_blocks > 0);
        let all = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: edited.revision,
                    request_id: 4,
                    selection: Some(ByteRange {
                        from: edited.byte_length,
                        to: 0,
                    }),
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(all.selected, Some(next.full.clone()));
        let undone = manager
            .history("main", &doc.document_id, edited.revision, false)
            .unwrap();
        let restored = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: undone.revision,
                    request_id: 5,
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(restored.full, first.full);
        let redone = manager
            .history("main", &doc.document_id, undone.revision, true)
            .unwrap();
        let restored = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: redone.revision,
                    request_id: 6,
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(restored.full, next.full);
        // A plain edit inside an existing paragraph resumes that checkpoint;
        // shifted cached spans must still answer a reversed partial-word query.
        let offset = "新增\n\n# 标题\n\nhe".len() as u64;
        let local = manager
            .apply(
                "main",
                MarkdownTransaction {
                    template_update: None,
                    derived: false,
                    document_id: doc.document_id.clone(),
                    base_revision: redone.revision,
                    operation_id: "local-stats".into(),
                    history_group_id: "local-stats".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: offset,
                        to_byte: offset,
                        inserted_text: "x".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let changed = manager
            .writing_stats(
                "main".into(),
                StatsQuery {
                    revision: local.revision,
                    request_id: 7,
                    selection: Some(ByteRange {
                        from: offset + 1,
                        to: offset,
                    }),
                    ..query.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(changed.full.words, next.full.words);
        assert_eq!(changed.full.visible_chars, next.full.visible_chars + 1);
        assert_eq!(changed.selected.unwrap().words, 1);
        assert!(changed.metrics.reused_blocks > 0);
        assert_eq!(
            fs::read_dir(
                fixture
                    .0
                    .join("sessions")
                    .join(&doc.document_id)
                    .join("writing-stats")
            )
            .unwrap()
            .count(),
            1
        );
        manager.close("main", &doc.document_id, true).unwrap();
        drop(manager);
    });
}

#[test]
fn failed_scratch_writes_and_cancelled_requests_release_the_session() {
    use super::super::markdown_session::MarkdownDocumentSessionManager;
    crate::async_runtime::block_on(async {
        let fixture = Fixture::new();
        let root = fixture.0.join("sessions");
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let doc = manager
            .open("main", None, Some("hello world".repeat(10000)))
            .unwrap();
        let query = StatsQuery {
            document_id: doc.document_id.clone(),
            revision: 0,
            request_id: 1,
            selection_version: 0,
            selection: None,
        };
        let scratch = root.join(&doc.document_id).join("writing-stats");
        fs::write(&scratch, "write failure fixture").unwrap();
        assert!(manager
            .writing_stats("main".into(), query.clone())
            .await
            .is_err());
        fs::remove_file(&scratch).unwrap();
        let copy = manager.clone();
        let waiting =
            crate::async_runtime::spawn(
                async move { copy.writing_stats("main".into(), query).await },
            );
        std::thread::sleep(std::time::Duration::from_millis(5));
        manager.cancel_writing_stats("main", Some(1));
        assert_eq!(waiting.await.unwrap().unwrap_err().code, "stats-cancelled");
        if scratch.exists() {
            assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
        }
        manager.close("main", &doc.document_id, true).unwrap();
        drop(manager);
    });
}

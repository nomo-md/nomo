use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    manager: MarkdownDocumentSessionManager,
    status: MarkdownStatus,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let root = std::env::temp_dir().join(id("nomo-search-test"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let status = manager.open("main", None, Some(text.into())).unwrap();
        Self {
            root,
            manager,
            status,
        }
    }
    fn query(&self, query: &str) -> SearchQuery {
        SearchQuery {
            document_id: self.status.document_id.clone(),
            base_revision: self.status.revision,
            request_id: id("request"),
            query: query.into(),
            case_sensitive: false,
            whole_word: false,
        }
    }
    fn prepare(&self, query: &str, replacement: &str) -> TaskIdentity {
        let task = self
            .manager
            .prepare_replace_all(
                "main",
                ReplaceQuery {
                    search: self.query(query),
                    replacement: replacement.into(),
                    operation_id: "operation".into(),
                    selection: Some(MarkdownSelection {
                        anchor_byte: 0,
                        head_byte: self.status.byte_length,
                    }),
                    anchors: vec![self.status.byte_length],
                },
                None,
            )
            .unwrap();
        self.ready(&task.task_id, "prepared");
        task
    }
    fn ready(&self, id: &str, state: &str) {
        let started = Instant::now();
        loop {
            let progress = self.manager.search_progress("main", id).unwrap();
            if progress.state == state {
                return;
            }
            assert!(
                !["failed", "conflict", "cancelled"].contains(&progress.state.as_str()),
                "{progress:?}"
            );
            assert!(started.elapsed() < Duration::from_secs(40), "{progress:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn text(&self) -> String {
        let handle = self.manager.session(&self.status.document_id).unwrap();
        let snapshot = lock(&handle).unwrap().current.clone();
        String::from_utf8(snapshot.read_range(0, snapshot.len() as usize).unwrap()).unwrap()
    }
    fn commit(&mut self, task: &TaskIdentity) -> ReplaceResult {
        let result = self
            .manager
            .commit_replace_all("main", &self.status.document_id, &task.task_id, "operation")
            .unwrap();
        self.status = result.status.clone();
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.search.cancel_owner("main");
        let _ = self.manager.close("main", &self.status.document_id, true);
        // 工作线程在终态事件后还可能短暂持有会话引用。
        for _ in 0..50 {
            if !self.root.exists() || fs::remove_dir_all(&self.root).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[test]
fn unicode_samples_share_the_browser_contract() {
    let samples: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../src/lib/markdown-document/searchSamples.json"
    ))
    .unwrap();
    for sample in samples.as_array().unwrap() {
        let fixture = Fixture::new(sample["source"].as_str().unwrap());
        let mut query = fixture.query(sample["query"].as_str().unwrap());
        query.case_sensitive = sample["caseSensitive"].as_bool().unwrap_or(false);
        query.whole_word = sample["wholeWord"].as_bool().unwrap_or(false);
        let task = fixture.manager.start_search("main", query, None).unwrap();
        fixture.ready(&task.task_id, "ready");
        let page = fixture
            .manager
            .search_page("main", &task.task_id, 0, 128)
            .unwrap();
        assert_eq!(
            page.total_count,
            sample["count"].as_u64().unwrap(),
            "{sample}"
        );
        let handle = fixture
            .manager
            .session(&fixture.status.document_id)
            .unwrap();
        for hit in page.matches {
            let snapshot = lock(&handle).unwrap().current.clone();
            assert!(boundary(&snapshot, hit.from_byte).unwrap());
            assert!(boundary(&snapshot, hit.to_byte).unwrap());
        }
    }
}

#[test]
fn chunk_edges_long_query_and_nonoverlap_resume() {
    let long = "中".repeat(24_000);
    let text = format!("{}😀{}\r\n{}", "x".repeat(65533), long, "a".repeat(200_003));
    let fixture = Fixture::new(&text);
    for (query, expected) in [(format!("😀{long}\r\n"), 1), ("aaa".into(), 66667)] {
        let task = fixture
            .manager
            .start_search("main", fixture.query(&query), None)
            .unwrap();
        fixture.ready(&task.task_id, "ready");
        let page = fixture
            .manager
            .search_page("main", &task.task_id, expected - 1, 128)
            .unwrap();
        assert_eq!(page.total_count, expected);
        assert_eq!(page.matches.len(), 1);
        assert_eq!(page.matches[0].index, expected - 1);
    }
}

#[test]
fn million_hits_keep_only_checkpoints_and_a_bounded_page() {
    let fixture = Fixture::new(&"a ".repeat(1_000_000));
    let identity = fixture
        .manager
        .start_search("main", fixture.query("a"), None)
        .unwrap();
    fixture.ready(&identity.task_id, "ready");
    let task = fixture
        .manager
        .search
        .get("main", &identity.task_id)
        .unwrap();
    let data = lock(&task.data).unwrap();
    let index = data.index.as_ref().unwrap();
    assert_eq!(index.count, 1_000_000);
    assert!(index.checkpoints.len() <= 4096);
    assert!(index.checkpoints.capacity() * std::mem::size_of::<Checkpoint>() <= 128 * 1024);
    drop(data);
    for from in [0, 128, 99_999, 999_872, 1_000_000] {
        let page = fixture
            .manager
            .search_page("main", &identity.task_id, from, 1000000)
            .unwrap();
        assert!(page.matches.len() <= 128);
        assert_eq!(page.total_count, 1_000_000);
        for hit in page.matches {
            assert_eq!(hit.from_byte, hit.index * 2);
        }
    }
}

#[test]
fn crlf_replacements_publish_once_and_undo_redo_as_one_group() {
    for (text, query, replacement) in [
        ("a\r\nb\r\n", "\n", ""),
        ("a\r\nb\n", "\r", "!"),
        ("\r\n\r\n\r\n", "\n\r", "x"),
        ("xx", "x", "\r\n"),
        ("a\nb", "a", "\r"),
        ("猫😀猫", "猫", "dog"),
    ] {
        let mut fixture = Fixture::new(text);
        let task = fixture.prepare(query, replacement);
        assert_eq!(fixture.text(), text);
        assert_eq!(
            fixture
                .manager
                .status("main", &fixture.status.document_id)
                .unwrap()
                .revision,
            0
        );
        let result = fixture.commit(&task);
        assert_eq!(result.status.revision, 1);
        assert_eq!(fixture.text(), text.replace(query, replacement));
        let handle = fixture
            .manager
            .session(&fixture.status.document_id)
            .unwrap();
        validate_selection(
            &lock(&handle).unwrap().current,
            result.status.selection.as_ref(),
        )
        .unwrap();
        fixture.status = fixture
            .manager
            .history("main", &fixture.status.document_id, 1, false)
            .unwrap();
        assert_eq!(fixture.text(), text);
        fixture.status = fixture
            .manager
            .history("main", &fixture.status.document_id, 2, true)
            .unwrap();
        assert_eq!(fixture.text(), text.replace(query, replacement));
    }
}

#[test]
fn batched_history_survives_eviction_recovery_and_save() {
    let text = "a ".repeat(10_000);
    let mut fixture = Fixture::new(&text);
    let task = fixture.prepare("a", "中");
    let result = fixture.commit(&task);
    assert_eq!(result.count, 10000);
    assert_eq!(result.status.revision, 1);
    let handle = fixture
        .manager
        .session(&fixture.status.document_id)
        .unwrap();
    let mut batches = 0;
    let mut cursor = lock(&handle).unwrap().manifest.undo;
    while let Some(offset) = cursor {
        let entry = lock(&handle).unwrap().history.read(offset).unwrap();
        assert!(entry.edits.len() <= 4096);
        assert!(serde_json::to_vec(&entry).unwrap().len() <= 1024 * 1024);
        cursor = entry.parent_undo;
        batches += 1;
    }
    assert!(batches > 1);
    lock(&handle).unwrap().history.evict_cache();
    drop(handle);
    fixture.status = fixture
        .manager
        .history("main", &fixture.status.document_id, 1, false)
        .unwrap();
    assert_eq!(fixture.text(), text);
    fixture.status = fixture
        .manager
        .history("main", &fixture.status.document_id, 2, true)
        .unwrap();
    fixture
        .manager
        .detach("main", &fixture.status.document_id)
        .unwrap();
    fixture.status = fixture
        .manager
        .recover("main", &fixture.status.document_id)
        .unwrap();
    assert_eq!(fixture.text(), text.replace('a', "中"));
    assert!(!fixture.status.can_undo); // 保留既有重启恢复契约。
    let snapshot = fixture
        .manager
        .capture("main", &fixture.status.document_id)
        .unwrap();
    let path = fixture.root.join("saved.md");
    fixture.status = fixture
        .manager
        .save("main", &snapshot.snapshot_id, path.clone(), false)
        .unwrap();
    fixture
        .manager
        .release("main", &snapshot.snapshot_id)
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), text.replace('a', "中"));
    assert!(!fixture.status.dirty);
}

#[test]
fn cancelled_or_stale_preparation_keeps_input_and_commit_retry_is_idempotent() {
    let mut fixture = Fixture::new("cat cat");
    let task = fixture.prepare("cat", "dog");
    fixture.manager.release_search("main", &task.task_id);
    assert!(fixture
        .manager
        .commit_replace_all(
            "main",
            &fixture.status.document_id,
            &task.task_id,
            "operation"
        )
        .is_err());
    assert_eq!(fixture.text(), "cat cat");
    let task = fixture.prepare("cat", "dog");
    fixture.status = fixture
        .manager
        .apply(
            "main",
            MarkdownTransaction {
                template_update: None,
                document_id: fixture.status.document_id.clone(),
                base_revision: 0,
                operation_id: "typing".into(),
                history_group_id: "typing".into(),
                derived: false,
                edits: vec![MarkdownEdit {
                    from_byte: 0,
                    to_byte: 0,
                    inserted_text: "new ".into(),
                }],
                selection_before: None,
                selection_after: None,
            },
        )
        .unwrap();
    assert!(fixture
        .manager
        .commit_replace_all(
            "main",
            &fixture.status.document_id,
            &task.task_id,
            "operation"
        )
        .is_err());
    assert_eq!(fixture.text(), "new cat cat");
    let task = fixture.prepare("cat", "dog");
    let result = fixture.commit(&task);
    fixture.manager.release_search("main", &task.task_id);
    assert_eq!(
        fixture
            .manager
            .commit_replace_all(
                "main",
                &fixture.status.document_id,
                &task.task_id,
                "operation"
            )
            .unwrap()
            .status
            .revision,
        result.status.revision
    );
    assert_eq!(fixture.text(), "new dog dog");
    assert!(fixture
        .manager
        .commit_replace_all(
            "other",
            &fixture.status.document_id,
            &task.task_id,
            "operation"
        )
        .is_err());
}

#[test]
fn unchanged_replacement_does_not_add_history_or_revision() {
    for (query, replacement) in [("cat", "cat"), ("missing", "dog")] {
        let mut fixture = Fixture::new("cat cat");
        let task = fixture.prepare(query, replacement);
        let result = fixture.commit(&task);
        assert_eq!(result.status.revision, 0);
        assert!(!result.status.can_undo);
        assert_eq!(result.changed_count, 0);
    }
}

#[test]
fn failed_manifest_publication_can_retry_without_exposing_staged_history() {
    let mut fixture = Fixture::new("cat cat");
    let task = fixture.prepare("cat", "dog");
    let root = lock(
        &fixture
            .manager
            .session(&fixture.status.document_id)
            .unwrap(),
    )
    .unwrap()
    .root
    .clone();
    fs::rename(root.join("manifest.json"), root.join("manifest.backup")).unwrap();
    fs::create_dir(root.join("manifest.json")).unwrap();
    assert!(fixture
        .manager
        .commit_replace_all(
            "main",
            &fixture.status.document_id,
            &task.task_id,
            "operation"
        )
        .is_err());
    assert_eq!(fixture.text(), "cat cat");
    assert!(
        !fixture
            .manager
            .status("main", &fixture.status.document_id)
            .unwrap()
            .can_undo
    );
    fs::remove_dir(root.join("manifest.json")).unwrap();
    fs::rename(root.join("manifest.backup"), root.join("manifest.json")).unwrap();
    fixture.commit(&task);
    assert_eq!(fixture.text(), "dog dog");
    fixture.status = fixture
        .manager
        .history("main", &fixture.status.document_id, 1, false)
        .unwrap();
    assert_eq!(fixture.text(), "cat cat");
}

#[test]
fn changed_savepoint_is_rechecked_before_publication() {
    let mut fixture = Fixture::new("cat");
    fixture.status = fixture
        .manager
        .apply(
            "main",
            MarkdownTransaction {
                template_update: None,
                document_id: fixture.status.document_id.clone(),
                base_revision: 0,
                operation_id: "typing".into(),
                history_group_id: "typing".into(),
                derived: false,
                edits: vec![MarkdownEdit {
                    from_byte: 0,
                    to_byte: 3,
                    inserted_text: "dog".into(),
                }],
                selection_before: None,
                selection_after: None,
            },
        )
        .unwrap();
    let task = fixture.prepare("dog", "cat");
    let snapshot = fixture
        .manager
        .capture("main", &fixture.status.document_id)
        .unwrap();
    fixture.status = fixture
        .manager
        .save(
            "main",
            &snapshot.snapshot_id,
            fixture.root.join("saved.md"),
            false,
        )
        .unwrap();
    fixture
        .manager
        .release("main", &snapshot.snapshot_id)
        .unwrap();
    let result = fixture.commit(&task);
    assert!(result.status.dirty);
    assert_eq!(fixture.text(), "cat");
}

#[test]
fn encoded_original_fragments_survive_bulk_replace_and_undo_save() {
    let text = "中文 a\r\n";
    let mut variants = vec![
        text.as_bytes().to_vec(),
        [vec![0xef, 0xbb, 0xbf], text.as_bytes().to_vec()].concat(),
    ];
    variants.push(
        [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
    );
    variants.push(
        [
            vec![0xfe, 0xff],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
    );
    variants.push(encoding_rs::GBK.encode(text).0.into_owned());
    for raw in variants {
        let mut fixture = Fixture::new("");
        fixture
            .manager
            .close("main", &fixture.status.document_id, true)
            .unwrap();
        fs::create_dir_all(&fixture.root).unwrap();
        let path = fixture.root.join("encoded.md");
        fs::write(&path, &raw).unwrap();
        fixture.status = fixture
            .manager
            .open("main", Some(path.clone()), None)
            .unwrap();
        let task = fixture.prepare("a", "b");
        fixture.commit(&task);
        assert_eq!(fixture.text(), text.replace('a', "b"));
        fixture.status = fixture
            .manager
            .history("main", &fixture.status.document_id, 1, false)
            .unwrap();
        let snapshot = fixture
            .manager
            .capture("main", &fixture.status.document_id)
            .unwrap();
        fixture
            .manager
            .save("main", &snapshot.snapshot_id, path.clone(), false)
            .unwrap();
        fixture
            .manager
            .release("main", &snapshot.snapshot_id)
            .unwrap();
        assert_eq!(fs::read(path).unwrap(), raw);
    }
}

#[test]
fn release_and_owner_destruction_cancel_tasks_without_pinning_a_closed_document() {
    let fixture = Fixture::new(&"a ".repeat(1_000_000));
    let task = fixture
        .manager
        .start_search("main", fixture.query("a"), None)
        .unwrap();
    fixture.manager.release_search("main", &task.task_id);
    assert!(fixture
        .manager
        .search_progress("main", &task.task_id)
        .is_err());
    let next = fixture
        .manager
        .start_search("main", fixture.query("a"), None)
        .unwrap();
    fixture.manager.detach_owner("main").unwrap();
    fixture.manager.release_search("main", &next.task_id);
    assert!(fixture
        .manager
        .search_progress("main", &next.task_id)
        .is_err());
}

#[test]
fn replace_all_maps_template_origins_and_undo_restores_consumed_origins() {
    for (query,replacement,expected) in [("cat","long cat",Some(10)),("****","text",None)] {
        let mut fixture = Fixture::new("cat\n\n");
        fixture.status = super::super::templates::insert_template(&fixture.manager, &fixture.status, 5);
        let task = fixture.prepare(query, replacement);
        fixture.commit(&task);
        let read = |fixture: &Fixture| fixture.manager.read_window("main", &fixture.status.document_id,
            fixture.status.revision, 0, fixture.status.byte_length).unwrap();
        assert_eq!(read(&fixture).inline_templates.first().map(|t| t.from_byte),expected);
        fixture.status = fixture.manager.history("main", &fixture.status.document_id, fixture.status.revision, false).unwrap();
        assert_eq!(read(&fixture).inline_templates[0].from_byte,5);
        assert_eq!(read(&fixture).text,"cat\n\n****");
        fixture.status = fixture.manager.history("main", &fixture.status.document_id, fixture.status.revision, true).unwrap();
        assert_eq!(read(&fixture).inline_templates.first().map(|t| t.from_byte),expected);
    }
}

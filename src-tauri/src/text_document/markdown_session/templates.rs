//! 模板来源与现有持久历史、派生事务、恢复共用同一发布点。
use super::*;
use crate::markdown_templates::InlineFormat;

pub(super) fn insert_template(
    manager: &MarkdownDocumentSessionManager,
    status: &MarkdownStatus,
    at: u64,
) -> MarkdownStatus {
    manager
        .apply(
            "main",
            MarkdownTransaction {
                derived: false,
                document_id: status.document_id.clone(),
                base_revision: status.revision,
                operation_id: id("template"),
                history_group_id: id("template"),
                edits: vec![MarkdownEdit {
                    from_byte: at,
                    to_byte: at,
                    inserted_text: "****".into(),
                }],
                selection_before: None,
                selection_after: Some(MarkdownSelection {
                    anchor_byte: at + 2,
                    head_byte: at + 2,
                }),
                template_update: Some(TemplateUpdate {
                    add: vec![InlineTemplate {
                        format: InlineFormat::Strong,
                        from_byte: at,
                        open_end_byte: at + 2,
                        close_start_byte: at + 2,
                        to_byte: at + 4,
                        html: false,
                    }],
                }),
            },
        )
        .unwrap()
}
fn window(manager: &MarkdownDocumentSessionManager, status: &MarkdownStatus) -> MarkdownWindow {
    manager
        .read_window(
            "main",
            &status.document_id,
            status.revision,
            0,
            status.byte_length,
        )
        .unwrap()
}
#[test]
fn derived_history_restores_templates_from_the_same_source_revision() {
    let root = std::env::temp_dir().join(id("template-history-test"));
    let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
    let initial = manager
        .open("main", None, Some("alpha\n\n".into()))
        .unwrap();
    let mut status = insert_template(&manager, &initial, 7);
    for (at, text, derived) in [(11, "z", false), (0, "toc\n\n", true)] {
        status = manager
            .apply(
                "main",
                MarkdownTransaction {
                    derived,
                    document_id: status.document_id.clone(),
                    base_revision: status.revision,
                    operation_id: id("edit"),
                    history_group_id: id("edit"),
                    edits: vec![MarkdownEdit {
                        from_byte: at,
                        to_byte: at,
                        inserted_text: text.into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                    template_update: None,
                },
            )
            .unwrap();
    }
    assert_eq!(window(&manager, &status).inline_templates[0].from_byte, 12);
    status = manager
        .history("main", &status.document_id, status.revision, false)
        .unwrap();
    let restored = window(&manager, &status);
    assert_eq!(restored.text, "alpha\n\n****");
    assert_eq!(restored.inline_templates[0].from_byte, 7);
    status = manager
        .history("main", &status.document_id, status.revision, true)
        .unwrap();
    assert_eq!(window(&manager, &status).text, "alpha\n\n****z");
    assert_eq!(window(&manager, &status).inline_templates[0].from_byte, 7);
    manager.close("main", &status.document_id, true).unwrap();
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn template_origins_survive_session_recovery_and_invalid_metadata_is_atomic() {
    let root = std::env::temp_dir().join(id("template-recovery-test"));
    let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
    let initial = manager.open("main", None, Some("中\r\n".into())).unwrap();
    let status = insert_template(&manager, &initial, 5);
    drop(manager);
    let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
    let recovered = manager.recover("main", &status.document_id).unwrap();
    let before = window(&manager, &recovered);
    assert_eq!(before.text, "中\r\n****");
    assert_eq!(before.inline_templates.len(), 1);
    let mut fake = before.inline_templates[0].clone();
    fake.format = InlineFormat::Subscript;
    let result = manager.apply(
        "main",
        MarkdownTransaction {
            derived: false,
            document_id: status.document_id.clone(),
            base_revision: recovered.revision,
            operation_id: id("invalid"),
            history_group_id: id("invalid"),
            edits: vec![],
            selection_before: None,
            selection_after: None,
            template_update: Some(TemplateUpdate { add: vec![fake] }),
        },
    );
    assert!(result.is_err());
    let after = window(&manager, &recovered);
    assert_eq!(after.text, before.text);
    assert_eq!(after.inline_templates, before.inline_templates);
    manager.close("main", &status.document_id, true).unwrap();
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}

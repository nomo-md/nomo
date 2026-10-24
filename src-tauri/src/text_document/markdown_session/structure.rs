//! 会话身份校验仅持短锁；解析读取不可变快照，不阻塞输入事务。
use super::*;
use crate::text_document::structure_index::{Identity, Query, Response};
impl MarkdownDocumentSessionManager {
    pub(crate) fn close_structure_outline(
        &self,
        owner: &str,
        document_id: &str,
        view_id: &str,
    ) -> TextDocumentResult<()> {
        self.status(owner, document_id)?;
        self.structure.close_outline(document_id, view_id)
    }
    pub(crate) fn structure_demand(
        &self,
        owner: &str,
        identity: Identity,
        active: bool,
        priority: u8,
    ) -> TextDocumentResult<()> {
        let status = self.status(owner, &identity.document_id)?;
        if status.revision != identity.revision {
            return Err(TextDocumentError::revision_conflict(
                identity.revision,
                status.revision,
            ));
        }
        self.structure.demand(owner, &identity, active, priority)
    }
    pub(crate) fn start_structure(
        &self,
        owner: &str,
        document_id: &str,
        revision: u64,
    ) -> TextDocumentResult<Identity> {
        let handle = self.session(document_id)?;
        let snapshot = {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if revision != session.current.revision {
                return Err(TextDocumentError::revision_conflict(
                    revision,
                    session.current.revision,
                ));
            }
            session.current.clone()
        };
        let identity = self
            .structure
            .request(owner, document_id, snapshot, Box::new(handle))?;
        let status = self.status(owner, document_id)?;
        if status.revision != revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                status.revision,
            ));
        }
        Ok(identity)
    }
    pub(crate) fn query_structure(
        &self,
        owner: &str,
        identity: Identity,
        query: Query,
    ) -> TextDocumentResult<Response> {
        let status = self.status(owner, &identity.document_id)?;
        if status.revision != identity.revision {
            return Err(TextDocumentError::revision_conflict(
                identity.revision,
                status.revision,
            ));
        }
        let creates_view = matches!(query, Query::OpenOutline { .. });
        let result = self.structure.query(owner, &identity, query)?;
        let status = self.status(owner, &identity.document_id)?;
        if status.revision != identity.revision {
            if creates_view {
                if let Some(view) = result
                    .items
                    .first()
                    .and_then(|item| item["viewId"].as_str())
                {
                    self.structure.close_outline(&identity.document_id, view)?;
                }
            }
            return Err(TextDocumentError::revision_conflict(
                identity.revision,
                status.revision,
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready(manager: &MarkdownDocumentSessionManager, identity: &Identity) {
        let started = std::time::Instant::now();
        loop {
            let response = manager
                .query_structure("main", identity.clone(), Query::Progress)
                .unwrap();
            assert_ne!(response.state, "failed", "{:?}", response.failure);
            if response.state == "ready" {
                return;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn structure_cancel_transfer_and_detach_release_inflight_generations() {
        let root = std::env::temp_dir().join(id("nomo-structure-lifecycle"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        for iteration in 0..20 {
            let status = manager.open("main", None, Some("# 标题\n\nbody\n\n".repeat(3000))).unwrap();
            let identity = manager.start_structure("main", &status.document_id, 0).unwrap();
            if iteration % 2 == 0 {
                manager.transfer_owner(&status.document_id,"main","second").unwrap();
                assert!(manager.query_structure("main",identity.clone(),Query::Progress).is_err());
                assert!(manager.query_structure("second",identity,Query::Progress).is_err());
                manager.detach("second",&status.document_id).unwrap();
            } else {
                manager.close("main",&status.document_id,true).unwrap();
                assert!(manager.query_structure("main",identity,Query::Progress).is_err());
            }
        }
        drop(manager);
        let _=std::fs::remove_dir_all(root);
    }
    #[test]
    fn incremental_reference_shadowing_and_history_match_full_rebuild() {
        let root = std::env::temp_dir().join(id("nomo-structure-history"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let prefix = "[id]: /first\n\n";
        let tail = format!(
            "# 重复🙂\n\n正文[^note]\n\n{}[id]: /second\n\n# 重复🙂\n\n[use][id]\n\n[^note]: 远端定义\n\n[^note]: 重复定义\n",
            "body\n\n".repeat(6000)
        );
        let original = format!("{prefix}{tail}");
        let initial = manager.open("main", None, Some(original.clone())).unwrap();
        let first = manager
            .start_structure("main", &initial.document_id, 0)
            .unwrap();
        ready(&manager, &first);
        let edited = manager
            .apply(
                "main",
                MarkdownTransaction {
                    derived: false,
                    document_id: initial.document_id.clone(),
                    base_revision: 0,
                    operation_id: "delete-first-definition".into(),
                    history_group_id: "definition".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: 0,
                        to_byte: prefix.len() as u64,
                        inserted_text: String::new(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let mut status = edited;
        for (redo, expected_source, expected_href) in [
            (None, &tail, "/second"),
            (Some(false), &original, "/first"),
            (Some(true), &tail, "/second"),
        ] {
            if let Some(redo) = redo {
                status = manager
                    .history("main", &status.document_id, status.revision, redo)
                    .unwrap();
            }
            let incremental = manager
                .start_structure("main", &status.document_id, status.revision)
                .unwrap();
            ready(&manager, &incremental);
            if redo.is_none() {
                let changes = manager.query_structure("main", incremental.clone(), Query::Changes { observed_generation: first.index_generation }).unwrap();
                assert_eq!(changes.items[0]["invalidateAll"], false);
                assert!(changes.items[0]["references"].as_array().unwrap().iter().any(|key| key["namespace"] == "link"));
                let use_byte = expected_source.find("[use][id]").unwrap() as u64;
                assert!(changes.items[0]["ranges"].as_array().unwrap().iter().any(|range| range["fromByte"].as_u64().unwrap() <= use_byte && range["toByte"].as_u64().unwrap() > use_byte));
            }
            let rebuilt_status = manager
                .open("main", None, Some(expected_source.clone()))
                .unwrap();
            let rebuilt = manager
                .start_structure("main", &rebuilt_status.document_id, 0)
                .unwrap();
            ready(&manager, &rebuilt);
            let facts = |identity: &Identity| {
                let result = manager
                    .query_structure("main", identity.clone(), Query::Outline { cursor: None })
                    .unwrap();
                result
                    .items
                    .into_iter()
                    .map(|item| {
                        serde_json::json!([
                            item["id"],
                            item["title"],
                            item["level"],
                            item["byte"],
                            item["line"]
                        ])
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(facts(&incremental), facts(&rebuilt));
            let refs = manager
                .query_structure(
                    "main",
                    incremental.clone(),
                    Query::References {
                        keys: vec!["id".into()],
                    },
                )
                .unwrap();
            assert_eq!(refs.items[0]["definition"]["href"], expected_href);
            for (namespace, offset) in [
                ("footnoteReference", expected_source.find("[^note]").unwrap()),
                ("footnote", expected_source.find("[^note]: 远端").unwrap()),
            ] {
                let target = manager.query_structure("main", incremental.clone(), Query::Resolve {
                    namespace: namespace.into(), key: "note".into(),
                }).unwrap();
                assert_eq!(target.items[0]["status"], "found");
                assert_eq!(target.items[0]["target"]["fromByte"], offset as u64);
            }
            let collect = |identity: &Identity| {
                let mut result = vec![];
                let mut cursor = None;
                loop {
                    let page = manager
                        .query_structure(
                            "main",
                            identity.clone(),
                            Query::Blocks {
                                from_byte: 0,
                                to_byte: expected_source.len() as u64,
                                cursor,
                            },
                        )
                        .unwrap();
                    for mut block in page.items {
                        let object = block.as_object_mut().unwrap();
                        for field in ["id", "version", "contentVersion"] {
                            object.remove(field);
                        }
                        result.push(block);
                    }
                    cursor = page.next;
                    if cursor.is_none() {
                        break;
                    }
                }
                result
            };
            assert_eq!(collect(&incremental), collect(&rebuilt));
            manager
                .close("main", &rebuilt_status.document_id, true)
                .unwrap();
        }
        manager.close("main", &status.document_id, true).unwrap();
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn prefix_edit_converges_and_shares_unchanged_tail_identity_and_reference_facts() {
        let root = std::env::temp_dir().join(id("nomo-structure-incremental"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let source = format!(
            "# A\n\n{}# A\n\n[id]: /tail\n",
            "ordinary body\n\n".repeat(3000)
        );
        let status = manager.open("main", None, Some(source.clone())).unwrap();
        let before = manager
            .start_structure("main", &status.document_id, 0)
            .unwrap();
        ready(&manager, &before);
        let old_tail = manager
            .query_structure(
                "main",
                before.clone(),
                Query::Blocks {
                    from_byte: source.len() as u64 - 20,
                    to_byte: source.len() as u64,
                    cursor: None,
                },
            )
            .unwrap();
        let status = manager
            .apply(
                "main",
                MarkdownTransaction {
                    derived: false,
                    document_id: status.document_id.clone(),
                    base_revision: 0,
                    operation_id: "rename".into(),
                    history_group_id: "typing".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: 2,
                        to_byte: 3,
                        inserted_text: "New title".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        assert!(manager
            .query_structure("main", before.clone(), Query::Progress)
            .is_err());
        let after = manager
            .start_structure("main", &status.document_id, status.revision)
            .unwrap();
        ready(&manager, &after);
        let changes = manager.query_structure("main", after.clone(), Query::Changes { observed_generation: before.index_generation }).unwrap();
        assert_eq!(changes.items[0]["invalidateAll"], false);
        assert_eq!(changes.items[0]["outlineChanged"], true);
        assert!(changes.items[0]["references"].as_array().unwrap().is_empty());
        assert!(changes.items[0]["mappings"].as_array().unwrap().iter().any(|m| m["byteDelta"] == 8));
        let expired = manager.query_structure("main", after.clone(), Query::Changes { observed_generation: 0 }).unwrap();
        assert_eq!(expired.items[0]["invalidateAll"], true);
        assert!(manager.structure.scanned_bytes(&after) < 32768);
        let tail = manager
            .query_structure(
                "main",
                after.clone(),
                Query::Blocks {
                    from_byte: status.byte_length - 20,
                    to_byte: status.byte_length,
                    cursor: None,
                },
            )
            .unwrap();
        assert_eq!(tail.items.len(), old_tail.items.len());
        for (old, new) in old_tail.items.iter().zip(&tail.items) {
            assert_eq!(new["id"], old["id"]);
            assert_eq!(new["contentVersion"], old["contentVersion"]);
            assert_eq!(
                new["fromByte"].as_u64().unwrap(),
                old["fromByte"].as_u64().unwrap() + 8
            );
        }
        let headings = manager
            .query_structure("main", after.clone(), Query::Outline { cursor: None })
            .unwrap();
        assert_eq!(headings.items[1]["id"], "a");
        let definitions = manager
            .query_structure(
                "main",
                after,
                Query::References {
                    keys: vec!["id".into()],
                },
            )
            .unwrap();
        assert_eq!(definitions.items[0]["definition"]["href"], "/tail");
        manager.close("main", &status.document_id, true).unwrap();
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn structure_is_owned_by_session_revision_and_released_on_close() {
        let root = std::env::temp_dir().join(id("nomo-native-structure"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let status = manager
            .open("main", None, Some("# 标题🙂\n\n[id]: /target\n".into()))
            .unwrap();
        let identity = manager
            .start_structure("main", &status.document_id, status.revision)
            .unwrap();
        assert!(manager
            .query_structure("other", identity.clone(), Query::Progress)
            .is_err());
        let started = std::time::Instant::now();
        loop {
            let response = manager
                .query_structure("main", identity.clone(), Query::Progress)
                .unwrap();
            assert_ne!(response.state, "failed", "{:?}", response.failure);
            if response.state == "ready" {
                break;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let result = manager
            .query_structure("main", identity.clone(), Query::Outline { cursor: None })
            .unwrap();
        assert_eq!(result.items[0]["title"], "标题🙂");
        manager.close("main", &status.document_id, true).unwrap();
        assert!(manager
            .query_structure("main", identity, Query::Progress)
            .is_err());
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn outline_fold_survives_title_edit_and_toc_checkpoint_rescan() {
        let root = std::env::temp_dir().join(id("nomo-outline-edit"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let source = "# A\n\n<!-- toc -->\n- [A](#a)\n<!-- /toc -->\n\n## B\n\n# C\n";
        let mut status = manager.open("main", None, Some(source.into())).unwrap();
        let mut identity = manager.start_structure("main", &status.document_id, 0).unwrap();
        ready(&manager, &identity);
        let view = manager.query_structure("main", identity.clone(), Query::OpenOutline { default_level: 6 })
            .unwrap().items[0]["viewId"].as_str().unwrap().to_owned();
        let page = |identity: &Identity| manager.query_structure("main", identity.clone(), Query::OutlineView {
            view_id: view.clone(), from_index: 0, view_generation: None,
        }).unwrap().items[0].clone();
        let block = page(&identity)["rows"][0]["blockId"].as_str().unwrap().to_owned();
        manager.query_structure("main", identity.clone(), Query::SetOutlineFold {
            view_id: view.clone(), heading_id: format!("{block}:0"), collapsed: true,
        }).unwrap();
        for (operation, from, to, text) in [("title", 3, 3, "X"), ("toc", 6, 42, "<!-- toc -->\n- [AX](#ax)\n<!-- /toc -->")] {
            status = manager.apply("main", MarkdownTransaction {
                derived: operation == "toc", document_id: status.document_id.clone(), base_revision: status.revision,
                operation_id: operation.into(), history_group_id: "typing".into(),
                edits: vec![MarkdownEdit { from_byte: from, to_byte: to, inserted_text: text.into() }],
                selection_before: None, selection_after: None,
            }).unwrap();
            identity = manager.start_structure("main", &status.document_id, status.revision).unwrap();
            ready(&manager, &identity);
            let result = page(&identity);
            assert_eq!(result["rows"][0]["blockId"], block, "{operation}");
            assert_eq!(result["rows"][0]["collapsed"], true, "{operation}");
            assert_eq!(result["visibleCount"], 2, "{operation}");
        }
        for redo in [false, true] {
            status = manager.history("main", &status.document_id, status.revision, redo).unwrap();
            identity = manager.start_structure("main", &status.document_id, status.revision).unwrap();
            ready(&manager, &identity);
            let result = page(&identity);
            assert_eq!(result["rows"][0]["blockId"], block, "redo={redo}");
            assert_eq!(result["rows"][0]["collapsed"], true, "redo={redo}");
        }
        manager.close("main", &status.document_id, true).unwrap();
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn outline_views_fold_independently_and_reject_stale_projection_positions() {
        let root = std::env::temp_dir().join(id("nomo-native-outline"));
        let manager = MarkdownDocumentSessionManager::new(root.clone()).unwrap();
        let status = manager
            .open("main", None, Some("# A\n\n## B\n\n### C\n\n# D\n".into()))
            .unwrap();
        let identity = manager
            .start_structure("main", &status.document_id, 0)
            .unwrap();
        ready(&manager, &identity);
        let open = |level| {
            manager
                .query_structure(
                    "main",
                    identity.clone(),
                    Query::OpenOutline {
                        default_level: level,
                    },
                )
                .unwrap()
                .items[0]["viewId"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let expanded = open(6);
        let folded = open(1);
        let page = manager
            .query_structure(
                "main",
                identity.clone(),
                Query::OutlineView {
                    view_id: expanded.clone(),
                    from_index: 0,
                    view_generation: None,
                },
            )
            .unwrap();
        assert_eq!(page.items[0]["visibleCount"], 4);
        assert_eq!(page.items[0]["rows"][0]["hasChildren"], true);
        let child = &page.items[0]["rows"][2];
        let located = manager.query_structure("main", identity.clone(), Query::LocateOutline {
            view_id: expanded.clone(), heading_id: format!("{}:0",child["blockId"].as_str().unwrap()),
            byte: child["byte"].as_u64().unwrap(), parent: true,
        }).unwrap();
        assert_eq!(located.items[0]["position"], 1);
        let other = manager
            .query_structure(
                "main",
                identity.clone(),
                Query::OutlineView {
                    view_id: folded,
                    from_index: 0,
                    view_generation: None,
                },
            )
            .unwrap();
        assert_eq!(other.items[0]["visibleCount"], 2);
        let root_heading = format!(
            "{}:0",
            page.items[0]["rows"][0]["blockId"].as_str().unwrap()
        );
        manager
            .query_structure(
                "main",
                identity.clone(),
                Query::SetOutlineFold {
                    view_id: expanded.clone(),
                    heading_id: root_heading,
                    collapsed: true,
                },
            )
            .unwrap();
        assert!(manager
            .query_structure(
                "main",
                identity.clone(),
                Query::OutlineView {
                    view_id: expanded.clone(),
                    from_index: 0,
                    view_generation: Some(0)
                }
            )
            .is_err());
        let collapsed = manager
            .query_structure(
                "main",
                identity.clone(),
                Query::OutlineView {
                    view_id: expanded.clone(),
                    from_index: 0,
                    view_generation: None,
                },
            )
            .unwrap();
        assert_eq!(collapsed.items[0]["visibleCount"], 2);
        manager
            .query_structure(
                "main",
                identity.clone(),
                Query::CloseOutline {
                    view_id: expanded.clone(),
                },
            )
            .unwrap();
        assert!(manager
            .query_structure(
                "main",
                identity,
                Query::OutlineView {
                    view_id: expanded,
                    from_index: 0,
                    view_generation: None
                }
            )
            .is_err());
        manager.close("main", &status.document_id, true).unwrap();
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }
}

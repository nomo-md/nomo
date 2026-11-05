//! 检查只借用固定修订的持久化树；不注册保存快照，不改变撤销分组或保存点。
use super::*;
use crate::text_document::markdown_lint::{self, Report, Request, RuleSet, SkipReason};

impl MarkdownDocumentSessionManager {
    pub(crate) async fn markdown_lint(
        &self,
        owner: &str,
        document_id: &str,
        revision: u64,
        rule_set: RuleSet,
        request: &Request,
    ) -> TextDocumentResult<Report> {
        static SCAN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        request.check()?;
        // GPUI 持有的任务被丢弃时，等待中的 permit 一并取消；不会积压正文或快照。
        let _permit = SCAN
            .acquire()
            .await
            .map_err(|_| error("lint-closed", "格式检查队列已关闭"))?;
        request.check()?;
        let handle = self.session(document_id)?;
        let snapshot = {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if session.current.revision != revision {
                return Err(TextDocumentError::revision_conflict(
                    revision,
                    session.current.revision,
                ));
            }
            session.current.clone()
        };
        let report = Report::new(document_id.to_owned(), revision, rule_set);
        let result = if snapshot.len() > markdown_lint::MAX_SOURCE_BYTES as u64 {
            report.skip(SkipReason::SourceBudget)
        } else {
            markdown_lint::scan(&snapshot, report, request)?
        };
        request.check()?;
        let current = self.status(owner, document_id)?;
        if current.revision != revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                current.revision,
            ));
        }
        // handle 保活到读取结束；关闭文档后允许派生任务退出，再释放 SessionCleanup。
        drop(snapshot);
        drop(handle);
        Ok(result)
    }
}

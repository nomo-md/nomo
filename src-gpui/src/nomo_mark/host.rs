//! 原版窗口会话的受控适配器：只管理当前投影和确认队列，不读取文件、不实现 undo。
use super::interaction::EditorInteraction;
use super::{mapping::local_range, parse::ParsedWindow, protocol::*};
use std::{collections::VecDeque, sync::Arc};

pub(crate) const MAX_WINDOW_BYTES: usize = 1024 * 1024;

pub struct WindowHostAdapter {
    pub parsed: Arc<ParsedWindow>,
    pub interaction: EditorInteraction,
    pub selection: DocumentSelection,
    pub presentation: Presentation,
    committed_revision: u64,
    pending: VecDeque<DocumentTransaction>,
    rewindow_operation: Option<String>,
    submitted: bool,
    serial: u64,
    failure: Option<String>,
    readonly: bool,
    layout: Option<LayoutStamp>,
    last_ack: Option<String>,
}
impl WindowHostAdapter {
    pub fn new(
        window: DocumentWindow,
        window_id: impl Into<String>,
        generation: u64,
        context: WindowContext,
    ) -> Result<Self, ControlError> {
        let parsed = Arc::new(ParsedWindow::parse(window, window_id, generation, context)?);
        Ok(Self {
            selection: DocumentSelection::caret(parsed.identity.from_byte),
            committed_revision: parsed.identity.revision,
            parsed,
            interaction: EditorInteraction::default(),
            presentation: Presentation::Preparing,
            pending: VecDeque::new(),
            rewindow_operation: None,
            submitted: false,
            serial: 0,
            failure: None,
            readonly: false,
            layout: None,
            last_ack: None,
        })
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    /// 全文长度来自会话；只叠加尚未确认事务的字节增量，不读取或复制全文。
    pub(crate) fn projected_document_length(&self, committed_length: u64) -> u64 {
        let delta: i128 = self.pending.iter().flat_map(|transaction| &transaction.edits)
            .map(|edit| edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128).sum();
        (committed_length as i128 + delta).max(0) as u64
    }
    pub fn pending_bytes(&self) -> usize {
        self.pending
            .iter()
            .map(|tx| {
                tx.edits
                    .iter()
                    .map(|e| e.inserted_text.len() + std::mem::size_of::<SourceEdit>())
                    .sum::<usize>()
                    + tx.operation_id.len()
                    + tx.history_group_id.len()
                    + tx.template_update.as_ref().map_or(0, |u| {
                        u.add.capacity() * std::mem::size_of::<InlineTemplate>()
                    })
                    + std::mem::size_of::<DocumentTransaction>()
            })
            .sum()
    }
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
        if matches!(
            self.presentation,
            Presentation::Editable | Presentation::ReadOnly
        ) {
            self.presentation = if readonly {
                Presentation::ReadOnly
            } else {
                Presentation::Editable
            };
        }
    }
    pub fn mark_layout_ready(
        &mut self,
        stamp: LayoutStamp,
        height: f32,
    ) -> Result<ControlEvent, ControlError> {
        if stamp.identity != self.parsed.identity {
            return Err(ControlError::Stale);
        }
        if matches!(
            self.presentation,
            Presentation::Detached | Presentation::Suspended | Presentation::Failed
        ) {
            return Err(ControlError::ReadOnly);
        }
        if !height.is_finite()
            || height < 0.0
            || !stamp.width.is_finite()
            || stamp.width <= 0.0
            || !stamp.zoom.is_finite()
            || stamp.zoom <= 0.0
        {
            return Err(ControlError::InvalidRange);
        }
        self.layout = Some(stamp.clone());
        self.presentation = if self.readonly {
            Presentation::ReadOnly
        } else {
            Presentation::Editable
        };
        Ok(ControlEvent::LayoutReady { stamp, height })
    }
    pub fn queue_edit(
        &mut self,
        edits: Vec<SourceEdit>,
        after: DocumentSelection,
        history_group: &str,
    ) -> Result<(), ControlError> {
        self.queue_template_edit(edits, after, history_group, None)
    }
    pub(crate) fn projected_window_bytes(&self, edits: &[SourceEdit]) -> Result<usize, ControlError> {
        let mut bytes = self.parsed.source.len() as i128;
        for edit in edits {
            let removed = edit.to_byte.checked_sub(edit.from_byte).ok_or(ControlError::InvalidRange)?;
            bytes += edit.inserted_text.len() as i128 - removed as i128;
        }
        usize::try_from(bytes).map_err(|_| ControlError::InvalidRange)
    }
    /// 大编辑沿用确认队列和幂等操作身份，只保留待提交补丁，不复制或解析扩张后的正文。
    pub(crate) fn queue_rewindow(
        &mut self,
        edits: Vec<SourceEdit>,
        after: DocumentSelection,
        history_group: &str,
        template_update: Option<TemplateUpdate>,
    ) -> Result<(), ControlError> {
        if self.presentation != Presentation::Editable || self.readonly {
            return Err(ControlError::ReadOnly);
        }
        if let Some(error) = &self.failure { return Err(ControlError::HostFailure(error.clone())); }
        if edits.is_empty() { return Ok(()); }
        for (index, edit) in edits.iter().enumerate() {
            if edit.from_byte > edit.to_byte || (index > 0 && edit.from_byte < edits[index - 1].to_byte) {
                return Err(ControlError::InvalidRange);
            }
        }
        self.serial += 1;
        let operation_id = format!("{}:{}:{}", self.parsed.identity.window_id, self.parsed.identity.view_generation, self.serial);
        self.pending.push_back(DocumentTransaction {
            base_revision: self.committed_revision,
            operation_id: operation_id.clone(),
            history_group_id: history_group.into(),
            edits,
            selection_before: self.selection,
            selection_after: after,
            template_update,
        });
        self.rewindow_operation = Some(operation_id);
        self.set_readonly(true);
        Ok(())
    }
    pub fn queue_template_edit(
        &mut self,
        edits: Vec<SourceEdit>,
        after: DocumentSelection,
        history_group: &str,
        template_update: Option<TemplateUpdate>,
    ) -> Result<(), ControlError> {
        if self.presentation != Presentation::Editable || self.readonly {
            return Err(ControlError::ReadOnly);
        }
        if let Some(error) = &self.failure {
            return Err(ControlError::HostFailure(error.clone()));
        }
        if edits.is_empty() {
            return Ok(());
        }
        let base = self.parsed.identity.from_byte;
        let source = &self.parsed.source;
        let mut last_end = 0;
        let mut ranges = Vec::new();
        for edit in &edits {
            let range = local_range(source, base, edit.from_byte..edit.to_byte)?;
            if !ranges.is_empty() && range.start < last_end {
                return Err(ControlError::InvalidRange);
            }
            last_end = range.end;
            ranges.push(range);
        }
        let (edits, ranges): (Vec<_>, Vec<_>) = edits
            .into_iter()
            .zip(ranges)
            .filter(|(edit, range)| source.get(range.clone()) != Some(edit.inserted_text.as_str()))
            .unzip();
        if self.projected_window_bytes(&edits)? > MAX_WINDOW_BYTES {
            return Err(ControlError::NeedsContext("编辑超过局部窗口预算，需要由宿主提交并重新分配窗口".into()));
        }
        let mut next = source.to_string();
        for (edit, range) in edits.iter().zip(ranges).rev() {
            next.replace_range(range, &edit.inserted_text);
        }
        local_range(&next, base, after.range())?;
        let mut context = self.parsed.context.clone();
        crate::markdown_templates::map_templates(
            &mut context.inline_templates,
            edits
                .iter()
                .rev()
                .map(|e| (e.from_byte, e.to_byte, e.inserted_text.len() as u64)),
            template_update.is_some(),
        );
        if let Some(update) = &template_update {
            for template in &update.add {
                if !template.valid_in(&next, base) {
                    return Err(ControlError::InvalidRange);
                }
                context.inline_templates.push(template.clone());
            }
        }
        crate::markdown_templates::canonicalize(&mut context.inline_templates);
        if !crate::markdown_templates::valid_set(&context.inline_templates) {
            return Err(ControlError::InvalidRange);
        }
        if edits.is_empty() && context.inline_templates == self.parsed.context.inline_templates {
            self.selection = after;
            return Ok(());
        }
        let mut details = std::collections::BTreeMap::new();
        for (&at, &open) in &self.interaction.details_open {
            if !edits
                .iter()
                .any(|edit| edit.from_byte <= at && at < edit.to_byte)
            {
                let mapped =
                    super::commands::map_selection(DocumentSelection::caret(at), &edits).head_byte;
                details.insert(mapped, open);
            }
        }
        self.interaction.details_open = details;
        let mut horizontal = std::collections::BTreeMap::new();
        for (&at, &offset) in &self.interaction.horizontal_offsets {
            if edits
                .iter()
                .any(|edit| edit.from_byte < at && at < edit.to_byte)
            {
                continue;
            }
            let before: Vec<_> = edits
                .iter()
                .filter(|edit| edit.to_byte <= at && edit.from_byte < at)
                .cloned()
                .collect();
            let mapped =
                super::commands::map_selection(DocumentSelection::caret(at), &before).head_byte;
            horizontal.insert(mapped, offset);
        }
        self.interaction.horizontal_offsets = horizontal;
        self.serial += 1;
        let transaction = DocumentTransaction {
            template_update,
            base_revision: self.committed_revision,
            operation_id: format!(
                "{}:{}:{}",
                self.parsed.identity.window_id, self.parsed.identity.view_generation, self.serial
            ),
            history_group_id: history_group.into(),
            edits,
            selection_before: self.selection,
            selection_after: after,
        };
        self.pending.push_back(transaction);
        self.selection = after;
        let mut identity = self.parsed.identity.clone();
        // 本地投影代数与原生修订分别管理；不能把乐观内容标成已提交的旧版本。
        identity.projection_generation += 1;
        identity.to_byte = base + next.len() as u64;
        self.parsed = Arc::new(ParsedWindow::from_source_with_context(
            identity,
            Arc::from(next),
            context,
        ));
        self.prune_view_state();
        self.layout = None;
        Ok(())
    }
    /// 目录补丁与用户输入共用队列；只更新当前窗口内的文本，其余位置仅映射坐标。
    pub(crate) fn queue_derived(&mut self, edits: Vec<SourceEdit>) -> Result<(), ControlError> {
        if self.pending_count() != 0 || self.failure.is_some() || self.readonly
            || self.interaction.protected()
            || !matches!(self.presentation, Presentation::Editable | Presentation::Suspended)
        {
            return Err(ControlError::Busy);
        }
        if edits.is_empty() { return Ok(()); }
        let size: usize = edits.iter().map(|edit| edit.inserted_text.len() + std::mem::size_of::<SourceEdit>()).sum();
        if size > 4 * 1024 * 1024 { return Err(ControlError::NeedsContext("目录更新超过 4 MiB 编辑预算".into())); }
        let base = self.parsed.identity.from_byte;
        let end = self.parsed.identity.to_byte;
        let selection = self.selection.range();
        let mut last_end = 0;
        let mut local = Vec::new();
        let mut before_delta = 0_i128;
        for (index, edit) in edits.iter().enumerate() {
            if edit.inserted_text.len() > crate::text_document::MAX_SEMANTIC_BYTES {
                return Err(ControlError::NeedsContext("目录超过单个语义结构块预算".into()));
            }
            if edit.from_byte > edit.to_byte || (index > 0 && edit.from_byte < last_end) {
                return Err(ControlError::InvalidRange);
            }
            last_end = edit.to_byte;
            let touches_selection = if selection.is_empty() {
                edit.from_byte <= selection.start && selection.start <= edit.to_byte
            } else {
                edit.from_byte < selection.end && selection.start < edit.to_byte
            };
            if touches_selection { return Err(ControlError::Busy); }
            if edit.to_byte <= base {
                before_delta += edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128;
            } else if edit.from_byte >= end {
                continue;
            } else if edit.from_byte >= base && edit.to_byte <= end {
                local.push((edit, local_range(&self.parsed.source, base, edit.from_byte..edit.to_byte)?));
            } else {
                return Err(ControlError::NeedsContext("目录跨越当前窗口边界，稍后再更新".into()));
            }
        }
        let new_base = u64::try_from(base as i128 + before_delta).map_err(|_| ControlError::InvalidRange)?;
        let mut next = self.parsed.source.to_string();
        for (edit, range) in local.into_iter().rev() { next.replace_range(range, &edit.inserted_text); }
        if next.len() > 1024 * 1024 { return Err(ControlError::NeedsContext("目录更新后的当前窗口超过 1 MiB 预算".into())); }
        let after = super::commands::map_selection(self.selection, &edits);
        // 全局选区的活动端也可能位于屏外；仅校验原本属于当前窗口的端点。
        if self.selection.head_byte >= base && self.selection.head_byte <= end {
            let head = after.head_byte.checked_sub(new_base).and_then(|at| usize::try_from(at).ok()).ok_or(ControlError::InvalidRange)?;
            if head > next.len() || !next.is_char_boundary(head) { return Err(ControlError::InvalidRange); }
        }
        let mut context = self.parsed.context.clone();
        crate::markdown_templates::map_templates(
            &mut context.inline_templates,
            edits.iter().rev().map(|e| (e.from_byte, e.to_byte, e.inserted_text.len() as u64)),
            false,
        );
        crate::markdown_templates::canonicalize(&mut context.inline_templates);
        if !crate::markdown_templates::valid_set(&context.inline_templates)
            || context.inline_templates.iter().any(|template| !template.valid_in(&next, new_base))
        { return Err(ControlError::InvalidRange); }
        self.interaction.details_open = std::mem::take(&mut self.interaction.details_open).into_iter()
            .filter(|(at, _)| !edits.iter().any(|edit| edit.from_byte <= *at && *at < edit.to_byte))
            .map(|(at, value)| (super::commands::map_selection(DocumentSelection::caret(at), &edits).head_byte, value)).collect();
        self.interaction.horizontal_offsets = std::mem::take(&mut self.interaction.horizontal_offsets).into_iter()
            .filter(|(at, _)| !edits.iter().any(|edit| edit.from_byte < *at && *at < edit.to_byte))
            .map(|(at, value)| (super::commands::map_selection(DocumentSelection::caret(at), &edits).head_byte, value)).collect();
        self.serial += 1;
        self.pending.push_back(DocumentTransaction {
            template_update: None,
            base_revision: self.committed_revision,
            operation_id: format!("{}:{}:{}", self.parsed.identity.window_id, self.parsed.identity.view_generation, self.serial),
            history_group_id: "automatic-toc".into(),
            edits,
            selection_before: self.selection,
            selection_after: after,
        });
        self.selection = after;
        let mut identity = self.parsed.identity.clone();
        identity.projection_generation += 1;
        identity.from_byte = new_base;
        identity.to_byte = new_base + next.len() as u64;
        self.parsed = Arc::new(ParsedWindow::from_source_with_context(identity, Arc::from(next), context));
        self.prune_view_state();
        self.layout = None;
        Ok(())
    }
    /// 仅发送队首。失败重试返回同一 operationId，沿用原版幂等契约。
    pub fn next_submission(&mut self) -> Option<ControlEvent> {
        if self.submitted || self.failure.is_some() {
            return None;
        }
        let front = self.pending.front_mut()?;
        front.base_revision = self.committed_revision;
        self.submitted = true;
        Some(if self.rewindow_operation.as_deref() == Some(front.operation_id.as_str()) {
            ControlEvent::SubmitRewindow(front.clone())
        } else if front.history_group_id == "automatic-toc" {
            ControlEvent::SubmitDerived(front.clone())
        } else {
            ControlEvent::Submit(front.clone())
        })
    }
    pub fn acknowledge(&mut self, operation_id: &str, revision: u64) -> Result<(), ControlError> {
        if self.last_ack.as_deref() == Some(operation_id) {
            return Ok(());
        }
        let front = self.pending.front().ok_or(ControlError::Stale)?;
        let rewindow = self.rewindow_operation.as_deref() == Some(operation_id);
        if front.operation_id != operation_id || revision < self.committed_revision
            || (revision == self.committed_revision && !rewindow)
        {
            return Err(ControlError::Stale);
        }
        self.pending.pop_front();
        if self.rewindow_operation.as_deref() == Some(operation_id) { self.rewindow_operation = None; }
        self.last_ack = Some(operation_id.into());
        self.committed_revision = revision;
        self.submitted = false;
        self.failure = None;
        if self.presentation == Presentation::Failed {
            self.presentation = if self.layout.is_some() {
                if self.readonly {
                    Presentation::ReadOnly
                } else {
                    Presentation::Editable
                }
            } else {
                Presentation::Preparing
            };
        }
        if self.pending.is_empty() {
            Arc::make_mut(&mut self.parsed).identity.revision = revision;
            // revision 确认不改变正文几何，但需要同时更新对应的结果身份。
            if let Some(stamp) = &mut self.layout {
                stamp.identity.revision = revision;
            }
        }
        Ok(())
    }
    pub fn submission_failed(&mut self, error: impl Into<String>) {
        self.failure = Some(error.into());
        self.submitted = false;
        self.presentation = Presentation::Failed;
    }
    pub fn retry(&mut self) -> Result<(), ControlError> {
        if self.pending.is_empty() {
            return Err(ControlError::Stale);
        }
        self.failure = None;
        self.presentation = Presentation::Preparing;
        self.submitted = false;
        Ok(())
    }
    /// 最后一笔乐观输入确认后核对宿主正文。相同内容无需重建视图或中断 IME。
    pub fn verify_authoritative(&mut self, window: &DocumentWindow) -> Result<(), ControlError> {
        if !self.pending.is_empty() {
            return Err(ControlError::Busy);
        }
        let identity = &self.parsed.identity;
        if window.document_id != identity.document_id
            || window.revision != self.committed_revision
            || window.from_byte != identity.from_byte
            || window.to_byte != identity.to_byte
        {
            return Err(ControlError::Stale);
        }
        if window.text.as_str() != self.parsed.source.as_ref()
            || window.inline_templates != self.parsed.context.inline_templates
        {
            self.submission_failed("宿主确认正文与局部投影不一致，保留输入并等待显式重基");
            return Err(ControlError::NeedsContext(
                "宿主正文与局部投影不一致".into(),
            ));
        }
        Ok(())
    }
    /// 窗口外的修改只移动全局锚点，不重新解析，也不结束正在进行的组字。
    /// 与当前窗口接触的修改必须由宿主重读并显式解决未确认输入。
    pub fn rebase_outside(
        &mut self,
        expected: &WindowIdentity,
        revision: u64,
        generation: u64,
        edits: &[NativeRangeEdit],
    ) -> Result<(), ControlError> {
        if expected != &self.parsed.identity
            || revision <= self.committed_revision
            || generation < expected.view_generation
        {
            return Err(ControlError::Stale);
        }
        if !self.pending.is_empty()
            || matches!(
                self.presentation,
                Presentation::Detached | Presentation::Failed
            )
        {
            return Err(ControlError::Busy);
        }
        let mut previous = 0;
        let mut shift = 0i128;
        for (index, edit) in edits.iter().enumerate() {
            if edit.from_byte > edit.to_byte || (index > 0 && edit.from_byte < previous) {
                return Err(ControlError::InvalidRange);
            }
            previous = edit.to_byte;
            if edit.from_byte <= expected.to_byte && edit.to_byte >= expected.from_byte {
                return Err(ControlError::NeedsContext(
                    "外部修改触及当前窗口，需要宿主提供新窗口".into(),
                ));
            }
            if edit.to_byte < expected.from_byte {
                shift += edit.inserted_bytes as i128 - (edit.to_byte - edit.from_byte) as i128;
            }
        }
        let map = |at: u64| -> Result<u64, ControlError> {
            u64::try_from(at as i128 + shift).map_err(|_| ControlError::InvalidRange)
        };
        let from = map(expected.from_byte)?;
        let to = map(expected.to_byte)?;
        // 窗口内选区随同窗口移动；跨窗口选区需要宿主显式回送，避免猜测外部位置。
        let selection = if self.selection.range().start >= expected.from_byte
            && self.selection.range().end <= expected.to_byte
        {
            DocumentSelection {
                anchor_byte: map(self.selection.anchor_byte)?,
                head_byte: map(self.selection.head_byte)?,
            }
        } else {
            return Err(ControlError::NeedsContext(
                "跨窗口选区需要宿主回送重基结果".into(),
            ));
        };
        let mut details = std::collections::BTreeMap::new();
        for (&at, &open) in &self.interaction.details_open {
            details.insert(map(at)?, open);
        }
        self.interaction.details_open = details;
        let parsed = Arc::make_mut(&mut self.parsed);
        for template in &mut parsed.context.inline_templates {
            template.from_byte = map(template.from_byte)?;
            template.open_end_byte = map(template.open_end_byte)?;
            template.close_start_byte = map(template.close_start_byte)?;
            template.to_byte = map(template.to_byte)?;
        }
        let identity = &mut parsed.identity;
        let mut horizontal = std::collections::BTreeMap::new();
        for (&at, &offset) in &self.interaction.horizontal_offsets {
            horizontal.insert(map(at)?, offset);
        }
        self.interaction.horizontal_offsets = horizontal;
        identity.from_byte = from;
        identity.to_byte = to;
        identity.revision = revision;
        identity.view_generation = generation;
        self.committed_revision = revision;
        self.selection = selection;
        self.layout = None;
        Ok(())
    }
    /// 引用补齐只替换局部解析上下文；正文 Arc、选区及会话历史保持原身份。
    pub(crate) fn refresh_references(&mut self, resolved: ParseContext) -> Result<bool, ControlError> {
        self.interaction.prepare_detach(self.pending.len(), self.presentation)?;
        if matches!(self.presentation, Presentation::Detached | Presentation::Suspended) { return Err(ControlError::ReadOnly); }
        if self.parsed.context.references == resolved.references
            && self.parsed.context.pending_references == resolved.pending_references { return Ok(false); }
        let mut context = self.parsed.context.clone();
        context.references = resolved.references;
        context.pending_references = resolved.pending_references;
        let mut identity = self.parsed.identity.clone();
        identity.view_generation += 1;
        self.parsed = Arc::new(ParsedWindow::from_source_with_context(identity, self.parsed.source.clone(), context));
        self.prune_view_state();
        self.layout = None;
        self.presentation = Presentation::Preparing;
        Ok(true)
    }
    pub fn replace_window(
        &mut self,
        window: DocumentWindow,
        generation: u64,
        context: WindowContext,
    ) -> Result<(), ControlError> {
        self.interaction
            .prepare_detach(self.pending.len(), self.presentation)?;
        if self.presentation == Presentation::Detached {
            return Err(ControlError::Detached);
        }
        if window.document_id != self.parsed.identity.document_id
            || window.revision < self.committed_revision
            || generation < self.parsed.identity.view_generation
        {
            return Err(ControlError::Stale);
        }
        self.parsed = Arc::new(ParsedWindow::parse(
            window,
            self.parsed.identity.window_id.clone(),
            generation,
            context,
        )?);
        self.committed_revision = self.parsed.identity.revision;
        self.prune_view_state();
        self.failure = None;
        self.layout = None;
        self.presentation = Presentation::Preparing;
        Ok(())
    }
    fn prune_view_state(&mut self) {
        let base = self.parsed.identity.from_byte;
        self.interaction.details_open.retain(|at, _| {
            self.parsed
                .details
                .iter()
                .any(|block| base + block.source.start as u64 == *at)
        });
        let mut tables = std::collections::BTreeSet::new();
        tables.extend(
            self.parsed
                .nodes
                .iter()
                .filter(|node| node.kind == "table")
                .map(|node| base + node.source.start as u64),
        );
        tables.extend(
            self.parsed
                .html_tables
                .iter()
                .map(|table| base + table.source.start as u64),
        );
        self.interaction
            .horizontal_offsets
            .retain(|at, offset| tables.contains(at) && offset.is_finite() && *offset >= 0.0);
    }
    pub fn suspend(&mut self) -> Result<(), ControlError> {
        self.interaction
            .prepare_detach(self.pending.len(), self.presentation)?;
        if self.presentation == Presentation::Detached {
            return Err(ControlError::Detached);
        }
        self.layout = None;
        self.presentation = Presentation::Suspended;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), ControlError> {
        if self.presentation != Presentation::Suspended {
            return Err(ControlError::Stale);
        }
        self.presentation = Presentation::Preparing;
        Ok(())
    }
    pub fn detach(&mut self, discard: bool) -> Result<(), ControlError> {
        if !discard {
            self.interaction
                .prepare_detach(self.pending.len(), self.presentation)?;
        }
        self.pending.clear();
        self.rewindow_operation = None;
        self.layout = None;
        self.interaction.reset();
        self.presentation = Presentation::Detached;
        let mut identity = self.parsed.identity.clone();
        identity.to_byte = identity.from_byte;
        self.parsed = Arc::new(ParsedWindow::from_source(identity, Arc::from("")));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host() -> WindowHostAdapter {
        let mut host = WindowHostAdapter::new(
            DocumentWindow {
                inline_templates: Vec::new(),
                document_id: "doc".into(),
                revision: 5,
                from_byte: 100,
                to_byte: 103,
                text: "中".into(),
            },
            "view",
            0,
            WindowContext::Complete,
        )
        .unwrap();
        ready(&mut host);
        host
    }
    fn ready(host: &mut WindowHostAdapter) {
        host.mark_layout_ready(
            LayoutStamp {
                identity: host.parsed.identity.clone(),
                width: 600.0,
                font_generation: 0,
                resource_generation: 0,
                zoom: 1.0,
            },
            24.0,
        )
        .unwrap();
    }
    #[test]
    fn own_pending_input_survives_ack_and_retry_without_a_second_history() {
        let mut h = host();
        h.queue_edit(
            vec![SourceEdit {
                from_byte: 103,
                to_byte: 103,
                inserted_text: "文".into(),
            }],
            DocumentSelection::caret(106),
            "typing",
        )
        .unwrap();
        let ControlEvent::Submit(first) = h.next_submission().unwrap() else {
            panic!()
        };
        h.queue_edit(
            vec![SourceEdit {
                from_byte: 106,
                to_byte: 106,
                inserted_text: "😀".into(),
            }],
            DocumentSelection::caret(110),
            "typing",
        )
        .unwrap();
        assert_eq!(h.suspend(), Err(ControlError::Busy));
        h.submission_failed("ack lost");
        assert_eq!(&*h.parsed.source, "中文😀");
        h.retry().unwrap();
        let ControlEvent::Submit(retry) = h.next_submission().unwrap() else {
            panic!()
        };
        assert_eq!(first, retry);
        h.acknowledge(&first.operation_id, 6).unwrap();
        let ControlEvent::Submit(second) = h.next_submission().unwrap() else {
            panic!()
        };
        assert_eq!(second.base_revision, 6);
        h.acknowledge(&second.operation_id, 7).unwrap();
        assert_eq!(&*h.parsed.source, "中文😀");
        ready(&mut h);
        h.suspend().unwrap();
        h.resume().unwrap();
        assert_eq!(h.presentation, Presentation::Preparing);
    }
    #[test]
    fn invalid_edits_and_stale_layout_do_not_mutate_projection() {
        let mut h = host();
        let old = h.parsed.identity.clone();
        assert_eq!(
            h.queue_edit(
                vec![SourceEdit {
                    from_byte: 101,
                    to_byte: 103,
                    inserted_text: "x".into()
                }],
                DocumentSelection::caret(102),
                "x"
            ),
            Err(ControlError::InvalidUtf8Boundary)
        );
        assert_eq!(&*h.parsed.source, "中");
        h.queue_edit(
            vec![SourceEdit {
                from_byte: 103,
                to_byte: 103,
                inserted_text: "x".into(),
            }],
            DocumentSelection::caret(104),
            "x",
        )
        .unwrap();
        assert_eq!(
            h.mark_layout_ready(
                LayoutStamp {
                    identity: old,
                    width: 600.0,
                    font_generation: 0,
                    resource_generation: 0,
                    zoom: 1.0
                },
                24.0
            ),
            Err(ControlError::Stale)
        );
    }
    #[test]
    fn original_protocol_wire_names_are_preserved() {
        let h = host();
        let value = serde_json::to_value(DocumentWindow {
            inline_templates: Vec::new(),
            document_id: h.parsed.identity.document_id.clone(),
            revision: 5,
            from_byte: 100,
            to_byte: 103,
            text: "中".into(),
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"documentId":"doc","revision":5,"fromByte":100,"toByte":103,"text":"中"})
        );
    }
    #[test]
    fn external_rebase_preserves_composition_and_host_generation() {
        let mut h = host();
        h.interaction.begin_composition(0);
        let previous = h.parsed.identity.clone();
        h.rebase_outside(
            &previous,
            6,
            9,
            &[NativeRangeEdit {
                from_byte: 1,
                to_byte: 2,
                inserted_bytes: 4,
            }],
        )
        .unwrap();
        assert_eq!(h.parsed.identity.from_byte, 103);
        assert_eq!(h.selection.head_byte, 103);
        assert!(h.interaction.composing);
        h.queue_edit(
            vec![SourceEdit {
                from_byte: 106,
                to_byte: 106,
                inserted_text: "字".into(),
            }],
            DocumentSelection::caret(109),
            "ime",
        )
        .unwrap();
        assert_eq!(h.parsed.identity.view_generation, 9);
        assert_eq!(h.parsed.identity.projection_generation, 1);
        assert_eq!(
            h.rebase_outside(&previous, 7, 10, &[]),
            Err(ControlError::Stale)
        );
    }
    #[test]
    fn mismatched_authoritative_text_cannot_be_recycled_silently() {
        let mut h = host();
        let input = DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "doc".into(),
            revision: 5,
            from_byte: 100,
            to_byte: 103,
            text: "文".into(),
        };
        assert!(matches!(
            h.verify_authoritative(&input),
            Err(ControlError::NeedsContext(_))
        ));
        assert_eq!(h.suspend(), Err(ControlError::Busy));
        assert_eq!(&*h.parsed.source, "中");
        h.detach(true).unwrap();
        assert!(h.parsed.source.is_empty());
    }
}

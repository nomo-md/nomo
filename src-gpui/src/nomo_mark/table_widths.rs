//! 自动列宽由当前可见内容和容器决定。旧宽度仅用于组字和连续删除的短暂显示。
use std::{ops::Range, sync::Arc, time::{Duration, Instant}};

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct LayoutKey {
    pub columns: usize,
    pub available: f32,
    pub font_size: f32,
    pub zoom: f32,
    pub scale: f32,
    pub font_generation: u64,
    pub html_grid: Option<u64>,
}

struct MeasuredColumns {
    anchor: u64,
    source: Range<u64>,
    revision: u64,
    key: LayoutKey,
    widths: Arc<[f32]>,
    hold_until: Option<Instant>,
    used: bool,
}

#[derive(Default)]
pub(crate) struct TableWidths {
    tables: Vec<MeasuredColumns>,
    edited: Option<Range<u64>>,
    deletion: bool,
    // 只缓存排版键和一个宽度。固定槽位覆盖碰撞项，不持有文字、布局或视图。
    intrinsic: Vec<Option<(u64, f32)>>,
}

impl TableWidths {
    pub fn measure(&mut self, key: u64, measure: impl FnOnce() -> f32) -> f32 {
        const SLOTS: usize = 512;
        if self.intrinsic.is_empty() { self.intrinsic.resize(SLOTS, None); }
        let slot = &mut self.intrinsic[key as usize % SLOTS];
        if let Some((stored, width)) = *slot && stored == key { return width; }
        let width = measure();
        *slot = Some((key, width));
        width
    }

    pub fn cached(&mut self, anchor: u64, key: LayoutKey, revision: u64, source: &Range<u64>, composition: Option<u64>) -> Option<Arc<[f32]>> {
        let index = self.tables.binary_search_by_key(&anchor, |t| t.anchor).ok()?;
        let table = &mut self.tables[index];
        if table.key != key { return None; }
        table.source = source.clone();
        let now = Instant::now();
        if self.edited.as_ref().is_some_and(|e| e.start <= source.end && source.start <= e.end) {
            table.hold_until = self.deletion.then(|| now + Duration::from_millis(180));
        }
        let composing = composition.is_some_and(|at| source.start <= at && at <= source.end);
        if composing || table.hold_until.is_some_and(|until| until > now)
            || (table.hold_until.is_none() && table.revision == revision) {
            table.used = true;
            return Some(table.widths.clone());
        }
        None
    }

    pub fn record(&mut self, anchor: u64, key: LayoutKey, revision: u64, source: Range<u64>, widths: Vec<f32>) -> Arc<[f32]> {
        let index = self.tables.binary_search_by_key(&anchor, |t| t.anchor);
        if let Ok(i) = index && self.tables[i].key == key && *self.tables[i].widths == widths {
            let table = &mut self.tables[i];
            table.used = true;
            table.hold_until = None;
            table.source = source;
            table.revision = revision;
            return table.widths.clone();
        }
        let widths: Arc<[f32]> = widths.into();
        let table = MeasuredColumns { anchor, key, source, revision, widths: widths.clone(), hold_until: None, used: true };
        match index {
            Ok(i) => self.tables[i] = table,
            Err(i) => self.tables.insert(i, table),
        }
        widths
    }

    pub fn reset(&mut self, anchor: u64) {
        if let Ok(i) = self.tables.binary_search_by_key(&anchor, |table| table.anchor) {
            self.tables.remove(i);
        }
    }

    pub fn contains(&self, at: u64) -> bool {
        self.tables.iter().any(|t| t.source.start <= at && at <= t.source.end)
    }

    pub fn finish_layout(&mut self) -> Option<Duration> {
        self.edited = None;
        self.tables.retain_mut(|table| std::mem::take(&mut table.used));
        if self.tables.is_empty() { self.intrinsic = Vec::new(); }
        if self.tables.len() < self.tables.capacity() / 4 { self.tables.shrink_to_fit(); }
        self.tables.iter().filter_map(|table| table.hold_until).min()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    pub fn estimated_bytes(&self) -> usize {
        self.tables.capacity() * std::mem::size_of::<MeasuredColumns>()
            + self.intrinsic.capacity() * std::mem::size_of::<Option<(u64, f32)>>()
            + self.tables.iter().map(|t| std::mem::size_of_val(&*t.widths) + 2 * std::mem::size_of::<usize>()).sum::<usize>()
    }

    /// 表和补丁均按原文位置排序，线性移动仍存在的结构锚点，无需查看单元格文字。
    pub fn rebase(&mut self, edits: impl Iterator<Item = (u64, u64, u64)> + Clone) {
        if self.tables.is_empty() { return; }
        self.deletion = edits.clone().count() == 1 && edits.clone().any(|(from, to, inserted)| to > from && inserted == 0);
        let mut shift = 0i128;
        for (from, to, inserted) in edits.clone() {
            let start = (i128::from(from) + shift) as u64;
            let end = start + inserted;
            self.edited = Some(match self.edited.take() {
                Some(dirty) => dirty.start.min(start)..dirty.end.saturating_sub(to - from).saturating_add(inserted).max(end),
                None => start..end,
            });
            shift += i128::from(inserted) - i128::from(to - from);
        }
        let mut edits = edits.peekable();
        let mut shift = 0i128;
        self.tables.retain_mut(|table| {
            while let Some(&(from, to, inserted)) = edits.peek() {
                if to > table.anchor { break; }
                shift += i128::from(inserted) - i128::from(to - from);
                edits.next();
            }
            if edits.peek().is_some_and(|&(from, _, _)| from <= table.anchor) { return false; }
            table.anchor = (i128::from(table.anchor) + shift) as u64;
            true
        });
    }

    pub fn replace_window(&mut self, old: &super::parse::ParsedWindow, new: &super::parse::ParsedWindow) {
        if self.tables.is_empty() { return; }
        let base = old.identity.from_byte;
        if base != new.identity.from_byte {
            if old.identity.revision != new.identity.revision { self.tables.clear(); }
            self.tables.retain(|t| (new.identity.from_byte..new.identity.to_byte).contains(&t.anchor));
        } else if old.source != new.source {
            // 宿主撤销/重做交付的是新窗口；保留相同前后缀中的锚点。
            let (a, b) = (old.source.as_bytes(), new.source.as_bytes());
            let prefix = a.iter().zip(b).take_while(|(a,b)| a == b).count();
            let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(a,b)| a == b).count();
            self.rebase(std::iter::once((base + prefix as u64, base + (a.len() - suffix) as u64, (b.len() - prefix - suffix) as u64)));
            self.deletion = false;
        }
    }
}


#[derive(Clone, Copy, Default)]
struct Column {
    minimum: f32,
    preferred: f32,
    fixed: bool,
}

#[derive(Default)]
struct MeasuredColumn {
    natural: Option<f32>,
    fixed: Option<f32>,
}

/// 普通单元格边测边归入列，只给合并格额外记录跨度。测量结果不持有正文。
pub(crate) struct ColumnMeasures {
    columns: Vec<MeasuredColumn>,
    spans: Vec<(Range<usize>, f32)>,
}

impl ColumnMeasures {
    pub fn new(count: usize) -> Self {
        Self { columns: (0..count).map(|_| MeasuredColumn::default()).collect(), spans: Vec::new() }
    }

    pub fn cell(&mut self, column: usize, span: usize, natural: f32, fixed: impl Iterator<Item = f32>) {
        let end = column.saturating_add(span).min(self.columns.len());
        let Some(group) = self.columns.get_mut(column..end).filter(|g| !g.is_empty()) else { return };
        for (col, width) in group.iter_mut().zip(fixed) {
            if width.is_finite() && width > 0.0 {
                col.fixed = Some(col.fixed.unwrap_or(0.0).max(width));
            }
        }
        if group.len() == 1 {
            group[0].natural = Some(group[0].natural.unwrap_or(0.0).max(natural));
        } else {
            self.spans.push((column..end, natural));
        }
    }

    pub fn allocate(mut self, available: f32, em: f32, chrome: f32) -> Vec<f32> {
        let fit = |content: f32| ((content + chrome) * 100.0).ceil() / 100.0;
        let mut columns: Vec<_> = self.columns.into_iter().map(|col| {
            if let Some(width) = col.fixed {
                return Column { minimum: width, preferred: width, fixed: true };
            }
            let Some(natural) = col.natural else { return Column::default() };
            let content = if natural == 0.0 { em } else { natural };
            Column { minimum: fit(content.min(4.0 * em)), preferred: fit(content), fixed: false }
        }).collect();
        self.spans.sort_by_key(|(span, _)| (span.len(), span.start));
        for (span, natural) in &self.spans {
            let content = if *natural == 0.0 { em } else { *natural };
            let group = &mut columns[span.clone()];
            let missing = fit(content.min(4.0 * em)) - group.iter().map(|c| c.minimum).sum::<f32>();
            if missing > 0.0 && let Some(col) = group.iter_mut().rfind(|c| !c.fixed) {
                col.minimum += missing;
                col.preferred = col.preferred.max(col.minimum);
            }
        }
        // 合并标题用完整跨度排版。先满足独立单元格，只有整表有余量时才为标题扩宽。
        let base = distribute(&columns, available);
        if self.spans.is_empty() || base.iter().sum::<f32>() >= available - 0.005 { return base; }
        for (col, width) in columns.iter_mut().zip(base) {
            col.minimum = width;
            col.preferred = width;
        }
        for (span, natural) in self.spans {
            let group = &mut columns[span];
            let missing = fit(natural) - group.iter().map(|c| c.preferred).sum::<f32>();
            if missing > 0.0 && let Some(col) = group.iter_mut().rfind(|c| !c.fixed) {
                col.preferred += missing;
            }
        }
        distribute(&columns, available)
    }
}

/// 找到共同的长列宽度。已放得下的短列和显式定宽列只取自身需要的空间。
fn distribute(columns: &[Column], available: f32) -> Vec<f32> {
    let total = |preferred: bool| columns.iter().map(|c| if preferred { c.preferred } else { c.minimum }).sum::<f32>();
    if total(true) <= available { return columns.iter().map(|c| c.preferred).collect(); }
    if total(false) >= available { return columns.iter().map(|c| c.minimum).collect(); }

    // 所有弹性列在同一水位上增长。求解区间受正文宽度约束，不受超长内容长度影响。
    let mut low = 0.0_f64;
    let mut high = f64::from(available);
    for _ in 0..32 {
        let level = (low + high) * 0.5;
        let used: f64 = columns.iter().map(|c| level.clamp(c.minimum.into(), c.preferred.into())).sum();
        if used > f64::from(available) { high = level; } else { low = level; }
    }
    columns.iter().map(|c| (low as f32).clamp(c.minimum, c.preferred)).collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_width_policy_keeps_short_tables_compact_and_preserves_content_minima() {
        let layout = |natural: &[f32], available: f32| {
            let mut table = ColumnMeasures::new(natural.len());
            for (i, &width) in natural.iter().enumerate() { table.cell(i, 1, width, std::iter::empty()); }
            table.allocate(available, 14.0, 23.0)
        };
        for n in [2, 6] {
            let cells = vec![28.0; n];
            assert_eq!(layout(&cells, 800.0), vec![51.0; n]);
            assert_eq!(layout(&cells, 20.0), vec![51.0; n]);
        }
        assert_eq!(layout(&[0.0, 0.0], 800.0), vec![37.0, 37.0]);
        assert_eq!(layout(&[180.0, 52.0], 1200.0), vec![203.0, 75.0]);
        assert_eq!(layout(&[14.0, 1600.0], 800.0), vec![37.0, 763.0]);
        assert_eq!(layout(&[10_000.0, 500.0, 28.0], 551.0), vec![250.0, 250.0, 51.0]);
        assert_eq!(layout(&[10_000.0, 500.0, 28.0], 180.0), vec![79.0, 79.0, 51.0]);

        let mut table = ColumnMeasures::new(3);
        table.cell(0, 1, 980.0, [120.0].into_iter());
        table.cell(1, 1, 28.0, std::iter::empty());
        table.cell(2, 1, 28.0, std::iter::empty());
        table.cell(1, 2, 280.0, std::iter::empty());
        assert_eq!(table.allocate(800.0, 14.0, 23.0), vec![120.0, 51.0, 252.0]);
    }
}

//! 原版 mountWindow/viewport 的平台无关规则适配；输入仅为原生索引发布的元数据。
//! 不扫描正文、不建立语法索引。高度树只服务 GPUI 占位与阅读锚点。
use super::{protocol::*, session_window::policy};
use serde::Deserialize;
use std::collections::VecDeque;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockSummary {
    pub id: String,
    pub from_byte: u64,
    pub to_byte: u64,
    pub from_line: u64,
    pub to_line: u64,
    pub mode: String,
    pub kind: String,
    pub semantic_complete: bool,
    pub layout_units: usize,
}
#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub from: u64,
    pub to: u64,
    pub estimate: f32,
    pub semantic: bool,
    blocks: usize,
}
impl Group {
    pub fn contains(&self, byte: u64) -> bool { self.from <= byte && byte < self.to }
    /// 活动控件先应用局部输入，组边界随后才收到事务确认；期间必须保持挂载。
    /// 只读邻窗仍须完整匹配边界，不能把过期正文当作当前内容显示。
    pub fn matches_mount(&self, identity: &WindowIdentity, active: bool) -> bool {
        self.from == identity.from_byte && (active || self.to == identity.to_byte)
    }
}

/// 与 MarkdownStableGroups 相同的 32 KiB / 64 块封口规则。
/// 已挂载窗口由宿主提供 retained 边界；编辑后不强制重建活动输入控件。
pub fn groups(blocks: impl IntoIterator<Item = BlockSummary>, retained: &[(u64, u64)]) -> Result<Vec<Group>, ControlError> {
    let mut result: Vec<Group> = Vec::new();
    for block in blocks {
        if block.to_byte <= block.from_byte { return Err(ControlError::InvalidRange); }
        let semantic = block.mode == "semantic" && block.semantic_complete && block.kind != "native_region";
        let lines = (block.to_line.saturating_sub(block.from_line)).max(1);
        let wrapped = (block.to_byte - block.from_byte).div_ceil(160);
        let height = if block.kind == "native_region" {
            (block.layout_units.max(1) * 32) as f32
        } else {
            lines.max(wrapped) as f32 * 24.0 + block.layout_units.max(1) as f32 * 8.0
        };
        let keep = retained.iter().find(|&&(from, to)| from <= block.from_byte && block.to_byte <= to);
        if let Some(previous) = result.last_mut() {
            if previous.to != block.from_byte { return Err(ControlError::NeedsContext("原版索引尚未提供连续结构范围".into())); }
            let same_retained = keep.is_some_and(|&(from, _)| previous.from == from);
            let retained_boundary = retained.iter().any(|&(from, to)| from == block.from_byte || to == block.from_byte);
            if same_retained || (!retained_boundary && previous.semantic && semantic && previous.blocks < policy().group_blocks && block.to_byte - previous.from <= policy().group_bytes) {
                previous.to = block.to_byte;
                previous.estimate += height;
                previous.semantic &= semantic;
                previous.blocks += 1;
                continue;
            }
        }
        result.push(Group { id: block.id, from: block.from_byte, to: block.to_byte, estimate: height, semantic, blocks: 1 });
        // 派生元数据也计字节；此预算限制适配器缓存，不限制文件打开尺寸。
        if result.len() * (std::mem::size_of::<Group>() + 64) > 32 * 1024 * 1024 {
            return Err(ControlError::NeedsContext("窗口元数据超过宿主缓存预算，需要原版索引提供更粗粒度的分页几何".into()));
        }
    }
    Ok(result)
}

#[derive(Default)]
pub struct Heights { values: Vec<f32>, tree: Vec<f64> }
impl Heights {
    pub fn new(values: impl IntoIterator<Item = f32>) -> Self {
        let values: Vec<_> = values.into_iter().map(|v| v.max(1.0)).collect();
        let mut tree = vec![0.0; values.len() + 1];
        for i in 1..tree.len() {
            tree[i] += values[i - 1] as f64;
            let parent = i + (i & i.wrapping_neg());
            if parent < tree.len() { tree[parent] += tree[i]; }
        }
        Self { values, tree }
    }
    pub fn top(&self, mut index: usize) -> f32 {
        index = index.min(self.values.len());
        let mut value = 0.0;
        while index > 0 { value += self.tree[index]; index -= index & index.wrapping_neg(); }
        value as f32
    }
    pub fn total(&self) -> f32 { self.top(self.values.len()) }
    pub fn at(&self, top: f32) -> usize {
        let mut index = 0;
        let mut height = 0.0;
        let mut bit = 1;
        while bit * 2 <= self.values.len() { bit *= 2; }
        while bit > 0 {
            let next = index + bit;
            if next < self.tree.len() && height + self.tree[next] <= top.max(0.0) as f64 {
                index = next; height += self.tree[next];
            }
            bit /= 2;
        }
        index.min(self.values.len().saturating_sub(1))
    }
    pub fn update(&mut self, index: usize, height: f32) -> f32 {
        if !height.is_finite() || height < 1.0 || index >= self.values.len() { return 0.0; }
        let delta = height - self.values[index];
        self.values[index] = height;
        let mut i = index + 1;
        while i < self.tree.len() { self.tree[i] += delta as f64; i += i & i.wrapping_neg(); }
        delta
    }
}

pub struct ScrollDemand {
    previous: Option<(f32, f64)>, velocity: f64, before: f64, after: f64,
    samples: VecDeque<f64>, last_scroll: f64,
}
impl Default for ScrollDemand {
    fn default() -> Self { Self { previous: None, velocity: 0.0, before: policy().base_screens as f64, after: policy().base_screens as f64, samples: VecDeque::new(), last_scroll: f64::NEG_INFINITY } }
}
impl ScrollDemand {
    pub fn sample(&mut self, top: f32, height: f32, now: f64) {
        let previous = self.previous.replace((top, now));
        self.last_scroll = now;
        let Some((at, time)) = previous else { self.velocity = 0.0; return; };
        if now - time > policy().quiet_ms as f64 || (top - at).abs() > height * policy().forward_screens as f32 { self.velocity = 0.0; return; }
        let speed = (top - at) as f64 / (now - time).max(1.0);
        self.velocity = self.velocity * 0.65 + speed * 0.35;
    }
    pub fn reset(&mut self, top: f32, now: f64) { self.previous = Some((top, now)); self.velocity = 0.0; }
    pub fn prepared(&mut self, ms: f64) { self.samples.push_back(ms.max(0.0)); if self.samples.len() > 32 { self.samples.pop_front(); } }
    pub fn range(&mut self, height: f32, now: f64) -> (f32, f32, f32, f32, bool) {
        let scrolling = now - self.last_scroll < policy().quiet_ms as f64;
        if !scrolling { self.velocity = 0.0; self.before = policy().base_screens as f64; self.after = self.before; }
        let preparation = if self.samples.is_empty() { 300.0 } else {
            let mut sorted: Vec<_> = self.samples.iter().copied().collect(); sorted.sort_by(f64::total_cmp);
            sorted[((sorted.len() as f64 * 0.95).ceil() as usize - 1).min(sorted.len() - 1)] + 150.0
        };
        let screens = (self.velocity.abs() * preparation / height.max(1.0) as f64).clamp(policy().base_screens as f64, policy().forward_screens as f64);
        if self.velocity < 0.0 { self.before = self.before.max(screens); } else { self.after = self.after.max(screens); }
        (self.before as f32 * height, self.after as f32 * height,
         (self.before + policy().retain_screens as f64) as f32 * height,
         (self.after + policy().retain_screens as f64) as f32 * height, scrolling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ime_mount_survives_pending_edits_partial_ack_and_deletion() {
        use crate::editor_control::host::WindowHostAdapter;
        let mut host = WindowHostAdapter::new(DocumentWindow {
            document_id: "ime".into(), revision: 1, from_byte: 100, to_byte: 106,
            text: "你好".into(), inline_templates: Vec::new(),
        }, "input", 0, WindowContext::Complete).unwrap();
        host.mark_layout_ready(LayoutStamp {
            identity: host.parsed.identity.clone(), width: 600.0,
            font_generation: 0, resource_generation: 0, zoom: 1.0,
        }, 24.0).unwrap();
        let mut group = Group { id: "g".into(), from: 100, to: 106, estimate: 24.0, semantic: true, blocks: 1 };
        host.queue_edit(vec![SourceEdit { from_byte: 106, to_byte: 106, inserted_text: "n".into() }],
            DocumentSelection::caret(107), "typing").unwrap();
        let Some(ControlEvent::Submit(first)) = host.next_submission() else { panic!("first input") };
        host.queue_edit(vec![SourceEdit { from_byte: 106, to_byte: 107, inserted_text: "ni".into() }],
            DocumentSelection::caret(108), "typing").unwrap();
        assert!(group.matches_mount(&host.parsed.identity, true));
        assert!(!group.matches_mount(&host.parsed.identity, false));

        // 第一笔确认时第二笔仍未确认，活动窗口继续使用自己的乐观边界。
        host.acknowledge(&first.operation_id, 2).unwrap();
        group.to = 107;
        assert!(group.matches_mount(&host.parsed.identity, true));
        assert!(!group.matches_mount(&host.parsed.identity, false));
        let Some(ControlEvent::Submit(second)) = host.next_submission() else { panic!("second input") };
        host.acknowledge(&second.operation_id, 3).unwrap();
        group.to = 108;
        assert!(group.matches_mount(&host.parsed.identity, false));

        host.queue_edit(vec![SourceEdit { from_byte: 100, to_byte: 108, inserted_text: String::new() }],
            DocumentSelection::caret(100), "delete").unwrap();
        assert_eq!(&*host.parsed.source, "");
        assert!(group.matches_mount(&host.parsed.identity, true));
        assert!(!group.matches_mount(&host.parsed.identity, false));
        let mut other = host.parsed.identity.clone();
        other.from_byte = 99;
        assert!(!group.matches_mount(&other, true));
    }
    #[test]
    fn original_grouping_retains_edit_window_without_overlaps() {
        let blocks = || (0..200).map(|i| BlockSummary { id: i.to_string(), from_byte: i*8, to_byte: (i+1)*8, from_line:i*2,to_line:i*2+2, mode:"semantic".into(), kind:"paragraph".into(), semantic_complete:true,layout_units:1 });
        let original = groups(blocks(), &[]).unwrap();
        assert_eq!(original.iter().map(|g|g.blocks).collect::<Vec<_>>(), vec![64,64,64,8]);
        let retained = groups(blocks(), &[(40,520)]).unwrap();
        assert!(retained.iter().any(|g|g.from==40 && g.to==520));
        assert!(retained.windows(2).all(|w|w[0].to==w[1].from));
        assert_eq!(retained.iter().map(|g|g.blocks).sum::<usize>(),200);
    }
    #[test]
    fn height_corrections_and_original_demand_keep_bounded_forward_range() {
        let mut heights=Heights::new([20.0,30.0,40.0]);
        assert_eq!(heights.at(49.0),1); assert_eq!(heights.total(),90.0);
        assert_eq!(heights.update(0,100.0),80.0); assert_eq!(heights.top(2),130.0); assert_eq!(heights.at(129.0),1);
        let mut demand=ScrollDemand::default();
        assert_eq!(demand.range(800.0,0.0).1,2400.0);
        demand.sample(0.0,800.0,0.0); demand.sample(4000.0,800.0,20.0);
        assert_eq!(demand.range(800.0,20.0).1,4800.0);
        demand.sample(0.0,800.0,40.0); assert_eq!(demand.range(800.0,40.0).1,4800.0);
        assert_eq!(demand.range(800.0,341.0).1,2400.0);
        demand.sample(100000.0,800.0,342.0); assert_eq!(demand.range(800.0,342.0).1,2400.0);
    }
}

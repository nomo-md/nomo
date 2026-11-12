//! 两处侧栏共用的拖拽布局。手势期间只保留路径和可见行位置，不扫描磁盘或持有文档。
use super::{components::{icon, IconName}, motion, path_drag::{ItemMotion, PathDrag}};
use crate::theme::Theme;
use gpui::{div, px, prelude::*, AnyElement};
use std::{cell::RefCell, collections::HashMap, ops::Range, path::{Path, PathBuf}, rc::Rc, sync::Arc, time::{Duration, Instant}};

pub(crate) const ROW_HEIGHT: f32 = 30.0;
const SLIDE: Duration = Duration::from_millis(180);

#[derive(Clone)]
pub(crate) struct Row {
    pub path: Arc<Path>,
    pub root: Arc<Path>,
    pub depth: usize,
    pub folder: bool,
}

#[derive(Clone, PartialEq)]
struct Target { parent: Option<usize>, boundary: usize }

#[derive(Clone)]
pub(crate) struct Placement {
    pub root: PathBuf,
    /// 空路径只表示主页入口的显示顺序，绝不用于文件系统移动。
    pub parent: PathBuf,
    pub before: Option<PathBuf>,
    pub after: Option<PathBuf>,
    pub siblings: Vec<PathBuf>,
}

#[derive(Clone, Default)]
pub(crate) struct Layout { source: Option<Range<usize>>, gap: Option<usize> }

impl Layout {
    fn removed_before(&self, index: usize) -> usize {
        self.source.as_ref().map_or(0, |source| index.saturating_sub(source.start).min(source.len()))
    }
    pub fn gap(&self) -> Option<usize> { self.gap.map(|index| index - self.removed_before(index)) }
    pub fn len(&self, count: usize) -> usize { count - self.source.as_ref().map_or(0, Range::len) + usize::from(self.gap.is_some()) }
    pub fn original(&self, mut index: usize) -> Option<usize> {
        if let Some(gap) = self.gap() {
            if index == gap { return None; }
            if index > gap { index -= 1; }
        }
        if let Some(source) = &self.source { if index >= source.start { index += source.len(); } }
        Some(index)
    }
    fn boundary(&self, index: usize, after_gap: bool) -> usize {
        let result = index - self.removed_before(index);
        result + usize::from(self.gap.is_some_and(|gap| gap < index || (gap == index && after_gap)))
    }
}

struct Position { spring: motion::Spring<1>, last: Instant }
impl Position {
    fn new(value: f32, now: Instant) -> Self { Self { spring: motion::Spring::layout([value]), last: now } }
    fn value(&self, _: Instant) -> f32 { self.spring.p[0] }
    fn retarget(&mut self, to: f32, now: Instant, reduced: bool) {
        self.spring.target = [to];
        if reduced { self.spring.snap(); } else { self.spring.step(now.saturating_duration_since(self.last).as_secs_f32()); }
        self.last = now;
    }
    fn running(&self, _: Instant) -> bool { self.spring.running() }
}

struct RowPosition { motion: Rc<RefCell<ItemMotion>>, frame: u64 }
struct Group {
    path: Arc<Path>, start: usize, end: usize, depth: usize, alpha: motion::ScalarMotion,
    edges: Option<(Position, Position)>,
}

#[derive(Default)]
pub(crate) struct TreeDrag {
    item: Option<PathDrag>,
    rows: Vec<Row>,
    ends: Vec<usize>,
    parents: Vec<Option<usize>>,
    target: Option<Target>,
    layout: Layout,
    folder_section: Option<usize>,
    positions: HashMap<Arc<Path>, RowPosition>,
    gap_position: Option<(Position, Position)>,
    section_position: Option<Position>,
    groups: Vec<Group>,
    frame: u64,
    frame_time: Option<Instant>,
    pub committing: bool,
    revision: usize,
}

impl TreeDrag {
    /// 主页恢复两组平铺入口；标题占空间但不作为路径或拖拽对象。
    pub fn set_sections(&mut self, folders: usize) { self.folder_section = Some(folders); }
    pub fn folder_section_end(&self) -> Option<usize> {
        self.folder_section.map(|folders| self.layout.boundary(folders,
            self.target.as_ref().is_some_and(|target| target.parent.is_some()) || self.item.as_ref().is_some_and(|item| item.directory)))
    }
    fn visual_index(&self, index: usize, end: bool) -> usize {
        self.folder_section_end().map_or(index, |folders| index + 1 + usize::from(index > folders || (index == folders && !end)))
    }
    fn row_indent(&self, depth: usize) -> f32 {
        (if self.folder_section.is_some() { 12.0 } else { 34.0 }) + depth as f32 * 12.0
    }
    pub fn active(&self) -> bool { self.item.is_some() }
    pub fn item(&self) -> Option<&PathDrag> { self.item.as_ref() }
    pub fn needs_commit_check(&self, revision: usize) -> bool { self.committing && self.revision != revision }
    pub fn layout(&self) -> Layout { self.layout.clone() }
    pub fn landing_index(&self) -> Option<usize> { self.layout.source.as_ref().filter(|_| self.committing).map(|source| source.start) }
    pub fn begin(&mut self, item: &PathDrag, rows: Vec<Row>, revision: usize, reduced: bool) {
        if self.item.as_ref().is_some_and(|old| old.path == item.path) && self.revision == revision { return; }
        self.rows = rows;
        self.revision = revision;
        self.ends = vec![self.rows.len(); self.rows.len()];
        self.parents = vec![None; self.rows.len()];
        let mut stack: Vec<usize> = Vec::new();
        for index in 0..self.rows.len() {
            while stack.last().is_some_and(|old| self.rows[*old].depth >= self.rows[index].depth) {
                self.ends[stack.pop().unwrap()] = index;
            }
            self.parents[index] = stack.last().copied();
            stack.push(index);
        }
        let source = self.rows.iter().position(|row| row.path.as_ref() == item.path);
        self.layout.source = source.map(|index| index..self.ends[index]);
        self.layout.gap = None;
        self.target = None;
        self.item = Some(item.clone());
        self.committing = false;
        if let Some(index) = source { self.set_target(Some(Target { parent: self.parents[index], boundary: index }), reduced); }
    }

    pub fn order_only(&self) -> bool {
        self.target.as_ref().zip(self.item.as_ref()).is_some_and(|(target, item)| target.parent.is_none_or(|index| item.path.parent() == Some(self.rows[index].path.as_ref())))
    }

    /// 等目录扫描和顺序写入均反映到模型后再撤掉落位预览，避免松手时闪回旧位置。
    pub fn check_commit(&mut self, revision: usize, rows: impl FnOnce() -> Vec<Row>, reduced: bool) {
        if !self.committing || self.revision == revision { return; }
        self.revision = revision;
        let Some((placement, item)) = self.placement().zip(self.item.as_ref()) else { return; };
        let rows = rows();
        let path = if placement.parent.as_os_str().is_empty() { item.path.clone() }
            else { placement.parent.join(item.path.file_name().unwrap_or_default()) };
        let moved = path != item.path;
        if moved && self.layout.source.is_some() && rows.iter().any(|row| row.path.as_ref() == item.path) { return; }
        let ready = if let Some(index) = rows.iter().position(|row| row.path.as_ref() == path) {
            let depth = rows[index].depth;
            let next = rows[index + 1..].iter().take_while(|row| row.depth >= depth)
                .find(|row| row.depth == depth && (self.folder_section.is_none() || row.folder == item.directory));
            next.map(|row| row.path.as_ref()) == placement.before.as_deref()
        } else {
            moved && rows.iter().position(|row| row.path.as_ref() == placement.parent)
                .is_some_and(|index| rows.get(index + 1).is_none_or(|next| next.depth <= rows[index].depth))
        };
        if ready { self.end(reduced); }
    }

    pub fn hover(&mut self, x: f32, y: f32, reduced: bool) -> bool {
        if self.committing || self.rows.is_empty() { return false; }
        let count = self.layout.len(self.rows.len());
        let physical = (y.max(0.0) / ROW_HEIGHT).floor() as usize;
        let part = y.max(0.0) % ROW_HEIGHT / ROW_HEIGHT;
        let display = if let Some(folders) = self.folder_section_end() {
            if physical == 0 || physical == folders + 1 {
                let boundary = if physical == 0 { 0 } else { self.folder_section.unwrap_or(0) };
                return self.set_target(Some(Target { parent: None, boundary }), reduced);
            }
            physical - 1 - usize::from(physical > folders + 1)
        } else { physical };
        // 主页只调整入口顺序；工作区的文件夹只在原父目录下排序，不再作为目录移动手势。
        if self.folder_section.is_some() || self.item.as_ref().is_some_and(|item| item.directory) {
            return self.reorder_at(display, part >= 0.5, reduced);
        }
        let candidate = if display >= count {
            // 文件夹最后一个子项以下的空白仍属于该文件夹。
            let last = count.checked_sub(1).and_then(|index| self.layout.original(index)
                .or_else(|| index.checked_sub(1).and_then(|previous| self.layout.original(previous))));
            last.map(|index| Target { parent: if self.rows[index].folder && x >= self.row_indent(self.rows[index].depth) { Some(index) } else { self.parents[index] }, boundary: self.ends[index] })
        } else if let Some(index) = self.layout.original(display) {
            let previous_folder = display.checked_sub(1).and_then(|previous| self.layout.original(previous))
                .and_then(|previous| if self.rows[previous].folder { Some(previous) } else { self.parents[previous] })
                .filter(|previous| part < 0.25 && self.ends[*previous] == index && self.folder_section != Some(index)
                    && x >= self.row_indent(self.rows[*previous].depth));
            if let Some(previous) = previous_folder {
                Some(Target { parent: Some(previous), boundary: index })
            } else if self.rows[index].folder && part >= 0.22 {
                Some(Target { parent: Some(index), boundary: index + 1 })
            } else {
                Some(Target { parent: self.parents[index], boundary: if part < 0.5 { index } else { self.ends[index] } })
            }
        } else { return false; };
        self.set_target(candidate, reduced)
    }

    /// 网格使用同一份移除／插槽布局，落点由卡片横向中线确定。
    pub fn reorder_at(&mut self, display: usize, after: bool, reduced: bool) -> bool {
        if self.committing { return false; }
        let Some(source) = self.layout.source.as_ref() else { return self.set_target(None, reduced); };
        let parent = self.parents[source.start];
        let candidate = if display >= self.layout.len(self.rows.len()) {
            self.rows.iter().enumerate().rfind(|(index, _)| self.parents[*index] == parent && !source.contains(index))
                .map(|(index, _)| Target { parent, boundary: self.ends[index] })
                .or(Some(Target { parent, boundary: source.start }))
        } else if let Some(original) = self.layout.original(display) {
            let mut sibling = Some(original);
            while sibling.is_some_and(|index| self.parents[index] != parent) {
                sibling = sibling.and_then(|index| self.parents[index]);
            }
            sibling.map(|index| Target { parent, boundary: if after || index != original { self.ends[index] } else { index } })
        } else { return false; };
        self.set_target(candidate, reduced)
    }

    pub fn leave(&mut self, reduced: bool) -> bool {
        if self.committing { return false; }
        self.set_target(None, reduced)
    }

    fn allowed(&self, target: &Target) -> bool {
        let Some(item) = &self.item else { return false; };
        if item.directory && self.layout.source.as_ref().is_none_or(|source| self.parents[source.start] != target.parent) { return false; }
        if self.folder_section.is_some() && target.parent.is_some() { return false; }
        if let Some(index) = target.parent {
            let parent = self.rows[index].path.as_ref();
            parent != item.path && !(item.directory && parent.starts_with(&item.path))
        } else {
            self.layout.source.as_ref().is_some_and(|source| self.rows[source.start].depth == 0)
                && self.folder_section.is_none_or(|folders| if item.directory { target.boundary <= folders } else { target.boundary >= folders })
        }
    }

    fn set_target(&mut self, target: Option<Target>, reduced: bool) -> bool {
        let target = target.filter(|target| self.allowed(target));
        if self.target == target { return false; }
        self.target = target;
        self.layout.gap = self.target.as_ref().map(|target| target.boundary);
        let parent = self.target.as_ref().and_then(|target| target.parent)
            .filter(|_| self.item.as_ref().is_some_and(|item| !item.directory));
        let path = parent.map(|index| self.rows[index].path.clone());
        self.groups.retain(|group| group.alpha.running() || group.alpha.value() > 0.01);
        for group in &mut self.groups { group.alpha.set(path.as_ref() == Some(&group.path), reduced, SLIDE); }
        if let Some(index) = parent {
            if !self.groups.iter().any(|group| group.path == self.rows[index].path) {
                if self.groups.len() >= 3 { self.groups.remove(0); }
                let mut alpha = motion::ScalarMotion::default();
                alpha.set(true, reduced, SLIDE);
                self.groups.push(Group { path: self.rows[index].path.clone(), start: index, end: self.ends[index], depth: self.rows[index].depth, alpha, edges: None });
            }
        }
        true
    }

    pub fn is_target(&self, path: &Path) -> bool {
        !self.committing && self.item.as_ref().is_some_and(|item| !item.directory)
            && self.target.as_ref().and_then(|target| target.parent).is_some_and(|index| self.rows[index].path.as_ref() == path)
    }

    pub fn placement(&self) -> Option<Placement> {
        let target = self.target.as_ref()?;
        if self.layout.source.as_ref().is_some_and(|source| self.parents[source.start] == target.parent
            && target.boundary >= source.start && target.boundary <= source.end) { return None; }
        let (parent, root) = target.parent.map(|index| (self.rows[index].path.to_path_buf(), self.rows[index].root.to_path_buf())).unwrap_or_default();
        let siblings: Vec<_> = self.rows.iter().enumerate().filter(|(index, _)| self.parents[*index] == target.parent && !self.layout.source.as_ref().is_some_and(|source| source.contains(index)))
            .map(|(index, row)| (index, row.path.to_path_buf())).collect();
        let same_group = |index: usize| self.folder_section.is_none()
            || self.item.as_ref().is_some_and(|item| item.directory == self.rows[index].folder);
        let before = siblings.iter().find(|(index, _)| *index >= target.boundary && same_group(*index)).map(|(_, path)| path.clone());
        let after = self.folder_section.and_then(|_| siblings.iter().rev()
            .find(|(index, _)| *index < target.boundary && same_group(*index)).map(|(_, path)| path.clone()));
        if self.folder_section.is_some() && before.is_none() && after.is_none() { return None; }
        Some(Placement { root, parent, before, after, siblings: siblings.into_iter().map(|(_, path)| path).collect() })
    }

    pub fn commit(&mut self, reduced: bool) {
        self.seed_landing(true);
        self.committing = true;
        for group in &mut self.groups { group.alpha.set(false, reduced, SLIDE); }
    }

    pub fn end(&mut self, reduced: bool) {
        if self.committing {
            if let Some((placement, item)) = self.placement().zip(self.item.as_ref()) {
                if !placement.parent.as_os_str().is_empty() && item.path.parent() != Some(&placement.parent) {
                    let path = placement.parent.join(item.path.file_name().unwrap_or_default());
                    if let Some(position) = self.positions.remove(item.path.as_path()) { self.positions.insert(path.into(), position); }
                }
            }
        }
        self.item = None;
        self.target = None;
        self.layout = Layout::default();
        self.gap_position = None;
        self.committing = false;
        for group in &mut self.groups { group.alpha.set(false, reduced, SLIDE); }
        self.rows.clear(); self.ends.clear(); self.parents.clear();
    }

    fn seed_landing(&mut self, landed: bool) {
        let Some(item) = &self.item else { return; };
        let mut visual = item.visual.lock().expect("drag geometry lock");
        let Some(pose) = visual.as_mut().filter(|pose| pose.row && !pose.settled) else { return; };
        pose.settled = true;
        let pose = *pose;
        drop(visual);
        let position = self.positions.entry(item.path.as_path().into()).or_insert_with(|| RowPosition { motion: Default::default(), frame: self.frame });
        position.frame = self.frame;
        position.motion.borrow_mut().seed(pose, landed);
    }

    pub fn release(&mut self, reduced: bool) {
        if self.committing { return; }
        self.seed_landing(false);
        self.end(reduced);
    }

    /// uniform_list 会先以单行范围测量，再渲染可见范围。按整帧保留状态，
    /// 不能在每次行回调中清理，否则测量调用会重置其他行正在进行的让位。
    pub fn begin_frame(&mut self, reduced: bool) {
        let now = Instant::now();
        self.frame = self.frame.wrapping_add(1);
        let previous = self.frame.wrapping_sub(1);
        self.positions.retain(|path, position| position.frame == previous || self.item.as_ref().is_some_and(|item| item.path.as_path() == path.as_ref()));
        self.frame_time = Some(now);
        if let Some(position) = &mut self.section_position { position.retarget(position.spring.target[0], now, reduced); }
        if let Some(index) = self.layout.gap() {
            let top = self.visual_index(index, false) as f32 * ROW_HEIGHT;
            let depth = self.target.as_ref().and_then(|target| target.parent).map_or(0, |index| self.rows[index].depth + 1);
            let left = self.row_indent(depth);
            let (y, x) = self.gap_position.get_or_insert_with(|| (Position::new(top, now), Position::new(left, now)));
            y.retarget(top, now, reduced);
            x.retarget(left, now, reduced);
        } else { self.gap_position = None; }
        if self.active() {
            let folder_end = self.folder_section_end();
            let visual_index = |index: usize, end: bool| folder_end.map_or(index, |folders| index + 1 + usize::from(index > folders || (index == folders && !end)));
            for group in &mut self.groups {
                let top = visual_index(self.layout.boundary(group.start, false), false) as f32 * ROW_HEIGHT;
                let bottom = visual_index(self.layout.boundary(group.end, true), true) as f32 * ROW_HEIGHT;
                let initial_top = self.positions.get(group.path.as_ref()).map_or(top, |position| position.motion.borrow().y());
                let initial_bottom = group.end.checked_sub(1).and_then(|index| self.rows.get(index))
                    .and_then(|row| self.positions.get(row.path.as_ref())).map_or(bottom, |position| position.motion.borrow().y() + ROW_HEIGHT);
                let (start, end) = group.edges.get_or_insert_with(|| (Position::new(initial_top, now), Position::new(initial_bottom, now)));
                start.retarget(top, now, reduced);
                end.retarget(bottom, now, reduced);
            }
        }
    }

    /// 占位块、组边界与可见行取同一帧的时间，连续换位不重新从旧端点起跳。
    pub fn animate_row(&mut self, path: &Path, index: usize, child: AnyElement, reduced: bool, theme: &Theme) -> AnyElement {
        let now = self.frame_time.unwrap_or_else(Instant::now);
        let position = self.positions.entry(path.into()).or_insert_with(|| RowPosition { motion: Default::default(), frame: self.frame });
        position.frame = self.frame;
        ItemMotion::draw(position.motion.clone(), [0.0, index as f32 * ROW_HEIGHT], child, true, now, reduced, theme)
    }

    pub fn section_offset(&mut self, index: usize, reduced: bool) -> f32 {
        let now = self.frame_time.unwrap_or_else(Instant::now);
        let top = index as f32 * ROW_HEIGHT;
        let position = self.section_position.get_or_insert_with(|| Position::new(top, now));
        position.retarget(top, now, reduced);
        position.value(now) - top
    }

    pub fn running(&self) -> bool {
        let now = Instant::now();
        self.positions.iter().any(|(path, position)| (self.committing || self.item.as_ref().is_none_or(|item| item.path.as_path() != path.as_ref())) && position.motion.borrow().running())
            || self.gap_position.as_ref().is_some_and(|(y, x)| y.running(now) || x.running(now))
            || self.section_position.as_ref().is_some_and(|position| position.running(now))
            || self.groups.iter().any(|group| group.alpha.running() || group.edges.as_ref().is_some_and(|(start, end)| start.running(now) || end.running(now)))
    }

    /// 此行仅为布局留空；移动中的反馈块独立绘在行之上，避免新槽先跳过去。
    pub fn gap_row(&self, _: &Theme) -> AnyElement {
        div().w_full().h(px(ROW_HEIGHT)).into_any_element()
    }

    pub fn feedback(&self, scroll: f32, inset: f32, theme: &Theme) -> AnyElement {
        let now = self.frame_time.unwrap_or_else(Instant::now);
        div().absolute().inset_0().overflow_hidden()
            .when(!self.committing, |el| el.children(self.gap_position.as_ref().map(|(y, x)| {
                div().absolute().top(px(scroll + y.value(now))).left(px(inset + x.value(now))).right(px(inset + 6.0))
                    .h(px(ROW_HEIGHT)).rounded(px(9.0)).bg(theme.sidebar_active)
            }))).into_any_element()
    }

    pub fn backdrop(&mut self, scroll: f32, inset: f32, theme: &Theme) -> AnyElement {
        self.groups.retain(|group| group.alpha.running() || group.alpha.value() > 0.01);
        let now = self.frame_time.unwrap_or_else(Instant::now);
        div().absolute().inset_0().overflow_hidden().children(self.groups.iter().map(|group| {
            let (top, bottom) = group.edges.as_ref().map(|(start, end)| (start.value(now), end.value(now)))
                .unwrap_or((group.start as f32 * ROW_HEIGHT, group.end as f32 * ROW_HEIGHT));
            let alpha = group.alpha.value();
            div().absolute().left(px(inset + group.depth as f32 * 12.0)).right(px(inset))
                .top(px(scroll + top - 3.0 * alpha)).h(px((bottom - top + 6.0 * alpha).max(ROW_HEIGHT)))
                .rounded(px(12.0)).bg(theme.sidebar_active).opacity(alpha * 0.52)
        })).into_any_element()
    }
}

pub(crate) fn receiving_icon(theme: &Theme) -> AnyElement {
    div().relative().size(px(14.0)).flex_none().child(icon(IconName::FolderOpen, 14.0, theme.fg))
        .child(div().absolute().right(px(-3.0)).bottom(px(-3.0)).rounded(px(2.0)).bg(theme.sidebar_active)
            .child(icon(IconName::ArrowDown, 9.0, theme.fg))).into_any_element()
}

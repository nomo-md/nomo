//! 手动顺序只存路径元数据；不改文件名、不复制正文，达到预算时拒绝新记录并保留旧顺序。
use super::*;
use std::collections::{HashMap, HashSet};

const ORDER_BYTES: usize = 2 * 1024 * 1024;
const ORDER_FOLDERS: usize = 256;

#[derive(Default, PartialEq, Eq)]
pub(crate) struct TreeOrder(pub HashMap<PathBuf, Vec<PathBuf>>);

impl TreeOrder {
    pub fn sort<T>(&self, parent: &Path, entries: &mut [T], path: impl Fn(&T) -> &Path) {
        let Some(order) = self.0.get(parent) else { return; };
        let ranks: HashMap<&Path, usize> = order.iter().enumerate().map(|(index, path)| (path.as_path(), index)).collect();
        entries.sort_by_key(|entry| ranks.get(path(entry)).copied().unwrap_or(usize::MAX));
    }
}

pub(super) fn read(connection: &Connection) -> Result<TreeOrder, UiMessage> {
    let mut query = connection.prepare("SELECT parent,paths FROM tree_order ORDER BY parent LIMIT 257").map_err(|e| e.to_string())?;
    let entries = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    let mut result = TreeOrder::default();
    let mut bytes = 0;
    for entry in entries {
        let (parent, json) = entry.map_err(|e| e.to_string())?;
        bytes += parent.len() + json.len();
        if bytes > ORDER_BYTES || result.0.len() >= ORDER_FOLDERS { return Err(UiMessage::new("tree.orderBudget")); }
        let paths: Vec<PathBuf> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let parent = PathBuf::from(path_key_message(Path::new(&parent))?);
        let order = result.0.entry(parent).or_default();
        let mut seen: HashSet<PathBuf> = order.iter().cloned().collect();
        for path in paths {
            let path = PathBuf::from(path_key_message(&path)?);
            if seen.insert(path.clone()) { order.push(path); }
        }
    }
    Ok(result)
}

pub(super) fn place(connection: &Connection, placement: crate::ui::tree_drag::Placement, path: PathBuf) -> Result<(), UiMessage> {
    let mut orders = read(connection)?;
    let paths = orders.0.entry(placement.parent.clone()).or_default();
    // 已展开的目录以本次可见兄弟顺序为准；折叠目录保留已有顺序。
    if !placement.siblings.is_empty() {
        if placement.parent.as_os_str().is_empty() {
            // 左侧按类型分组，卡片可以混排；侧栏的一次调整不应重置其他卡片的相对顺序。
            paths.retain(|old| *old == path || placement.siblings.contains(old));
            for sibling in placement.siblings { if !paths.contains(&sibling) { paths.push(sibling); } }
        } else { *paths = placement.siblings; }
    }
    paths.retain(|old| *old != path);
    let index = placement.before.as_ref().and_then(|before| paths.iter().position(|old| old == before))
        .or_else(|| placement.after.as_ref().and_then(|after| paths.iter().position(|old| old == after).map(|index| index + 1)))
        .unwrap_or(paths.len());
    paths.insert(index, path);
    write(connection, &orders)
}

pub(super) fn rename(connection: &Connection, renames: &[(PathBuf, PathBuf)]) -> Result<(), UiMessage> {
    let old = read(connection)?;
    let remap = |path: &Path| renames.iter().find_map(|(from, to)| path.strip_prefix(from).ok().map(|suffix| to.join(suffix))).unwrap_or_else(|| path.to_path_buf());
    let mut next = TreeOrder::default();
    for (parent, paths) in old.0 {
        let parent = remap(&parent);
        let mut seen = HashSet::new();
        let paths: Vec<_> = paths.iter().map(|path| remap(path))
            .filter(|path| (parent.as_os_str().is_empty() || path.parent() == Some(&parent)) && seen.insert(path.clone())).collect();
        if !paths.is_empty() { next.0.insert(parent, paths); }
    }
    write(connection, &next)
}

fn write(connection: &Connection, orders: &TreeOrder) -> Result<(), UiMessage> {
    if orders.0.len() > ORDER_FOLDERS { return Err(UiMessage::new("tree.orderBudget")); }
    let mut records = Vec::new();
    let mut bytes: usize = 0;
    for (parent, paths) in &orders.0 {
        let parent = path_key_message(parent)?;
        for path in paths { path_key_message(path)?; }
        let json = serde_json::to_string(paths).map_err(|e| e.to_string())?;
        bytes = bytes.saturating_add(parent.len() + json.len());
        if bytes > ORDER_BYTES { return Err(UiMessage::new("tree.orderBudget")); }
        records.push((parent, json));
    }
    connection.execute("DELETE FROM tree_order", []).map_err(|e| e.to_string())?;
    for (parent, json) in records {
        connection.execute("INSERT INTO tree_order(parent,paths) VALUES (?1,?2)", params![parent,json]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

import type { StructureOutlineRow } from './structureProtocol';

export interface LocalHeadingWindow {
  fromByte: number;
  toByte: number;
  ownedIds: readonly string[];
  headings: Array<{ id: string; title: string; level: number }>;
}

/** 只投影当前编辑窗口；临时身份不充当 URL slug 或原生写入目标。 */
export function projectLocalHeadings(rows: readonly StructureOutlineRow[], windows: Iterable<LocalHeadingWindow>, lastPage = true): StructureOutlineRow[] {
  let result = [...rows];
  for (const window of windows) {
    const owned = new Set(window.ownedIds);
    const first = result.findIndex(row => owned.has(row.id));
    const originals = new Map(result.filter(row => owned.has(row.id)).map(row => [row.id, row]));
    // 当前页没有覆盖编辑范围时，不把屏外临时标题混入分页窗口。
    if (first < 0 && (window.toByte <= (result[0]?.byte ?? 0) || (!lastPage && window.fromByte > (result.at(-1)?.byte ?? 0)))) continue;
    const insertion = first >= 0 ? first : result.findIndex(row => row.byte >= window.fromByte);
    result = result.filter(row => !owned.has(row.id));
    const local: StructureOutlineRow[] = [];
    for (const heading of window.headings) {
      const old = originals.get(heading.id);
      if (old) local.push({ ...old, title: heading.title, level: heading.level as StructureOutlineRow['level'] });
      else local.push({
        ...heading, level: heading.level as StructureOutlineRow['level'], local: true,
        blockId: '', blockHeadingIndex: 0, byte: window.fromByte, line: 0,
        position: 0, collapsed: false, hasChildren: false, parentByte: null,
      });
    }
    // 包含折叠子树的编辑窗口不应展开其隐藏行。
    const visible: StructureOutlineRow[] = [];
    let collapsedLevel = 7;
    for (const row of local) {
      if (row.level > collapsedLevel) continue;
      collapsedLevel = row.collapsed ? row.level : 7;
      visible.push(row);
    }
    result.splice(insertion < 0 ? result.length : insertion, 0, ...visible);
  }
  return result.map((row, index) => ({ ...row, position: (rows[0]?.position ?? 0) + index }));
}

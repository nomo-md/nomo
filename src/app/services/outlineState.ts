import type { OutlineItem } from '../../lib/outline/outlineService';

export function isOutlineItemExpandable(outline: readonly OutlineItem[], index: number) {
  const item = outline[index];
  const next = outline[index + 1];
  return Boolean(item && next && next.level > item.level);
}

export function isOutlineItemExpanded(collapsedOutlineIds: Set<string>, item: OutlineItem) {
  return !collapsedOutlineIds.has(item.id);
}

export function isOutlineItemVisible(
  outline: readonly OutlineItem[],
  collapsedOutlineIds: Set<string>,
  index: number,
) {
  const item = outline[index];
  if (!item) {
    return false;
  }

  let parentLevel = item.level;
  for (let previousIndex = index - 1; previousIndex >= 0; previousIndex -= 1) {
    const candidate = outline[previousIndex];
    if (candidate.level >= parentLevel) {
      continue;
    }
    if (collapsedOutlineIds.has(candidate.id)) {
      return false;
    }
    parentLevel = candidate.level;
  }
  return true;
}

export function toggleOutlineItemExpanded(collapsedOutlineIds: Set<string>, item: OutlineItem) {
  const nextCollapsedIds = new Set(collapsedOutlineIds);
  if (nextCollapsedIds.has(item.id)) {
    nextCollapsedIds.delete(item.id);
  } else {
    nextCollapsedIds.add(item.id);
  }
  return nextCollapsedIds;
}

export function pruneCollapsedOutlineIds(outline: readonly OutlineItem[], collapsedOutlineIds: Set<string>) {
  if (collapsedOutlineIds.size === 0) {
    return collapsedOutlineIds;
  }

  const visibleIds = new Set(outline.map((item) => item.id));
  return new Set(Array.from(collapsedOutlineIds).filter((id) => visibleIds.has(id)));
}

export function getOutlineItemAtLine(outline: readonly OutlineItem[], line: number) {
  let current = outline[0] ?? null;
  for (const item of outline) {
    if (item.line > line) {
      break;
    }
    current = item;
  }
  return current;
}

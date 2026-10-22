export const DEFAULT_MARKDOWN_MOUNT_LIMITS = Object.freeze({
  bytes: 1_048_576,
  layoutUnits: 6_000,
  returnBytes: 838_860,
  returnLayoutUnits: 4_800,
  idleMilliseconds: 2_000,
});

export const MARKDOWN_RESIDENT_LIMITS = Object.freeze({
  views: 32, sourceBytes: 1024 * 1024, domNodes: 20_000, imageBytes: 128 * 1024 * 1024,
});

export type MarkdownMountLimits = Readonly<Record<keyof typeof DEFAULT_MARKDOWN_MOUNT_LIMITS, number>>;

export function normalizeMarkdownWindowingScalePercent(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 100;
  return Math.round(Math.max(75, Math.min(400, value)) / 25) * 25;
}

/** 只缩放容量；时间、视口距离、分组和真实资源成本估算不随比例改变。 */
export function resolveMarkdownWindowingLimits(value: unknown) {
  const ratio = normalizeMarkdownWindowingScalePercent(value) / 100;
  const bytes = Math.floor(DEFAULT_MARKDOWN_MOUNT_LIMITS.bytes * ratio);
  const layoutUnits = Math.floor(DEFAULT_MARKDOWN_MOUNT_LIMITS.layoutUnits * ratio);
  return {
    mount: {
      bytes, layoutUnits,
      returnBytes: Math.floor(bytes * 0.8),
      returnLayoutUnits: Math.floor(layoutUnits * 0.8),
      idleMilliseconds: DEFAULT_MARKDOWN_MOUNT_LIMITS.idleMilliseconds,
    },
    resident: {
      views: Math.floor(MARKDOWN_RESIDENT_LIMITS.views * ratio),
      sourceBytes: Math.floor(MARKDOWN_RESIDENT_LIMITS.sourceBytes * ratio),
      domNodes: Math.floor(MARKDOWN_RESIDENT_LIMITS.domNodes * ratio),
      imageBytes: Math.floor(MARKDOWN_RESIDENT_LIMITS.imageBytes * ratio),
    },
  };
}

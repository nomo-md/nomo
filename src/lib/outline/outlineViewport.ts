/** 大纲导航与章节高亮共用编辑区参考线；返回可见区域内的 CSS 像素。 */
export function getOutlineViewportOffset(viewportHeight: number): number {
  return Math.min(180, Math.max(64, viewportHeight * 0.2));
}

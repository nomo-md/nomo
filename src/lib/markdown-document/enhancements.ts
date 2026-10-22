/** 分类只描述结构；不把不同资源的数量相加作为挂载准入阈值。 */
export interface MarkdownEnhancements {
  inlineCode: number;
  codeBlocks: number;
  diagrams: number;
  inlineMath: number;
  blockMath: number;
  images: number;
  htmlDetails: number;
}
export function emptyEnhancements(): MarkdownEnhancements {
  return { inlineCode: 0, codeBlocks: 0, diagrams: 0, inlineMath: 0, blockMath: 0, images: 0, htmlDetails: 0 };
}
export function addEnhancements(target: MarkdownEnhancements, value: MarkdownEnhancements, sign = 1) {
  for (const key of Object.keys(target) as (keyof MarkdownEnhancements)[]) target[key] += value[key] * sign;
}
export function enhancementTotal(value: MarkdownEnhancements) {
  return Object.values(value).reduce((sum, count) => sum + count, 0);
}

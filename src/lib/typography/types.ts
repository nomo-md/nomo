export type TypographyProfile = 'zh-CN' | 'zh-TW';

export interface TypographyOptions {
  enabled: boolean;
  profile: TypographyProfile;
  hanging: boolean;
}

export const DEFAULT_TYPOGRAPHY: Readonly<TypographyOptions> = {
  // Release gate: switch to true only after Windows/macOS native IME acceptance (P5).
  enabled: false,
  profile: 'zh-CN',
  hanging: true,
};

/** Measured units use UTF-16 source offsets. Zero-length glue is visual only. */
export interface LayoutItem {
  kind: 'box' | 'glue' | 'penalty';
  from: number;
  to: number;
  text: string;
  width: number;
  ascent: number;
  descent: number;
  fontSize: number;
  stretch?: number;
  shrink?: number;
  weight?: number;
  /** Comfortable adjustment, distinct from the hard stretch/shrink capacity. */
  comfortStretch?: number;
  comfortShrink?: number;
  /** Measured code boxes select ragged layout without changing their contents. */
  code?: boolean;
  hyphenatable?: boolean;
  discardable?: boolean;
  /** Literal text whitespace, unlike automatic glue or a projected soft break. */
  sourceWhitespace?: boolean;
  trimStart?: number;
  trimEnd?: number;
  hang?: number;
  leadingSpace?: number;
  trailingSpace?: number;
  /** Actual advance of the visible hyphen, used only when this unit ends a line. */
  breakWidth?: number;
}

export interface BreakPoint {
  /** Exclusive item index. */
  at: number;
  penalty: number;
  required?: boolean;
  flagged?: boolean;
  width?: number;
}

export interface ParagraphInput {
  items: LayoutItem[];
  breaks: BreakPoint[];
  width: number;
  firstLineIndent?: number;
  lineWidths?: number[];
  minLineHeight: number;
  hanging: boolean;
  /** Keep literal edge whitespace at its measured advance in an editable view. */
  editableWhitespace?: boolean;
  alignment?: 'auto' | 'justify' | 'ragged';
  /** Maximum line badness, including the largest actual gap adjustment. */
  tolerance?: number;
  lastLineMinWidth?: number;
  lastLinePenalty?: number;
  maxCandidates?: number;
  timeBudgetMs?: number;
}

export interface LineLayout {
  start: number;
  end: number;
  visibleStart: number;
  visibleEnd: number;
  width: number;
  naturalWidth: number;
  ratio: number;
  badness: number;
  fitness: number;
  adjustments: number[];
  trimStart: number;
  trimEnd: number;
  hang: number;
  hyphenWidth: number;
  ascent: number;
  descent: number;
  height: number;
  baseline: number;
  last: boolean;
}

export type LayoutFallbackReason =
  | 'invalid-metrics'
  | 'no-solution'
  | 'budget-exceeded'
  | 'unsupported-content'
  | 'measurement-failed'
  | 'worker-failed';

export type ParagraphLayout =
  | {
      status: 'ready';
      lines: LineLayout[];
      demerits: number;
      candidates: number;
      alignment?: 'justify' | 'ragged';
    }
  | { status: 'fallback'; reason: LayoutFallbackReason; candidates: number };

export interface LayoutRevision {
  content: number;
  geometry: number;
}

export interface TypographyController {
  flush(): Promise<void>;
  invalidate(): void;
  destroy(): void;
}

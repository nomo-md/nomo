import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { Plugin, PluginKey, type EditorState, type Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet, type EditorView } from 'prosemirror-view';
import { Step, StepResult, type Mappable } from 'prosemirror-transform';
import { analyzeInlineSource } from '../InlineSourceCodec';

export type InlineSourceFormat =
  | 'strong'
  | 'em'
  | 'code'
  | 'strikethrough'
  | 'underline'
  | 'highlight';

/** Commands describe ranges in the resulting document, never DOM/widget positions. */
export const INLINE_SOURCE_TEMPLATE_META = 'inlineSourceTemplate';

export interface EmptyInlineSourceTemplate {
  from: number;
  to: number;
  cursor: number;
  type: InlineSourceFormat;
}

export type InlineSourceTemplateMeta =
  | { action: 'add'; template: EmptyInlineSourceTemplate }
  | { action: 'remove'; from: number; to: number }
  | { action: 'clear' };

interface TrackedTemplate extends EmptyInlineSourceTemplate {
  openTo: number;
  closeFrom: number;
}

const TEMPLATE_STEP_TYPE = 'nomoInlineSourceTemplate';

/** History replays steps, not transaction metadata. This step carries command
 * provenance without putting a sentinel character/mark into the document. */
class TemplateOriginStep extends Step {
  readonly inlineTemplateOrigin = true;

  constructor(
    readonly template: TrackedTemplate,
    readonly action: 'add' | 'remove' = 'add',
  ) {
    super();
  }

  apply(doc: ProseMirrorNode): StepResult {
    return StepResult.ok(doc);
  }
  invert(): TemplateOriginStep {
    return new TemplateOriginStep(this.template, this.action === 'add' ? 'remove' : 'add');
  }
  map(mapping: Mappable): TemplateOriginStep | null {
    const template = mapTemplate(this.template, mapping);
    return template.from < template.to ? new TemplateOriginStep(template, this.action) : null;
  }
  toJSON() {
    return { stepType: TEMPLATE_STEP_TYPE, template: this.template, action: this.action };
  }
  static fromJSON(
    _schema: unknown,
    value: { template: TrackedTemplate; action: 'add' | 'remove' },
  ): TemplateOriginStep {
    return new TemplateOriginStep(value.template, value.action);
  }
}

// Vite may re-evaluate this module while the PM Step registry remains alive.
const stepRegistration = Symbol.for('nomo.inlineSourceTemplateStep.registered');
const registeredSteps = globalThis as typeof globalThis & { [stepRegistration]?: boolean };
if (!registeredSteps[stepRegistration]) {
  Step.jsonID(TEMPLATE_STEP_TYPE, TemplateOriginStep);
  registeredSteps[stepRegistration] = true;
}

function isTemplateOriginStep(step: Step): step is TemplateOriginStep {
  return 'inlineTemplateOrigin' in step && step.inlineTemplateOrigin === true;
}

function mapTemplate(template: TrackedTemplate, mapping: Mappable): TrackedTemplate {
  return {
    ...template,
    from: mapping.map(template.from, 1),
    to: mapping.map(template.to, -1),
    openTo: mapping.map(template.openTo, -1),
    closeFrom: mapping.map(template.closeFrom, 1),
    cursor: mapping.map(template.cursor, -1),
  };
}

interface BlockInfo {
  node: ProseMirrorNode;
  pos: number;
}

interface SourceRange {
  from: number;
  to: number;
}

interface EditingMeta {
  composing?: boolean;
  dragging?: boolean;
  flushTemplates?: boolean;
}

export interface InlineSourceEditingState {
  readonly decorations: DecorationSet;
  readonly restingDecorations: DecorationSet;
  readonly composing: boolean;
  readonly dragging: boolean;
  readonly dragSelectionStarted: boolean;
  readonly expandedBlocks: ReadonlySet<number>;
  readonly expandedMarkers: readonly SourceRange[];
  readonly blocks: readonly BlockInfo[];
  readonly templates: readonly TrackedTemplate[];
}

export interface InlineSourceEditingOptions {
  readonly?: boolean | (() => boolean);
}

export const inlineSourceEditingPluginKey = new PluginKey<InlineSourceEditingState>(
  'inline-source-editing',
);

const analyses = new WeakMap<ProseMirrorNode, ReturnType<typeof analyzeInlineSource>>();
const omittedSourceRanges = new WeakMap<ProseMirrorNode, readonly { from: number; to: number }[]>();
const unmarkedText = new WeakMap<ProseMirrorNode, readonly { from: number; to: number }[]>();

function isReadonly(options: InlineSourceEditingOptions): boolean {
  return typeof options.readonly === 'function' ? options.readonly() : Boolean(options.readonly);
}

function analyze(block: ProseMirrorNode) {
  let analysis = analyses.get(block);
  if (!analysis) {
    analysis = analyzeInlineSource(block);
    analyses.set(block, analysis);
  }
  return analysis;
}

function collectBlocks(doc: ProseMirrorNode): BlockInfo[] {
  const blocks: BlockInfo[] = [];
  doc.descendants((node, pos) => {
    if (node.type.spec.code || node.type.name === 'html_block') return false;
    if (!node.isTextblock) return true;
    blocks.push({ node, pos });
    return false;
  });
  return blocks;
}

function markerDecoration(from: number, to: number, active: boolean): Decoration | null {
  if (from >= to) return null;
  return Decoration.inline(
    from,
    to,
    {
      class: `pm-inline-source-marker${active ? '' : ' is-hidden'}`,
      ...(active ? {} : { 'aria-hidden': 'true' }),
    },
    { inlineSource: true, marker: true },
  );
}

function omittedRanges(block: ProseMirrorNode) {
  let ranges = omittedSourceRanges.get(block);
  if (ranges) return ranges;
  const analysis = analyze(block);
  const retained = new Uint8Array(analysis.source.length);
  for (let i = 0; i < analysis.visiblePositions.length; i++) {
    const position = analysis.visiblePositions[i];
    retained[position] = 1;
    if (analysis.visibleText[i] === analysis.source[position]) continue;
    // Some entities decode to a glyph absent from their original source (e.g.
    // &lt;). Hiding their suffix would display the wrong '&' glyph. Keep those
    // spellings readable instead of inventing editable replacement characters.
    const entity = /^&(?:#[xX][\da-fA-F]+|#\d+|[A-Za-z][\dA-Za-z]*);/.exec(
      analysis.source.slice(position),
    );
    if (entity) retained.fill(1, position, position + entity[0].length);
  }
  for (const span of analysis.spans) {
    retained.fill(1, span.openFrom, span.openTo);
    retained.fill(1, span.closeFrom, span.closeTo);
  }
  const missing: { from: number; to: number }[] = [];
  for (let from = 0; from < retained.length; ) {
    if (retained[from]) {
      from++;
      continue;
    }
    let to = from + 1;
    while (to < retained.length && !retained[to]) to++;
    missing.push({ from, to });
    from = to;
  }
  omittedSourceRanges.set(block, missing);
  return missing;
}

function blockDecorations(
  block: BlockInfo,
  expanded: readonly SourceRange[],
  templates: readonly TrackedTemplate[],
) {
  const result: Decoration[] = [];
  const offset = block.pos + 1;
  const markerRanges: { from: number; to: number }[] = [];
  const addMarker = (from: number, to: number) => {
    if (from < to) markerRanges.push({ from, to });
  };
  const analysis = analyze(block.node);
  for (const span of analysis.spans) {
    addMarker(offset + span.openFrom, offset + span.openTo);
    addMarker(offset + span.closeFrom, offset + span.closeTo);
  }
  for (const range of omittedRanges(block.node)) addMarker(offset + range.from, offset + range.to);
  // An empty pair is deliberately not parsed as valid Markdown. Its delimiters
  // still have real positions while the command's temporary editing state lives.
  for (const template of templates) {
    if (template.from < offset || template.to > offset + block.node.content.size) continue;
    addMarker(template.from, template.openTo);
    addMarker(template.closeFrom, template.to);
  }
  const mergedMarkers: { from: number; to: number }[] = [];
  for (const range of markerRanges.sort((a, b) => a.from - b.from || a.to - b.to)) {
    const previous = mergedMarkers[mergedMarkers.length - 1];
    if (previous && range.from <= previous.to) previous.to = Math.max(previous.to, range.to);
    else mergedMarkers.push({ ...range });
  }
  for (const range of mergedMarkers) {
    let from = range.from;
    for (const visible of expanded) {
      if (visible.to <= from) continue;
      if (visible.from >= range.to) break;
      const hidden = markerDecoration(from, Math.min(range.to, visible.from), false);
      if (hidden) result.push(hidden);
      const decoration = markerDecoration(
        Math.max(from, visible.from),
        Math.min(range.to, visible.to),
        true,
      );
      if (decoration) result.push(decoration);
      from = Math.min(range.to, visible.to);
    }
    const hidden = markerDecoration(from, range.to, false);
    if (hidden) result.push(hidden);
  }
  for (const span of analysis.spans) {
    let from = offset + span.from;
    const to = offset + span.to;
    const addFormat = (start: number, end: number) => {
      if (start < end)
        result.push(
          Decoration.inline(
            start,
            end,
            {
              class: `pm-inline-source-format pm-inline-source-${span.type}`,
            },
            { inlineSource: true, format: span.type },
          ),
        );
    };
    // A nested marker must never inherit a format wrapper: text-decoration from
    // an ancestor cannot be cancelled by the marker's own text-decoration:none.
    for (const marker of mergedMarkers) {
      if (marker.to <= from) continue;
      if (marker.from >= to) break;
      addFormat(from, marker.from);
      from = Math.max(from, marker.to);
    }
    addFormat(from, to);
  }
  return result;
}

function selectionTouches(state: EditorState, from: number, to: number): boolean {
  return state.selection.ranges.some((range) =>
    range.$from.pos === range.$to.pos
      ? range.$from.pos >= from && range.$from.pos <= to
      : range.$from.pos < to && range.$to.pos > from,
  );
}

function mergeSourceRanges(ranges: readonly SourceRange[]): SourceRange[] {
  const result: SourceRange[] = [];
  for (const range of [...ranges].sort((a, b) => a.from - b.from || a.to - b.to)) {
    if (range.from >= range.to) continue;
    const previous = result[result.length - 1];
    if (previous && range.from <= previous.to) previous.to = Math.max(previous.to, range.to);
    else result.push({ ...range });
  }
  return result;
}

function selectedMarkerRanges(
  state: EditorState,
  blocks: readonly BlockInfo[],
  templates: readonly TrackedTemplate[],
): SourceRange[] {
  const selected: SourceRange[] = [];
  for (const block of blocks) {
    const offset = block.pos + 1;
    if (!selectionTouches(state, offset, offset + block.node.content.size)) continue;
    const analysis = analyze(block.node);
    const activeSpans = analysis.spans.filter((span) =>
      selectionTouches(state, offset + span.openFrom, offset + span.closeTo),
    );
    for (const span of activeSpans) {
      selected.push(
        { from: offset + span.openFrom, to: offset + span.openTo },
        { from: offset + span.closeFrom, to: offset + span.closeTo },
      );
    }
    for (const range of omittedRanges(block.node)) {
      const owners = analysis.spans.filter(
        (span) => range.from >= span.openFrom && range.to <= span.closeTo,
      );
      const ownerSize = Math.min(...owners.map((span) => span.closeTo - span.openFrom));
      const belongsToActiveSpan = owners.some(
        (span) => span.closeTo - span.openFrom === ownerSize && activeSpans.includes(span),
      );
      // Standalone escapes are local to their escaped character. Entity suffixes
      // whose glyph is retained in the source are local to the preceding '&'.
      const entitySuffix = analysis.source[range.from - 1] === '&';
      const adjacentFrom = offset + range.from - (entitySuffix ? 1 : 0);
      const adjacentTo =
        offset + Math.min(analysis.source.length, range.to + (entitySuffix ? 0 : 1));
      if (
        belongsToActiveSpan ||
        (!owners.length && selectionTouches(state, adjacentFrom, adjacentTo))
      ) {
        selected.push({ from: offset + range.from, to: offset + range.to });
      }
    }
  }
  for (const template of templates) {
    if (selectionTouches(state, template.from, template.to)) {
      selected.push(
        { from: template.from, to: template.openTo },
        { from: template.closeFrom, to: template.to },
      );
    }
  }
  return mergeSourceRanges(selected);
}

function expandDecorations(
  doc: ProseMirrorNode,
  resting: DecorationSet,
  blocks: readonly BlockInfo[],
  expanded: ReadonlySet<number>,
  expandedMarkers: readonly SourceRange[],
  templates: readonly TrackedTemplate[],
): DecorationSet {
  let decorations = resting;
  for (const block of blocks) {
    if (!expanded.has(block.pos)) continue;
    const from = block.pos + 1;
    const to = from + block.node.content.size;
    decorations = decorations.remove(decorations.find(from, to, (spec) => spec.inlineSource));
    decorations = decorations.add(doc, blockDecorations(block, expandedMarkers, templates));
  }
  return decorations;
}

function buildState(
  state: EditorState,
  previous?: InlineSourceEditingState,
  tr?: Transaction,
): InlineSourceEditingState {
  const meta = tr?.getMeta(inlineSourceEditingPluginKey) as EditingMeta | undefined;
  const composing = meta?.composing ?? previous?.composing ?? false;
  const dragging = meta?.dragging ?? previous?.dragging ?? false;
  // A held click is not a drag. Only retain visited syntax after the pointer
  // has produced a nonempty selection, including if it later collapses again.
  const dragSelectionStarted =
    dragging &&
    meta?.dragging !== true &&
    Boolean(previous?.dragSelectionStarted || (tr?.selectionSet && !state.selection.empty));
  const templates = previous && tr ? mapTemplates(previous.templates, tr) : [];
  if (composing && previous && tr) {
    // IME owns its DOM until compositionend. Mapping existing decorations keeps
    // the composing text stable instead of reparsing/replacing its ancestors.
    return {
      ...previous,
      composing,
      dragging,
      dragSelectionStarted,
      templates,
      decorations: previous.decorations.map(tr.mapping, state.doc),
      restingDecorations: previous.restingDecorations.map(tr.mapping, state.doc),
      expandedBlocks: new Set([...previous.expandedBlocks].map((pos) => tr.mapping.map(pos, 1))),
      expandedMarkers: mergeSourceRanges(
        previous.expandedMarkers.map((range) => ({
          from: tr.mapping.map(range.from, 1),
          to: tr.mapping.map(range.to, -1),
        })),
      ),
    };
  }
  const rebuild =
    !previous ||
    tr?.docChanged ||
    previous.composing ||
    Boolean(tr?.getMeta(INLINE_SOURCE_TEMPLATE_META));
  const blocks = rebuild ? collectBlocks(state.doc) : previous!.blocks;
  const restingDecorations = rebuild
    ? DecorationSet.create(
        state.doc,
        blocks.flatMap((block) => blockDecorations(block, [], templates)),
      )
    : previous!.restingDecorations;
  let expandedMarkers = selectedMarkerRanges(state, blocks, templates);
  if (dragSelectionStarted && previous) {
    expandedMarkers = mergeSourceRanges([
      ...expandedMarkers,
      ...previous.expandedMarkers.map((range) => ({
        from: tr ? tr.mapping.map(range.from, 1) : range.from,
        to: tr ? tr.mapping.map(range.to, -1) : range.to,
      })),
    ]);
  }
  const expandedBlocks = new Set(
    blocks
      .filter((block) =>
        expandedMarkers.some(
          (range) =>
            range.from < block.pos + 1 + block.node.content.size && range.to > block.pos + 1,
        ),
      )
      .map((block) => block.pos),
  );
  return {
    blocks,
    restingDecorations,
    decorations: expandDecorations(
      state.doc,
      restingDecorations,
      blocks,
      expandedBlocks,
      expandedMarkers,
      templates,
    ),
    composing,
    dragging,
    dragSelectionStarted,
    expandedBlocks,
    expandedMarkers,
    templates,
  };
}

function mapTemplates(templates: readonly TrackedTemplate[], tr: Transaction): TrackedTemplate[] {
  const meta = tr.getMeta(INLINE_SOURCE_TEMPLATE_META) as InlineSourceTemplateMeta | undefined;
  if (meta?.action === 'clear') return [];
  let mapped = templates.filter(
    (template) =>
      !(meta?.action === 'remove' && meta.from === template.from && meta.to === template.to),
  );
  for (let index = 0; index < tr.steps.length; index++) {
    const step = tr.steps[index];
    const map = tr.mapping.maps[index];
    if (isTemplateOriginStep(step)) {
      mapped = mapped.filter(
        (template) =>
          template.from !== step.template.from ||
          template.to !== step.template.to ||
          template.type !== step.template.type,
      );
      if (step.action === 'add') mapped.push({ ...step.template });
    }
    mapped = mapped.flatMap((template) => {
      let touched = false;
      if (!meta) {
        map.forEach((from, to, nextFrom, nextTo) => {
          if (
            (from === to
              ? from > template.from && from < template.to
              : to > template.from && from < template.to) &&
            !isTemplateHistoryEdit(tr, index, from, to, nextFrom, nextTo)
          )
            touched = true;
        });
      }
      const current = mapTemplate(template, map);
      return !touched && current.from < current.to ? [current] : [];
    });
  }
  if (meta?.action === 'add') {
    const template = meta.template;
    if (
      template.from >= 0 &&
      template.to <= tr.doc.content.size &&
      template.from < template.cursor &&
      template.cursor < template.to
    ) {
      mapped.push({ ...template, openTo: template.cursor, closeFrom: template.cursor });
    }
  }
  return mapped;
}

function isTemplateHistoryEdit(
  tr: Transaction,
  index: number,
  from: number,
  to: number,
  nextFrom: number,
  nextTo: number,
): boolean {
  return tr.steps.some((step, originIndex) => {
    if (!isTemplateOriginStep(step)) return false;
    if (step.action === 'add' && originIndex > index && from === to) {
      const mapping = tr.mapping.slice(index + 1, originIndex);
      return (
        mapping.map(nextFrom, 1) === step.template.from &&
        mapping.map(nextTo, -1) === step.template.to
      );
    }
    if (step.action === 'remove' && originIndex < index && nextFrom === nextTo) {
      const mapping = tr.mapping.slice(originIndex + 1, index);
      return (
        mapping.map(step.template.from, 1) === from && mapping.map(step.template.to, -1) === to
      );
    }
    return false;
  });
}

function hasTemplateSelection(state: EditorState, template: TrackedTemplate): boolean {
  return state.selection.ranges.some(
    (range) => range.$from.pos < template.to && range.$to.pos > template.from,
  );
}

function disposableTemplates(state: EditorState, value: InlineSourceEditingState, flush: boolean) {
  return value.templates.filter((template) => flush || !hasTemplateSelection(state, template));
}

function deleteTemplates(tr: Transaction, templates: readonly TrackedTemplate[]): void {
  const ranges: { from: number; to: number }[] = [];
  for (const template of [...templates].sort((a, b) => a.from - b.from || b.to - a.to)) {
    const previous = ranges[ranges.length - 1];
    if (previous && template.from <= previous.to) previous.to = Math.max(previous.to, template.to);
    else ranges.push({ from: template.from, to: template.to });
  }
  for (const range of ranges.reverse()) tr.delete(range.from, range.to);
}

function markMissingSource(tr: Transaction, state: EditorState): boolean {
  const mark = state.schema.marks.inline_source;
  if (!mark) return false;
  let changed = false;
  for (const block of collectBlocks(tr.doc)) {
    let ranges = unmarkedText.get(block.node);
    if (!ranges) {
      const missing: { from: number; to: number }[] = [];
      block.node.forEach((node, pos) => {
        if (node.isText && !mark.isInSet(node.marks))
          missing.push({ from: pos, to: pos + node.nodeSize });
      });
      ranges = missing;
      unmarkedText.set(block.node, ranges);
    }
    for (const range of ranges) {
      tr.addMark(block.pos + 1 + range.from, block.pos + 1 + range.to, mark.create());
      changed = true;
    }
  }
  return changed;
}

/** Real source characters stay in the document; only their appearance changes. */
export function inlineSourceEditingPlugin(
  options: InlineSourceEditingOptions = {},
): Plugin<InlineSourceEditingState> {
  const lifecycles = new WeakMap<
    EditorView,
    { destroyed: boolean; compositionTimer?: ReturnType<typeof setTimeout> }
  >();
  return new Plugin({
    key: inlineSourceEditingPluginKey,
    state: {
      init: (_, state) => buildState(state),
      apply: (tr, value, _oldState, state) =>
        !tr.docChanged &&
        !tr.selectionSet &&
        !tr.getMeta(inlineSourceEditingPluginKey) &&
        !tr.getMeta(INLINE_SOURCE_TEMPLATE_META)
          ? value
          : buildState(state, value, tr),
    },
    props: {
      decorations(state) {
        const value = inlineSourceEditingPluginKey.getState(state);
        return isReadonly(options) ? value?.restingDecorations : value?.decorations;
      },
      handleDOMEvents: {
        mousedown(view, event) {
          if (event.button === 0 && !isReadonly(options)) {
            view.dispatch(
              view.state.tr.setMeta(inlineSourceEditingPluginKey, {
                dragging: true,
              } satisfies EditingMeta),
            );
          }
          return false;
        },
        compositionstart(view) {
          const lifecycle = lifecycles.get(view);
          if (lifecycle?.compositionTimer !== undefined) clearTimeout(lifecycle.compositionTimer);
          view.dispatch(
            view.state.tr.setMeta(inlineSourceEditingPluginKey, {
              composing: true,
            } satisfies EditingMeta),
          );
          return false;
        },
        compositionend(view) {
          const lifecycle = lifecycles.get(view);
          if (!lifecycle) return false;
          if (lifecycle.compositionTimer !== undefined) clearTimeout(lifecycle.compositionTimer);
          // Native PM first flushes composition DOM in a microtask. A macrotask
          // lets that final text transaction finish before decorations rebuild.
          lifecycle.compositionTimer = setTimeout(() => {
            lifecycle.compositionTimer = undefined;
            if (!lifecycle.destroyed && !view.composing) {
              view.dispatch(
                view.state.tr.setMeta(inlineSourceEditingPluginKey, {
                  composing: false,
                } satisfies EditingMeta),
              );
            }
          }, 0);
          return false;
        },
      },
    },
    appendTransaction(transactions, _oldState, state) {
      const value = inlineSourceEditingPluginKey.getState(state);
      if (!value || value.composing || isReadonly(options)) return null;
      const flush = transactions.some(
        (tr) =>
          (tr.getMeta(inlineSourceEditingPluginKey) as EditingMeta | undefined)?.flushTemplates,
      );
      const disposable = value.dragging && !flush ? [] : disposableTemplates(state, value, flush);
      const tr = state.tr;
      if (disposable.length) deleteTemplates(tr, disposable);
      if (
        transactions.some(
          (transaction) =>
            transaction.docChanged ||
            (transaction.getMeta(inlineSourceEditingPluginKey) as EditingMeta | undefined)
              ?.composing === false,
        )
      ) {
        markMissingSource(tr, state);
      }
      for (let index = 0; index < transactions.length; index++) {
        const templateMeta = transactions[index].getMeta(INLINE_SOURCE_TEMPLATE_META) as
          | InlineSourceTemplateMeta
          | undefined;
        if (templateMeta?.action !== 'add') continue;
        let template: TrackedTemplate = {
          ...templateMeta.template,
          openTo: templateMeta.template.cursor,
          closeFrom: templateMeta.template.cursor,
        };
        for (const subsequent of transactions.slice(index + 1))
          template = mapTemplate(template, subsequent.mapping);
        if (
          !value.templates.some(
            (current) =>
              current.from === template.from &&
              current.to === template.to &&
              current.type === template.type,
          )
        )
          continue;
        template = mapTemplate(template, tr.mapping);
        if (template.from < template.to) tr.step(new TemplateOriginStep(template));
      }
      if (!tr.docChanged) return null;
      // Removing a never-used command placeholder is view housekeeping. It must
      // not create a separate undo stop or revive a discarded empty pair.
      if (disposable.length) tr.setMeta('addToHistory', false);
      return tr;
    },
    view(view) {
      const lifecycle: { destroyed: boolean; compositionTimer?: ReturnType<typeof setTimeout> } = {
        destroyed: false,
      };
      lifecycles.set(view, lifecycle);
      const stopDrag = () => {
        if (lifecycle.destroyed || !inlineSourceEditingPluginKey.getState(view.state)?.dragging)
          return;
        view.dispatch(
          view.state.tr.setMeta(inlineSourceEditingPluginKey, {
            dragging: false,
          } satisfies EditingMeta),
        );
      };
      view.dom.ownerDocument.addEventListener('mouseup', stopDrag);
      view.dom.ownerDocument.defaultView?.addEventListener('blur', stopDrag);
      return {
        destroy() {
          lifecycle.destroyed = true;
          if (lifecycle.compositionTimer !== undefined) clearTimeout(lifecycle.compositionTimer);
          view.dom.ownerDocument.removeEventListener('mouseup', stopDrag);
          view.dom.ownerDocument.defaultView?.removeEventListener('blur', stopDrag);
          lifecycles.delete(view);
        },
      };
    },
  });
}

export function isInlineSourceComposing(target: EditorView | EditorState): boolean {
  const state = 'state' in target ? target.state : target;
  return Boolean(
    ('composing' in target && target.composing) ||
    inlineSourceEditingPluginKey.getState(state)?.composing,
  );
}

export function getEmptyInlineSourceTemplates(
  state: EditorState,
): readonly EmptyInlineSourceTemplate[] {
  return inlineSourceEditingPluginKey.getState(state)?.templates ?? [];
}

export function findEmptyInlineSourceTemplate(
  state: EditorState,
  type?: InlineSourceFormat,
): EmptyInlineSourceTemplate | undefined {
  return inlineSourceEditingPluginKey
    .getState(state)
    ?.templates.find(
      (template) => (!type || template.type === type) && hasTemplateSelection(state, template),
    );
}

export function getEmptyInlineTemplateFormats(state: EditorState): Set<InlineSourceFormat> {
  return new Set(
    getEmptyInlineSourceTemplates(state)
      .filter(
        (template) => state.selection.from > template.from && state.selection.to < template.to,
      )
      .map((template) => template.type),
  );
}

/** Explicit save/flush only. Ordinary Markdown reads must not delete a live pair. */
export function clearEmptyInlineTemplates(view: EditorView): boolean {
  if (isInlineSourceComposing(view) || !getEmptyInlineSourceTemplates(view.state).length)
    return false;
  const before = view.state.doc;
  view.dispatch(
    view.state.tr.setMeta(inlineSourceEditingPluginKey, {
      flushTemplates: true,
    } satisfies EditingMeta),
  );
  return view.state.doc !== before;
}

<script lang="ts">
  import type { ComparisonWorkspaceItem, MarkdownTabState } from '../types';
  import type { MarkdownDocumentRuntime } from '../services/markdownDocumentRuntime';
  import ComparisonPane from './ComparisonPane.svelte';
  import type { ComponentProps } from 'svelte';
  import { t } from '../i18n';
  export let item: ComparisonWorkspaceItem;
  export let leftTab: MarkdownTabState;
  export let rightTab: MarkdownTabState;
  export let leftRuntime: MarkdownDocumentRuntime;
  export let rightRuntime: MarkdownDocumentRuntime;
  export let paneProps: Omit<ComponentProps<ComparisonPane>, 'tab' | 'runtime' | 'mode' | 'focused'>;
  export let onResize: (itemId: string, percent: number, persist: boolean) => void;
  let container: HTMLDivElement;
  let pointerId: number | null = null;
  $: effectiveMode = leftTab.largeDocumentMode || rightTab.largeDocumentMode ? 'source' : item.mode;
  $: interactionDisabled = paneProps.disabled ?? false;
  function clampPercent(value: number) { return Math.min(80, Math.max(20, value)); }
  function resize(event: PointerEvent, persist = false) {
    const rect = container.getBoundingClientRect();
    if (interactionDisabled || rect.width <= 0) return;
    onResize(item.id, clampPercent((event.clientX - rect.left) * 100 / rect.width), persist);
  }
  function begin(event: PointerEvent) {
    if (interactionDisabled || event.button !== 0) return;
    event.preventDefault();
    pointerId = event.pointerId;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function finish(event: PointerEvent) {
    if (event.pointerId !== pointerId) return;
    resize(event, true);
    pointerId = null;
    const divider = event.currentTarget as HTMLElement;
    if (divider.hasPointerCapture(event.pointerId)) divider.releasePointerCapture(event.pointerId);
  }
  function cancel(event: PointerEvent) {
    if (event.pointerId !== pointerId) return;
    pointerId = null;
    onResize(item.id, item.leftPercent, true);
  }
  function resizeFromKeyboard(event: KeyboardEvent) {
    if (interactionDisabled) return;
    const step = event.shiftKey ? 10 : 2;
    let nextPercent = item.leftPercent;
    if (event.key === 'ArrowLeft') nextPercent -= step;
    else if (event.key === 'ArrowRight') nextPercent += step;
    else if (event.key === 'Home') nextPercent = 20;
    else if (event.key === 'End') nextPercent = 80;
    else return;
    event.preventDefault();
    onResize(item.id, clampPercent(nextPercent), true);
  }
</script>

<div class="comparison-workspace" bind:this={container} style={`grid-template-columns: minmax(0, ${item.leftPercent}fr) 8px minmax(0, ${100 - item.leftPercent}fr)`}>
  {#key leftTab.id}<ComparisonPane {...paneProps} tab={leftTab} runtime={leftRuntime} mode={effectiveMode} focused={item.focusedTabId === leftTab.id} />{/key}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
  <div class="comparison-divider" class:resizing={pointerId !== null} role="separator" aria-label={t.resizeComparisonPanes()} aria-orientation="vertical"
    aria-controls={`comparison-document-${leftTab.id} comparison-document-${rightTab.id}`} aria-disabled={interactionDisabled}
    aria-valuemin="20" aria-valuemax="80" aria-valuenow={Math.round(item.leftPercent)} tabindex={interactionDisabled ? -1 : 0}
    on:pointerdown={begin} on:pointermove={(event) => { if (event.pointerId === pointerId) resize(event); }}
    on:pointerup={finish} on:pointercancel={cancel} on:lostpointercapture={cancel} on:keydown={resizeFromKeyboard}></div>
  {#key rightTab.id}<ComparisonPane {...paneProps} tab={rightTab} runtime={rightRuntime} mode={effectiveMode} focused={item.focusedTabId === rightTab.id} />{/key}
</div>

<style>
  .comparison-workspace { display: grid; flex: 1; min-width: 0; min-height: 0; overflow: hidden; }
  .comparison-divider { cursor: col-resize; touch-action: none; position: relative; background: transparent; }
  .comparison-divider::after { content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 1px; background: var(--md-editor-border, #8883); }
  .comparison-divider:hover::after, .comparison-divider:focus-visible::after, .comparison-divider.resizing::after { background: var(--md-editor-accent); width: 2px; }
</style>

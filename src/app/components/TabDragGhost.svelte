<script lang="ts">
  import { FileText } from '@lucide/svelte';

  export let title: string;
  export let x: number;
  export let y: number;
  export let comparison = false;

  let width = 0;
  let height = 0;
  let viewportWidth = 0;
  let viewportHeight = 0;

  $: left = Math.max(8, Math.min(x + 14, viewportWidth - width - 8));
  $: top = Math.max(8, Math.min(y + 14, viewportHeight - height - 8));
  $: outside = x < 0 || y < 0 || x >= viewportWidth || y >= viewportHeight;
</script>

<svelte:window bind:innerWidth={viewportWidth} bind:innerHeight={viewportHeight} />

<div
  class="tab-drag-ghost"
  class:comparison
  class:outside
  bind:clientWidth={width}
  bind:clientHeight={height}
  style:transform={`translate3d(${left}px, ${top}px, 0)`}
  aria-hidden="true"
>
  <FileText size={14} />
  <span>{title}</span>
</div>

<style>
  .tab-drag-ghost {
    position: fixed;
    top: 0;
    left: 0;
    z-index: 10000;
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    min-width: min(120px, calc(100vw - 16px));
    max-width: min(240px, calc(100vw - 16px));
    height: var(--md-editor-tab-height);
    padding: 0 12px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: var(--md-editor-surface);
    color: var(--md-editor-heading-fg);
    box-shadow: var(--md-editor-shadow-raised);
    font-size: var(--md-editor-ui-font-size);
    font-weight: 500;
    opacity: 0.94;
    pointer-events: none;
    user-select: none;
  }

  .comparison {
    min-width: min(200px, calc(100vw - 16px));
    max-width: min(320px, calc(100vw - 16px));
  }

  .outside {
    visibility: hidden;
  }

  .tab-drag-ghost :global(svg) {
    flex-shrink: 0;
    color: var(--md-editor-accent);
  }

  span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

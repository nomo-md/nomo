<script lang="ts">
  import { tick, onDestroy } from 'svelte';
  import { History, FileText, FolderOpen, X } from '@lucide/svelte';
  import type { RecentEntry } from '../../lib/desktop/tauriStorage';
  import { t } from '../i18n';

  export let recentFiles: RecentEntry[];
  export let missingRecentPaths: Set<string>;
  export let openRecentEntry: (path: string, type: 'file' | 'folder') => void;
  export let removeRecentEntry: (path: string) => void;
  export let clearRecentEntriesList: () => void;
  export let refreshRecentFiles: () => Promise<void>;
  let open = false;
  let opening = false;
  let disposed = false;
  onDestroy(() => {
    disposed = true;
  });
  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  let panel: HTMLDivElement;
  let left = 0;
  let top = 0;

  function close() {
    open = false;
    trigger?.focus();
  }
  async function toggle() {
    if (opening) return;
    if (open) {
      close();
      return;
    }
    opening = true;
    try {
      await refreshRecentFiles();
    } finally {
      opening = false;
    }
    if (disposed) return;
    open = true;
    await tick();
    if (disposed || !open || !panel) return;
    panel.showPopover();
    const rect = trigger.getBoundingClientRect();
    left = Math.max(
      12,
      Math.min(rect.right - panel.offsetWidth, window.innerWidth - panel.offsetWidth - 12),
    );
    top =
      rect.bottom + panel.offsetHeight + 8 <= window.innerHeight - 12
        ? rect.bottom + 8
        : Math.max(12, rect.top - panel.offsetHeight - 8);
    panel?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }
  function outside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) close();
  }
  function keydown(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
    }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      const buttons = [...panel.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
      const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const index =
        event.key === 'Home'
          ? 0
          : event.key === 'End'
            ? buttons.length - 1
            : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
      buttons[index]?.focus();
    }
  }
  function focusout() {
    setTimeout(() => {
      if (open && !root.contains(document.activeElement)) open = false;
    }, 0);
  }
  async function remove(path: string, index: number) {
    await removeRecentEntry(path);
    await tick();
    const buttons = panel?.querySelectorAll<HTMLButtonElement>('.remove');
    buttons?.[Math.min(index, buttons.length - 1)]?.focus();
    if (!buttons?.length) panel?.focus();
  }
</script>

<svelte:window
  on:pointerdown={outside}
  on:keydown={keydown}
  on:resize={() => {
    if (open) close();
  }}
/>
<div class="recent-popover" bind:this={root} on:focusout={focusout}>
  <button
    type="button"
    bind:this={trigger}
    aria-expanded={open}
    aria-controls="recent-open-panel"
    on:click={toggle}
  >
    <History size={16} /><span>{t.recentOpenLabel()}</span>
  </button>
  {#if open}
    <div
      id="recent-open-panel"
      class="recent-panel"
      popover="manual"
      style:left={`${left}px`}
      style:top={`${top}px`}
      bind:this={panel}
      role="region"
      aria-label={t.recentOpenLabel()}
      tabindex="-1"
    >
      <div class="recent-heading">{t.recentOpenLabel()}</div>
      <div class="recent-list">
        {#each recentFiles.slice(0, 20) as entry, index (entry.path)}
          {@const missing = missingRecentPaths.has(entry.path)}
          <div class="recent-row">
            <button
              class="recent-target"
              type="button"
              disabled={missing}
              title={entry.path}
              on:click={() => {
                close();
                openRecentEntry(entry.path, entry.entryType);
              }}
            >
              {#if entry.entryType === 'folder'}<FolderOpen size={17} />{:else}<FileText
                  size={17}
                />{/if}
              <span class="recent-copy">
                <strong
                  >{entry.title ||
                    entry.path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ||
                    entry.path}</strong
                >
                <small>{entry.path}</small>
                {#if missing}<small>{t.unavailableSuffix()}</small>{/if}
              </span>
            </button>
            <button
              class="remove"
              type="button"
              aria-label={`${t.removeRecentLabel()} ${entry.path}`}
              title={t.removeRecentLabel()}
              on:click={() => remove(entry.path, index)}><X size={15} /></button
            >
          </div>
        {:else}
          <p class="recent-empty">{t.noRecentEntries()}</p>
        {/each}
      </div>
      {#if recentFiles.length}
        <button
          class="recent-clear"
          type="button"
          on:click={() => {
            close();
            clearRecentEntriesList();
          }}>{t.clearRecentFiles()}</button
        >
      {/if}
    </div>
  {/if}
</div>

<style>
  .recent-popover {
    position: relative;
  }
  .recent-panel {
    position: fixed;
    inset: auto;
    margin: 0;
    z-index: 300;
    width: min(440px, 80vw);
    text-align: left;
    color: var(--md-editor-fg);
    background: var(--md-editor-surface);
    border: 1px solid var(--md-editor-border);
    border-radius: 10px;
    box-shadow: 0 8px 28px #0002;
    overflow: hidden;
  }
  .recent-heading {
    padding: 12px 14px 8px;
    font-size: 13px;
    font-weight: 600;
  }
  .recent-list {
    max-height: min(330px, 35vh);
    overflow-y: auto;
  }
  .recent-row {
    display: flex;
    align-items: center;
    padding: 2px 6px;
    gap: 4px;
  }
  .recent-panel .recent-target {
    display: flex;
    justify-content: flex-start;
    flex: 1;
    min-width: 0;
    height: auto;
    padding: 8px;
    gap: 10px;
    border: 0;
    background: transparent;
    font-size: 13px;
    box-shadow: none;
  }
  .recent-target :global(svg) {
    flex-shrink: 0;
  }
  .recent-copy {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    text-align: left;
  }
  .recent-copy strong,
  .recent-copy small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-copy small {
    font-size: 11px;
    font-weight: 400;
    opacity: 0.65;
  }
  .recent-panel .remove {
    padding: 6px;
    min-width: 28px;
    height: 28px;
    border: 0;
    background: transparent;
    box-shadow: none;
  }
  .recent-panel button:hover {
    background: var(--md-editor-chrome);
  }
  .recent-panel button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .recent-panel .recent-clear {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--md-editor-border);
    border-radius: 0;
    font-size: 12px;
    background: transparent;
    box-shadow: none;
  }
  .recent-empty {
    padding: 10px 14px 20px;
    margin: 0;
    font-size: 13px;
    opacity: 0.65;
  }
</style>

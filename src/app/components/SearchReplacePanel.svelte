<script lang="ts">
  import { ChevronDown, Search, X } from '@lucide/svelte';
  import { t } from '../i18n';

  const POSITION_STORAGE_KEY = 'newmd.searchReplace.position.v1';
  const VIEWPORT_MARGIN = 8;
  const DEFAULT_TOP = 92;
  const DEFAULT_RIGHT = 14;
  const FALLBACK_TITLEBAR_HEIGHT = 40;

  type PanelPosition = { left: number; top: number };

  function loadSavedPosition(): PanelPosition | undefined {
    if (typeof window === 'undefined') return undefined;
    try {
      const saved = JSON.parse(window.localStorage.getItem(POSITION_STORAGE_KEY) ?? 'null') as {
        left?: unknown;
        top?: unknown;
      } | null;
      if (
        !saved ||
        typeof saved.left !== 'number' ||
        !Number.isFinite(saved.left) ||
        typeof saved.top !== 'number' ||
        !Number.isFinite(saved.top)
      ) {
        return undefined;
      }

      return {
        left: saved.left,
        top: saved.top,
      };
    } catch {
      return undefined;
    }
  }

  export let interfaceLocale: string;
  export let open: boolean;
  export let replaceVisible: boolean;
  export let query: string;
  export let replacement: string;
  export let caseSensitive: boolean;
  export let wholeWord: boolean;
  export let backwards: boolean;
  export let wrapAround: boolean;
  export let activeIndex: number;
  export let matchCount: number;
  export let showActivePosition = true;
  export let readonly: boolean;
  export let busy = false;
  export let progressText = '';
  export let updateQuery: (event: Event) => void;
  export let updateReplacement: (event: Event) => void;
  export let toggleCaseSensitive: () => void;
  export let toggleWholeWord: () => void;
  export let toggleBackwards: () => void;
  export let toggleWrapAround: () => void;
  export let toggleReplaceVisible: () => void;
  export let findPrevious: () => void;
  export let findNext: () => void;
  export let countMatches: () => void;
  export let replaceCurrent: () => void;
  export let replaceAll: () => void;
  export let close: () => void;

  let panel: HTMLDivElement | undefined;
  let searchInput: HTMLInputElement;
  let hasOpened = false;
  let positionReady = false;
  let positionConstraintFrame = 0;
  const initialPosition = loadSavedPosition();
  let panelLeft = initialPosition?.left;
  let panelTop = initialPosition?.top ?? DEFAULT_TOP;
  let dragStart:
    | {
        pointerX: number;
        pointerY: number;
        pointerId: number;
        left: number;
        top: number;
      }
    | undefined;

  $: if (open && !hasOpened && searchInput) {
    hasOpened = true;
    queueMicrotask(() => {
      searchInput?.focus();
      searchInput?.select();
    });
  }

  $: if (!open) {
    hasOpened = false;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      if (busy) return;
      if (event.shiftKey ? !backwards : backwards) {
        findPrevious();
      } else {
        findNext();
      }
    }
  }

  function handleTitlePointerDown(event: PointerEvent) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
    if (!panel) return;
    event.preventDefault();
    const rect = panel.getBoundingClientRect();
    panelLeft = rect.left;
    panelTop = rect.top;
    dragStart = {
      pointerX: event.clientX,
      pointerY: event.clientY,
      pointerId: event.pointerId,
      left: rect.left,
      top: rect.top,
    };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function handleTitlePointerMove(event: PointerEvent) {
    if (!dragStart || dragStart.pointerId !== event.pointerId || !panel) return;
    const rect = panel.getBoundingClientRect();
    const nextLeft = dragStart.left + event.clientX - dragStart.pointerX;
    const nextTop = dragStart.top + event.clientY - dragStart.pointerY;
    const constrained = constrainPanelPosition(nextLeft, nextTop, rect);
    panelLeft = constrained.left;
    panelTop = constrained.top;
  }

  function handleTitlePointerUp(event: PointerEvent) {
    if (!dragStart || dragStart.pointerId !== event.pointerId) return;
    finishTitleDrag(event.currentTarget as HTMLElement, event.pointerId, true);
  }

  function handleTitleLostPointerCapture(event: PointerEvent) {
    if (!dragStart || dragStart.pointerId !== event.pointerId) return;
    finishTitleDrag(event.currentTarget as HTMLElement, event.pointerId, false);
  }

  function finishTitleDrag(target: HTMLElement, pointerId: number, releaseCapture: boolean) {
    const moved = Boolean(dragStart);
    dragStart = undefined;
    if (releaseCapture && target.hasPointerCapture(pointerId)) {
      target.releasePointerCapture(pointerId);
    }
    if (moved) {
      constrainCurrentPanelPosition();
      queueMicrotask(saveCurrentPosition);
    }
  }

  function constrainPanelPosition(
    left: number,
    top: number,
    rect: Pick<DOMRect, 'width' | 'height'>,
  ): PanelPosition {
    const availableWidth = Math.max(0, window.innerWidth - VIEWPORT_MARGIN * 2);
    const visibleWidth = Math.min(rect.width, availableWidth);
    const maxLeft = Math.max(
      VIEWPORT_MARGIN,
      window.innerWidth - visibleWidth - VIEWPORT_MARGIN,
    );
    const titlebarHeight =
      panel?.querySelector<HTMLElement>('.search-dialog-titlebar')?.getBoundingClientRect().height ??
      FALLBACK_TITLEBAR_HEIGHT;
    const availableHeight = Math.max(0, window.innerHeight - VIEWPORT_MARGIN * 2);
    const visibleHeight =
      rect.height <= availableHeight ? rect.height : Math.min(titlebarHeight, availableHeight);
    const maxTop = Math.max(
      VIEWPORT_MARGIN,
      window.innerHeight - visibleHeight - VIEWPORT_MARGIN,
    );

    return {
      left: Math.min(Math.max(left, VIEWPORT_MARGIN), maxLeft),
      top: Math.min(Math.max(top, VIEWPORT_MARGIN), maxTop),
    };
  }

  function constrainCurrentPanelPosition() {
    if (!panel || typeof window === 'undefined') return;
    const rect = panel.getBoundingClientRect();
    const constrained = constrainPanelPosition(rect.left, rect.top, rect);
    panelLeft = constrained.left;
    panelTop = constrained.top;
    positionReady = true;
  }

  function schedulePanelPositionConstraint() {
    if (positionConstraintFrame !== 0) {
      window.cancelAnimationFrame(positionConstraintFrame);
    }
    positionConstraintFrame = window.requestAnimationFrame(() => {
      positionConstraintFrame = 0;
      constrainCurrentPanelPosition();
    });
  }

  function floatingPanelPortal(node: HTMLDivElement) {
    panel = node;
    document.body.appendChild(node);
    const handleViewportResize = () => schedulePanelPositionConstraint();
    const resizeObserver =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver(() => schedulePanelPositionConstraint());

    window.addEventListener('resize', handleViewportResize);
    resizeObserver?.observe(node);
    schedulePanelPositionConstraint();

    return {
      destroy() {
        window.removeEventListener('resize', handleViewportResize);
        resizeObserver?.disconnect();
        if (positionConstraintFrame !== 0) {
          window.cancelAnimationFrame(positionConstraintFrame);
          positionConstraintFrame = 0;
        }
        if (panel === node) {
          panel = undefined;
        }
        dragStart = undefined;
        positionReady = false;
      },
    };
  }

  function saveCurrentPosition() {
    if (!panel || typeof window === 'undefined') return;
    try {
      const rect = panel.getBoundingClientRect();
      window.localStorage.setItem(
        POSITION_STORAGE_KEY,
        JSON.stringify({ left: Math.round(rect.left), top: Math.round(rect.top) }),
      );
    } catch {
      // Position persistence is optional and must not block search.
    }
  }
</script>

{#key interfaceLocale}
  {#if open}
    <div
      class="search-replace-panel"
      class:dragging={dragStart}
      class:position-ready={positionReady}
      role="search"
      aria-label={t.searchReplace()}
      use:floatingPanelPortal
      style={panelLeft === undefined
        ? `top:${panelTop}px;right:${DEFAULT_RIGHT}px`
        : `top:${panelTop}px;left:${panelLeft}px`}
    >
      <header
        class="search-dialog-titlebar"
        role="group"
        aria-label={t.searchReplace()}
        on:pointerdown={handleTitlePointerDown}
        on:pointermove={handleTitlePointerMove}
        on:pointerup={handleTitlePointerUp}
        on:pointercancel={handleTitlePointerUp}
        on:lostpointercapture={handleTitleLostPointerCapture}
      >
        <div class="search-dialog-title">
          <Search size={15} aria-hidden="true" />
          <strong>{t.searchReplace()}</strong>
          <span>{t.currentDocument()}</span>
        </div>
        <button
          type="button"
          class="close-button"
          title={t.closeSearch()}
          aria-label={t.closeSearch()}
          on:click={close}
        >
          <X size={14} />
        </button>
      </header>

      <div class="search-dialog-body">
        <div class="search-form-row">
          <label for="document-search-query">{t.search()}</label>
          <div class="search-input-shell">
            <input
              id="document-search-query"
              bind:this={searchInput}
              value={query}
              placeholder={t.searchInDocument()}
              autocomplete="off"
              on:input={updateQuery}
              on:keydown={handleKeydown}
            />
            <span class="result-count" aria-live="polite">
              {query ? t.searchResultCount({ count: matchCount }) : t.searchReady()}
            </span>
          </div>
          <div class="search-action-group find-actions">
            <button
              type="button"
              class="primary-action"
              disabled={!query || busy}
              on:click={() => (backwards ? findPrevious() : findNext())}
            >
              {t.nextMatch()}
            </button>
            <button
              type="button"
              class="replace-toggle"
              title={replaceVisible ? t.hideReplace() : t.showReplace()}
              aria-label={replaceVisible ? t.hideReplace() : t.showReplace()}
              aria-expanded={replaceVisible}
              on:click={toggleReplaceVisible}
            >
              <ChevronDown size={15} class={replaceVisible ? '' : 'collapsed'} />
            </button>
          </div>
        </div>

        {#if replaceVisible}
          <div class="search-form-row">
            <label for="document-search-replacement">{t.replaceWith()}</label>
            <input
              id="document-search-replacement"
              class="replacement-input"
              value={replacement}
              placeholder={t.replaceWith()}
              autocomplete="off"
              disabled={readonly || busy}
              on:input={updateReplacement}
              on:keydown={handleKeydown}
            />
            <div class="search-action-group">
              <button
                type="button"
                disabled={readonly || busy || matchCount === 0}
                on:click={replaceCurrent}
              >
                {t.replace()}
              </button>
              <button
                type="button"
                disabled={readonly || busy || matchCount === 0}
                on:click={replaceAll}
              >
                {t.replaceAll()}
              </button>
            </div>
          </div>
        {/if}

        <div class="search-option-strip" aria-label={t.searchOptions()}>
          <label class="search-option">
            <input type="checkbox" checked={backwards} on:change={toggleBackwards} />
            <span>← {t.reverseSearch()}</span>
          </label>
          <label class="search-option">
            <input type="checkbox" checked={wholeWord} on:change={toggleWholeWord} />
            <span>{t.wholeWord()}</span>
          </label>
          <label class="search-option">
            <input type="checkbox" checked={caseSensitive} on:change={toggleCaseSensitive} />
            <span>Aa {t.matchCase()}</span>
          </label>
          <label class="search-option">
            <input type="checkbox" checked={wrapAround} on:change={toggleWrapAround} />
            <span>↻ {t.wrapSearch()}</span>
          </label>
          <span class="option-spacer"></span>
          <button
            type="button"
            class="count-action"
            disabled={!query || busy}
            on:click={countMatches}
          >
            {t.countMatches()}
          </button>
        </div>

        <div class="search-feedback" aria-live="polite">
          {#if busy && progressText}
            {progressText}
          {:else if query && matchCount > 0}
            {showActivePosition
              ? t.searchMatchCount({
                  current: Math.min(Math.max(activeIndex + 1, 1), matchCount),
                  total: matchCount,
                })
              : t.searchResultCount({ count: matchCount })}
          {:else if query}
            {t.noSearchResults()}
          {:else}
            {t.searchReady()}
          {/if}
        </div>
      </div>
    </div>
  {/if}
{/key}

<style>
  .search-replace-panel {
    position: fixed;
    z-index: 70;
    width: min(520px, calc(100vw - 16px));
    max-width: calc(100vw - 16px);
    box-sizing: border-box;
    overflow: hidden;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-lg);
    background: var(--md-editor-surface);
    box-shadow: var(--md-editor-shadow-dialog);
    color: var(--md-editor-fg);
    font-size: var(--md-editor-ui-font-size);
    opacity: 0;
    pointer-events: none;
  }

  .search-replace-panel.position-ready {
    opacity: 1;
    pointer-events: auto;
  }

  .search-replace-panel.dragging {
    user-select: none;
  }

  .search-dialog-titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--md-editor-control-height-lg);
    padding: 0 9px 0 17px;
    border-bottom: 1px solid var(--md-editor-border);
    background: var(--md-editor-chrome);
    cursor: move;
    touch-action: none;
    -webkit-user-select: none;
    user-select: none;
  }

  .search-dialog-title {
    display: flex;
    align-items: center;
    gap: var(--md-editor-space-sm);
    min-width: 0;
  }

  .search-dialog-title :global(svg) {
    color: var(--md-editor-accent);
  }

  .search-dialog-title strong {
    font-size: var(--md-editor-ui-font-size);
    font-weight: 600;
  }

  .search-dialog-title span {
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
  }

  button,
  input {
    font: inherit;
  }

  button {
    color: inherit;
    cursor: pointer;
  }

  button:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .close-button {
    display: grid;
    place-items: center;
    width: var(--md-editor-control-height-sm);
    height: var(--md-editor-control-height-sm);
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--md-editor-radius-sm);
    background: transparent;
    color: var(--md-editor-muted-fg);
  }

  .close-button:hover {
    border-color: var(--md-editor-border);
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
  }

  .search-dialog-body {
    padding: var(--md-editor-space-lg) var(--md-editor-space-lg) var(--md-editor-space-md);
  }

  .search-form-row {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr) 156px;
    align-items: center;
    gap: var(--md-editor-space-sm);
  }

  .search-form-row + .search-form-row {
    margin-top: 8px;
  }

  .search-form-row > label {
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
    text-align: right;
    white-space: nowrap;
  }

  .search-input-shell {
    position: relative;
    display: flex;
    align-items: center;
    min-width: 0;
    height: var(--md-editor-control-height-lg);
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: var(--md-editor-surface);
  }

  .search-input-shell:focus-within,
  .replacement-input:focus {
    border-color: var(--md-editor-accent);
    box-shadow: none;
  }

  .search-input-shell input,
  .replacement-input {
    min-width: 0;
    height: var(--md-editor-control-height-lg);
    padding: 0 12px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    outline: 0;
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
  }

  .search-input-shell input {
    width: 100%;
    height: 100%;
    padding-right: 92px;
    border: 0;
    background: transparent;
  }

  .replacement-input {
    width: 100%;
  }

  .search-input-shell input:focus-visible,
  .replacement-input:focus-visible {
    outline: 0;
    outline-offset: 0;
  }

  input::placeholder {
    color: var(--md-editor-muted-fg);
  }

  .result-count {
    position: absolute;
    right: 10px;
    color: var(--md-editor-muted-fg);
    font-family: var(--md-editor-font-mono);
    font-size: var(--md-editor-ui-font-size-xs);
    white-space: nowrap;
    pointer-events: none;
    -webkit-user-select: none;
    user-select: none;
  }

  .search-action-group {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    width: 156px;
  }

  .search-action-group.find-actions {
    grid-template-columns: minmax(0, 1fr) 32px;
    gap: 0;
  }

  .search-action-group button {
    width: 100%;
    height: var(--md-editor-control-height-lg);
    padding: 0 5px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: var(--md-editor-surface);
    font-size: var(--md-editor-ui-font-size-sm);
  }

  .search-action-group button:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--md-editor-accent) 42%, var(--md-editor-border));
    background: var(--md-editor-chrome);
  }

  .search-action-group .primary-action,
  .search-action-group .replace-toggle {
    border-color: var(--md-editor-accent-fill);
    background: var(--md-editor-accent-fill);
    color: var(--md-editor-on-accent);
    font-weight: 600;
  }

  .search-action-group .primary-action {
    border-radius: var(--md-editor-radius-md) 0 0 var(--md-editor-radius-md);
  }

  .search-action-group .replace-toggle {
    display: grid;
    place-items: center;
    padding: 0;
    border-left-color: color-mix(in srgb, white 28%, var(--md-editor-accent));
    border-radius: 0 var(--md-editor-radius-md) var(--md-editor-radius-md) 0;
  }

  .search-action-group .primary-action:hover:not(:disabled),
  .search-action-group .replace-toggle:hover:not(:disabled) {
    border-color: var(--md-editor-accent-strong);
    background: var(--md-editor-accent-strong);
    color: var(--md-editor-surface);
  }

  .replace-toggle :global(svg) {
    transition: transform 160ms ease;
  }

  .replace-toggle :global(svg.collapsed) {
    transform: rotate(-90deg);
  }

  .search-option-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 12px;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .search-option-strip::-webkit-scrollbar {
    display: none;
  }

  .search-option {
    position: relative;
    flex: 0 0 auto;
  }

  .search-option input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }

  .search-option span {
    display: inline-flex;
    align-items: center;
    height: var(--md-editor-control-height-sm);
    padding: 0 9px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-pill);
    background: var(--md-editor-chrome);
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-xs);
    white-space: nowrap;
    cursor: pointer;
  }

  .search-option input:checked + span {
    border-color: color-mix(in srgb, var(--md-editor-accent) 32%, var(--md-editor-border));
    background: var(--md-editor-sidebar-active);
    color: var(--md-editor-accent-strong);
    font-weight: 600;
  }

  .option-spacer {
    min-width: 4px;
    flex: 1;
  }

  .count-action {
    height: var(--md-editor-control-height-sm);
    padding: 0 8px;
    border: 0;
    background: transparent;
    color: var(--md-editor-accent-strong);
    font-size: var(--md-editor-ui-font-size-xs);
    white-space: nowrap;
  }

  .search-feedback {
    display: flex;
    align-items: center;
    min-height: 22px;
    margin: 7px 2px -4px;
    color: var(--md-editor-muted-fg);
    font-family: var(--md-editor-font-mono);
    font-size: var(--md-editor-ui-font-size-xs);
    -webkit-user-select: none;
    user-select: none;
  }

  button:focus-visible,
  .search-option input:focus-visible + span {
    outline: 2px solid color-mix(in srgb, var(--md-editor-accent) 55%, transparent);
    outline-offset: 2px;
  }

  @media (max-width: 580px) {
    .search-dialog-body {
      padding-inline: 12px;
    }

    .search-form-row {
      grid-template-columns: 1fr;
    }

    .search-form-row > label {
      text-align: left;
    }

    .search-action-group {
      width: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .replace-toggle :global(svg) {
      transition: none;
    }
  }
</style>

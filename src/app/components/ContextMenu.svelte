<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    AlignCenter,
    AlignLeft,
    AlignRight,
    BetweenHorizontalStart,
    Check,
    ChevronsDown,
    ChevronsUp,
    ChevronRight,
    Clipboard,
    Code2,
    Copy,
    Crop,
    Eye,
    Focus,
    FilePlus2,
    FolderOpen,
    FolderPlus,
    Heading,
    Image,
    Link,
    Link2Off,
    List,
    ListPlus,
    LocateFixed,
    Maximize2,
    Minus,
    PanelLeft,
    PanelTop,
    Pencil,
    Quote,
    Redo2,
    RefreshCw,
    Scissors,
    Search,
    SeparatorHorizontal,
    Sigma,
    Table2,
    TextCursorInput,
    Trash2,
    Undo2,
    X,
    ZoomIn,
  } from '@lucide/svelte';
  import type {
    ContextMenuIcon,
    ContextMenuItem,
  } from '../../lib/editor-core/plugins/contextMenu';
  import { formatShortcutLabel } from '../services/platform';

  export let x: number;
  export let y: number;
  export let items: ContextMenuItem[];
  export let onClose: () => void;

  const iconComponents: Record<ContextMenuIcon, typeof Undo2> = {
    undo: Undo2,
    redo: Redo2,
    cut: Scissors,
    copy: Copy,
    paste: Clipboard,
    search: Search,
    'select-all': TextCursorInput,
    insert: ListPlus,
    format: Crop,
    link: Link,
    open: Eye,
    unlink: Link2Off,
    edit: Pencil,
    delete: Trash2,
    'align-left': AlignLeft,
    'align-center': AlignCenter,
    'align-right': AlignRight,
    image: Image,
    heading: Heading,
    list: List,
    quote: Quote,
    code: Code2,
    table: Table2,
    formula: Sigma,
    diagram: Maximize2,
    separator: SeparatorHorizontal,
    outline: PanelLeft,
    toolbar: PanelTop,
    focus: Focus,
    width: BetweenHorizontalStart,
    zoom: ZoomIn,
    'new-file': FilePlus2,
    'new-folder': FolderPlus,
    folder: FolderOpen,
    refresh: RefreshCw,
    collapse: ChevronsUp,
    expand: ChevronsDown,
    jump: LocateFixed,
    minimize: Minus,
    maximize: Maximize2,
    restore: Copy,
    close: X,
  };

  let menuEl: HTMLDivElement | null = null;
  let adjustedX = x;
  let adjustedY = y;
  let activeIndex = -1;
  let openSubmenuIndex = -1;
  let submenuActiveIndex = -1;
  let submenuOpensLeft = false;
  let submenuOffsetY = -4;

  function selectableIndexes(menuItems: ContextMenuItem[]) {
    return menuItems
      .map((item, index) => ({ item, index }))
      .filter(({ item }) => !item.separator && !item.disabled)
      .map(({ index }) => index);
  }

  async function adjustPosition() {
    await tick();
    if (!menuEl) return;
    const rect = menuEl.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;

    adjustedX = Math.max(8, Math.min(Math.max(8, x), Math.max(8, vw - rect.width - 8)));
    adjustedY = Math.max(8, Math.min(Math.max(8, y), Math.max(8, vh - rect.height - 8)));
    submenuOpensLeft = adjustedX + rect.width + 236 > vw - 8;
  }

  async function adjustSubmenuPosition() {
    await tick();
    if (!menuEl || openSubmenuIndex < 0) return;
    const shell = menuEl
      .querySelector<HTMLButtonElement>(`[data-root-index="${openSubmenuIndex}"]`)
      ?.closest<HTMLElement>('.context-menu-item-shell');
    const submenu = shell?.querySelector<HTMLElement>('.context-menu-submenu');
    if (!shell || !submenu) return;
    const shellRect = shell.getBoundingClientRect();
    const submenuRect = submenu.getBoundingClientRect();
    const top = Math.max(8, Math.min(shellRect.top - 4, window.innerHeight - submenuRect.height - 8));
    submenuOffsetY = top - shellRect.top;
  }

  function focusRootItem(index: number) {
    activeIndex = index;
    submenuActiveIndex = -1;
    void tick().then(() => {
      menuEl?.querySelector<HTMLButtonElement>(`[data-root-index="${index}"]`)?.focus();
    });
  }

  function focusSubmenuItem(index: number) {
    submenuActiveIndex = index;
    void tick().then(() => {
      menuEl?.querySelector<HTMLButtonElement>(`[data-submenu-index="${index}"]`)?.focus();
    });
  }

  function moveFocus(menuItems: ContextMenuItem[], current: number, direction: 1 | -1, submenu = false) {
    const indexes = selectableIndexes(menuItems);
    if (indexes.length === 0) return;
    const currentPosition = indexes.indexOf(current);
    const nextPosition =
      currentPosition < 0
        ? direction > 0
          ? 0
          : indexes.length - 1
        : (currentPosition + direction + indexes.length) % indexes.length;
    if (submenu) focusSubmenuItem(indexes[nextPosition]);
    else focusRootItem(indexes[nextPosition]);
  }

  function activateItem(item: ContextMenuItem) {
    if (item.disabled) return;
    if (item.children?.length) return;
    onClose();
    void item.action?.();
  }

  function handleRootKeydown(event: KeyboardEvent, item: ContextMenuItem, index: number) {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      moveFocus(items, index, event.key === 'ArrowDown' ? 1 : -1);
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const indexes = selectableIndexes(items);
      if (indexes.length) focusRootItem(event.key === 'Home' ? indexes[0] : indexes[indexes.length - 1]);
      return;
    }
    if (event.key === 'ArrowRight' && item.children?.length) {
      event.preventDefault();
      openSubmenuIndex = index;
      const indexes = selectableIndexes(item.children);
      if (indexes.length) focusSubmenuItem(indexes[0]);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      if (item.children?.length) {
        openSubmenuIndex = index;
        const indexes = selectableIndexes(item.children);
        if (indexes.length) focusSubmenuItem(indexes[0]);
      } else {
        activateItem(item);
      }
    }
  }

  function handleSubmenuKeydown(
    event: KeyboardEvent,
    parentIndex: number,
    item: ContextMenuItem,
    index: number,
  ) {
    const submenuItems = items[parentIndex].children ?? [];
    if (event.key === 'Escape' || event.key === 'ArrowLeft') {
      event.preventDefault();
      openSubmenuIndex = -1;
      focusRootItem(parentIndex);
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      moveFocus(submenuItems, index, event.key === 'ArrowDown' ? 1 : -1, true);
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const indexes = selectableIndexes(submenuItems);
      if (indexes.length) focusSubmenuItem(event.key === 'Home' ? indexes[0] : indexes[indexes.length - 1]);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activateItem(item);
    }
  }

  function handleDocumentKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && !menuEl?.contains(document.activeElement)) {
      event.preventDefault();
      onClose();
    }
  }

  function handleMousedownOutside(event: MouseEvent) {
    if (menuEl && !menuEl.contains(event.target as Node)) onClose();
  }

  $: if (x !== undefined && y !== undefined) void adjustPosition();
  $: if (openSubmenuIndex >= 0) void adjustSubmenuPosition();

  onMount(() => {
    const returnFocusElement = document.activeElement as HTMLElement | null;
    void adjustPosition().then(() => {
      const first = selectableIndexes(items)[0];
      if (first !== undefined) focusRootItem(first);
    });
    document.addEventListener('mousedown', handleMousedownOutside, true);
    document.addEventListener('keydown', handleDocumentKeydown);
    document.addEventListener('scroll', onClose, true);
    return () => {
      document.removeEventListener('mousedown', handleMousedownOutside, true);
      document.removeEventListener('keydown', handleDocumentKeydown);
      document.removeEventListener('scroll', onClose, true);
      requestAnimationFrame(() => {
        if (document.activeElement === document.body && returnFocusElement?.isConnected) {
          returnFocusElement.focus();
        }
      });
    };
  });
</script>

<div
  bind:this={menuEl}
  class="context-menu"
  style="position: fixed; left: {adjustedX}px; top: {adjustedY}px;"
  role="menu"
  tabindex="-1"
  aria-label="Context menu"
  on:contextmenu|preventDefault
>
  {#each items as item, index}
    {#if item.separator}
      {#if index > 0}<div class="context-menu-separator" role="separator"></div>{/if}
    {:else}
      <div
        class="context-menu-item-shell"
        role="presentation"
        on:mouseenter={() => {
          activeIndex = index;
          openSubmenuIndex = !item.disabled && item.children?.length ? index : -1;
        }}
      >
        <button
          type="button"
          class="context-menu-item"
          class:active={item.active || activeIndex === index}
          class:danger={item.danger}
          role={item.active ? 'menuitemcheckbox' : 'menuitem'}
          aria-checked={item.active ? true : undefined}
          aria-disabled={item.disabled || undefined}
          aria-haspopup={item.children?.length ? 'menu' : undefined}
          aria-expanded={item.children?.length ? openSubmenuIndex === index : undefined}
          disabled={item.disabled}
          tabindex={activeIndex === index ? 0 : -1}
          data-root-index={index}
          on:mousedown|preventDefault
          on:focus={() => (activeIndex = index)}
          on:click={() => (item.children?.length ? (openSubmenuIndex = index) : activateItem(item))}
          on:keydown={(event) => handleRootKeydown(event, item, index)}
        >
          <span class="context-menu-item-icon" aria-hidden="true">
            {#if item.active}
              <Check size={15} />
            {:else if item.icon}
              <svelte:component this={iconComponents[item.icon]} size={15} />
            {/if}
          </span>
          <span class="context-menu-item-label">{item.label}</span>
          {#if item.shortcut}<span class="context-menu-item-shortcut">{formatShortcutLabel(item.shortcut)}</span>{/if}
          {#if item.children?.length}<ChevronRight class="context-menu-chevron" size={14} />{/if}
        </button>

        {#if item.children?.length && openSubmenuIndex === index}
          <div
            class="context-menu context-menu-submenu"
            class:opens-left={submenuOpensLeft}
            style="top: {submenuOffsetY}px"
            role="menu"
            tabindex="-1"
            aria-label={item.label}
          >
            {#each item.children as child, childIndex}
              {#if child.separator}
                {#if childIndex > 0}<div class="context-menu-separator" role="separator"></div>{/if}
              {:else}
                <button
                  type="button"
                  class="context-menu-item"
                  class:active={child.active || submenuActiveIndex === childIndex}
                  class:danger={child.danger}
                  role={child.active ? 'menuitemcheckbox' : 'menuitem'}
                  aria-checked={child.active ? true : undefined}
                  aria-disabled={child.disabled || undefined}
                  disabled={child.disabled}
                  tabindex={submenuActiveIndex === childIndex ? 0 : -1}
                  data-submenu-index={childIndex}
                  on:mousedown|preventDefault
                  on:mouseenter={() => (submenuActiveIndex = childIndex)}
                  on:focus={() => (submenuActiveIndex = childIndex)}
                  on:click={() => activateItem(child)}
                  on:keydown={(event) => handleSubmenuKeydown(event, index, child, childIndex)}
                >
                  <span class="context-menu-item-icon" aria-hidden="true">
                    {#if child.active}
                      <Check size={15} />
                    {:else if child.icon}
                      <svelte:component this={iconComponents[child.icon]} size={15} />
                    {/if}
                  </span>
                  <span class="context-menu-item-label">{child.label}</span>
                  {#if child.shortcut}<span class="context-menu-item-shortcut">{formatShortcutLabel(child.shortcut)}</span>{/if}
                </button>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .context-menu {
    z-index: var(--md-editor-z-context-menu, 80);
    min-width: 210px;
    max-width: min(300px, calc(100vw - 16px));
    max-height: calc(100dvh - 16px);
    padding: 4px;
    overflow: visible;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: var(--md-editor-surface);
    box-shadow: var(--md-editor-shadow);
    animation: context-menu-in 120ms ease-out forwards;
    color: var(--md-editor-fg);
  }

  .context-menu-item-shell {
    position: relative;
  }

  .context-menu-submenu {
    position: absolute;
    top: -4px;
    left: calc(100% + 4px);
    max-height: min(520px, calc(100dvh - 24px));
    overflow-y: auto;
  }

  .context-menu-submenu.opens-left {
    right: calc(100% + 4px);
    left: auto;
  }

  @media (max-width: 767px), (pointer: coarse) {
    .context-menu {
      overflow: auto;
    }
  }

  @keyframes context-menu-in {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .context-menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 32px;
    padding: 5px 9px;
    border: 0;
    border-radius: var(--md-editor-radius-sm);
    background: transparent;
    color: var(--md-editor-fg);
    font: inherit;
    font-size: var(--md-editor-ui-font-size-sm);
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
    transition: background-color 150ms ease, color 150ms ease;
  }

  .context-menu-item:hover,
  .context-menu-item:focus-visible,
  .context-menu-item.active {
    outline: none;
    background: var(--md-editor-hover-bg, rgba(128, 128, 128, 0.1));
  }

  .context-menu-item:focus-visible {
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--md-editor-accent) 68%, transparent);
  }

  .context-menu-item:disabled {
    opacity: 0.46;
    cursor: default;
  }

  .context-menu-item.danger {
    color: var(--md-editor-danger);
  }

  .context-menu-item.danger:hover,
  .context-menu-item.danger:focus-visible {
    background: color-mix(in srgb, var(--md-editor-danger) 9%, transparent);
  }

  .context-menu-item-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    color: currentColor;
    flex-shrink: 0;
  }

  .context-menu-item-label {
    display: block;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: currentColor;
  }

  .context-menu-item-shortcut {
    padding-left: 20px;
    color: var(--md-editor-muted-fg);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', system-ui, sans-serif;
    font-size: inherit;
    letter-spacing: 0.06em;
    white-space: nowrap;
    flex-shrink: 0;
  }

  :global(.context-menu-chevron) {
    color: var(--md-editor-muted-fg);
    flex-shrink: 0;
  }

  .context-menu-separator {
    height: 1px;
    margin: 4px 7px;
    background: var(--md-editor-border);
  }

  @media (prefers-reduced-motion: reduce) {
    .context-menu {
      animation: none;
    }
    .context-menu-item {
      transition: none;
    }
  }
</style>

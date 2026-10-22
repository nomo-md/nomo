<script lang="ts">
  import { tick } from 'svelte';
  import { Check, ChevronDown, X } from '@lucide/svelte';

  export let id: string;
  export let label: string;
  export let value: string;
  export let options: { value: string; label: string }[];
  export let editable = false;
  export let placeholder = '';
  export let describedBy: string | undefined = undefined;
  export let invalid = false;
  export let mono = false;
  export let resetLabel = '';
  export let onValueChange: (value: string) => void;
  export let onInputValue: (value: string) => void = () => {};

  let root: HTMLDivElement;
  let control: HTMLInputElement | HTMLButtonElement;
  let panel: HTMLDivElement | undefined;
  let open = false;
  let inputValue = value;
  let query = '';
  let activeIndex = -1;
  let typeahead = '';
  let lastTypedAt = 0;

  // 外部设置同步才覆盖草稿；逐字输入不触发主题应用或正文重排。
  $: inputValue = value;
  $: filteredOptions = options.filter((option) =>
    option.label.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()),
  );
  $: activeId = open && filteredOptions[activeIndex] ? `${id}-option-${activeIndex}` : undefined;
  $: selectedLabel = options.find((option) => option.value === value)?.label ?? value;
  $: if (open && !filteredOptions.length) close();

  function close() {
    open = false;
    activeIndex = -1;
    typeahead = '';
  }

  function show() {
    query = '';
    activeIndex = options.findIndex((option) => option.value === value);
    open = true;
    void scrollActiveOption();
  }

  function toggle() {
    control.focus();
    if (open) close();
    else show();
  }

  function choose(nextValue: string) {
    inputValue = nextValue;
    close();
    control.focus();
    onValueChange(nextValue);
  }

  async function scrollActiveOption() {
    await tick();
    if (!open || !panel) return;
    const option = panel.children[activeIndex] as HTMLElement | undefined;
    if (!option) return;
    const top = option.offsetTop;
    const bottom = top + option.offsetHeight;
    if (top < panel.scrollTop) panel.scrollTop = top;
    else if (bottom > panel.scrollTop + panel.clientHeight) {
      panel.scrollTop = bottom - panel.clientHeight;
    }
  }

  function handleInput(event: Event) {
    inputValue = (event.currentTarget as HTMLInputElement).value;
    query = inputValue;
    activeIndex = -1;
    open = true;
    if (!(event as InputEvent).isComposing) onInputValue(inputValue);
  }

  function commitInput() {
    if (inputValue !== value) onValueChange(inputValue);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape' && open) {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (event.key === 'Tab') {
      close();
      return;
    }
    if (event.key === 'Enter' || (!editable && event.key === ' ')) {
      event.preventDefault();
      if (open && filteredOptions[activeIndex]) choose(filteredOptions[activeIndex].value);
      else if (editable) {
        commitInput();
        close();
      } else if (open) close();
      else show();
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (event.altKey && event.key === 'ArrowUp') {
        close();
        return;
      }
      if (!open) {
        show();
        if (activeIndex < 0) activeIndex = event.key === 'ArrowDown' ? 0 : options.length - 1;
      } else {
        activeIndex = Math.max(
          0,
          Math.min(filteredOptions.length - 1, activeIndex + (event.key === 'ArrowDown' ? 1 : -1)),
        );
      }
      void scrollActiveOption();
      return;
    }
    // 可编辑输入保留 Home/End、空格和组合快捷键的原生文本编辑行为。
    if (editable || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      if (!open) show();
      activeIndex = event.key === 'Home' ? 0 : options.length - 1;
      void scrollActiveOption();
    } else if (event.key.length === 1) {
      event.preventDefault();
      if (!open) show();
      const now = Date.now();
      typeahead = (now - lastTypedAt < 700 ? typeahead : '') + event.key.toLocaleLowerCase();
      lastTypedAt = now;
      const index = options.findIndex((option) =>
        option.label.toLocaleLowerCase().startsWith(typeahead),
      );
      if (index >= 0) {
        activeIndex = index;
        void scrollActiveOption();
      }
    }
  }

  /** 弹层进入顶层以避开设置页滚动容器；关闭或切换分类即释放全部监听。 */
  function mountPanel(node: HTMLDivElement) {
    panel = node;
    node.showPopover?.();
    const rect = root.getBoundingClientRect();
    const gutter = 8;
    const gap = 6;
    const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
    const viewportHeight = window.innerHeight;
    const below = Math.max(0, viewportHeight - rect.bottom - gap - gutter);
    const above = Math.max(0, rect.top - gap - gutter);
    const upwards = below < 240 && above > below;
    const width = Math.min(rect.width, viewportWidth - gutter * 2);
    node.style.width = `${width}px`;
    node.style.maxHeight = `${Math.min(288, upwards ? above : below)}px`;
    node.style.left = `${Math.max(gutter, Math.min(rect.left, viewportWidth - width - gutter))}px`;
    node.style.top = upwards ? 'auto' : `${rect.bottom + gap}px`;
    node.style.bottom = upwards ? `${viewportHeight - rect.top + gap}px` : 'auto';

    function outside(event: Event) {
      if (!root.contains(event.target as Node)) close();
    }
    function scroll(event: Event) {
      if (!node.contains(event.target as Node)) close();
    }
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusin', outside);
    document.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', close);
    void scrollActiveOption();
    return {
      destroy() {
        document.removeEventListener('pointerdown', outside, true);
        document.removeEventListener('focusin', outside);
        document.removeEventListener('scroll', scroll, true);
        window.removeEventListener('resize', close);
        if (panel === node) panel = undefined;
      },
    };
  }
</script>

<div class="settings-select" class:open class:invalid bind:this={root}>
  <div class="control">
    {#if editable}
      <input
        {id}
        bind:this={control}
        class="value input"
        class:mono
        type="text"
        role="combobox"
        aria-label={label}
        aria-autocomplete="list"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-options` : undefined}
        aria-activedescendant={activeId}
        aria-describedby={describedBy}
        aria-invalid={invalid}
        value={inputValue}
        {placeholder}
        autocomplete="off"
        spellcheck={false}
        on:input={handleInput}
        on:click={() => {
          if (!open) show();
        }}
        on:blur={commitInput}
        on:keydown={handleKeydown}
      />
      {#if inputValue && resetLabel}
        <button
          class="accessory reset"
          type="button"
          tabindex="-1"
          title={resetLabel}
          aria-label={resetLabel}
          on:mousedown|preventDefault
          on:click={() => choose('')}><X size={14} aria-hidden="true" /></button
        >
      {/if}
      <button
        class="accessory arrow"
        type="button"
        tabindex="-1"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-options` : undefined}
        on:mousedown|preventDefault
        on:click={toggle}><ChevronDown size={16} aria-hidden="true" /></button
      >
    {:else}
      <button
        {id}
        bind:this={control}
        type="button"
        class="value trigger"
        role="combobox"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-options` : undefined}
        aria-activedescendant={activeId}
        aria-describedby={describedBy}
        on:click={toggle}
        on:keydown={handleKeydown}
        ><span>{selectedLabel}</span><ChevronDown size={16} aria-hidden="true" /></button
      >
    {/if}
  </div>
  {#if open && filteredOptions.length}
    <div
      id={`${id}-options`}
      class="options"
      popover="manual"
      role="listbox"
      aria-label={label}
      use:mountPanel
    >
      {#each filteredOptions as option, index (option.value)}
        <button
          id={`${id}-option-${index}`}
          type="button"
          class="option"
          class:active={index === activeIndex}
          class:selected={option.value === value}
          role="option"
          aria-selected={index === activeIndex}
          tabindex="-1"
          on:mousedown|preventDefault
          on:pointermove={() => {
            activeIndex = index;
          }}
          on:click={() => choose(option.value)}
          ><span>{option.label}</span><span class="check" aria-hidden="true"
            >{#if option.value === value}<Check size={15} />{/if}</span
          ></button
        >
      {/each}
    </div>
  {/if}
</div>

<style>
  .settings-select {
    width: 100%;
    min-width: 0;
    font-family: var(--md-editor-font-body);
    font-size: var(--md-editor-ui-font-size);
  }
  .control {
    display: flex;
    align-items: center;
    height: var(--md-editor-control-height-md);
    box-sizing: border-box;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-sm);
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
  }
  .control:hover {
    border-color: color-mix(in srgb, var(--md-editor-fg) 30%, var(--md-editor-border));
  }
  .control:focus-within,
  .open .control {
    border-color: var(--md-editor-accent);
    outline: 2px solid color-mix(in srgb, var(--md-editor-accent) 25%, transparent);
    outline-offset: 2px;
  }
  .invalid .control {
    border-color: var(--md-editor-danger);
  }
  .value {
    min-width: 0;
    width: 100%;
    height: 100%;
    padding: 0 10px;
    border: 0;
    border-radius: inherit;
    background: transparent;
    color: inherit;
    font: inherit;
    outline: none;
  }
  .input {
    flex: 1;
  }
  .input::placeholder {
    color: var(--md-editor-muted-fg);
    opacity: 1;
  }
  .input.mono {
    font-family: var(--md-editor-font-mono);
  }
  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    cursor: pointer;
    text-align: left;
  }
  .trigger span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trigger :global(svg) {
    flex: none;
    color: var(--md-editor-muted-fg);
  }
  .accessory {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 28px;
    height: 100%;
    padding: 0;
    border: 0;
    border-radius: inherit;
    background: transparent;
    color: var(--md-editor-muted-fg);
    cursor: pointer;
  }
  .arrow {
    width: 36px;
  }
  .accessory:hover {
    background: var(--md-editor-hover-bg);
    color: var(--md-editor-fg);
  }
  .options {
    position: fixed;
    inset: auto;
    z-index: 300;
    box-sizing: border-box;
    margin: 0;
    padding: 4px;
    overflow-y: auto;
    overscroll-behavior: contain;
    scrollbar-width: thin;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-sm);
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
    font-family: var(--md-editor-font-body);
    font-size: var(--md-editor-ui-font-size);
    box-shadow: 0 6px 20px #0002;
  }
  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    min-height: var(--md-editor-control-height-md);
    padding: 7px 8px;
    border: 0;
    border-radius: var(--md-editor-radius-xs);
    background: transparent;
    color: inherit;
    font: inherit;
    line-height: 1.4;
    text-align: left;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .option.selected {
    color: var(--md-editor-accent);
  }
  .option.active {
    background: var(--md-editor-hover-bg);
  }
  .option.active.selected {
    background: color-mix(in srgb, var(--md-editor-accent) 10%, var(--md-editor-surface));
  }
  .option:focus-visible {
    outline: 2px solid var(--md-editor-accent);
    outline-offset: -2px;
  }
  .check {
    display: flex;
    flex: 0 0 15px;
  }
</style>

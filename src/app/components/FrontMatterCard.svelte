<script lang="ts">
  import { tick } from 'svelte';
  import { FileText, PencilLine, Trash2 } from '@lucide/svelte';
  import { fade } from 'svelte/transition';
  import type { FrontMatterBlock } from '../../lib/markdown/frontMatter';
  import { clickOutside } from '../actions/clickOutside';
  import { motionIn, transitionDuration } from '../actions/motion';
  import { t } from '../i18n';

  type FrontMatterFocusTarget = 'default' | 'title-value';

  export let interfaceLocale: string;
  export let frontMatter: FrontMatterBlock;
  export let editing: boolean;
  export let focusRequest = 0;
  export let focusTarget: FrontMatterFocusTarget = 'default';
  export let readonly = false;
  export let enterEdit: () => void;
  export let leaveEdit: () => void;
  export let updateContent: (content: string) => void;
  export let deleteFrontMatter: () => void;

  let textarea: HTMLTextAreaElement;
  let confirmingDelete = false;
  let wasEditing = false;
  let lastHandledFocusRequest = -1;

  $: {
    const enteredEditing = editing && !wasEditing;
    const receivedFocusRequest = editing && focusRequest !== lastHandledFocusRequest;

    if (editing && textarea && !readonly && (enteredEditing || receivedFocusRequest)) {
      lastHandledFocusRequest = focusRequest;
      focusTextarea(focusTarget);
    }

    wasEditing = editing;
  }

  async function focusTextarea(target: FrontMatterFocusTarget) {
    await tick();
    await waitForNextFrame();
    if (!editing || readonly || !textarea) {
      return;
    }

    textarea.focus({ preventScroll: true });
    if (target === 'title-value') {
      selectTitleValue(textarea);
      return;
    }

    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  }

  function waitForNextFrame() {
    return new Promise<void>((resolve) => {
      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(() => resolve());
        return;
      }
      window.setTimeout(resolve, 0);
    });
  }

  function selectTitleValue(targetTextarea: HTMLTextAreaElement) {
    const match = /(^|\n)([ \t]*title:[ \t]*)([^\n\r]*)/.exec(targetTextarea.value);
    if (!match || match.index === undefined) {
      targetTextarea.setSelectionRange(targetTextarea.value.length, targetTextarea.value.length);
      return;
    }

    const lineStartOffset = match[1].length;
    const valueStart = match.index + lineStartOffset + match[2].length;
    const valueEnd = valueStart + match[3].length;
    targetTextarea.setSelectionRange(valueStart, valueEnd);
  }

  function handleInput(event: Event) {
    if (readonly) {
      return;
    }
    updateContent((event.currentTarget as HTMLTextAreaElement).value);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      leaveEdit();
    }
  }

  function handleFocusOut(event: FocusEvent) {
    const nextTarget = event.relatedTarget;
    if (nextTarget instanceof Node && event.currentTarget instanceof Node) {
      if (event.currentTarget.contains(nextTarget)) {
        return;
      }
    }
    leaveEdit();
  }

  function requestDelete(event: MouseEvent) {
    event.stopPropagation();
    if (readonly) {
      return;
    }
    if (confirmingDelete) {
      confirmingDelete = false;
      deleteFrontMatter();
      return;
    }
    confirmingDelete = true;
  }

  function resetDeleteConfirmation() {
    confirmingDelete = false;
  }
</script>

{#key interfaceLocale}
{#if editing}
  <section
    class="front-matter-card is-editing"
    data-interface-locale={interfaceLocale}
    aria-label={t.documentMetadataEditing()}
    use:motionIn={{ kind: 'panel', y: 8 }}
    use:clickOutside={leaveEdit}
    transition:fade={{ duration: transitionDuration('mode') }}
    on:focusout={handleFocusOut}
  >
    <div class="front-matter-editor-shell">
      <textarea
        bind:this={textarea}
        aria-label={t.editDocumentMetadataContent()}
        spellcheck="false"
        readonly={readonly}
        value={frontMatter.content}
        on:input={handleInput}
        on:keydown={handleKeydown}
      ></textarea>
    </div>
    {#if frontMatter.fields.parseWarning}
      <p class="front-matter-warning">{frontMatter.fields.parseWarning}</p>
    {/if}
  </section>
{:else}
  <section
    class="front-matter-card"
    data-interface-locale={interfaceLocale}
    aria-label={readonly ? t.viewDocumentMetadata() : t.editDocumentMetadata()}
    use:motionIn={{ kind: 'panel', y: 8 }}
    transition:fade={{ duration: transitionDuration('mode') }}
  >
    <span class="front-matter-icon" aria-hidden="true"><FileText size={18} /></span>
    <button
      type="button"
      class="front-matter-main"
      disabled={readonly}
      on:click={enterEdit}
    >
      <span class="front-matter-kicker">
        <strong class="front-matter-title">{frontMatter.fields.title || t.documentMetadata()}</strong>
        {#if frontMatter.fields.status}
          <span class="front-matter-status">{frontMatter.fields.status}</span>
        {/if}
      </span>
      <span class="front-matter-meta">
        {#if frontMatter.fields.created}
          <span>{t.metadataCreated({ date: frontMatter.fields.created })}</span>
        {/if}
        {#if frontMatter.fields.updated}
          <span>{t.metadataUpdated({ date: frontMatter.fields.updated })}</span>
        {/if}
        {#if frontMatter.fields.extraFieldCount > 0}
          <span>{t.metadataMoreFields({ count: frontMatter.fields.extraFieldCount })}</span>
        {/if}
      </span>
      {#if frontMatter.fields.tags.length > 0}
        <span class="front-matter-tags" aria-label={t.tags()}>
          {#each frontMatter.fields.tags as tag}
            <span>{tag}</span>
          {/each}
        </span>
      {/if}
      {#if frontMatter.fields.parseWarning}
        <span class="front-matter-warning">{frontMatter.fields.parseWarning}</span>
      {/if}
    </button>
    <span class="front-matter-actions">
      <span class="front-matter-edit-hint" aria-hidden="true"><PencilLine size={16} /></span>
      <button
        type="button"
        class:confirming={confirmingDelete}
        class="front-matter-delete"
        disabled={readonly}
        title={confirmingDelete ? t.confirmDeleteMetadata() : t.deleteMetadata()}
        aria-label={confirmingDelete ? t.confirmDeleteMetadata() : t.deleteMetadata()}
        on:click={requestDelete}
        on:blur={resetDeleteConfirmation}
      >
        {#if confirmingDelete}
          <span>{t.confirmDelete()}</span>
        {:else}
          <Trash2 size={16} />
        {/if}
      </button>
    </span>
  </section>
{/if}
{/key}

<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { FileText, GripVertical } from '@lucide/svelte';
  import type { EditorMode, ContextMenuRequest } from '../../lib/editor-core';
  import type { OutlineItem } from '../../lib/outline/outlineService';
  import { extractFrontMatterBlock, replaceFrontMatterContent, removeFrontMatter } from '../../lib/markdown/frontMatter';
  import type { MarkdownTabState } from '../types';
  import type { MarkdownDocumentRuntime } from '../services/markdownDocumentRuntime';
  import type { MarkdownSourceEditorHandle, MarkdownSourceRuntimeState } from './markdownSourceEditor';
  import { createOutlineInteractionController } from '../services/outlineInteractionController';
  import {
    getSemanticScrollAnchor,
    getSourceScrollAnchor,
    restoreSemanticReadingPosition,
    restoreSourceReadingPosition,
    type OutlineScrollAnchor,
  } from '../services/outlineNavigation';
  import EditorWorkspace from './EditorWorkspace.svelte';
  import { t } from '../i18n';

  export let tab: MarkdownTabState;
  export let runtime: MarkdownDocumentRuntime;
  export let interfaceLocale: string;
  export let mode: EditorMode;
  export let focused: boolean;
  export let disabled = false;
  export let outlineVisible = false;
  export let frontMatterEditRequest = 0;
  export let focusDocument: (tabId: string) => void;
  export let onReady: (runtime: MarkdownDocumentRuntime) => void;
  export let onMemberPointerDown: (tabId: string, event: PointerEvent) => void;
  export let onSourceSelection: (tabId: string, range: { from: number; to: number; contentRevision: number }) => void;
  export let onContextMenu: (tabId: string, event: MouseEvent) => void;
  export let openContextMenu: (request: ContextMenuRequest) => void;
  export let copyContextText: (text: string) => void | Promise<void>;
  export let setStatusMessage: (message: string) => void = () => undefined;
  export let flushRuntime: (tabId: string) => void;
  export let jumpToOutline: (tabId: string, item: import('../../lib/outline/outlineService').OutlineItem) => void;
  export let moveOutline: (tabId: string, request: { sourceIndex: number; targetIndex: number; placement: 'before' | 'inside' | 'after' }) => boolean;

  let frontMatterEditing = false;
  let lastEditRequest = frontMatterEditRequest;
  let mountedHost: HTMLDivElement | undefined;
  let imageContextMenuHost: HTMLDivElement | undefined;
  let host: HTMLDivElement;
  let sourceEditor: MarkdownSourceEditorHandle;
  let sourcePane: HTMLElement;
  let semanticPane: HTMLElement;
  let activeOutlineId = runtime.activeOutlineId ?? '';
  let collapsedOutlineIds = runtime.collapsedOutlineIds ?? new Set<string>();
  let destroyed = false;
  let restoreGeneration = 0;
  let appliedMode: EditorMode | undefined;
  let appliedReadonly: boolean | undefined;
  let readyHost: HTMLDivElement | undefined;
  let readySourceEditor: MarkdownSourceEditorHandle | undefined;
  let readyFocused = false;
  const outlineController = createOutlineInteractionController({
    getMode: () => runtime.mode === 'source' ? 'source' : 'semantic',
    getMarkdown: () => runtime.editor.flushMarkdown(),
    getOutline: () => runtime.outline,
    getCollapsedOutlineIds: () => collapsedOutlineIds,
    setCollapsedOutlineIds: (value) => { collapsedOutlineIds = value; runtime.collapsedOutlineIds = value; },
    getOutlineVisible: () => outlineVisible,
    setOutlineVisible: (value) => { outlineVisible = value; },
    setActiveOutlineId: (value) => { activeOutlineId = value; runtime.activeOutlineId = value; },
    getSuppressOutlineScrollUntil: () => runtime.suppressOutlineScrollUntil ?? 0,
    setSuppressOutlineScrollUntil: (value) => { runtime.suppressOutlineScrollUntil = value; },
    getSemanticPane: () => semanticPane,
    getSourcePane: () => sourcePane,
    getSourceEditor: () => sourceEditor,
    getEditor: () => runtime.editor,
    getReadonly: () => tab.readonlyDocumentMode,
    setStatusMessage: (value) => setStatusMessage(value),
    isActive: () => !destroyed && !runtime.disposed && runtime.outlineInteraction === outlineController && Boolean(host && sourceEditor),
  });
  runtime.outlineInteraction = outlineController;

  $: frontMatter = extractFrontMatterBlock(tab.markdown);
  $: readonlyMode = tab.readonlyDocumentMode;
  $: visibleOutlineIds = new Set(runtime.outline.map((item) => item.id));
  $: updateCoreOptions(mode, readonlyMode);
  $: synchronizeBindings(host, sourceEditor, sourcePane, semanticPane, focused);
  $: if (frontMatterEditRequest !== lastEditRequest) {
    lastEditRequest = frontMatterEditRequest;
    if (focused) frontMatterEditing = true;
  }

  function rememberSourceState(state: MarkdownSourceRuntimeState) {
    if (runtime.disposed || state.documentId !== tab.id) return;
    if (runtime.host !== host || runtime.sourceEditor !== sourceEditor) return;
    runtime.sourceState = appliedMode === 'source'
      ? state
      : { ...state, scrollTop: runtime.sourceScrollTop };
  }

  function rememberScroll(visibleMode: EditorMode = appliedMode ?? mode) {
    if (visibleMode === 'semantic' && semanticPane) runtime.semanticScrollTop = semanticPane.scrollTop;
    if (visibleMode === 'source' && sourcePane) runtime.sourceScrollTop = sourcePane.scrollTop;
  }

  function getReadingAnchor(visibleMode: EditorMode): OutlineScrollAnchor | null {
    return visibleMode === 'semantic'
      ? getSemanticScrollAnchor(runtime.outline, semanticPane, runtime.semanticScrollTop)
      : getSourceScrollAnchor(runtime.outline, runtime.sourceScrollTop, sourceEditor?.getLineHeight() ?? 24, sourceEditor, sourcePane);
  }

  function updateCoreOptions(nextMode: EditorMode, readonly: boolean) {
    if (destroyed || runtime.disposed || (appliedMode === nextMode && appliedReadonly === readonly)) return;
    const previousMode = appliedMode;
    if (previousMode) rememberScroll(previousMode);
    const anchor = previousMode && previousMode !== nextMode ? getReadingAnchor(previousMode) : null;
    appliedMode = nextMode;
    appliedReadonly = readonly;
    runtime.mode = nextMode;
    runtime.editor.updateOptions({ mode: nextMode, readonly });
    if (mountedHost && previousMode && previousMode !== nextMode) void restoreLayout(++restoreGeneration, anchor, previousMode);
  }

  function synchronizeBindings(nextHost: HTMLDivElement, nextSource: MarkdownSourceEditorHandle,
    nextSourcePane: HTMLElement, nextSemanticPane: HTMLElement, isFocused: boolean) {
    if (destroyed || runtime.disposed) return;
    if (runtime.sourceEditor !== nextSource) runtime.sourceEditor = nextSource;
    if (runtime.sourcePane !== nextSourcePane) runtime.sourcePane = nextSourcePane;
    if (runtime.semanticPane !== nextSemanticPane) runtime.semanticPane = nextSemanticPane;
    if (runtime.host !== nextHost) runtime.host = nextHost;
    synchronizeImageContextMenuHost(nextHost);
    if (nextHost && nextHost !== mountedHost) {
      mountedHost = nextHost;
      runtime.editor.mount(nextHost);
      const selection = runtime.transferSelection?.semantic;
      if (selection) runtime.editor.restoreSelectionSnapshot(selection);
      void restoreLayout(++restoreGeneration, runtime.readingAnchor ?? null, runtime.readingAnchorMode ?? mode);
    }
    if (nextHost && nextSource && nextSemanticPane &&
      (readyHost !== nextHost || readySourceEditor !== nextSource || readyFocused !== isFocused)) {
      readyHost = nextHost;
      readySourceEditor = nextSource;
      readyFocused = isFocused;
      onReady(runtime);
    }
  }

  function synchronizeImageContextMenuHost(nextHost: HTMLDivElement | undefined) {
    if (imageContextMenuHost === nextHost) return;
    imageContextMenuHost?.removeEventListener('image-context-menu', handleImageContextMenu);
    imageContextMenuHost = nextHost;
    imageContextMenuHost?.addEventListener('image-context-menu', handleImageContextMenu);
  }

  function handleImageContextMenu(event: Event) {
    if (disabled || destroyed || runtime.disposed || runtime.host !== imageContextMenuHost) return;
    const detail = (event as CustomEvent<ContextMenuRequest>).detail;
    if (!detail?.items) return;
    focus();
    openContextMenu(detail);
  }

  async function restoreLayout(generation: number, anchor: OutlineScrollAnchor | null, anchorMode: EditorMode) {
    await tick();
    if (destroyed || runtime.disposed || runtime.host !== host || generation !== restoreGeneration) return;
    const transfer = runtime.transferSelection;
    if (transfer?.source && sourceEditor) sourceEditor.setSelection(transfer.source.anchor, transfer.source.head);
    if (mode === 'source') await sourceEditor?.requestMeasure();
    if (destroyed || runtime.disposed || runtime.host !== host || generation !== restoreGeneration) return;
    if (mode === 'semantic' && semanticPane) {
      if (anchor) restoreSemanticReadingPosition(runtime.outline, semanticPane, anchor, { anchorMode, behavior: 'instant' });
      else semanticPane.scrollTop = runtime.semanticScrollTop;
    }
    if (mode === 'source' && sourcePane) {
      if (anchor) restoreSourceReadingPosition(runtime.outline, sourcePane, sourceEditor, anchor, { anchorMode, behavior: 'instant' });
      else sourcePane.scrollTop = runtime.sourceScrollTop;
    }
    runtime.transferSelection = undefined;
    runtime.readingAnchor = null;
    runtime.readingAnchorMode = undefined;
    rememberScroll();
    onReady(runtime);
  }

  function focus() { if (!disabled && !destroyed) focusDocument(tab.id); }
  function updateSourceScroll() { rememberScroll(); outlineController.updateActiveOutlineFromSourceScroll(); }
  function updateSemanticScroll() { rememberScroll(); outlineController.updateActiveOutlineFromSemanticScroll(); }
  function jumpToItem(item: OutlineItem) {
    focus();
    jumpToOutline(tab.id, item);
  }
  function moveSection(request: { sourceIndex: number; targetIndex: number; placement: 'before' | 'inside' | 'after' }) {
    focus();
    return moveOutline(tab.id, request);
  }
  function resetOutlineCollapse() {
    collapsedOutlineIds = new Set();
    runtime.collapsedOutlineIds = collapsedOutlineIds;
  }

  onDestroy(() => {
    destroyed = true;
    restoreGeneration++;
    synchronizeImageContextMenuHost(undefined);
    if (runtime.outlineInteraction === outlineController) runtime.outlineInteraction = undefined;
    if (runtime.disposed) return;
    // 交换左右时，新 pane 可能已先接管同一文档；旧实例只能清理自己。
    if (runtime.host !== host) return;
    rememberScroll();
    // 源码子组件已通过 rememberSourceState 保存最后状态，解除旧引用后再刷新正文。
    if (runtime.host === host) runtime.host = undefined;
    if (runtime.sourceEditor === sourceEditor) runtime.sourceEditor = undefined;
    if (runtime.sourcePane === sourcePane) runtime.sourcePane = undefined;
    if (runtime.semanticPane === semanticPane) runtime.semanticPane = undefined;
    flushRuntime(tab.id);
    runtime.editor.unmount();
  });
</script>

<section id={`comparison-document-${tab.id}`} class="comparison-pane" class:focused inert={disabled} on:focusin={focus} on:pointerdown={focus} aria-label={tab.fileName}>
  <button type="button" class="comparison-member-title" title={tab.nativePath ?? tab.fileName}
    aria-label={t.dragComparisonMember({ name: tab.fileName })}
    on:pointerdown={(event) => { focus(); onMemberPointerDown(tab.id, event); }} on:click={focus}>
    <GripVertical size={12} class="member-grip" /><FileText size={13} />
    <span>{tab.fileName}</span>{#if tab.dirty}<i aria-label={t.unsavedChanges()}></i>{/if}
  </button>
  <EditorWorkspace {interfaceLocale} {mode} markdown={tab.markdown} sourceDocumentId={tab.id}
    sourceRuntimeState={runtime.sourceState} onSourceRuntimeStateChange={rememberSourceState}
    largeDocumentMode={tab.largeDocumentMode} readonlyDocumentMode={readonlyMode}
    {frontMatter} {frontMatterEditing} frontMatterFocusRequest={frontMatterEditRequest} frontMatterFocusTarget="default"
    outlineVisible={focused && outlineVisible} outline={runtime.outline} {activeOutlineId} {collapsedOutlineIds} {visibleOutlineIds}
    bind:sourceEditor bind:sourcePane bind:semanticPane
    bind:editorHost={host} editorCore={runtime.editor}
    updateMarkdown={(value) => runtime.interaction.updateMarkdown(value)}
    onSourceSelectionChange={(range) => onSourceSelection(tab.id, range)}
    enterFrontMatterEdit={() => { focus(); frontMatterEditing = true; }} leaveFrontMatterEdit={() => { frontMatterEditing = false; }}
    updateFrontMatterContent={(content) => runtime.editor.setMarkdown(replaceFrontMatterContent(runtime.editor.getMarkdown(), content))}
    deleteFrontMatter={() => { frontMatterEditing = false; runtime.editor.setMarkdown(removeFrontMatter(runtime.editor.getMarkdown())); }}
    updateActiveOutlineFromSourceScroll={updateSourceScroll} updateActiveOutlineFromSemanticScroll={updateSemanticScroll}
    handleEditorPaste={runtime.images.handleEditorPaste} handleEditorDrop={runtime.images.handleEditorDrop}
    handleWorkspaceContextMenu={(event) => onContextMenu(tab.id, event)} {openContextMenu} {copyContextText}
    isOutlineItemExpandable={outlineController.isOutlineItemExpandable}
    toggleOutlineItemExpanded={outlineController.toggleOutlineItemExpanded} expandAllOutline={outlineController.expandAllOutline}
    collapseAllOutline={outlineController.collapseAllOutline}
    collapseOutlineToDefaultLevel={resetOutlineCollapse}
    jumpToOutlineItem={jumpToItem}
    moveOutlineSection={moveSection} />
</section>

<style>
  .comparison-pane { display: flex; flex: 1 1 0; min-width: 0; min-height: 0; flex-direction: column; position: relative; }
  .comparison-member-title { display: flex; align-items: center; gap: 6px; min-height: 29px; padding: 4px 12px;
    border: 0; border-bottom: 1px solid var(--md-editor-border, #8883); background: transparent; color: var(--md-editor-muted, #888);
    font: inherit; font-size: 11px; text-align: left; cursor: grab; flex-shrink: 0; }
  .comparison-member-title span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .focused .comparison-member-title { color: var(--md-editor-fg); box-shadow: inset 0 -2px 0 var(--md-editor-accent); }
  .comparison-member-title i { width: 5px; height: 5px; border-radius: 50%; background: var(--md-editor-accent); flex-shrink: 0; }
  .comparison-pane :global(.editor-grid) { flex: 1; min-height: 0; }
  .comparison-pane :global(.member-grip) { opacity: .5; flex-shrink: 0; }
</style>

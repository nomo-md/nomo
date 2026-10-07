<script lang="ts">
  import { ChevronDown, ChevronsDownUp, ChevronsUpDown } from '@lucide/svelte';
  import { onDestroy } from 'svelte';
  import { slide } from 'svelte/transition';
  import type { ContextMenuItem, ContextMenuRequest, EditorCore } from '../../lib/editor-core';
  import type { FrontMatterBlock } from '../../lib/markdown/frontMatter';
  import type { OutlineItem } from '../../lib/outline/outlineService';
  import {
    planOutlineSectionMove,
    type OutlineDropPlacement,
  } from '../../lib/outline/outlineReorder';
  import {
    modePaneMotion,
    motionIn,
    outlinePanelTransition,
    outlineRowTransition,
    transitionDuration,
  } from '../actions/motion';
  import FrontMatterCard from './FrontMatterCard.svelte';
  import MarkdownSourceEditor from './MarkdownSourceEditor.svelte';
  import type { MarkdownSourceEditorHandle, MarkdownSourceRuntimeState, MarkdownSourceSelectionSnapshot } from './markdownSourceEditor';
  import { syncEditorPanes } from '../services/markdownScrollSyncWorkspace';
  import type { MarkdownWorkspaceRuntimeBinding } from '../services/markdownDocumentRuntime';
  import { t } from '../i18n';
  import type { EditorViewMode, SplitActivePane, SplitViewLayout } from '../types';

  export let interfaceLocale: string;
  export let mode: EditorViewMode;
  export let splitViewLayout: SplitViewLayout = 'semantic-source';
  export let splitLeftPercent = 50;
  export let splitActivePane: SplitActivePane = 'semantic';
  export let splitAlignmentGuideVisible = false;
  export let markdown: string;
  export let sourceDocumentId = '';
  export let sourceRuntimeState: MarkdownSourceRuntimeState | undefined = undefined;
  export let onSourceRuntimeStateChange: (state: MarkdownSourceRuntimeState) => void = () => undefined;
  export let onRuntimeReady: (binding: MarkdownWorkspaceRuntimeBinding) => void = () => undefined;
  export let onRuntimeDestroy: (binding: MarkdownWorkspaceRuntimeBinding) => void = () => undefined;
  export let largeDocumentMode: boolean;
  export let frontMatter: FrontMatterBlock | null;
  export let frontMatterEditing: boolean;
  export let frontMatterFocusRequest: number;
  export let frontMatterFocusTarget: 'default' | 'title-value';
  export let readonlyDocumentMode: boolean;
  export let outlineVisible: boolean;
  export let outline: OutlineItem[];
  export let activeOutlineId: string;
  export let collapsedOutlineIds: Set<string>;
  export let visibleOutlineIds: Set<string>;
  export let sourceEditor: MarkdownSourceEditorHandle;
  export let sourcePane: HTMLElement;
  export let semanticPane: HTMLElement;
  export let editorHost: HTMLDivElement;
  export let editorCore: EditorCore;
  export let updateMarkdown: (markdown: string) => void;
  export let setSplitActivePane: (pane: SplitActivePane) => void = () => undefined;
  export let updateSplitLeftPercent: (percent: number, persist: boolean) => void = () => undefined;
  export let onSourceSelectionChange: (range: MarkdownSourceSelectionSnapshot) => void = () => undefined;
  export let enterFrontMatterEdit: () => void;
  export let leaveFrontMatterEdit: () => void;
  export let updateFrontMatterContent: (content: string) => void;
  export let deleteFrontMatter: () => void;
  export let updateActiveOutlineFromSourceScroll: () => void;
  export let updateActiveOutlineFromSemanticScroll: () => void;
  export let handleEditorPaste: (event: ClipboardEvent) => void;
  export let handleEditorDrop: (event: DragEvent) => void;
  export let handleWorkspaceContextMenu: (event: MouseEvent) => void;
  export let openContextMenu: (request: ContextMenuRequest) => void = () => undefined;
  export let copyContextText: (text: string) => void | Promise<void> = () => undefined;
  export let isOutlineItemExpandable: (index: number) => boolean;
  export let toggleOutlineItemExpanded: (item: OutlineItem) => void;
  export let expandAllOutline: () => void = () => undefined;
  export let collapseAllOutline: () => void = () => undefined;
  export let collapseOutlineToDefaultLevel: () => void = () => undefined;
  export let toggleOutlineVisible: () => void = () => undefined;
  export let jumpToOutlineItem: (item: OutlineItem) => void;
  export let moveOutlineSection: (request: {
    sourceIndex: number;
    targetIndex: number;
    placement: 'before' | 'inside' | 'after';
  }) => boolean;
  export let onSourceScroll: (() => void) | undefined = undefined;
  export let onSemanticScroll: (() => void) | undefined = undefined;

  interface PendingOutlineDrag {
    pointerId: number;
    startX: number;
    startY: number;
    sourceIndex: number;
    row: HTMLElement;
  }

  let outlinePanel: HTMLElement;
  let editorGrid: HTMLDivElement;
  let sourcePaneContainer: HTMLElement;
  let splitResizePointerId: number | null = null;
  let splitResizeGrabOffset = 4;
  let pendingSplitLeftPercent = splitLeftPercent;
  let pendingOutlineDrag: PendingOutlineDrag | null = null;
  let outlineDragging = false;
  let outlineDragPreview: HTMLElement | null = null;
  let outlineDropTargetIndex = -1;
  let outlineDropPlacement: OutlineDropPlacement | null = null;
  let outlineDropValid = false;
  let suppressOutlineClick = false;
  let outlineExpandTimer: ReturnType<typeof setTimeout> | null = null;
  let outlineExpandTargetIndex = -1;
  let outlineAutoScrollFrame = 0;
  let outlinePointerX = 0;
  let outlinePointerY = 0;
  let hasExpandableOutline = false;
  let hasCollapsedExpandableOutline = false;
  let runtimeLifetimeEnded = false;
  let registeredSourceHandle: MarkdownSourceEditorHandle | null = null;
  let registeredSourceContent: HTMLElement | null = null;
  let registeredSourceDocumentId = '';
  let ownedRuntime: {
    binding: MarkdownWorkspaceRuntimeBinding;
    destroy: (binding: MarkdownWorkspaceRuntimeBinding) => void;
  } | null = null;

  $: publishRuntime(sourceDocumentId, editorCore, editorHost, sourceEditor, sourcePane, semanticPane, mode);

  function publishRuntime(documentId: string, core: EditorCore, host: HTMLDivElement,
    source: MarkdownSourceEditorHandle, sourceScrollPane: HTMLElement, semanticScrollPane: HTMLElement,
    visibleMode: EditorViewMode) {
    if (runtimeLifetimeEnded || !host?.isConnected || !semanticScrollPane?.isConnected ||
      !sourceScrollPane?.isConnected || source !== registeredSourceHandle ||
      registeredSourceDocumentId !== documentId || !registeredSourceContent?.isConnected) return;
    const previous = ownedRuntime?.binding;
    if (previous?.documentId === documentId && previous.editorCore === core && previous.host === host &&
      previous.sourceEditor === source && previous.sourcePane === sourceScrollPane && previous.semanticPane === semanticScrollPane) {
      previous.mode = visibleMode;
      return;
    }
    releaseRuntime();
    const binding = {
      documentId, editorCore: core, host, sourceEditor: source, sourcePane: sourceScrollPane,
      semanticPane: semanticScrollPane, mode: visibleMode, sourceRuntimeState: source.getRuntimeState?.(),
    };
    ownedRuntime = { binding, destroy: onRuntimeDestroy };
    onRuntimeReady(binding);
  }

  function releaseRuntime(captureSourceState = true) {
    const owner = ownedRuntime;
    if (!owner) return;
    ownedRuntime = null;
    if (captureSourceState && owner.binding.sourceEditor === registeredSourceHandle && registeredSourceContent?.isConnected) {
      owner.binding.sourceRuntimeState = owner.binding.sourceEditor.getRuntimeState?.() ?? owner.binding.sourceRuntimeState;
    }
    owner.destroy(owner.binding);
  }

  function handleSourceRuntimeStateChange(state: MarkdownSourceRuntimeState) {
    if (ownedRuntime?.binding.documentId === state.documentId) ownedRuntime.binding.sourceRuntimeState = state;
    onSourceRuntimeStateChange(state);
  }

  $: hasExpandableOutline = outline.some((_item, index) => isOutlineItemExpandable(index));
  $: hasCollapsedExpandableOutline = outline.some(
    (item, index) => isOutlineItemExpandable(index) && collapsedOutlineIds.has(item.id),
  );
  $: if (splitResizePointerId === null) {
    pendingSplitLeftPercent = splitLeftPercent;
  }

  const SPLIT_DIVIDER_WIDTH_PX = 8;

  function clampSplitPercent(value: number) {
    return Math.min(75, Math.max(25, Math.round(value * 10) / 10));
  }

  function updateSplitResizeFromPointer(event: PointerEvent, persist: boolean) {
    if (!editorGrid || mode !== 'split') return;
    const rect = editorGrid.getBoundingClientRect();
    const usableWidth = rect.width - SPLIT_DIVIDER_WIDTH_PX;
    if (usableWidth <= 0) return;
    const leftTrackWidth = event.clientX - rect.left - splitResizeGrabOffset;
    pendingSplitLeftPercent = clampSplitPercent((leftTrackWidth / usableWidth) * 100);
    updateSplitLeftPercent(pendingSplitLeftPercent, persist);
  }

  function handleSplitResizePointerDown(event: PointerEvent) {
    if (mode !== 'split' || event.button !== 0) return;
    event.preventDefault();
    splitResizePointerId = event.pointerId;
    const target = event.currentTarget as HTMLElement;
    const dividerRect = target.getBoundingClientRect();
    splitResizeGrabOffset = Math.min(
      SPLIT_DIVIDER_WIDTH_PX,
      Math.max(0, event.clientX - dividerRect.left),
    );
    target.setPointerCapture(event.pointerId);
    updateSplitResizeFromPointer(event, false);
  }

  function handleSplitResizePointerMove(event: PointerEvent) {
    if (splitResizePointerId !== event.pointerId) return;
    updateSplitResizeFromPointer(event, false);
  }

  function finishSplitResize(event: PointerEvent) {
    if (splitResizePointerId !== event.pointerId) return;
    if (event.type === 'pointerup') {
      updateSplitResizeFromPointer(event, true);
    } else {
      updateSplitLeftPercent(pendingSplitLeftPercent, true);
    }
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) {
      target.releasePointerCapture(event.pointerId);
    }
    splitResizePointerId = null;
  }

  function handleSplitResizeKeydown(event: KeyboardEvent) {
    if (mode !== 'split') return;
    const step = event.shiftKey ? 10 : 2;
    let nextPercent = splitLeftPercent;
    if (event.key === 'ArrowLeft') nextPercent -= step;
    else if (event.key === 'ArrowRight') nextPercent += step;
    else if (event.key === 'Home') nextPercent = 25;
    else if (event.key === 'End') nextPercent = 75;
    else return;
    event.preventDefault();
    updateSplitLeftPercent(clampSplitPercent(nextPercent), true);
  }

  function handleSourceEditorReady(handle: MarkdownSourceEditorHandle) {
    registeredSourceHandle = handle;
    registeredSourceContent = handle.getContentElement();
    registeredSourceDocumentId = sourceDocumentId;
    sourceEditor = handle;
    sourcePane = handle.getScrollElement();
    requestAnimationFrame(() => {
      editorGrid?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
    });
  }

  // 末行最多上移四分之一视口；只有真实内容已经溢出视口时才启用，
  // 避免短文档在缩放过程中因辅助留白误出现滚动条。
  const SCROLL_PAST_END_VIEWPORT_RATIO = 0.25;
  const LAYOUT_ROUNDING_TOLERANCE_PX = 1;

  interface EditorPaneGeometryParams {
    mode: EditorViewMode;
    contentVersion: string;
  }

  interface PaneGeometry {
    mode: 'source' | 'semantic';
    pane: HTMLElement;
    layout: HTMLElement;
    bottomPadding: number;
    naturalExtent: number;
    viewportHeight: number;
  }

  function coordinateEditorPaneGeometry(
    node: HTMLElement,
    initialParams: EditorPaneGeometryParams,
  ) {
    let params = initialParams;
    let animationFrame = 0;
    const observedTargets = new Set<Element>();

    const resizeObserver =
      typeof ResizeObserver === 'function' ? new ResizeObserver(scheduleUpdate) : null;

    const getPane = (paneMode: 'source' | 'semantic') => {
      return node.querySelector<HTMLElement>(`.${paneMode}-pane`);
    };

    const getEditorSurface = (pane: HTMLElement, paneMode: 'source' | 'semantic') => {
      if (paneMode === 'source') {
        return pane.querySelector<HTMLElement>('.cm-content');
      }
      return pane.querySelector<HTMLElement>('.ProseMirror');
    };

    const getContentEnd = (editorSurface: HTMLElement | null) =>
      editorSurface?.lastElementChild ?? editorSurface;

    const syncObservedTargets = (nextTargets: Element[]) => {
      const nextTargetSet = new Set(nextTargets);
      for (const target of observedTargets) {
        if (!nextTargetSet.has(target)) {
          resizeObserver?.unobserve(target);
          observedTargets.delete(target);
        }
      }
      for (const target of nextTargetSet) {
        if (!observedTargets.has(target)) {
          resizeObserver?.observe(target);
          observedTargets.add(target);
        }
      }
    };

    const getElementBottomInPane = (pane: HTMLElement, element: Element) => {
      const paneRect = pane.getBoundingClientRect();
      const elementRect = element.getBoundingClientRect();
      return elementRect.bottom - paneRect.top + pane.scrollTop;
    };

    const getEditorZoom = () => {
      const value = Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--md-editor-zoom'),
      );
      return Number.isFinite(value) && value > 0 ? value : 1;
    };

    const setPixelVariable = (target: HTMLElement, name: string, value: number) => {
      const nextValue = Math.max(0, Math.round(value * 10) / 10);
      const currentValue = Number.parseFloat(target.style.getPropertyValue(name));
      if (
        Number.isFinite(currentValue) &&
        Math.abs(currentValue - nextValue) <= LAYOUT_ROUNDING_TOLERANCE_PX
      ) {
        return false;
      }
      target.style.setProperty(name, `${nextValue}px`);
      return true;
    };

    const getSplitSourceFrame = () => {
      const style = getComputedStyle(node);
      const top = Number.parseFloat(style.getPropertyValue('--md-editor-split-padding-top'));
      const bottom = Number.parseFloat(style.getPropertyValue('--md-editor-split-padding-bottom'));
      if (!Number.isFinite(top) || !Number.isFinite(bottom)) return null;
      return { top: Math.max(0, top), bottom: Math.max(0, bottom) };
    };

    const syncSplitSourceFrame = (pane: HTMLElement, top: number, bottom: number) => {
      const zoom = getEditorZoom();
      const topChanged = setPixelVariable(pane, '--md-editor-source-frame-padding-top', top / zoom);
      const bottomChanged = setPixelVariable(
        pane,
        '--md-editor-source-frame-padding-bottom',
        bottom / zoom,
      );
      if (topChanged || bottomChanged) sourceEditor?.requestMeasure();
      return topChanged || bottomChanged;
    };

    const syncContentMinHeight = (
      variableTarget: HTMLElement,
      scrollElement: HTMLElement,
      editorSurface: HTMLElement,
      bottomPadding: number,
      source: boolean,
    ) => {
      const paneRect = scrollElement.getBoundingClientRect();
      const surfaceRect = editorSurface.getBoundingClientRect();
      const scrollScale = paneRect.height / scrollElement.clientHeight || 1;
      const surfaceTop = (surfaceRect.top - paneRect.top) / scrollScale + scrollElement.scrollTop;
      const availableVisualHeight = Math.max(
        0,
        scrollElement.clientHeight - surfaceTop - bottomPadding - LAYOUT_ROUNDING_TOLERANCE_PX,
      );
      const minHeight = source ? availableVisualHeight : availableVisualHeight / getEditorZoom();
      setPixelVariable(variableTarget, '--md-editor-content-min-height', minHeight);
    };

    const measurePane = (paneMode: 'source' | 'semantic'): PaneGeometry | null => {
      const pane = getPane(paneMode);
      if (!pane || pane.clientHeight <= 0) return null;
      const layout = pane.querySelector<HTMLElement>(':scope > .document-layout');
      const editorSurface = getEditorSurface(pane, paneMode);
      const contentEnd = getContentEnd(editorSurface);
      if (!layout || !editorSurface || !contentEnd) return null;

      const sourceFrame = paneMode === 'source' ? getSplitSourceFrame() : null;
      const scrollElement =
        paneMode === 'source' ? sourceEditor?.getScrollElement() : pane;
      if (!scrollElement || scrollElement.clientHeight <= 0) return null;

      const layoutBottomPadding = Number.parseFloat(getComputedStyle(layout).paddingBottom) || 0;
      const bottomPadding = paneMode === 'source' ? (sourceFrame?.bottom ?? 0) / getEditorZoom() : layoutBottomPadding;
      if (sourceFrame && syncSplitSourceFrame(pane, sourceFrame.top, sourceFrame.bottom)) {
        scheduleUpdate();
        return null;
      }
      syncContentMinHeight(
        pane,
        scrollElement,
        editorSurface,
        paneMode === 'source' && sourceFrame ? 0 : bottomPadding,
        paneMode === 'source',
      );
      let contentBottom: number;
      if (paneMode === 'source' && sourceEditor) {
        if (sourceFrame) {
          contentBottom = sourceFrame.top / getEditorZoom() + sourceEditor.getContentHeight();
        } else {
          contentBottom = sourceEditor.getLineTop(1) + sourceEditor.getContentHeight();
        }
      } else {
        contentBottom = getElementBottomInPane(pane, contentEnd);
      }
      return {
        mode: paneMode,
        pane,
        layout,
        bottomPadding,
        naturalExtent: contentBottom + bottomPadding,
        viewportHeight: scrollElement.clientHeight,
      };
    };

    const getIndividualSpacerHeight = (geometry: PaneGeometry) => {
      const hasNaturalOverflow =
        geometry.naturalExtent > geometry.viewportHeight + LAYOUT_ROUNDING_TOLERANCE_PX;
      const totalTrailingSpace = hasNaturalOverflow
        ? Math.max(geometry.bottomPadding, geometry.viewportHeight * SCROLL_PAST_END_VIEWPORT_RATIO)
        : geometry.bottomPadding;
      return Math.max(0, Math.round(totalTrailingSpace - geometry.bottomPadding));
    };

    const clearSharedLayoutHeight = (geometry: PaneGeometry | null) => {
      geometry?.layout.style.removeProperty('--md-editor-split-layout-min-height');
      if (geometry?.mode === 'source') {
        geometry.pane.style.removeProperty('--md-editor-split-content-min-height');
      }
    };

    const applyIndividualGeometry = (geometry: PaneGeometry | null) => {
      if (!geometry) return;
      clearSharedLayoutHeight(geometry);
      setPixelVariable(
        geometry.pane,
        '--md-editor-scroll-past-end-space',
        getIndividualSpacerHeight(geometry),
      );
    };

    const update = () => {
      animationFrame = 0;
      const holdSplitGeometry =
        node.dataset.modeTransitionFrom === 'split' && node.getAttribute('aria-busy') === 'true';
      const needsSourceGeometry = params.mode !== 'semantic' || holdSplitGeometry;
      const needsSemanticGeometry = params.mode !== 'source' || holdSplitGeometry;
      const sourcePane = needsSourceGeometry ? getPane('source') : null;
      const semanticPane = needsSemanticGeometry ? getPane('semantic') : null;
      const sourceSurface = sourcePane ? getEditorSurface(sourcePane, 'source') : null;
      const semanticSurface = semanticPane ? getEditorSurface(semanticPane, 'semantic') : null;
      syncObservedTargets(
        [
          node,
          sourcePane,
          semanticPane,
          sourcePane?.querySelector(':scope > .document-layout'),
          semanticPane?.querySelector(':scope > .document-layout'),
          sourceSurface,
          semanticSurface,
          getContentEnd(sourceSurface),
          getContentEnd(semanticSurface),
        ].filter((target): target is Element => Boolean(target)),
      );

      const sourceGeometry = needsSourceGeometry ? measurePane('source') : null;
      const semanticGeometry = needsSemanticGeometry ? measurePane('semantic') : null;

      applyIndividualGeometry(sourceGeometry);
      applyIndividualGeometry(semanticGeometry);

      const targetGeometryReady =
        params.mode === 'split'
          ? Boolean(sourceGeometry && semanticGeometry)
          : Boolean(params.mode === 'source' ? sourceGeometry : semanticGeometry);
      if (targetGeometryReady) {
        node.dispatchEvent(
          new CustomEvent('nomo:editor-pane-geometry-ready', {
            detail: { mode: params.mode },
          }),
        );
      }
    };

    const handleViewportLayoutRefresh = (event: Event) => {
      const detail = (event as CustomEvent<{ synchronous?: boolean }>).detail;
      if (!detail?.synchronous) {
        scheduleUpdate();
        return;
      }
      if (animationFrame) cancelAnimationFrame(animationFrame);
      animationFrame = 0;
      update();
    };
    const handleModeTransitionComplete = () => scheduleUpdate();
    node.addEventListener('nomo:editor-viewport-layout-refresh', handleViewportLayoutRefresh);
    node.addEventListener('nomo:mode-pane-transition-complete', handleModeTransitionComplete);

    function scheduleUpdate() {
      if (animationFrame) return;
      if (typeof requestAnimationFrame !== 'function') {
        update();
        return;
      }
      animationFrame = requestAnimationFrame(update);
    }

    const mutationObserver =
      typeof MutationObserver === 'function' ? new MutationObserver(scheduleUpdate) : null;
    mutationObserver?.observe(node, { childList: true, subtree: true, characterData: true });
    window.addEventListener('resize', scheduleUpdate);
    scheduleUpdate();

    return {
      update(nextParams: EditorPaneGeometryParams) {
        params = nextParams;
        scheduleUpdate();
      },
      destroy() {
        resizeObserver?.disconnect();
        mutationObserver?.disconnect();
        node.removeEventListener(
          'nomo:editor-viewport-layout-refresh',
          handleViewportLayoutRefresh,
        );
        node.removeEventListener(
          'nomo:mode-pane-transition-complete',
          handleModeTransitionComplete,
        );
        window.removeEventListener('resize', scheduleUpdate);
        if (animationFrame) cancelAnimationFrame(animationFrame);
        for (const paneMode of ['source', 'semantic'] as const) {
          const pane = getPane(paneMode);
          pane?.style.removeProperty('--md-editor-content-min-height');
          pane?.style.removeProperty('--md-editor-scroll-past-end-space');
          pane?.style.removeProperty('--md-editor-source-frame-padding-top');
          pane?.style.removeProperty('--md-editor-source-frame-padding-bottom');
          pane?.style.removeProperty('--md-editor-split-content-min-height');
          pane
            ?.querySelector<HTMLElement>(':scope > .document-layout')
            ?.style.removeProperty('--md-editor-split-layout-min-height');
        }
      },
    };
  }

  function toggleAllOutlineItems() {
    if (hasCollapsedExpandableOutline) {
      expandAllOutline();
      return;
    }
    collapseOutlineToDefaultLevel();
  }

  function handleOutlineToggle(event: MouseEvent, item: OutlineItem) {
    event.preventDefault();
    event.stopPropagation();
    toggleOutlineItemExpanded(item);
  }

  function handleOutlineLinkClick(event: MouseEvent, item: OutlineItem) {
    if (suppressOutlineClick) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    jumpToOutlineItem(item);
  }

  function handleOutlinePointerDown(event: PointerEvent, sourceIndex: number) {
    if (
      readonlyDocumentMode ||
      event.button !== 0 ||
      !event.isPrimary ||
      event.pointerType === 'touch' ||
      (event.target as HTMLElement | null)?.closest('.outline-toggle')
    ) {
      return;
    }
    const row = event.currentTarget as HTMLElement;
    pendingOutlineDrag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      sourceIndex,
      row,
    };
    outlinePointerX = event.clientX;
    outlinePointerY = event.clientY;
  }

  function handleOutlinePointerMove(event: PointerEvent) {
    const pending = pendingOutlineDrag;
    if (!pending || event.pointerId !== pending.pointerId) return;
    outlinePointerX = event.clientX;
    outlinePointerY = event.clientY;
    if (!outlineDragging) {
      const distance = Math.hypot(event.clientX - pending.startX, event.clientY - pending.startY);
      if (distance < 5) return;
      beginOutlineDrag(pending, event.clientX, event.clientY);
    }
    event.preventDefault();
    positionOutlineDragPreview(event.clientX, event.clientY);
    updateOutlineDropTarget(event.clientX, event.clientY);
    scheduleOutlineAutoScroll();
  }

  function beginOutlineDrag(pending: PendingOutlineDrag, clientX: number, clientY: number) {
    outlineDragging = true;
    suppressOutlineClick = true;
    pending.row.setPointerCapture?.(pending.pointerId);
    const preview = pending.row.cloneNode(true) as HTMLElement;
    const rect = pending.row.getBoundingClientRect();
    preview.classList.add('outline-drag-preview');
    preview.style.width = `${rect.width}px`;
    preview.setAttribute('aria-hidden', 'true');
    document.body.appendChild(preview);
    outlineDragPreview = preview;
    positionOutlineDragPreview(clientX, clientY);
  }

  function positionOutlineDragPreview(clientX: number, clientY: number) {
    if (!outlineDragPreview) return;
    outlineDragPreview.style.transform = `translate3d(${clientX + 12}px, ${clientY + 12}px, 0)`;
  }

  function updateOutlineDropTarget(clientX: number, clientY: number) {
    const element = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    const row = element?.closest<HTMLElement>('[data-outline-index]');
    if (!row || !outlinePanel?.contains(row) || !pendingOutlineDrag) {
      clearOutlineDropTarget();
      return;
    }
    const targetIndex = Number(row.dataset.outlineIndex);
    const rect = row.getBoundingClientRect();
    const relativeY = rect.height > 0 ? (clientY - rect.top) / rect.height : 0.5;
    const placement: OutlineDropPlacement =
      relativeY < 0.3 ? 'before' : relativeY > 0.7 ? 'after' : 'inside';
    const plan = planOutlineSectionMove(outline, {
      sourceIndex: pendingOutlineDrag.sourceIndex,
      targetIndex,
      placement,
    });
    outlineDropTargetIndex = targetIndex;
    outlineDropPlacement = placement;
    outlineDropValid = plan.ok;
    scheduleCollapsedOutlineExpansion(targetIndex);
  }

  function clearOutlineDropTarget() {
    outlineDropTargetIndex = -1;
    outlineDropPlacement = null;
    outlineDropValid = false;
    clearOutlineExpandTimer();
  }

  function scheduleCollapsedOutlineExpansion(targetIndex: number) {
    if (
      targetIndex === outlineExpandTargetIndex ||
      !isOutlineItemExpandable(targetIndex) ||
      !collapsedOutlineIds.has(outline[targetIndex]?.id)
    ) {
      if (!collapsedOutlineIds.has(outline[targetIndex]?.id)) clearOutlineExpandTimer();
      return;
    }
    clearOutlineExpandTimer();
    outlineExpandTargetIndex = targetIndex;
    outlineExpandTimer = setTimeout(() => {
      const item = outline[targetIndex];
      if (outlineDragging && item && collapsedOutlineIds.has(item.id)) {
        toggleOutlineItemExpanded(item);
      }
      clearOutlineExpandTimer();
    }, 500);
  }

  function clearOutlineExpandTimer() {
    if (outlineExpandTimer) clearTimeout(outlineExpandTimer);
    outlineExpandTimer = null;
    outlineExpandTargetIndex = -1;
  }

  function scheduleOutlineAutoScroll() {
    if (outlineAutoScrollFrame) return;
    outlineAutoScrollFrame = requestAnimationFrame(runOutlineAutoScroll);
  }

  function runOutlineAutoScroll() {
    outlineAutoScrollFrame = 0;
    if (!outlineDragging || !outlinePanel) return;
    const rect = outlinePanel.getBoundingClientRect();
    const edge = 36;
    let delta = 0;
    if (outlinePointerY < rect.top + edge) {
      delta = -Math.ceil(((rect.top + edge - outlinePointerY) / edge) * 12);
    } else if (outlinePointerY > rect.bottom - edge) {
      delta = Math.ceil(((outlinePointerY - (rect.bottom - edge)) / edge) * 12);
    }
    if (delta !== 0) {
      outlinePanel.scrollTop += delta;
      updateOutlineDropTarget(outlinePointerX, outlinePointerY);
      scheduleOutlineAutoScroll();
    }
  }

  function handleOutlinePointerUp(event: PointerEvent) {
    const pending = pendingOutlineDrag;
    if (!pending || event.pointerId !== pending.pointerId) return;
    if (outlineDragging && outlineDropTargetIndex >= 0 && outlineDropPlacement) {
      moveOutlineSection({
        sourceIndex: pending.sourceIndex,
        targetIndex: outlineDropTargetIndex,
        placement: outlineDropPlacement,
      });
    }
    finishOutlineDrag();
  }

  function cancelOutlineDrag() {
    if (!pendingOutlineDrag) return;
    if (outlineDragging) suppressOutlineClick = true;
    finishOutlineDrag();
  }

  function finishOutlineDrag() {
    const pending = pendingOutlineDrag;
    if (pending?.row.hasPointerCapture?.(pending.pointerId)) {
      pending.row.releasePointerCapture(pending.pointerId);
    }
    pendingOutlineDrag = null;
    outlineDragging = false;
    outlineDragPreview?.remove();
    outlineDragPreview = null;
    clearOutlineDropTarget();
    if (outlineAutoScrollFrame) cancelAnimationFrame(outlineAutoScrollFrame);
    outlineAutoScrollFrame = 0;
    setTimeout(() => {
      suppressOutlineClick = false;
    }, 0);
  }

  function handleOutlineWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && pendingOutlineDrag) {
      event.preventDefault();
      cancelOutlineDrag();
    }
  }

  onDestroy(() => {
    runtimeLifetimeEnded = true;
    // 子组件销毁回调已通过 onRuntimeStateChange 提交最后状态，旧 handle 不再可用。
    releaseRuntime(false);
    outlineDragPreview?.remove();
    clearOutlineExpandTimer();
    if (outlineAutoScrollFrame) cancelAnimationFrame(outlineAutoScrollFrame);
  });

  function handleSemanticContextMenu(event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    if (!target || target.closest('input, textarea, select')) {
      return;
    }
    if (target.closest('.front-matter-card')) {
      event.preventDefault();
      return;
    }
    if (target.closest('.prosemirror-host')) return;
    handleWorkspaceContextMenu(event);
  }

  function outlineSeparator(): ContextMenuItem {
    return { label: '', separator: true };
  }

  function buildOutlineViewItems(): ContextMenuItem[] {
    const expandableItems = outline.filter((_item, index) => isOutlineItemExpandable(index));
    const allExpandableItemsCollapsed =
      expandableItems.length > 0 &&
      expandableItems.every((item) => collapsedOutlineIds.has(item.id));
    return [
      {
        label: t.expandAll(),
        icon: 'expand',
        disabled: collapsedOutlineIds.size === 0,
        action: expandAllOutline,
      },
      {
        label: t.collapseAll(),
        icon: 'collapse',
        disabled: expandableItems.length === 0 || allExpandableItemsCollapsed,
        action: collapseAllOutline,
      },
      outlineSeparator(),
      { label: t.hideOutline(), icon: 'outline', action: toggleOutlineVisible },
    ];
  }

  function handleOutlineContextMenu(event: MouseEvent) {
    if (event.defaultPrevented) return;
    event.preventDefault();
    openContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: buildOutlineViewItems(),
    });
  }

  function handleOutlineItemContextMenu(event: MouseEvent, item: OutlineItem, index: number) {
    event.preventDefault();
    const expandable = isOutlineItemExpandable(index);
    const items: ContextMenuItem[] = [
      {
        label: t.jumpToHeading({ title: item.title }),
        icon: 'jump',
        action: () => jumpToOutlineItem(item),
      },
      { label: t.copyHeading(), icon: 'copy', action: () => copyContextText(item.title) },
    ];
    if (expandable) {
      items.push({
        label: collapsedOutlineIds.has(item.id) ? t.expandHeading() : t.collapseHeading(),
        icon: collapsedOutlineIds.has(item.id) ? 'expand' : 'collapse',
        action: () => toggleOutlineItemExpanded(item),
      });
    }
    items.push(outlineSeparator(), ...buildOutlineViewItems());
    openContextMenu({ x: event.clientX, y: event.clientY, items });
  }

  // 拆分标题中的数字前缀与正文，如 "1.2 标题" → ["1.2 ", "标题"]
  function splitTitleNumber(title: string): [string, string] {
    const match = title.match(/^(\d+(?:\.\d+)*\.?\s*)/);
    return match ? [match[1], title.slice(match[1].length)] : ['', title];
  }
</script>

<svelte:window
  on:pointermove={handleOutlinePointerMove}
  on:pointerup={handleOutlinePointerUp}
  on:pointercancel={cancelOutlineDrag}
  on:blur={cancelOutlineDrag}
  on:keydown={handleOutlineWindowKeydown}
/>

{#key interfaceLocale}
  <div
    bind:this={editorGrid}
    class="editor-grid"
    class:source-only={mode === 'source'}
    class:split-view={mode === 'split'}
    class:split-semantic-source={mode === 'split' && splitViewLayout === 'semantic-source'}
    class:split-source-semantic={mode === 'split' && splitViewLayout === 'source-semantic'}
    class:split-resizing={splitResizePointerId !== null}
    style={`--split-left-track: ${splitLeftPercent}fr; --split-right-track: ${100 - splitLeftPercent}fr`}
    use:coordinateEditorPaneGeometry={{ mode, contentVersion: markdown }}
    use:syncEditorPanes={{
      mode,
      documentId: sourceDocumentId,
      markdown,
      sourceEditor,
      editorCore,
      largeDocumentMode,
      activePane: splitActivePane,
      paused: splitResizePointerId !== null,
    }}
    use:modePaneMotion={{ mode, disabled: largeDocumentMode }}
  >
    <section
      id={`source-editor-pane-${sourceDocumentId}`}
      bind:this={sourcePaneContainer}
      class="editor-pane source-pane"
      class:split-pane-left={splitViewLayout === 'source-semantic'}
      class:split-pane-right={splitViewLayout === 'semantic-source'}
      class:split-pane-active={mode === 'split' && splitActivePane === 'source'}
      aria-label={t.markdownSource()}
      on:contextmenu|preventDefault
      on:pointerdown={() => mode === 'split' && setSplitActivePane('source')}
      on:focusin={() => mode === 'split' && setSplitActivePane('source')}
    >
      <div class="document-layout">
        <MarkdownSourceEditor
          bind:sourceEditor
          {markdown}
          documentId={sourceDocumentId}
          runtimeState={sourceRuntimeState}
          onRuntimeStateChange={handleSourceRuntimeStateChange}
          {readonlyDocumentMode}
          onMarkdownChange={updateMarkdown}
          onSelectionSnapshotChange={(selected) => {
            onSourceSelectionChange(selected);
            editorGrid?.dispatchEvent(new Event('nomo:source-caret-change'));
          }}
          onLayoutChange={() => editorGrid?.dispatchEvent(new Event('nomo:source-layout-change'))}
          onPaste={handleEditorPaste}
          onDrop={handleEditorDrop}
          onReady={handleSourceEditorReady}
          onScroll={() => {
            updateActiveOutlineFromSourceScroll();
            onSourceScroll?.();
          }}
        />
      </div>
    </section>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
    <div
      class="split-divider"
      class:active={splitResizePointerId !== null}
      role="separator"
      aria-label={t.splitDivider()}
      aria-hidden={mode !== 'split'}
      aria-orientation="vertical"
      aria-controls={`semantic-editor-pane-${sourceDocumentId} source-editor-pane-${sourceDocumentId}`}
      aria-valuemin="25"
      aria-valuemax="75"
      aria-valuenow={Math.round(splitLeftPercent)}
      tabindex={mode === 'split' ? 0 : -1}
      on:pointerdown={handleSplitResizePointerDown}
      on:pointermove={handleSplitResizePointerMove}
      on:pointerup={finishSplitResize}
      on:pointercancel={finishSplitResize}
      on:lostpointercapture={finishSplitResize}
      on:keydown={handleSplitResizeKeydown}
    ></div>

    {#if mode === 'split' && !largeDocumentMode}
      <div
        class="split-alignment-guide"
        class:visible={splitAlignmentGuideVisible}
        aria-hidden="true"
      ></div>
    {/if}

    <section
      id={`semantic-editor-pane-${sourceDocumentId}`}
      bind:this={semanticPane}
      class="semantic-pane"
      class:split-pane-left={splitViewLayout === 'semantic-source'}
      class:split-pane-right={splitViewLayout === 'source-semantic'}
      class:split-pane-active={mode === 'split' && splitActivePane === 'semantic'}
      aria-label={t.semanticEditorArea()}
      on:scroll={() => {
        updateActiveOutlineFromSemanticScroll();
        onSemanticScroll?.();
      }}
      on:paste={handleEditorPaste}
      on:drop={handleEditorDrop}
      on:dragover|preventDefault
      on:contextmenu={handleSemanticContextMenu}
      on:pointerdown={() => mode === 'split' && setSplitActivePane('semantic')}
      on:focusin={() => mode === 'split' && setSplitActivePane('semantic')}
    >
      <div class="document-layout">
        {#if frontMatter}
          <FrontMatterCard
            {frontMatter}
            {interfaceLocale}
            editing={frontMatterEditing}
            focusRequest={frontMatterFocusRequest}
            focusTarget={frontMatterFocusTarget}
            readonly={readonlyDocumentMode}
            enterEdit={enterFrontMatterEdit}
            leaveEdit={leaveFrontMatterEdit}
            updateContent={updateFrontMatterContent}
            {deleteFrontMatter}
          />
        {/if}
        <div bind:this={editorHost} class="prosemirror-host"></div>
        <div class="editor-scroll-past-end" data-scroll-past-end aria-hidden="true"></div>
      </div>
    </section>

    {#if outlineVisible}
      <aside
        bind:this={outlinePanel}
        class="content-outline"
        class:outline-dragging={outlineDragging}
        class:outline-readonly={readonlyDocumentMode}
        aria-label={t.documentOutline()}
        transition:outlinePanelTransition
        on:contextmenu={handleOutlineContextMenu}
      >
        <div class="content-outline-header">
          <strong>{t.documentOutline()}</strong>
          {#if hasExpandableOutline}
            <button
              type="button"
              class="outline-bulk-toggle"
              title={hasCollapsedExpandableOutline
                ? t.expandAllOutline()
                : t.collapseOutlineToDefaultLevel()}
              aria-label={hasCollapsedExpandableOutline
                ? t.expandAllOutline()
                : t.collapseOutlineToDefaultLevel()}
              on:click={toggleAllOutlineItems}
            >
              {#if hasCollapsedExpandableOutline}
                <ChevronsUpDown size={15} />
              {:else}
                <ChevronsDownUp size={15} />
              {/if}
            </button>
          {/if}
        </div>
        {#if outline.length > 0}
          <div class="content-outline-list">
            {#each outline as item, index (item.id)}
              {#if visibleOutlineIds.has(item.id)}
                <div
                  class:active={activeOutlineId === item.id}
                  class:outline-drag-source={outlineDragging &&
                    pendingOutlineDrag?.sourceIndex === index}
                  class:outline-drop-before={outlineDropValid &&
                    outlineDropTargetIndex === index &&
                    outlineDropPlacement === 'before'}
                  class:outline-drop-inside={outlineDropValid &&
                    outlineDropTargetIndex === index &&
                    outlineDropPlacement === 'inside'}
                  class:outline-drop-after={outlineDropValid &&
                    outlineDropTargetIndex === index &&
                    outlineDropPlacement === 'after'}
                  class:outline-drop-invalid={outlineDragging &&
                    !outlineDropValid &&
                    outlineDropTargetIndex === index}
                  class="content-outline-row"
                  data-outline-index={index}
                  role="group"
                  style={`padding-left: ${(item.level - 1) * 16}px`}
                  transition:outlineRowTransition
                  on:pointerdown={(event) => handleOutlinePointerDown(event, index)}
                  on:contextmenu={(event) => handleOutlineItemContextMenu(event, item, index)}
                >
                  {#if isOutlineItemExpandable(index)}
                    <button
                      type="button"
                      class:collapsed={collapsedOutlineIds.has(item.id)}
                      class="outline-toggle"
                      title={collapsedOutlineIds.has(item.id)
                        ? t.expandHeading()
                        : t.collapseHeading()}
                      aria-label={collapsedOutlineIds.has(item.id)
                        ? t.expandNamedHeading({ title: item.title })
                        : t.collapseNamedHeading({ title: item.title })}
                      aria-expanded={!collapsedOutlineIds.has(item.id)}
                      on:click={(event) => handleOutlineToggle(event, item)}
                    >
                      <ChevronDown size={13} />
                    </button>
                  {:else}
                    <span class="outline-toggle-placeholder"></span>
                  {/if}
                  <button
                    type="button"
                    class="outline-link"
                    title={item.title}
                    on:click={(event) => handleOutlineLinkClick(event, item)}
                  >
                    <span>
                      {#if splitTitleNumber(item.title)[0]}
                        <span class="outline-num">{splitTitleNumber(item.title)[0]}</span
                        >{splitTitleNumber(item.title)[1]}
                      {:else}
                        {item.title}
                      {/if}
                    </span>
                  </button>
                </div>
              {/if}
            {/each}
          </div>
        {:else}
          <p>{t.documentHasNoHeadings()}</p>
        {/if}
      </aside>
    {/if}
  </div>
{/key}

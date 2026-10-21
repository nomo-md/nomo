import type { EditorCore, EditorMode, EditorSelectionSnapshot } from '../../lib/editor-core';
import { getImageLoader } from '../../lib/editor-core/renderers';
import type { ImageContext } from '../../lib/services/render';
import { Transaction } from '@codemirror/state';
import { getSourceTextChanges, type MarkdownSourceEditorHandle, type MarkdownSourceRuntimeState } from '../components/markdownSourceEditor';
import { createPerfTimer, logError, logInfo } from '../../lib/services/logger';
import { t } from '../i18n';
import { createImageMarkdown, getImageFiles } from './imageMarkdown';
import { getMarkdownApplicationSession } from './markdownApplicationSession';

interface ImageInsertionOptions {
  getEditor(): EditorCore;
  getMode(): EditorMode;
  getFileName(): string;
  getNativePath(): string | null;
  getSourceEditor(): MarkdownSourceEditorHandle | undefined;
  getReadonly?(): boolean;
  isDocumentOpen?(): boolean;
  isDocumentFocused?(): boolean;
  getSourceRuntimeState?(): MarkdownSourceRuntimeState | undefined;
  setSourceRuntimeState?(state: MarkdownSourceRuntimeState): void;
  getImageContext(): ImageContext;
  saveMarkdownFile(saveAs?: boolean): Promise<boolean | void> | boolean | void;
  setMarkdown(markdown: string): void;
  setStatusMessage(message: string): void;
  syncSourceTextareaHeight(): void;
}

interface ImageInsertionTarget {
  selection: EditorSelectionSnapshot;
  contentRevision: number;
}

export function createImageInsertionHandlers(options: ImageInsertionOptions) {
  const pendingInsertions = new Set<Promise<void>>();

  function insertImageFiles(files: File[], target?: ImageInsertionTarget): Promise<void> {
    const insertion = performImageInsertion(files, target);
    pendingInsertions.add(insertion);
    void insertion.then(
      () => pendingInsertions.delete(insertion),
      (error) => {
        pendingInsertions.delete(insertion);
        logError('ImageInsertion', 'Failed to insert images', {
          error: error instanceof Error ? error.message : String(error),
        });
        options.setStatusMessage(t.imagesInsertedWithFailures({ inserted: 0, failed: files.length }));
      },
    );
    return insertion;
  }

  /** 迁移捕获正文前等候整笔导入，避免销毁来源后丢失已经开始的图片插入。 */
  async function awaitPendingInsertions(): Promise<void> {
    let failure: PromiseRejectedResult | undefined;
    while (pendingInsertions.size > 0) {
      const results = await Promise.allSettled([...pendingInsertions]);
      failure ??= results.find((result): result is PromiseRejectedResult => result.status === 'rejected');
    }
    if (failure) throw failure.reason;
  }

  function handleEditorDrop(event: DragEvent) {
    const files = getImageFiles(event.dataTransfer?.files);
    if (files.length === 0) {
      return;
    }

    event.preventDefault();
    void insertImageFiles(files);
  }

  function handleEditorPaste(event: ClipboardEvent) {
    const files = getImageFiles(event.clipboardData?.files);
    if (files.length === 0) {
      return;
    }

    event.preventDefault();
    void insertImageFiles(files);
  }

  async function performImageInsertion(files: File[], target?: ImageInsertionTarget) {
    const timer = createPerfTimer('ImageInsertion', '插入图片');
    const editor = options.getEditor();
    const documentOpen = () => options.isDocumentOpen?.() !== false && options.getEditor() === editor;
    const cancelIfClosed = () => {
      const open = documentOpen();
      const readonly = options.getReadonly?.() === true || !editor.canExecute({ type: 'insertImage', src: '' });
      if (open && !readonly) return false;
      timer.end({ cancelled: true, reason: readonly ? 'source-document-readonly' : 'source-document-closed' });
      return true;
    };
    const cancelIfTargetChanged = () => {
      if (!target) return false;
      const snapshot = editor.getSnapshot();
      if (
        options.getMode() === 'semantic' &&
        editor.canExecute({ type: 'insertImage', src: '' }) &&
        (snapshot.contentRevision ?? snapshot.version) === target.contentRevision
      )
        return false;
      options.setStatusMessage(t.quickInsertSelectionChanged());
      timer.end({ cancelled: true, reason: 'insertion-target-changed' });
      return true;
    };

    if (cancelIfClosed() || cancelIfTargetChanged()) return;
    logInfo('ImageInsertion', '开始插入图片', { count: files.length });
    const loader = getImageLoader();
    if (!loader) {
      timer.end({ failed: true, reason: 'loader-not-ready' });
      options.setStatusMessage(t.imageServiceNotReady());
      return;
    }

    const application = getMarkdownApplicationSession(editor);
    if (application) await application.ready;
    if (target) editor.restoreSelectionSnapshot(target.selection);
    const bookmark = application?.createInsertionBookmark();
    try {
    const context = options.getImageContext();
    const strategy = context.settings?.imageInsertStrategy ?? 'copy-assets';
    if (strategy !== 'upload' && !options.getNativePath()) {
      options.setStatusMessage(t.saveBeforeInsertLocalImage());
      await options.saveMarkdownFile(true);
      if (cancelIfClosed()) return;
      if (!options.getNativePath()) {
        timer.end({ cancelled: true, reason: 'document-not-saved' });
        options.setStatusMessage(t.imageInsertCancelled());
        return;
      }
    }

    const imported: Array<{ src: string; alt: string }> = [];
    let failed = 0;

    for (const file of files) {
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        if (cancelIfClosed()) return;
        const result = await loader.import(
          {
            fileName: getInsertFileName(file, imported.length),
            bytes,
          },
          options.getImageContext(),
        );
        if (cancelIfClosed()) return;
        imported.push({ src: result.markdownSrc, alt: file.name || 'image' });
      } catch (error) {
        failed += 1;
        logError('ImageInsertion', 'Failed to import image', {
          fileName: file.name,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    if (cancelIfClosed()) return;
    if (imported.length > 0) {
      // 文件选择和资源导入期间光标可能移动，只在来源正文未变化时替换触发符号。
      if (cancelIfTargetChanged()) return;
      if (target) editor.restoreSelectionSnapshot(target.selection);
      if (bookmark) {
        await bookmark.insert(`\n\n${buildImageSnippet(imported)}\n\n`);
      } else if (options.getMode() === 'source') {
        insertSourceMarkdown(imported, editor);
      } else {
        for (const item of imported) {
          const imageSettings = context.settings;
          const defaultAlign =
            imageSettings?.defaultImageAlign && imageSettings.defaultImageAlign !== 'none'
              ? imageSettings.defaultImageAlign
              : null;
          editor.execute({
            type: 'insertImage',
            src: item.src,
            alt: item.alt,
            width: imageSettings?.defaultImageWidth || null,
            align: defaultAlign,
          });
        }
        if (options.isDocumentFocused?.() !== false) editor.focus();
      }
    }

    if (failed > 0) {
      options.setStatusMessage(t.imagesInsertedWithFailures({ inserted: imported.length, failed }));
    } else {
      options.setStatusMessage(t.imagesInserted({ inserted: imported.length }));
    }
    timer.end({ inserted: imported.length, failed });
    logInfo('ImageInsertion', '图片插入完成', { inserted: imported.length, failed });
    } finally { bookmark?.dispose(); }
  }

  /** HTML 属性值转义：& " < > */
  function escapeHtmlAttr(s: string): string {
    return s
      .replace(/&/g, '&amp;')
      .replace(/"/g, '&quot;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  function insertSourceMarkdown(items: Array<{ src: string; alt: string }>, editor: EditorCore) {
    const sourceEditor = options.getSourceEditor();
    const markdown = editor.flushMarkdown();
    const cachedRuntime = options.getSourceRuntimeState?.();
    // 隐藏标签没有 DOM；直接在同一文档缓存的 CodeMirror State 上提交，保留撤销历史。
    const cachedState = cachedRuntime?.state;
    const detachedState = cachedState
      ? cachedState.update({
          changes: getSourceTextChanges(cachedState.doc.toString(), markdown),
          annotations: Transaction.addToHistory.of(false),
        }).state
      : undefined;
    const selection = sourceEditor?.getSelection() ?? detachedState?.selection.main;
    const start = selection?.from ?? markdown.length;
    const end = selection?.to ?? start;
    const snippet = buildImageSnippet(items);
    const prefix = markdown.slice(0, start);
    const suffix = markdown.slice(end);
    const before = prefix.endsWith('\n') || prefix.length === 0 ? '' : '\n';
    const after = suffix.startsWith('\n') || suffix.length === 0 ? '' : '\n';
    const nextMarkdown = `${prefix}${before}${snippet}${after}${suffix}`;
    const nextSelection = prefix.length + before.length + snippet.length;

    if (sourceEditor) {
      sourceEditor.setMarkdown(nextMarkdown, { addToHistory: true });
      sourceEditor.setSelection(nextSelection);
    } else {
      if (detachedState && cachedRuntime) {
        options.setSourceRuntimeState?.({
          ...cachedRuntime,
          state: detachedState.update({
            changes: { from: start, to: end, insert: `${before}${snippet}${after}` },
            selection: { anchor: nextSelection },
          }).state,
          contentRevision: cachedRuntime.contentRevision + 1,
        });
      }
      editor.setMarkdown(nextMarkdown, { reason: 'source-input', sourceInput: true });
    }
    requestAnimationFrame(() => {
      if (!sourceEditor || options.isDocumentOpen?.() === false || options.isDocumentFocused?.() === false ||
        options.getEditor() !== editor || options.getSourceEditor() !== sourceEditor) {
        return;
      }
      sourceEditor.focus();
      options.syncSourceTextareaHeight();
    });
  }

  function buildImageSnippet(items: Array<{ src: string; alt: string }>) {
    const settings = options.getImageContext().settings;
    const width = settings?.defaultImageWidth || '';
    const align = settings?.defaultImageAlign ?? 'none';
    return items.map(item => {
      if (!width && align === 'none') return createImageMarkdown(item.alt, item.src);
      const img = `<img src="${escapeHtmlAttr(item.src)}" alt="${escapeHtmlAttr(item.alt)}"${width ? ` width="${escapeHtmlAttr(width)}"` : ''}>`;
      return align === 'left' || align === 'center' || align === 'right' ? `<p align="${align}">\n  ${img}\n</p>` : img;
    }).join('\n');
  }

  function getInsertFileName(file: File, index: number) {
    if (file.name?.trim()) {
      return file.name;
    }
    return index === 0 ? 'image.png' : `image-${index + 1}.png`;
  }

  return {
    handleEditorDrop,
    handleEditorPaste,
    insertImageFiles,
    awaitPendingInsertions,
  };
}

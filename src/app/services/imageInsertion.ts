import type { EditorCore, EditorMode } from '../../lib/editor-core';
import { getImageLoader } from '../../lib/editor-core/renderers';
import type { ImageContext } from '../../lib/services/render';
import { Transaction } from '@codemirror/state';
import { getSourceTextChanges, type MarkdownSourceEditorHandle, type MarkdownSourceRuntimeState } from '../components/markdownSourceEditor';
import { createPerfTimer, logError, logInfo } from '../../lib/services/logger';
import { t } from '../i18n';
import { createImageMarkdown, getImageFiles } from './imageMarkdown';

interface ImageInsertionOptions {
  getEditor(): EditorCore;
  getMode(): EditorMode;
  getFileName(): string;
  getNativePath(): string | null;
  getSourceEditor(): MarkdownSourceEditorHandle | undefined;
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

export function createImageInsertionHandlers(options: ImageInsertionOptions) {
  const pendingInsertions = new Set<Promise<void>>();

  function insertImageFiles(files: File[]): Promise<void> {
    const insertion = performImageInsertion(files);
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

  async function performImageInsertion(files: File[]) {
    const timer = createPerfTimer('ImageInsertion', '插入图片');
    const editor = options.getEditor();
    const documentOpen = () => options.isDocumentOpen?.() !== false && options.getEditor() === editor;
    const cancelIfClosed = () => {
      if (documentOpen()) return false;
      timer.end({ cancelled: true, reason: 'source-document-closed' });
      return true;
    };
    if (cancelIfClosed()) return;
    logInfo('ImageInsertion', '开始插入图片', { count: files.length });
    const loader = getImageLoader();
    if (!loader) {
      timer.end({ failed: true, reason: 'loader-not-ready' });
      options.setStatusMessage(t.imageServiceNotReady());
      return;
    }

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
      if (options.getMode() === 'source') {
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
    const imageSettings = options.getImageContext().settings;
    const width = imageSettings?.defaultImageWidth || '';
    const align = imageSettings?.defaultImageAlign ?? 'none';

    const snippet = items
      .map((item) => {
        if (align === 'left' || align === 'center' || align === 'right') {
          let imgTag = `<img src="${escapeHtmlAttr(item.src)}" alt="${escapeHtmlAttr(item.alt)}"`;
          if (width) imgTag += ` width="${escapeHtmlAttr(width)}"`;
          imgTag += '>';
          return `<p align="${align}">\n  ${imgTag}\n</p>`;
        } else if (width) {
          return `<img src="${escapeHtmlAttr(item.src)}" alt="${escapeHtmlAttr(item.alt)}" width="${escapeHtmlAttr(width)}">`;
        } else {
          return createImageMarkdown(item.alt, item.src);
        }
      })
      .join('\n');
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

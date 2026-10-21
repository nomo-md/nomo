import type { EditorSnapshot } from '../../../lib/editor-core/types';
import type { MarkdownApplicationSession } from '../markdownApplicationSession';
import { getCalloutLabel, CALLOUT_TYPES } from '../../../lib/editor-core/callout/calloutTypes';
import { getTypographyOptions } from '../../../lib/typography/options';
import { t } from '../../i18n';
import { resolveTheme } from '../themeManager';
import { THEME_TOKEN_CSS_VARIABLES, THEME_STYLE_TOKEN_CSS_VARIABLES } from '../themeRegistry';
import { awaitExport, throwIfExportAborted, type ExportAppearance, type ExportSnapshot } from './types';

interface SnapshotRuntime {
  session?: MarkdownApplicationSession;
  disposed: boolean;
  editor: { awaitCompositionEnd?(): Promise<void>; getSnapshot(): EditorSnapshot };
  sourceEditor?: { isComposing?(): boolean };
  images?: { awaitPendingInsertions(): Promise<void> };
}

/** 在首次异步等待之前调用；视图宽度和主题随用户继续操作而变化，不能稍后重读。 */
export function captureExportAppearance(host?: HTMLElement): ExportAppearance {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const scheme = root.dataset.theme === 'dark' ? 'dark' : 'light';
  const resolved = resolveTheme({
    themeMode: scheme, colorThemeId: root.dataset.colorTheme, documentStyleId: root.dataset.documentStyle,
  }, scheme);
  const variables: Record<string, string> = {};
  for (const [key, name] of Object.entries(THEME_TOKEN_CSS_VARIABLES)) {
    variables[name] = style.getPropertyValue(name).trim() || resolved.tokens[key as keyof typeof resolved.tokens];
  }
  for (const [key, name] of Object.entries(THEME_STYLE_TOKEN_CSS_VARIABLES)) {
    variables[name] = style.getPropertyValue(name).trim() || resolved.styleTokens[key as keyof typeof resolved.styleTokens];
  }
  for (const name of ['--md-inline-code-border', '--md-inline-code-background', '--md-inline-code-foreground']) {
    const value = style.getPropertyValue(name).trim();
    if (value) variables[name] = value;
  }
  variables['--md-editor-font-size'] = style.getPropertyValue('--md-editor-font-size').trim() || '16px';
  variables['--md-editor-line-height'] = style.getPropertyValue('--md-editor-line-height').trim() || '1.7';
  const content = host?.querySelector<HTMLElement>('.ProseMirror');
  // offsetWidth 不受祖先 zoom / DPI 影响；视图未挂载时使用固定回退值。
  const width = content?.offsetWidth || host?.offsetWidth || 960;
  const locale = root.lang || 'zh-CN';
  const labels: Record<string, string> = { toc: t.toc(), emptyToc: t.documentHasNoHeadings() };
  for (const config of CALLOUT_TYPES) {
    labels[config.type] = getCalloutLabel(config.type, locale === 'zh-CN' || locale === 'zh-TW' ? locale : 'en-US');
  }
  return Object.freeze({
    cssVariables: Object.freeze(variables),
    attributes: Object.freeze({
      'data-theme': scheme, 'data-color-theme': resolved.preferences.colorThemeId,
      'data-theme-style': resolved.styleProfile, 'data-document-style': resolved.preferences.documentStyleId,
      'data-block-style': root.dataset.blockStyle === 'classic' ? 'classic' : 'modern',
    }),
    maxWidthPx: width > 0 && Number.isFinite(width) ? width : 960,
    typography: Object.freeze({ ...getTypographyOptions() }),
    shikiTheme: resolved.editorTheme.shikiTheme,
    mermaid: Object.freeze({ ...resolved.editorTheme.mermaid,
      themeVariables: resolved.editorTheme.mermaid.themeVariables
        ? Object.freeze({ ...resolved.editorTheme.mermaid.themeVariables }) : undefined }),
    locale, labels: Object.freeze(labels),
  });
}

export async function captureExportSnapshot(
  tab: { id: string; nativePath: string | null; fileName: string },
  runtime: SnapshotRuntime,
  options: { appearance: ExportAppearance; signal?: AbortSignal; isAvailable(): boolean; flush(tabId: string): void },
): Promise<ExportSnapshot | null> {
  const metadata = { id: tab.id, nativePath: tab.nativePath, fileName: tab.fileName };
  await awaitExport(runtime.editor.awaitCompositionEnd?.() ?? Promise.resolve(), options.signal);
  const deadline = performance.now() + 30_000;
  while (runtime.sourceEditor?.isComposing?.()) {
    throwIfExportAborted(options.signal);
    if (runtime.disposed || !options.isAvailable()) return null;
    if (performance.now() > deadline) throw new Error('等待输入法结束超时');
    await awaitExport(new Promise<void>(resolve => setTimeout(resolve, 25)), options.signal);
  }
  await awaitExport(runtime.images?.awaitPendingInsertions() ?? Promise.resolve(), options.signal);
  throwIfExportAborted(options.signal);
  if (runtime.disposed || !options.isAvailable()) return null;
  options.flush(metadata.id);
  const captured = await runtime.session?.capture(options.signal);
  const state = captured ? { markdown: captured.markdown, version: captured.revision, contentRevision: captured.revision } : runtime.editor.getSnapshot();
  return Object.freeze({
    documentId: metadata.id, contentRevision: state.contentRevision ?? state.version,
    markdown: state.markdown, documentPath: metadata.nativePath,
    suggestedFileName: metadata.fileName.replace(/\.(md|markdown)$/i, '') || 'Untitled',
    title: metadata.fileName || 'Untitled', appearance: options.appearance,
  });
}

<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import type { EditorThemeOptions } from '../../lib/editor-core';
  import { getCodeTokenizer, getDiagramRenderer } from '../../lib/editor-core/renderers';
  import type { CodeTokenLine } from '../../lib/services/render';
  import { renderMarkdownPreviewBody } from '../../quicklook/preview';
  import { t } from '../i18n';
  import { createDomTypography } from '../../lib/typography/dom';
  import type { TypographyController } from '../../lib/typography/types';
  import { subscribeTypographyInteraction } from '../../lib/typography/interaction';
  let typography: TypographyController | undefined;
  const layoutInteraction = subscribeTypographyInteraction((snapshot) => {
    // 选项同值不再广播后，仍保留小窗原有的几何失效与延迟重排策略。
    if (snapshot.phase !== 'idle') typography?.invalidate();
    if (snapshot.phase === 'settling')
      queueMicrotask(() => layoutInteraction.complete(snapshot.generation));
  });

  export let markdown: string;
  export let nativePath: string | null;
  export let editorTheme: EditorThemeOptions;

  let scrollPane: HTMLElement;
  let staticContainer: HTMLDivElement;
  let staticHtml = '';
  let lazyObserver: IntersectionObserver | null = null;
  let renderGeneration = 0;
  const sourceByFigure = new WeakMap<HTMLElement, { language: string; source: string }>();

  $: schedulePreviewRender(markdown, nativePath, editorTheme);

  onDestroy(() => {
    layoutInteraction.destroy();
    typography?.destroy();
    renderGeneration += 1;
    lazyObserver?.disconnect();
  });

  async function schedulePreviewRender(
    nextMarkdown: string,
    nextNativePath: string | null,
    _theme: EditorThemeOptions,
  ) {
    const generation = ++renderGeneration;
    typography?.destroy();
    staticHtml = '';
    await tick();
    await waitForVisibleFrame();
    if (generation !== renderGeneration) return;
    staticHtml = renderMarkdownPreviewBody(nextMarkdown, {
      documentDirectory: getParentPath(nextNativePath),
    });
    await tick();
    if (generation !== renderGeneration) return;
    setupLazyRendering();
    typography = createDomTypography(staticContainer);
  }

  function waitForVisibleFrame() {
    return new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  }

  function setupLazyRendering() {
    if (!staticContainer || !scrollPane) return;
    lazyObserver?.disconnect();
    const figures = Array.from(
      staticContainer.querySelectorAll<HTMLElement>('figure.code-card, figure.mermaid-block'),
    );

    for (const figure of figures) {
      const code = figure.querySelector('code');
      if (code) {
        sourceByFigure.set(figure, {
          language: figure.classList.contains('mermaid-block') ? 'mermaid' : getCodeLanguage(code),
          source: code.textContent ?? '',
        });
      }
    }

    if (typeof IntersectionObserver !== 'function') {
      for (const figure of figures) void renderFigure(figure);
      return;
    }

    lazyObserver = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const figure = entry.target as HTMLElement;
          lazyObserver?.unobserve(figure);
          void renderFigure(figure);
        }
      },
      { root: scrollPane, rootMargin: '260px 0px' },
    );
    for (const figure of figures) lazyObserver.observe(figure);
  }

  async function renderFigure(figure: HTMLElement) {
    const source = sourceByFigure.get(figure);
    if (!source) return;
    const renderToken = String(Number(figure.dataset.renderToken ?? '0') + 1);
    figure.dataset.renderToken = renderToken;

    if (source.language === 'mermaid') {
      const renderer = getDiagramRenderer();
      if (!renderer) return;
      const result = await renderer.renderMermaid(source.source, {
        theme: editorTheme.mermaid,
      });
      if (figure.dataset.renderToken !== renderToken) return;
      let target = figure.querySelector<HTMLElement>('.mini-mermaid-rendered');
      if (!target) {
        target = document.createElement('div');
        target.className = 'mini-mermaid-rendered';
        figure.querySelector('pre')?.replaceWith(target);
      }
      if (result.svg) {
        target.innerHTML = result.svg;
        target.classList.remove('is-error');
      } else {
        target.textContent = result.error ?? t.mermaidRenderFailed();
        target.classList.add('is-error');
      }
      figure.dataset.miniRendered = 'true';
      return;
    }

    const tokenizer = getCodeTokenizer();
    if (!tokenizer) return;
    const result = await tokenizer.tokenize({
      code: source.source,
      language: source.language,
      theme: editorTheme.shikiTheme,
    });
    if (figure.dataset.renderToken !== renderToken) return;
    const code = figure.querySelector('code');
    if (!code) return;
    code.replaceChildren(createHighlightedCode(result.tokens));
    figure.dataset.miniRendered = 'true';
  }

  function createHighlightedCode(lines: CodeTokenLine[]) {
    const fragment = document.createDocumentFragment();
    lines.forEach((line, lineIndex) => {
      const lineElement = document.createElement('span');
      lineElement.className = 'mini-code-line';
      for (const token of line.tokens) {
        const span = document.createElement('span');
        span.textContent = token.content;
        if (token.color) span.style.color = token.color;
        applyTokenFontStyle(span, token.fontStyle);
        lineElement.append(span);
      }
      fragment.append(lineElement);
      if (lineIndex < lines.length - 1) fragment.append(document.createTextNode('\n'));
    });
    return fragment;
  }

  function applyTokenFontStyle(element: HTMLElement, value?: string) {
    const fontStyle = Number(value ?? 0);
    if (fontStyle & 1) element.style.fontStyle = 'italic';
    if (fontStyle & 2) element.style.fontWeight = '700';
    if (fontStyle & 4) element.style.textDecoration = 'underline';
  }

  function getCodeLanguage(code: Element) {
    const className = Array.from(code.classList).find((value) => value.startsWith('language-'));
    return className?.slice('language-'.length) || 'text';
  }

  function getParentPath(path: string | null) {
    if (!path) return undefined;
    const normalized = path.replace(/\\/g, '/');
    const index = normalized.lastIndexOf('/');
    return index > 0 ? path.slice(0, index) : undefined;
  }
</script>

<section
  bind:this={scrollPane}
  class="markdown-mini-large-preview"
  aria-label={t.markdownMiniWindow()}
>
  {#if staticHtml}
    <div bind:this={staticContainer} class="rich-markdown mini-static-markdown">
      {@html staticHtml}
    </div>
  {:else}
    <div class="markdown-mini-large-loading" role="status">{t.markdownMiniLoading()}</div>
  {/if}
</section>

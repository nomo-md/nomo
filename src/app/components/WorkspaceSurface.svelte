<script lang="ts">
  import type { TabIndicatorGeometry } from '../actions/motion';

  export let indicatorGeometry: TabIndicatorGeometry | null = null;
  export let hasOpenDocument: boolean;
  export let appearanceKey: string;
  export let scheme: 'light' | 'dark';

  const filterId = `workspace-surface-${crypto.getRandomValues(new Uint32Array(4)).join('-')}`;
  let width = 0;
  let height = 0;
  let bodyTop = 0;
  let radius = 0;
  let returnRadius = 8;
  let nearBlur = 0.8;
  let softBlur = 5;

  $: shadowPadding = 3 * Math.max(nearBlur, softBlur);
  $: path = buildPath(width, height, bodyTop, radius, returnRadius, indicatorGeometry);

  // 只构造外轮廓，标签底边与面板顶边的重合部分属于内部区域。
  function buildPath(
    panelWidth: number,
    panelHeight: number,
    top: number,
    outerRadius: number,
    concaveRadius: number,
    tab: TabIndicatorGeometry | null,
  ): string {
    if (panelWidth <= 0 || panelHeight <= top) return '';

    const tabLeft = tab ? Math.max(0, Math.min(panelWidth, tab.x)) : 0;
    const tabRight = tab ? Math.max(tabLeft, Math.min(panelWidth, tab.x + tab.width)) : 0;
    const tabTop = tab ? Math.max(0, Math.min(top, tab.y)) : top;
    const tabHeight = tab ? Math.min(tab.height, top - tabTop) : 0;
    const hasTab = tabRight > tabLeft && tabHeight > 0;
    const bodyRadius = Math.min(outerRadius, panelHeight - top, hasTab ? tabLeft : panelWidth);
    const segments = [
      `M 0 ${panelHeight}`,
      `V ${top + bodyRadius}`,
      `A ${bodyRadius} ${bodyRadius} 0 0 1 ${bodyRadius} ${top}`,
    ];

    if (hasTab) {
      // 先收缩凹圆角，再收缩相邻外圆角，避免首个标签附近的路径交叉。
      const leftReturn = Math.min(concaveRadius, tabLeft - bodyRadius, tabHeight / 2);
      const rightReturn = Math.min(concaveRadius, panelWidth - tabRight, tabHeight / 2);
      const tabRadius = Math.min(outerRadius, tabHeight / 2, (tabRight - tabLeft) / 2);
      segments.push(
        `H ${tabLeft - leftReturn}`,
        `A ${leftReturn} ${leftReturn} 0 0 0 ${tabLeft} ${top - leftReturn}`,
        `V ${tabTop + tabRadius}`,
        `A ${tabRadius} ${tabRadius} 0 0 1 ${tabLeft + tabRadius} ${tabTop}`,
        `H ${tabRight - tabRadius}`,
        `A ${tabRadius} ${tabRadius} 0 0 1 ${tabRight} ${tabTop + tabRadius}`,
        `V ${top - rightReturn}`,
        `A ${rightReturn} ${rightReturn} 0 0 0 ${tabRight + rightReturn} ${top}`,
      );
    }

    segments.push(`H ${panelWidth}`, `V ${panelHeight}`, 'H 0', 'Z');
    return segments.join(' ');
  }

  function measureSurface(
    node: HTMLDivElement,
    params: { appearanceKey: string; hasOpenDocument: boolean },
  ) {
    const shell = node.parentElement!;
    let currentParams = params;
    let frame = 0;
    let observedBody: Element | null = null;
    const resizeObserver = new ResizeObserver(queueMeasure);

    function measure() {
      frame = 0;
      const body = shell.querySelector(
        currentParams.hasOpenDocument ? ':scope > .editor-card' : ':scope > .empty-workspace',
      );
      if (body !== observedBody) {
        if (observedBody) resizeObserver.unobserve(observedBody);
        if (body) resizeObserver.observe(body);
        observedBody = body;
      }
      if (!body) {
        width = 0;
        height = 0;
        return;
      }

      const shellBounds = shell.getBoundingClientRect();
      const bodyBounds = body.getBoundingClientRect();
      const surfaceStyle = getComputedStyle(node);
      width = shellBounds.width;
      height = bodyBounds.bottom - shellBounds.top;
      bodyTop = bodyBounds.top - shellBounds.top;
      radius = Number.parseFloat(getComputedStyle(shell).getPropertyValue('--md-editor-radius-lg'));
      returnRadius = Number.parseFloat(surfaceStyle.getPropertyValue('--workspace-return-radius'));
      nearBlur = Number.parseFloat(surfaceStyle.getPropertyValue('--workspace-near-blur'));
      softBlur = Number.parseFloat(surfaceStyle.getPropertyValue('--workspace-soft-blur'));
    }

    function queueMeasure() {
      if (frame) return;
      frame = requestAnimationFrame(measure);
    }

    resizeObserver.observe(shell);
    queueMeasure();
    return {
      update(nextParams: typeof params) {
        if (
          nextParams.appearanceKey !== currentParams.appearanceKey ||
          nextParams.hasOpenDocument !== currentParams.hasOpenDocument
        ) {
          currentParams = nextParams;
          queueMeasure();
        }
      },
      destroy() {
        resizeObserver.disconnect();
        if (frame) cancelAnimationFrame(frame);
      },
    };
  }
</script>

<div
  class="workspace-surface"
  data-scheme={scheme}
  aria-hidden="true"
  use:measureSurface={{ appearanceKey, hasOpenDocument }}
>
  <svg class="surface-background" {width} {height} viewBox={`0 0 ${width} ${height}`}>
    <defs>
      <filter
        id={filterId}
        filterUnits="userSpaceOnUse"
        x={-shadowPadding}
        y={-shadowPadding}
        width={width + shadowPadding * 2}
        height={height + shadowPadding * 2}
        color-interpolation-filters="sRGB"
      >
        <feGaussianBlur in="SourceAlpha" stdDeviation={nearBlur} result="near-alpha" />
        <feFlood class="near-shadow-color" result="near-color" />
        <feComposite in="near-color" in2="near-alpha" operator="in" result="near-shadow" />
        <feGaussianBlur in="SourceAlpha" stdDeviation={softBlur} result="soft-alpha" />
        <feFlood class="soft-shadow-color" result="soft-color" />
        <feComposite in="soft-color" in2="soft-alpha" operator="in" result="soft-shadow" />
        <feMerge>
          <feMergeNode in="soft-shadow" />
          <feMergeNode in="near-shadow" />
          <feMergeNode in="SourceGraphic" />
        </feMerge>
      </filter>
    </defs>
    <path class="surface-fill" d={path} filter={`url(#${filterId})`} />
  </svg>
  <svg class="surface-outline" {width} {height} viewBox={`0 0 ${width} ${height}`}>
    <path d={path} />
  </svg>
</div>

<style>
  .workspace-surface {
    --workspace-fill: var(--md-editor-surface);
    --workspace-edge: color-mix(in srgb, var(--md-editor-border) 76%, transparent);
    --workspace-border-width: 1px;
    --workspace-return-radius: 8px;
    --workspace-near-blur: 0.8px;
    --workspace-soft-blur: 5px;
    --workspace-shadow-color: rgb(22, 32, 44);
    --workspace-near-opacity: 0.04;
    --workspace-soft-opacity: 0.06;
    display: contents;
    pointer-events: none;
  }

  .workspace-surface[data-scheme='dark'] {
    --workspace-edge: color-mix(in srgb, var(--md-editor-border) 90%, transparent);
    --workspace-shadow-color: #000000;
    --workspace-near-opacity: 0.16;
    --workspace-soft-opacity: 0.22;
  }

  svg {
    position: absolute;
    top: 0;
    left: 0;
    overflow: visible;
    pointer-events: none;
  }

  .surface-background {
    z-index: 0;
  }

  .surface-fill {
    fill: var(--workspace-fill);
    transition: fill 160ms ease;
  }

  .surface-outline {
    z-index: 3;
    fill: none;
    stroke: var(--workspace-edge);
    stroke-width: var(--workspace-border-width);
    transition: stroke 160ms ease;
  }

  .near-shadow-color,
  .soft-shadow-color {
    flood-color: var(--workspace-shadow-color);
    transition:
      flood-color 160ms ease,
      flood-opacity 160ms ease;
  }

  .near-shadow-color {
    flood-opacity: var(--workspace-near-opacity);
  }

  .soft-shadow-color {
    flood-opacity: var(--workspace-soft-opacity);
  }

  @media (prefers-reduced-motion: reduce) {
    .surface-fill,
    .surface-outline,
    .near-shadow-color,
    .soft-shadow-color {
      transition: none;
    }
  }
</style>

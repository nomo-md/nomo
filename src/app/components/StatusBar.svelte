<script lang="ts">
  import { AlertTriangle, Check, LoaderCircle, Minus, RefreshCw, X } from '@lucide/svelte';
  import type {
    MarkdownLintIssue,
    MarkdownLintRuleSet,
    MarkdownLintState,
  } from '../../lib/markdown-lint/types';
  import { getMarkdownLintRuleTitle } from '../../lib/markdown-lint/ruleTitles';
  import type { DocumentStats } from '../../lib/outline/outlineService';
  import { clickOutside } from '../actions/clickOutside';
  import { pulseOnChange } from '../actions/motion';
  import { typographyRange } from '../actions/typographyRange';
  import { t, type EffectiveInterfaceLocale } from '../i18n';

  type StatsMetric = 'lines' | 'words' | 'visibleChars' | 'chars';

  export let interfaceLocale: EffectiveInterfaceLocale;
  export let stats: DocumentStats;
  export let statsStatus: 'pending' | 'ready' | 'error' = 'ready';
  export let writingStatsVisible: boolean;
  export let activeMetric: StatsMetric = 'words';
  export let readingTimeVisible = false;
  export let zoomPercent: number;
  export let markdownLintEnabled: boolean;
  export let markdownLintRuleSet: MarkdownLintRuleSet;
  export let markdownLintState: MarkdownLintState;
  export let onMetricChange: (metric: StatsMetric) => void = () => undefined;
  export let onZoomChange: (percent: number, options?: { transition?: boolean }) => void = () =>
    undefined;
  export let onRetryMarkdownLint: () => void = () => undefined;
  export let onMarkdownLintIssueSelect: (issue: MarkdownLintIssue) => boolean = () => false;

  let statsOpen = false;
  let zoomOpen = false;
  let lintOpen = false;
  let locationHint = '';

  $: statsOptions = [
    { key: 'lines' as const, label: t.lines(), value: stats.lines, unit: t.lineUnit() },
    { key: 'words' as const, label: t.words(), value: stats.words, unit: t.wordUnit() },
    {
      key: 'visibleChars' as const,
      label: t.visibleChars(),
      value: stats.visibleChars,
      unit: t.visibleCharUnit(),
    },
    { key: 'chars' as const, label: t.chars(), value: stats.chars, unit: t.charUnit() },
  ];
  $: activeStatsOption =
    statsOptions.find((option) => option.key === activeMetric) ?? statsOptions[1];
  $: lintStatusLabel = getLintStatusLabel(markdownLintState.status, markdownLintState.total);
  $: checkedAtLabel = markdownLintState.checkedAt
    ? new Intl.DateTimeFormat(interfaceLocale, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      }).format(markdownLintState.checkedAt)
    : t.markdownLintNotCheckedYet();

  function getLintStatusLabel(status: MarkdownLintState['status'], total: number) {
    switch (status) {
      case 'checking':
        return t.markdownLintChecking();
      case 'clean':
        return t.markdownLintClean();
      case 'issues':
        return t.markdownLintIssues({ count: total });
      case 'skipped':
        return t.markdownLintSkipped();
      case 'failed':
        return t.markdownLintFailed();
      default:
        return t.markdownLintStatus();
    }
  }

  function toggleLint() {
    lintOpen = !lintOpen;
    locationHint = '';
  }
  function closeLint() {
    lintOpen = false;
    locationHint = '';
  }
  function selectIssue(issue: MarkdownLintIssue) {
    locationHint = onMarkdownLintIssueSelect(issue) ? '' : t.markdownLintSourceRequired();
  }
  function handleLintKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') closeLint();
  }
  function toggleStats() {
    statsOpen = !statsOpen;
  }
  function selectMetric(metric: StatsMetric) {
    onMetricChange(metric);
    statsOpen = false;
  }
  function closeStats() {
    statsOpen = false;
  }
  function handleStatsKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') closeStats();
  }
  function toggleZoom() {
    zoomOpen = !zoomOpen;
  }
  function closeZoom() {
    zoomOpen = false;
  }
  function handleZoomKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') closeZoom();
  }
  function adjustZoom(delta: number) {
    const next = Math.min(160, Math.max(80, zoomPercent + delta));
    if (next !== zoomPercent) onZoomChange(next);
  }
  function handleZoomSlider(event: Event) {
    const value = parseInt((event.target as HTMLInputElement).value, 10);
    if (!isNaN(value)) onZoomChange(value, { transition: false });
  }
</script>

{#key interfaceLocale}
  <div class="statusbar" aria-label={t.documentStats()} data-interface-locale={interfaceLocale}>
    {#if markdownLintEnabled}
      <div class="statusbar-lint" use:clickOutside={closeLint}>
        <button
          class="statusbar-lint-trigger status-{markdownLintState.status}"
          type="button"
          aria-haspopup="dialog"
          aria-expanded={lintOpen}
          aria-controls="markdown-lint-popover"
          title={lintStatusLabel}
          on:click={toggleLint}
          on:keydown={handleLintKeydown}
        >
          {#if markdownLintState.status === 'checking'}
            <LoaderCircle size={13} class="statusbar-lint-spinner" />
          {:else if markdownLintState.status === 'clean'}
            <Check size={13} />
          {:else if markdownLintState.status === 'issues'}
            <AlertTriangle size={13} />
            <span>{markdownLintState.total}</span>
          {:else if markdownLintState.status === 'skipped'}
            <Minus size={13} />
          {:else if markdownLintState.status === 'failed'}
            <X size={13} />
          {:else}
            <Minus size={13} />
          {/if}
        </button>

        {#if lintOpen}
          <div
            id="markdown-lint-popover"
            class="markdown-lint-popover"
            role="dialog"
            aria-labelledby="markdown-lint-title"
            tabindex="-1"
            on:keydown={handleLintKeydown}
          >
            <div class="markdown-lint-header">
              <div>
                <h2 id="markdown-lint-title">{t.markdownLintStatus()}</h2>
                <p>{lintStatusLabel}</p>
              </div>
              <button class="markdown-lint-retry" type="button" on:click={onRetryMarkdownLint}>
                <RefreshCw size={13} />
                {t.markdownLintRetry()}
              </button>
            </div>
            <dl class="markdown-lint-summary">
              <div>
                <dt>{t.markdownLintRuleMode()}</dt>
                <dd>
                  {markdownLintRuleSet === 'relaxed'
                    ? t.markdownLintRelaxed()
                    : t.markdownLintDefault()}
                </dd>
              </div>
              <div>
                <dt>{t.markdownLintCheckedAt()}</dt>
                <dd>{checkedAtLabel}</dd>
              </div>
              <div>
                <dt>{t.markdownLintIssueCount({ count: markdownLintState.total })}</dt>
                <dd></dd>
              </div>
            </dl>

            {#if locationHint}<p class="markdown-lint-location-hint">{locationHint}</p>{/if}
            {#if markdownLintState.status === 'clean'}
              <div class="markdown-lint-empty"><Check size={20} />{t.markdownLintNoIssues()}</div>
            {:else if markdownLintState.status === 'skipped'}
              <div class="markdown-lint-message">{t.markdownLintSkippedDescription()}</div>
            {:else if markdownLintState.status === 'failed'}
              <div class="markdown-lint-message markdown-lint-error">
                {t.markdownLintFailureDescription()}
                {#if markdownLintState.failureMessage}<code>{markdownLintState.failureMessage}</code
                  >{/if}
              </div>
            {:else if markdownLintState.status === 'checking'}
              <div class="markdown-lint-message">{t.markdownLintChecking()}</div>
            {:else if markdownLintState.issues.length > 0}
              <div class="markdown-lint-issues">
                {#each markdownLintState.issues as issue, index (`${issue.ruleId}-${issue.lineNumber}-${issue.columnNumber}-${index}`)}
                  <button
                    class="markdown-lint-issue"
                    type="button"
                    on:click={() => selectIssue(issue)}
                  >
                    <span class="markdown-lint-rule">{issue.ruleId}</span>
                    <strong
                      >{getMarkdownLintRuleTitle(
                        issue.ruleId,
                        issue.ruleDescription,
                        interfaceLocale,
                      )}</strong
                    >
                    <span class="markdown-lint-position">
                      {issue.columnNumber
                        ? t.markdownLintLocation({
                            line: issue.lineNumber,
                            column: issue.columnNumber,
                          })
                        : t.markdownLintLine({ line: issue.lineNumber })}
                    </span>
                    {#if issue.errorContext}<code>{issue.errorContext}</code>{/if}
                    {#if issue.errorDetail}<small>{issue.errorDetail}</small>{/if}
                  </button>
                {/each}
              </div>
              {#if markdownLintState.total > markdownLintState.issues.length}
                <p class="markdown-lint-remaining">
                  {t.markdownLintRemaining({
                    shown: markdownLintState.issues.length,
                    remaining: markdownLintState.total - markdownLintState.issues.length,
                  })}
                </p>
              {/if}
            {/if}
          </div>
        {/if}
      </div>
    {/if}

    {#if writingStatsVisible}
      <div class="statusbar-zoom" use:clickOutside={closeZoom}>
        <button
          class="statusbar-zoom-trigger"
          type="button"
          aria-haspopup="dialog"
          aria-expanded={zoomOpen}
          aria-controls="zoom-popover"
          title={t.zoomLevel()}
          use:pulseOnChange={zoomPercent}
          on:click={toggleZoom}
          on:keydown={handleZoomKeydown}
        >
          {zoomPercent}%
        </button>
        {#if zoomOpen}
          <div
            id="zoom-popover"
            class="zoom-popover"
            role="dialog"
            aria-labelledby="zoom-popover-title"
          >
            <div class="zoom-popover-header">
              <button
                class="zoom-step-btn"
                type="button"
                aria-label={t.decreaseZoom()}
                on:click={() => adjustZoom(-5)}
                disabled={zoomPercent <= 80}>−</button
              >
              <span id="zoom-popover-title" class="zoom-popover-value">{zoomPercent}%</span>
              <button
                class="zoom-step-btn"
                type="button"
                aria-label={t.increaseZoom()}
                on:click={() => adjustZoom(5)}
                disabled={zoomPercent >= 160}>+</button
              >
            </div>
            <div class="zoom-slider-wrap">
              <input
                type="range"
                min="80"
                max="160"
                step="5"
                value={zoomPercent}
                aria-label={t.zoomLevel()}
                use:typographyRange={'zoom'}
                on:input={handleZoomSlider}
              />
            </div>
          </div>
        {/if}
      </div>

      <div class="statusbar-stats" use:clickOutside={closeStats}>
        <button
          class="statusbar-stats-trigger"
          type="button"
          aria-haspopup="dialog"
          aria-expanded={statsOpen}
          aria-controls="writing-stats-popover"
          title={t.wordCountStats()}
          on:click={toggleStats}
          on:keydown={handleStatsKeydown}
        >
          {#if statsStatus === 'error'}
            {t.writingStatsUnavailable()}
          {:else}
            {statsStatus === 'pending' ? '—' : activeStatsOption.value}{activeStatsOption.unit}
          {/if}
        </button>
        {#if statsOpen}
          <div
            id="writing-stats-popover"
            class="writing-stats-popover"
            role="dialog"
            aria-labelledby="writing-stats-title"
          >
            <h2 id="writing-stats-title">{t.documentStats()}</h2>
            {#if readingTimeVisible && statsStatus === 'ready'}
              <div class="reading-time">
                {t.estimatedReadingMinutes({ minutes: stats.readingMinutes })}
              </div>
            {/if}
            <div class="writing-stats-options" role="group" aria-label={t.selectStatsMetric()}>
              {#each statsOptions as option (option.key)}
                <button
                  class="writing-stats-option"
                  class:active={activeMetric === option.key}
                  type="button"
                  aria-pressed={activeMetric === option.key}
                  on:click={() => selectMetric(option.key)}
                >
                  <span>{option.label}</span><strong>{statsStatus === 'ready' ? option.value : '—'}</strong>
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/key}

<script lang="ts">
  import { Download, ExternalLink, X } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import type { SoftwareUpdateSnapshot } from '../../lib/desktop/tauriUpdater';
  import { openExternalLink } from '../../lib/desktop/tauriStorage';
  import nomoLogoLight from '../../../src-tauri/icons/nomo/source/nomo-app-light.svg?url';
  import nomoLogoDark from '../../../src-tauri/icons/nomo/source/nomo-app-dark.svg?url';
  import { t } from '../i18n';
  import { renderSoftwareUpdateReleaseNotes } from '../services/softwareUpdateReleaseNotes';

  export let state: SoftwareUpdateSnapshot;
  export let onClose: () => void;
  export let onLater: () => void;
  export let onDownload: () => void;
  export let onInstall: () => void;
  export let onRetry: () => void;
  export let onOpenStore: () => void = () => undefined;

  let closeButton: HTMLButtonElement;
  let dialogElement: HTMLDivElement;

  $: version = state.version ?? state.candidate?.version ?? '';
  $: notesHtml = renderSoftwareUpdateReleaseNotes(
    state.body ?? state.candidate?.body,
    t.softwareUpdateReleaseFallback(),
  );
  $: isPortable = state.installationKind === 'portable';
  $: isStore = state.installationKind === 'store';
  $: isDownloading = state.status === 'downloading';
  $: isDownloaded = state.status === 'downloaded';
  $: isInstalling = state.status === 'installing';
  $: progressPercent = state.progress?.percent ?? 0;

  onMount(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton?.focus();
    return () => {
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  });

  function formatDate(value: string | undefined) {
    if (!value) return '';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString();
  }

  function formatSize(value: number | undefined) {
    if (!value) return '';
    return `${Math.max(1, Math.round(value / 1024 / 1024))} MB`;
  }

  function handleNotesClick(event: MouseEvent) {
    const anchor = (event.target as HTMLElement).closest('a');
    const href = anchor?.getAttribute('href');
    if (!href) return;
    event.preventDefault();
    void openExternalLink(href);
  }

  function handleNotesKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    const anchor = (event.target as HTMLElement).closest('a');
    const href = anchor?.getAttribute('href');
    if (!href) return;
    event.preventDefault();
    void openExternalLink(href);
  }

  function handleDialogKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && !isInstalling) {
      onClose();
      return;
    }
    if (event.key !== 'Tab' || !dialogElement) return;

    const focusable = Array.from(
      dialogElement.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function handlePrimaryAction() {
    if (isStore) {
      onOpenStore();
      return;
    }
    if (isPortable) {
      const href = state.candidate?.downloadUrl;
      if (href) void openExternalLink(href);
      return;
    }
    if (isDownloaded) {
      onInstall();
      return;
    }
    if (state.status === 'error') {
      onRetry();
      return;
    }
    onDownload();
  }
</script>

<svelte:window on:keydown={handleDialogKeydown} />

<div
  class="dialog-layer"
  role="presentation"
  on:mousedown={(event) => {
    if (event.currentTarget === event.target && !isInstalling) onClose();
  }}
>
  <div
    bind:this={dialogElement}
    class="update-dialog"
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="software-update-dialog-title"
    on:click={handleNotesClick}
    on:keydown={handleNotesKeydown}
  >
    <header class="dialog-header">
      <div class="app-mark" aria-hidden="true">
        <img class="logo-light" src={nomoLogoLight} alt="" draggable="false" />
        <img class="logo-dark" src={nomoLogoDark} alt="" draggable="false" />
      </div>
      <div class="dialog-title">
        <h2 id="software-update-dialog-title">
          {t.softwareUpdateReleaseTitle({ version })}
        </h2>
        <div class="release-meta">
          <code>v{version}</code>
          {#if state.date}<span>{formatDate(state.date)}</span>{/if}
          {#if state.candidate?.assetSize}<span>{formatSize(state.candidate.assetSize)}</span>{/if}
        </div>
      </div>
      <button
        bind:this={closeButton}
        class="dialog-close"
        type="button"
        aria-label={t.close()}
        disabled={isInstalling}
        on:click={onClose}
      >
        <X size={16} />
      </button>
    </header>

    <div class="release-scroll release-notes">
      {@html notesHtml}
    </div>

    <footer class="dialog-footer">
      <div class="footer-note">
        {#if isStore}
          <strong>{t.softwareUpdateStorePill()}</strong>
          {t.softwareUpdateStoreManagedHint()}
        {:else if isPortable}
          <strong>{t.softwareUpdatePortableTitle()}</strong>
          {t.softwareUpdatePortableHint()}
        {:else if isDownloaded}
          <strong>{t.softwareUpdateReady()}</strong>
          {t.softwareUpdateReadyHint()}
        {:else}
          <strong>{t.softwareUpdateInstallSafeTitle()}</strong>
          {t.softwareUpdateInstallSafeHint()}
        {/if}
      </div>

      {#if isDownloading}
        <div class="download-status" aria-live="polite">
          <div class="download-label">
            <span>{t.softwareUpdateDownloading()}</span>
            <span>{progressPercent}%</span>
          </div>
          <div class="progress-track">
            <div class="progress-value" style={`width: ${progressPercent}%`}></div>
          </div>
        </div>
      {:else}
        <div class="footer-actions">
          <button class="button" type="button" disabled={isInstalling} on:click={onLater}>
            {isStore ? t.close() : t.softwareUpdateLater()}
          </button>
          {#if !isStore || state.storeProductId}
            <button
              class="button primary"
              type="button"
              disabled={(!isStore && !state.candidate && !isDownloaded) || isInstalling}
              on:click={handlePrimaryAction}
            >
              {#if isStore}
                <ExternalLink size={14} />
                {t.softwareUpdateOpenStore()}
              {:else if isPortable}
                <Download size={14} />
                {t.softwareUpdateDownloadPortable()}
              {:else if isDownloaded}
                {t.softwareUpdateRestartAndInstall()}
              {:else if state.status === 'error'}
                {t.softwareUpdateCheckAgain()}
              {:else}
                <Download size={14} />
                {t.softwareUpdateDownloadNow()}
              {/if}
            </button>
          {/if}
        </div>
      {/if}
    </footer>
  </div>
</div>

<style>
  .dialog-layer {
    position: fixed;
    z-index: 150;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(17, 24, 39, 0.32);
    backdrop-filter: blur(2px);
    animation: layer-in 170ms ease;
  }

  .update-dialog {
    width: min(640px, calc(100vw - 28px));
    max-height: min(720px, calc(100vh - 36px));
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--md-editor-border) 78%, var(--md-editor-accent) 22%);
    border-radius: 14px;
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
    box-shadow:
      0 36px 90px rgba(19, 27, 35, 0.2),
      0 8px 28px rgba(19, 27, 35, 0.11);
    animation: dialog-in 220ms cubic-bezier(0.2, 0.85, 0.35, 1);
  }

  .dialog-header {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr) auto;
    gap: 14px;
    align-items: center;
    padding: 20px 22px 18px;
    border-bottom: 1px solid var(--md-editor-border);
  }

  .app-mark {
    width: 48px;
    height: 48px;
    overflow: hidden;
    border-radius: 11px;
  }

  .app-mark img {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: contain;
  }

  .app-mark .logo-dark {
    display: none;
  }

  :global(:root[data-theme='dark']) .app-mark .logo-light {
    display: none;
  }

  :global(:root[data-theme='dark']) .app-mark .logo-dark {
    display: block;
  }

  .dialog-title h2 {
    margin: 0 0 5px;
    color: var(--md-editor-heading-fg);
    font-size: 19px;
    font-weight: 720;
    letter-spacing: -0.015em;
  }

  .release-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    color: var(--md-editor-muted-fg);
    font-size: 11px;
  }

  .release-meta code {
    padding: 2px 6px;
    border-radius: 999px;
    background: var(--md-editor-sidebar-active);
    color: var(--md-editor-accent-strong);
    font-family: var(--md-editor-font-mono);
    font-size: 10px;
    font-weight: 700;
  }

  .dialog-close {
    align-self: start;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--md-editor-muted-fg);
    cursor: pointer;
  }

  .dialog-close:hover {
    background: var(--md-editor-rail);
    color: var(--md-editor-fg);
  }

  .release-scroll {
    min-height: 0;
    overflow-y: auto;
    padding: 22px;
    -webkit-user-select: text;
    user-select: text;
  }

  .release-notes :global(h1),
  .release-notes :global(h2),
  .release-notes :global(h3) {
    margin: 0 0 14px;
    color: var(--md-editor-heading-fg);
    font-size: 16px;
  }

  .release-notes :global(p),
  .release-notes :global(li) {
    color: var(--md-editor-fg);
    font-size: 13px;
    line-height: 1.7;
  }

  .release-notes :global(ul),
  .release-notes :global(ol) {
    display: grid;
    gap: 10px;
    margin: 0;
    padding-left: 20px;
  }

  .release-notes :global(a) {
    color: var(--md-editor-link-fg);
    text-decoration: none;
  }

  .release-notes :global(code) {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--md-editor-code-bg);
    font-family: var(--md-editor-font-mono);
  }

  .dialog-footer {
    min-height: 68px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 22px;
    border-top: 1px solid var(--md-editor-border);
    background: color-mix(in srgb, var(--md-editor-rail) 62%, var(--md-editor-surface));
  }

  .footer-note {
    color: var(--md-editor-muted-fg);
    font-size: 11px;
    line-height: 1.45;
  }

  .footer-note strong {
    display: block;
    color: var(--md-editor-fg);
    font-weight: 650;
  }

  .footer-actions {
    display: flex;
    flex: 0 0 auto;
    gap: 8px;
  }

  .button {
    min-height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    padding: 0 13px;
    border: 1px solid var(--md-editor-border);
    border-radius: 7px;
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
    font-size: 12px;
    font-weight: 650;
    cursor: pointer;
  }

  .button.primary {
    border-color: var(--md-editor-accent-fill);
    background: var(--md-editor-accent-fill);
    color: var(--md-editor-on-accent);
  }

  .button:disabled {
    cursor: default;
    opacity: 0.55;
  }

  .download-status {
    width: min(260px, 45vw);
  }

  .download-label {
    display: flex;
    justify-content: space-between;
    margin-bottom: 7px;
    color: var(--md-editor-muted-fg);
    font-size: 11px;
  }

  .progress-track {
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--md-editor-border);
  }

  .progress-value {
    height: 100%;
    border-radius: inherit;
    background: var(--md-editor-accent);
    transition: width 160ms ease;
  }

  button:focus-visible {
    outline: 2px solid var(--md-editor-accent);
    outline-offset: 2px;
  }

  @keyframes layer-in {
    from {
      opacity: 0;
    }
  }

  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.985);
    }
  }

  @media (max-width: 520px) {
    .dialog-layer {
      align-items: end;
      padding: 12px;
    }

    .dialog-header {
      grid-template-columns: 42px minmax(0, 1fr) auto;
      padding: 17px;
    }

    .app-mark {
      width: 42px;
      height: 42px;
    }

    .dialog-footer {
      align-items: stretch;
      flex-direction: column;
    }

    .download-status {
      width: 100%;
    }

    .footer-actions,
    .footer-actions .button {
      flex: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dialog-layer,
    .update-dialog {
      animation: none;
    }
  }
</style>

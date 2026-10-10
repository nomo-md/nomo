<script lang="ts">
  import { X } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import type { InstalledReleaseNotes } from '../../lib/desktop/tauriUpdater';
  import { openExternalLink } from '../../lib/desktop/tauriStorage';
  import { logError } from '../../lib/services/logger';
  import nomoLogoLight from '../../../src-tauri/icons/nomo/source/nomo-app-light.svg?url';
  import nomoLogoDark from '../../../src-tauri/icons/nomo/source/nomo-app-dark.svg?url';
  import { t } from '../i18n';
  import { renderSoftwareUpdateReleaseNotes } from '../services/softwareUpdateReleaseNotes';

  export let notes: InstalledReleaseNotes;
  export let onShown: () => void;
  export let onClose: () => void;

  let primaryButton: HTMLButtonElement;
  let dialogElement: HTMLDivElement;
  $: notesHtml = renderSoftwareUpdateReleaseNotes(notes.body, t.softwareUpdateReleaseFallback());

  onMount(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    primaryButton?.focus();
    // 组件已挂载后才消耗提醒，领取成功不代表用户已看到日志。
    onShown();
    return () => {
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  });

  function handleNotesClick(event: MouseEvent | KeyboardEvent) {
    if (event instanceof KeyboardEvent && event.key !== 'Enter' && event.key !== ' ') return;
    const href = (event.target as HTMLElement).closest('a')?.getAttribute('href');
    if (!href) return;
    event.preventDefault();
    void openExternalLink(href).catch((error) => {
      logError('Update', '打开更新日志链接失败', { error: String(error) });
    });
  }

  function handleDialogKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = Array.from(
      dialogElement.querySelectorAll<HTMLElement>('a[href], button:not([disabled])'),
    );
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
</script>

<svelte:window on:keydown={handleDialogKeydown} />

<div class="dialog-layer" role="presentation">
  <div
    bind:this={dialogElement}
    class="update-dialog"
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="software-update-success-title"
    on:click={handleNotesClick}
    on:keydown={handleNotesClick}
  >
    <header class="dialog-header">
      <div class="app-mark" aria-hidden="true">
        <img class="logo-light" src={nomoLogoLight} alt="" draggable="false" />
        <img class="logo-dark" src={nomoLogoDark} alt="" draggable="false" />
      </div>
      <div class="dialog-title">
        <h2 id="software-update-success-title">
          {t.softwareUpdateInstalledTitle({ version: notes.version })}
        </h2>
        <div class="release-meta"><code>v{notes.version}</code></div>
      </div>
      <button class="dialog-close" type="button" aria-label={t.close()} on:click={onClose}>
        <X size={16} />
      </button>
    </header>
    <div class="release-scroll release-notes">
      {@html notesHtml}
    </div>
    <footer class="dialog-footer">
      <div class="footer-note">{t.softwareUpdateInstalledHint()}</div>
      <div class="footer-actions">
        <button bind:this={primaryButton} class="button primary" type="button" on:click={onClose}>
          {t.softwareUpdateStartUsing()}
        </button>
      </div>
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
    overflow-wrap: anywhere;
  }

  .release-notes :global(ul),
  .release-notes :global(ol) {
    display: grid;
    gap: 10px;
    margin: 0;
    padding-left: 20px;
  }

  .release-notes :global(pre) {
    overflow-x: auto;
    white-space: pre;
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

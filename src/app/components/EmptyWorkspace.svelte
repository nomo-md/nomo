<script lang="ts">
  import { FilePlus2, FileText, FolderOpen } from '@lucide/svelte';
  import { t } from '../i18n';
  import RecentOpenPopover from './RecentOpenPopover.svelte';
  import type { RecentEntry } from '../../lib/desktop/tauriStorage';
  export let recentFiles: RecentEntry[];
  export let missingRecentPaths: Set<string>;
  export let openRecentEntry: (path: string, type: 'file' | 'folder') => void;
  export let removeRecentEntry: (path: string) => void;
  export let clearRecentEntriesList: () => void;
  export let refreshRecentFiles: () => Promise<void>;

  export let interfaceLocale: string;
  export let createNewFile: () => void;
  export let openFileDialog: () => void;
  export let openFolderDialog: () => void;
</script>

{#key interfaceLocale}
  <section
    class="empty-workspace"
    aria-label={t.noOpenDocument()}
    on:contextmenu|preventDefault
  >
    <div class="empty-workspace-panel">
      <div class="empty-workspace-mark" aria-hidden="true">
        <FileText size={24} strokeWidth={1.8} />
      </div>
      <div class="empty-workspace-copy">
        <h1>{t.noOpenDocument()}</h1>
        <p>{t.noOpenDocumentDescription()}</p>
      </div>
      <div class="empty-workspace-actions">
        <button type="button" class="primary" on:click={createNewFile}>
          <FilePlus2 size={16} />
          <span>{t.newFile()}</span>
        </button>
        <button type="button" on:click={openFileDialog}>
          <FileText size={16} />
          <span>{t.openFile()}</span>
        </button>
        <button type="button" on:click={openFolderDialog}>
          <FolderOpen size={16} />
          <span>{t.openFolder()}</span>
        </button>
        <RecentOpenPopover
          {recentFiles}
          {missingRecentPaths}
          {openRecentEntry}
          {removeRecentEntry}
          {clearRecentEntriesList}
          {refreshRecentFiles}
        />
      </div>
    </div>
  </section>
{/key}

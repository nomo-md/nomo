import { isTauriRuntime } from './tauriStorage';

export type SoftwareUpdateStatus =
  | 'idle'
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'downloading'
  | 'downloaded'
  | 'installing'
  | 'managed'
  | 'unsupported'
  | 'error';

export type SoftwareUpdateInstallationKind = 'installer' | 'portable' | 'store' | 'unsupported';
export type SoftwareUpdateAssetKind = 'windowsInstaller' | 'windowsPortable';

export interface SoftwareUpdateCandidate {
  version: string;
  date?: string;
  body?: string;
  assetKind?: SoftwareUpdateAssetKind;
  assetName: string;
  assetSize?: number;
  downloadUrl: string;
  md5: string;
}

export interface DownloadedSoftwareUpdate {
  version: string;
  assetName: string;
  filePath: string;
  md5: string;
  downloadedBytes: number;
}

export interface SoftwareUpdateCheckResult {
  supported: boolean;
  available: boolean;
  currentVersion: string;
  installationKind?: SoftwareUpdateInstallationKind;
  version?: string;
  date?: string;
  body?: string;
  candidate?: SoftwareUpdateCandidate;
  storeProductId?: string;
}

export interface SoftwareUpdateProgress {
  downloadedBytes: number;
  totalBytes?: number;
  percent?: number;
}

export interface SoftwareUpdateUiState {
  status: SoftwareUpdateStatus;
  message: string;
  version?: string;
  progress?: SoftwareUpdateProgress;
  error?: string;
}

export interface SoftwareUpdateProgressEvent extends SoftwareUpdateProgress {
  requestId: string;
}

export interface SoftwareUpdateSnapshot {
  status: SoftwareUpdateStatus;
  currentVersion: string;
  installationKind: SoftwareUpdateInstallationKind;
  version?: string;
  date?: string;
  body?: string;
  candidate?: SoftwareUpdateCandidate;
  storeProductId?: string;
  downloadedUpdate?: DownloadedSoftwareUpdate;
  progress?: SoftwareUpdateProgressEvent;
  error?: string;
  noticeWindowLabel?: string;
}

const SOFTWARE_UPDATE_STATE_EVENT = 'nomo://software-update-state';

export interface InstalledReleaseNotes {
  version: string;
  body: string;
}

export async function claimInstalledReleaseNotes(): Promise<InstalledReleaseNotes | null> {
  if (!isTauriRuntime()) return null;
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<InstalledReleaseNotes | null>('claim_installed_release_notes');
}

export async function confirmInstalledReleaseNotesShown(version: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('confirm_installed_release_notes_shown', { version });
}

export async function releaseInstalledReleaseNotes(): Promise<void> {
  if (!isTauriRuntime()) return;
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('claim_installed_release_notes', { release: true });
}

interface SoftwareUpdateRuntime {
  isDesktopRuntime: () => boolean;
  isWindowsRuntime: () => boolean;
  isInstallerRuntime: () => Promise<boolean>;
  getCurrentVersion: () => Promise<string>;
  check: (startup?: boolean) => Promise<SoftwareUpdateCheckResult>;
  download: (
    candidate: SoftwareUpdateCandidate,
    requestId: string,
  ) => Promise<DownloadedSoftwareUpdate>;
  install: (downloadedUpdate: DownloadedSoftwareUpdate) => Promise<void>;
  listenProgress: (handler: (event: SoftwareUpdateProgressEvent) => void) => Promise<() => void>;
}

export function isSoftwareUpdateSupported(runtime: Partial<SoftwareUpdateRuntime> = {}): boolean {
  return (
    (runtime.isDesktopRuntime ?? isTauriRuntime)() && (runtime.isWindowsRuntime ?? isWindows)()
  );
}

export async function isSoftwareUpdateInstallerSupported(
  runtime: Partial<SoftwareUpdateRuntime> = {},
): Promise<boolean> {
  const fullRuntime = await resolveRuntime(runtime);
  return isSoftwareUpdateSupported(fullRuntime) && (await fullRuntime.isInstallerRuntime());
}

export async function checkSoftwareUpdate(
  runtime: Partial<SoftwareUpdateRuntime> = {},
  startup = false,
): Promise<SoftwareUpdateCheckResult> {
  const fullRuntime = await resolveRuntime(runtime);
  const currentVersion = await fullRuntime.getCurrentVersion().catch(() => '');

  if (
    !isSoftwareUpdateSupported(fullRuntime) ||
    (runtime.isInstallerRuntime && !(await fullRuntime.isInstallerRuntime()))
  ) {
    return {
      supported: false,
      available: false,
      currentVersion,
    };
  }

  const result = await fullRuntime.check(startup);
  return {
    ...result,
    currentVersion: result.currentVersion || currentVersion,
  };
}

export async function downloadSoftwareUpdate(
  candidate: SoftwareUpdateCandidate,
  onProgress?: (progress: SoftwareUpdateProgress) => void,
  runtime: Partial<SoftwareUpdateRuntime> = {},
): Promise<DownloadedSoftwareUpdate> {
  const fullRuntime = await resolveRuntime(runtime);
  const requestId = createSoftwareUpdateRequestId();
  const unlisten = onProgress
    ? await fullRuntime.listenProgress((event) => {
        if (event.requestId !== requestId) {
          return;
        }
        onProgress(createSoftwareUpdateProgress(event.downloadedBytes, event.totalBytes));
      })
    : null;

  try {
    return await fullRuntime.download(candidate, requestId);
  } finally {
    unlisten?.();
  }
}

export async function installSoftwareUpdate(
  downloadedUpdate: DownloadedSoftwareUpdate,
  runtime: Partial<SoftwareUpdateRuntime> = {},
): Promise<void> {
  const fullRuntime = await resolveRuntime(runtime);
  await fullRuntime.install(downloadedUpdate);
}

export function createSoftwareUpdateProgress(
  downloadedBytes: number,
  totalBytes?: number,
): SoftwareUpdateProgress {
  const safeDownloaded = Math.max(0, downloadedBytes);
  const safeTotal = typeof totalBytes === 'number' && totalBytes > 0 ? totalBytes : undefined;

  return {
    downloadedBytes: safeDownloaded,
    totalBytes: safeTotal,
    percent: safeTotal ? Math.min(100, Math.round((safeDownloaded / safeTotal) * 100)) : undefined,
  };
}

async function getCachedSoftwareUpdate(): Promise<DownloadedSoftwareUpdate | null> {
  if (!isSoftwareUpdateSupported()) {
    return null;
  }
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<DownloadedSoftwareUpdate | null>('get_cached_software_update').catch(() => null);
}

export async function getSoftwareUpdateState(): Promise<SoftwareUpdateSnapshot> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<SoftwareUpdateSnapshot>('get_software_update_state');
}

export async function listenSoftwareUpdateState(
  handler: (state: SoftwareUpdateSnapshot) => void,
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<SoftwareUpdateSnapshot>(SOFTWARE_UPDATE_STATE_EVENT, (event) =>
    handler(event.payload),
  );
}

export function isSoftwareUpdateIntegrityFailure(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /MD5|校验失败|integrity|checksum/i.test(message);
}

async function resolveRuntime(
  runtime: Partial<SoftwareUpdateRuntime>,
): Promise<SoftwareUpdateRuntime> {
  return {
    isDesktopRuntime: runtime.isDesktopRuntime ?? isTauriRuntime,
    isWindowsRuntime: runtime.isWindowsRuntime ?? isWindows,
    isInstallerRuntime: runtime.isInstallerRuntime ?? defaultIsInstallerRuntime,
    getCurrentVersion: runtime.getCurrentVersion ?? defaultGetCurrentVersion,
    check: runtime.check ?? defaultCheck,
    download: runtime.download ?? defaultDownload,
    install: runtime.install ?? defaultInstall,
    listenProgress: runtime.listenProgress ?? defaultListenProgress,
  };
}

async function defaultGetCurrentVersion(): Promise<string> {
  const { getVersion } = await import('@tauri-apps/api/app');
  return getVersion();
}

async function defaultCheck(startup = false): Promise<SoftwareUpdateCheckResult> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<SoftwareUpdateCheckResult>('check_software_update', { startup });
}

async function defaultDownload(
  candidate: SoftwareUpdateCandidate,
  requestId: string,
): Promise<DownloadedSoftwareUpdate> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<DownloadedSoftwareUpdate>('download_software_update', {
    candidate,
    requestId,
  });
}

async function defaultInstall(downloadedUpdate: DownloadedSoftwareUpdate): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('install_software_update', { downloadedUpdate });
}

async function defaultListenProgress(
  handler: (event: SoftwareUpdateProgressEvent) => void,
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<SoftwareUpdateProgressEvent>('nomo://software-update-download-progress', (event) =>
    handler(event.payload),
  );
}

async function defaultIsInstallerRuntime(): Promise<boolean> {
  if (!isTauriRuntime()) {
    return false;
  }

  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<boolean>('is_windows_installer_installation').catch(() => false);
}

function createSoftwareUpdateRequestId(): string {
  return `update-download-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function isWindows(): boolean {
  if (typeof navigator === 'undefined') {
    return false;
  }

  const userAgentNavigator = navigator as Navigator & {
    userAgentData?: { platform?: string };
  };
  const platform =
    userAgentNavigator.userAgentData?.platform ?? navigator.platform ?? navigator.userAgent;
  return /win/i.test(platform);
}

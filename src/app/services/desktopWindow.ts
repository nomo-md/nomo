import { LogicalPosition } from '@tauri-apps/api/dpi';
import {
  getCurrentWindow,
  type BackgroundThrottlingPolicy,
} from '@tauri-apps/api/window';
import { createPerfTimer, logError, logInfo } from '../../lib/services/logger';
import { getPlatformCapabilities, isMobilePlatform } from './platform';

export type OpenTarget =
  | { kind: 'documents'; paths: string[] }
  | { kind: 'folder'; path: string };

export type OpenTargetRouteDecision =
  | { action: 'handled' }
  | { action: 'activate-current'; target: OpenTarget }
  | { action: 'open-current'; target: OpenTarget }
  | { action: 'create-window'; windowLabel: string; target: OpenTarget };

export interface WindowOpenTargetsSnapshot {
  folderPath?: string | null;
  filePaths: string[];
}

function getNewWindowChromeOptions() {
  const platformCapabilities = getPlatformCapabilities();

  if (platformCapabilities.isMac) {
    return {
      decorations: true,
      titleBarStyle: 'overlay' as const,
      trafficLightPosition: new LogicalPosition(16, 24),
      hiddenTitle: true,
      backgroundThrottling: 'disabled' as BackgroundThrottlingPolicy,
    };
  }

  if (platformCapabilities.usesCustomWindowsTitlebar) {
    return {
      decorations: false,
      shadow: true,
    };
  }

  return {
    decorations: platformCapabilities.windowDecorations,
  };
}

export async function closeAppWindow(desktopEnabled: boolean, closeToTrayEnabled = false) {
  if (!desktopEnabled) {
    return;
  }

  try {
    logInfo('DesktopWindow', closeToTrayEnabled ? '隐藏窗口到托盘' : '关闭窗口');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke(closeToTrayEnabled ? 'hide_window_to_tray' : 'close_window');
  } catch (error) {
    logError('DesktopWindow', 'Failed to close window', { error: formatError(error) });
  }
}

export async function exitApp(desktopEnabled: boolean) {
  if (!desktopEnabled) {
    return;
  }

  try {
    logInfo('DesktopWindow', '退出应用');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('exit_app');
  } catch (error) {
    logError('DesktopWindow', 'Failed to exit app', { error: formatError(error) });
  }
}

export async function createAppWindow(
  desktopEnabled: boolean,
  preparedWindowLabel?: string,
): Promise<string | undefined> {
  if (!desktopEnabled) {
    return undefined;
  }

  const { invoke } = await import('@tauri-apps/api/core');
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  const timer = createPerfTimer('DesktopWindow', '创建新窗口');
  try {
    const platformCapabilities = getPlatformCapabilities();
    logInfo('DesktopWindow', '开始创建新窗口', { preparedWindowLabel });
    const windowId = preparedWindowLabel ?? (await invoke<string>('create_new_window'));
    const appWindow = new WebviewWindow(windowId, {
      url: '/',
      title: 'Nomo',
      width: 1180,
      height: 760,
      minWidth: 920,
      minHeight: 640,
      center: true,
      visible: !platformCapabilities.usesCustomWindowsTitlebar,
      ...getNewWindowChromeOptions(),
      resizable: true,
      maximizable: true,
      minimizable: true,
      closable: true,
    });

    await new Promise<void>((resolve, reject) => {
      appWindow
        .once('tauri://created', () => {
          resolve();
        })
        .catch(reject);
      appWindow
        .once<string>('tauri://error', (event) => {
          reject(event.payload);
        })
        .catch(reject);
    });
    timer.end({ windowId });
    logInfo('DesktopWindow', '新窗口创建完成', { windowId });
    return windowId;
  } catch (error) {
    if (preparedWindowLabel) {
      await invoke('release_open_target_reservation', {
        windowLabel: preparedWindowLabel,
      }).catch(() => undefined);
    }
    timer.end({ failed: true });
    logError('DesktopWindow', 'Failed to create new window', { error: formatError(error) });
    return undefined;
  }
}

export async function prepareOpenTargetWindow(
  desktopEnabled: boolean,
  target: OpenTarget,
  createIfMissing: boolean,
): Promise<OpenTargetRouteDecision> {
  if (!desktopEnabled || isMobilePlatform()) {
    return { action: 'open-current', target };
  }
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<OpenTargetRouteDecision>('prepare_open_target_window', {
    target,
    createIfMissing,
  });
}

export async function syncWindowOpenTargets(
  desktopEnabled: boolean,
  snapshot: WindowOpenTargetsSnapshot,
): Promise<void> {
  if (!desktopEnabled || isMobilePlatform()) {
    return;
  }
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('sync_window_open_targets', { input: snapshot });
}

export async function openSettingsWindow(desktopEnabled: boolean) {
  if (!desktopEnabled) {
    return;
  }

  try {
    const timer = createPerfTimer('DesktopWindow', '打开设置窗口');
    logInfo('DesktopWindow', '打开设置窗口');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_settings_window');
    timer.end();
  } catch (error) {
    logError('DesktopWindow', 'Failed to open settings window', { error: formatError(error) });
  }
}

export async function enterMarkdownMiniMode(desktopEnabled: boolean, pinned: boolean) {
  if (!desktopEnabled) {
    return;
  }

  const timer = createPerfTimer('DesktopWindow', '主窗口切换为 Markdown 小窗');
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('enter_markdown_mini_mode', { pinned });
    timer.end();
  } catch (error) {
    timer.end({ failed: true });
    throw error;
  }
}

export async function exitMarkdownMiniMode(desktopEnabled: boolean) {
  if (!desktopEnabled) {
    return;
  }

  const timer = createPerfTimer('DesktopWindow', 'Markdown 小窗恢复为主窗口');
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('exit_markdown_mini_mode');
    timer.end();
  } catch (error) {
    timer.end({ failed: true });
    throw error;
  }
}

export async function setMarkdownMiniModePinned(desktopEnabled: boolean, pinned: boolean) {
  if (!desktopEnabled) {
    return;
  }

  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('set_markdown_mini_mode_pinned', { pinned });
}

export async function setDesktopIconTheme(
  desktopEnabled: boolean,
  theme: 'light' | 'dark',
  captionBackground: string,
) {
  if (!desktopEnabled) {
    return;
  }

  try {
    logInfo('DesktopWindow', '同步桌面图标主题', { theme });
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('set_desktop_icon_theme', { theme, captionBackground });
  } catch (error) {
    logError('DesktopWindow', 'Failed to sync desktop icon theme', { error: formatError(error) });
  }
}

export async function refreshInterfaceLanguageChrome(desktopEnabled: boolean) {
  if (!desktopEnabled) {
    return;
  }

  try {
    logInfo('DesktopWindow', '刷新界面语言 chrome');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('refresh_interface_language_chrome');
  } catch (error) {
    logError('DesktopWindow', 'Failed to refresh interface language chrome', {
      error: formatError(error),
    });
  }
}

export async function getDesktopSystemTheme(
  desktopEnabled: boolean,
): Promise<'light' | 'dark' | undefined> {
  if (!desktopEnabled) {
    return undefined;
  }

  try {
    const timer = createPerfTimer('DesktopWindow', '读取系统主题');
    const { invoke } = await import('@tauri-apps/api/core');
    const theme = await invoke<'light' | 'dark'>('get_desktop_system_theme');
    timer.end({ theme });
    return theme === 'dark' ? 'dark' : 'light';
  } catch (error) {
    logError('DesktopWindow', '读取系统主题失败', { error: formatError(error) });
    return undefined;
  }
}

export async function updateAppWindowTitle(
  desktopEnabled: boolean,
  fileName: string,
  dirty: boolean,
) {
  if (!desktopEnabled) {
    return;
  }

  const title = `${fileName}${dirty ? ' *' : ''} - Nomo`;
  // macOS 上标题被 hiddenTitle + Overlay 隐藏，无需写入 NSWindow；调用 setTitle 会使
  // AppKit 重置标题栏布局，把 trafficLightPosition 摆回默认位置（红绿灯偏上），直到
  // 下次窗口重绘才恢复。托盘菜单标题走 report_window_title，不受影响。
  if (!getPlatformCapabilities().isMac) {
    const win = getCurrentWindow();
    await win.setTitle(title).catch(() => undefined);
  }
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('report_window_title', { title }).catch(() => undefined);
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

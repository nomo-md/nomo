export type AppPlatform = 'macos' | 'windows' | 'linux' | 'unknown';
export type WindowChromeMode = 'native' | 'windows-custom';

export function isMobilePlatform(userAgent = globalThis.navigator?.userAgent ?? ''): boolean {
  return /Android|iPhone|iPad|iPod/i.test(userAgent);
}

export interface PlatformCapabilities {
  platform: AppPlatform;
  isMac: boolean;
  isWindows: boolean;
  windowChromeMode: WindowChromeMode;
  usesCustomWindowsTitlebar: boolean;
  showsInAppWindowMenu: boolean;
  windowDecorations: boolean;
}

export function detectAppPlatform(userAgent = globalThis.navigator?.userAgent ?? ''): AppPlatform {
  if (/\bMacintosh\b|\bMac OS\b|\bMac\b/i.test(userAgent)) {
    return 'macos';
  }
  if (/\bWindows\b|\bWin64\b|\bWin32\b/i.test(userAgent)) {
    return 'windows';
  }
  if (/\bLinux\b|\bX11\b/i.test(userAgent)) {
    return 'linux';
  }
  return 'unknown';
}

export function getPlatformCapabilities(
  platform: AppPlatform = detectAppPlatform(),
): PlatformCapabilities {
  const isWindows = platform === 'windows';
  const windowChromeMode: WindowChromeMode = isWindows ? 'windows-custom' : 'native';

  return {
    platform,
    isMac: platform === 'macos',
    isWindows,
    windowChromeMode,
    usesCustomWindowsTitlebar: isWindows,
    showsInAppWindowMenu: isWindows,
    windowDecorations: !isWindows,
  };
}

export const HOMEBREW_SETUP_COMMAND =
  'brew tap nomo-md/nomo https://github.com/nomo-md/nomo && brew install --cask nomo';

const MAC_MODIFIER_ORDER = ['ctrl', 'alt', 'shift', 'meta'] as const;
const MAC_SHORTCUT_GAP = '\u202F';

interface ShortcutModifiers {
  meta: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

/**
 * 把内部 `Ctrl+…` 加速键转成当前平台的菜单展示文案。
 *
 * 存储值和快捷键匹配仍用 `Ctrl` 表示 CmdOrCtrl；macOS 右键菜单应显示 ⌘，
 * 而不是 Windows 风格的 `Ctrl+X`。
 */
export function formatShortcutLabel(
  shortcut: string,
  platform: AppPlatform = detectAppPlatform(),
): string {
  const tokens = shortcut
    .split('+')
    .map((token) => token.trim())
    .filter(Boolean);
  if (tokens.length === 0) {
    return shortcut;
  }

  const modifiers: ShortcutModifiers = {
    meta: false,
    ctrl: false,
    alt: false,
    shift: false,
  };
  const keys: string[] = [];

  for (const token of tokens) {
    const normalized = token.toLowerCase();
    if (
      normalized === 'cmdorctrl' ||
      normalized === 'cmd' ||
      normalized === 'command' ||
      normalized === 'meta'
    ) {
      modifiers.meta = true;
    } else if (normalized === 'ctrl' || normalized === 'control') {
      if (platform === 'macos') {
        modifiers.meta = true;
      } else {
        modifiers.ctrl = true;
      }
    } else if (normalized === 'alt' || normalized === 'option') {
      modifiers.alt = true;
    } else if (normalized === 'shift') {
      modifiers.shift = true;
    } else {
      keys.push(token.length === 1 ? token.toUpperCase() : token);
    }
  }

  const key = keys.join('+');
  if (platform !== 'macos') {
    return [
      modifiers.ctrl ? 'Ctrl' : null,
      modifiers.alt ? 'Alt' : null,
      modifiers.shift ? 'Shift' : null,
      modifiers.meta ? 'Win' : null,
      key || null,
    ]
      .filter(Boolean)
      .join('+');
  }

  const glyphs: Record<(typeof MAC_MODIFIER_ORDER)[number], string> = {
    ctrl: '⌃',
    alt: '⌥',
    shift: '⇧',
    meta: '⌘',
  };
  return [
    ...MAC_MODIFIER_ORDER.filter((name) => modifiers[name]).map((name) => glyphs[name]),
    key || null,
  ]
    .filter(Boolean)
    .join(MAC_SHORTCUT_GAP);
}

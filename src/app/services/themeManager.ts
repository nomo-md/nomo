import type {
  AppearancePreferences,
  ColorScheme,
  EditorThemeOptions,
  ThemeColorTokens,
  ThemeMode,
  ThemeStyleProfile,
  ThemeStyleTokens,
} from '../../lib/theme/types';
import { logError } from '../../lib/services/logger';
import { getDesktopSystemTheme, setDesktopIconTheme } from './desktopWindow';
import {
  DEFAULT_COLOR_THEME_ID,
  DEFAULT_DOCUMENT_STYLE_ID,
  THEME_STYLE_TOKEN_CSS_VARIABLES,
  THEME_TOKEN_CSS_VARIABLES,
  isRegisteredDocumentStyleId,
  isRegisteredThemeId,
  themeRegistry,
} from './themeRegistry';

export const THEME_BOOT_SNAPSHOT_KEY = 'nomo.themeBootSnapshot.v2';
export const LEGACY_THEME_BOOT_SNAPSHOT_KEY = 'nomo.themeBootSnapshot.v1';
const THEME_BOOT_SNAPSHOT_SCHEMA_VERSION = 2;
const SYSTEM_THEME_CHANGED_EVENT = 'nomo://system-theme-changed';

const THEME_TRANSITION_CLASS = 'theme-transitioning';
const THEME_TRANSITION_MS = 180;
let themeTransitionTimer: number | null = null;

export interface ResolvedTheme {
  preferences: AppearancePreferences;
  effectiveScheme: ColorScheme;
  themeVersion: string;
  tokens: ThemeColorTokens;
  styleProfile: ThemeStyleProfile;
  styleTokens: ThemeStyleTokens;
  editorTheme: EditorThemeOptions;
}

export interface ThemeBootSnapshot {
  schemaVersion: 2;
  themeVersion: string;
  themeMode: ThemeMode;
  colorThemeId: string;
  documentStyleId: string;
  effectiveScheme: ColorScheme;
  tokens: ThemeColorTokens;
  styleProfile: ThemeStyleProfile;
  styleTokens: ThemeStyleTokens;
}

interface LegacyThemeBootSnapshot {
  schemaVersion: 1;
  themeVersion: string;
  themeMode: ThemeMode;
  colorThemeId: string;
  documentStyleId: string;
  effectiveScheme: ColorScheme;
  tokens: ThemeColorTokens;
}

export interface ThemeRuntimeEditor {
  updateTheme(theme: EditorThemeOptions): void;
}

/** `applyThemeRuntime` 的可选行为：过渡、系统深浅色提示、编辑器与原生图标同步。 */
export interface ApplyThemeRuntimeOptions {
  /**
   * 历史参数，运行时会被忽略。
   * 切主题必须瞬间翻转，不能再给 chrome / 正文加 160ms 分元素过渡。
   */
  transition?: boolean;
  /**
   * 解析 `themeMode === 'system'` 时使用的系统深浅色。
   * 缺失时回退到浏览器 `prefers-color-scheme`，不得在调用方为等 IPC 而阻塞 CSS 提交。
   */
  systemScheme?: ColorScheme;
  /**
   * 当前是否运行在桌面壳里。
   * 仅在需要同步 Dock / 窗口图标时传入；`undefined` 表示本窗不负责原生图标。
   */
  desktopEnabled?: boolean;
  /**
   * 需要同步 Shiki / Mermaid 的编辑器实例。
   * 设置窗没有编辑器时应传 `null` 或缺省，避免空调用。
   */
  editor?: ThemeRuntimeEditor | null;
  /**
   * 是否在本次提交里同步 Dock / 窗口图标。
   * 默认在传入 `desktopEnabled` 时为 `true`；设置窗应先广播再同步，此时传 `false`。
   */
  syncDesktopIcons?: boolean;
}

let themeEffectsId = 0;

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'system' || value === 'light' || value === 'dark';
}

export function isColorScheme(value: unknown): value is ColorScheme {
  return value === 'light' || value === 'dark';
}

export function resolveThemeMode(mode: ThemeMode, systemScheme = getBrowserSystemScheme()) {
  return mode === 'system' ? systemScheme : mode;
}

export function getBrowserSystemScheme(): ColorScheme {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return 'light';
  }
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function normalizeAppearancePreferences(
  value: Partial<AppearancePreferences> | null | undefined,
): AppearancePreferences {
  return {
    themeMode: isThemeMode(value?.themeMode) ? value.themeMode : 'system',
    colorThemeId: isRegisteredThemeId(value?.colorThemeId)
      ? value.colorThemeId
      : DEFAULT_COLOR_THEME_ID,
    documentStyleId: isRegisteredDocumentStyleId(value?.documentStyleId)
      ? value.documentStyleId
      : DEFAULT_DOCUMENT_STYLE_ID,
  };
}

export function resolveTheme(
  value: Partial<AppearancePreferences>,
  systemScheme = getBrowserSystemScheme(),
): ResolvedTheme {
  const preferences = normalizeAppearancePreferences(value);
  const effectiveScheme = resolveThemeMode(preferences.themeMode, systemScheme);
  const theme =
    themeRegistry.getTheme(preferences.colorThemeId) ??
    themeRegistry.getTheme(DEFAULT_COLOR_THEME_ID);
  if (!theme) {
    throw new Error('默认主题未注册');
  }
  const variant = theme.variants[effectiveScheme] ?? theme.variants.light;
  if (!variant) {
    throw new Error(`主题缺少可用变体：${theme.id}`);
  }

  return {
    preferences: {
      ...preferences,
      colorThemeId: theme.id,
    },
    effectiveScheme,
    themeVersion: theme.version,
    tokens: variant.tokens,
    styleProfile: theme.styleProfile,
    styleTokens: variant.styleTokens,
    editorTheme: {
      name: effectiveScheme,
      colorThemeId: theme.id,
      shikiTheme: variant.shikiTheme,
      mermaid: variant.mermaid,
    },
  };
}

/**
 * 把已解析主题写到指定根节点：dataset、CSS 变量，以及 inline `color-scheme`。
 *
 * `color-scheme` 必须和 token 同一轮写入。只改变量、晚写 `color-scheme` 时，
 * 旧的 inline 值会盖住 `:root[data-theme]`，chrome 和正文会各走一套外观。
 *
 * @param resolved 已解析的完整主题，包含 token 和有效深浅色。
 * @param options.transition 是否加上短过渡 class。运行时切主题不要开，否则会一块一块变。
 * @param options.root 写入目标；缺省为 `document.documentElement`。
 * @param options.nativeColorScheme 是否立刻写 inline `color-scheme`。默认 `true`。
 * @returns 原样返回 `resolved`，便于调用方继续写快照或广播。
 */
export function applyResolvedTheme(
  resolved: ResolvedTheme,
  options?: { transition?: boolean; root?: HTMLElement; nativeColorScheme?: boolean },
) {
  const root = options?.root ?? document.documentElement;
  startThemeTransition(root, options?.transition === true);

  root.dataset.theme = resolved.effectiveScheme;
  root.dataset.themePreference = resolved.preferences.themeMode;
  root.dataset.colorTheme = resolved.preferences.colorThemeId;
  root.dataset.themeStyle = resolved.styleProfile;
  root.dataset.documentStyle = resolved.preferences.documentStyleId;

  const documentStyle =
    themeRegistry.getDocumentStyle(resolved.preferences.documentStyleId) ??
    themeRegistry.getDocumentStyle(DEFAULT_DOCUMENT_STYLE_ID);
  root.dataset.blockStyle = documentStyle?.legacyBlockStyle ?? 'modern';

  for (const [tokenName, cssVariable] of Object.entries(THEME_TOKEN_CSS_VARIABLES)) {
    root.style.setProperty(cssVariable, resolved.tokens[tokenName as keyof ThemeColorTokens]);
  }
  for (const [tokenName, cssVariable] of Object.entries(THEME_STYLE_TOKEN_CSS_VARIABLES)) {
    root.style.setProperty(cssVariable, resolved.styleTokens[tokenName as keyof ThemeStyleTokens]);
  }
  if (options?.nativeColorScheme !== false) {
    applyNativeColorScheme(root, resolved.effectiveScheme);
  }
  return resolved;
}

/**
 * 写入会触发 AppKit / WKWebView 原生外观重建的 inline `color-scheme`。
 *
 * @param root 主题根节点，通常是 `document.documentElement`。
 * @param scheme 要声明的有效深浅色。
 */
function applyNativeColorScheme(root: HTMLElement, scheme: ColorScheme) {
  root.style.colorScheme = scheme;
}

/**
 * 取消尚未执行的主题收尾（编辑器高亮、Dock 图标）。
 *
 * 测试在用例之间调用，避免上一例的微任务泄漏到下一例。运行时由
 * `scheduleThemeEffects` 在新提交时自动作废旧任务。
 */
export function cancelPendingThemeEffects() {
  themeEffectsId += 1;
}

/**
 * 在本轮提交结束后启动编辑器高亮和图标更新，同一轮只应用最新主题。
 *
 * WKWebView 可以推迟失焦窗口的 rAF；主题正确性不能依赖窗口是否绘制。
 * 微任务让调用方先发出跨窗广播，又不额外等待两帧。高亮和图表内部仍负责
 * 丢弃已经启动但过期的异步渲染结果。
 *
 * @param task 本轮 CSS 提交后的收尾，不得回写旧的 CSS token。
 */
function scheduleThemeEffects(task: () => void) {
  const id = ++themeEffectsId;
  queueMicrotask(() => {
    if (id !== themeEffectsId) {
      return;
    }
    task();
  });
}

/**
 * 把偏好解析成已解析主题，并一次性写入可见外观。
 *
 * dataset、CSS 变量和 inline `color-scheme` 同步提交，避免本窗 CSS 分阶段换色。
 * 不启用颜色过渡，避免 160ms 分元素动画造成「先变一部分」。
 * Shiki / Mermaid 和 Dock 图标在本轮微任务启动，不等待失焦窗口的绘制回调。
 *
 * @param preferences 本次要落地的外观偏好；允许缺字段，缺省走注册表默认主题。
 * @param options 系统深浅色提示、编辑器实例及是否同步原生图标。
 *   `transition` 会被忽略，运行时切主题始终瞬间切换。
 * @returns 本次写入根节点的已解析主题，可供调用方写快照或广播 `effectiveScheme`。
 */
export function applyThemeRuntime(
  preferences: Partial<AppearancePreferences>,
  options?: ApplyThemeRuntimeOptions,
) {
  const resolved = resolveTheme(preferences, options?.systemScheme);
  applyResolvedTheme(resolved, {
    transition: false,
    nativeColorScheme: true,
  });
  const shouldSyncDesktopIcons = options?.syncDesktopIcons ?? options?.desktopEnabled !== undefined;
  const followUp = () => {
    options?.editor?.updateTheme(resolved.editorTheme);
    if (shouldSyncDesktopIcons) {
      syncDesktopThemeChrome(options?.desktopEnabled, resolved);
    }
  };
  scheduleThemeEffects(followUp);
  return resolved;
}

/**
 * 把已解析主题的 Dock / 窗口图标同步到桌面壳，不回写 CSS。
 *
 * 调用方应在跨窗事件发出之后再调，避免 `set_desktop_icon_theme` 占用 IPC / AppKit
 * 主线程，拖慢主窗收到 `nomo://settings-updated`。
 *
 * @param desktopEnabled 是否处于 Tauri 桌面运行时；`undefined` 或 `false` 时为空操作。
 * @param resolved 已经写入前端的主题；只使用其中的有效深浅色和标题栏背景。
 */
function syncDesktopThemeChrome(
  desktopEnabled: boolean | undefined,
  resolved: ResolvedTheme,
) {
  if (!desktopEnabled) {
    return;
  }
  // 原生图标同步不参与前端主题提交，避免 IPC 延迟让旧请求覆盖较新的主题状态。
  void setDesktopIconTheme(
    desktopEnabled,
    resolved.effectiveScheme,
    resolved.tokens.titlebarBackground,
  );
}

/**
 * 读取桌面壳报告的系统深浅色；桌面 IPC 失败时回退到浏览器媒体查询。
 *
 * 这条路径可能触发原生查询，不能放在 CSS 提交之前。外观切换应先用
 * `getBrowserSystemScheme()` 或事件里带来的 `effectiveScheme` 上色，再视需要校正。
 *
 * @param desktopEnabled 是否处于 Tauri 桌面运行时；为 `false` 时只读浏览器方案。
 * @returns 最终采用的 `light` 或 `dark`，桌面与浏览器都不可用时为浏览器默认浅色。
 */
export async function readEffectiveSystemScheme(desktopEnabled: boolean) {
  return (
    (await getDesktopSystemTheme(desktopEnabled).catch(() => null)) ?? getBrowserSystemScheme()
  );
}

export function writeThemeBootSnapshot(resolved: ResolvedTheme) {
  if (typeof localStorage === 'undefined') {
    return;
  }
  localStorage.setItem(THEME_BOOT_SNAPSHOT_KEY, JSON.stringify(createThemeBootSnapshot(resolved)));
  localStorage.removeItem(LEGACY_THEME_BOOT_SNAPSHOT_KEY);
}

function createThemeBootSnapshot(resolved: ResolvedTheme): ThemeBootSnapshot {
  return {
    schemaVersion: THEME_BOOT_SNAPSHOT_SCHEMA_VERSION,
    themeVersion: resolved.themeVersion,
    themeMode: resolved.preferences.themeMode,
    colorThemeId: resolved.preferences.colorThemeId,
    documentStyleId: resolved.preferences.documentStyleId,
    effectiveScheme: resolved.effectiveScheme,
    tokens: resolved.tokens,
    styleProfile: resolved.styleProfile,
    styleTokens: resolved.styleTokens,
  };
}

export function readThemeBootSnapshot(): ThemeBootSnapshot | null {
  if (typeof localStorage === 'undefined') {
    return null;
  }
  const currentSnapshot = parseThemeBootSnapshot(localStorage.getItem(THEME_BOOT_SNAPSHOT_KEY));
  if (currentSnapshot && isValidThemeBootSnapshot(currentSnapshot)) {
    return currentSnapshot as ThemeBootSnapshot;
  }

  return readLegacyThemeBootSnapshot();
}

function parseThemeBootSnapshot(value: string | null): Partial<ThemeBootSnapshot> | null {
  if (!value) {
    return null;
  }
  try {
    return JSON.parse(value) as Partial<ThemeBootSnapshot>;
  } catch {
    return null;
  }
}

function readLegacyThemeBootSnapshot(): ThemeBootSnapshot | null {
  const value = localStorage.getItem(LEGACY_THEME_BOOT_SNAPSHOT_KEY);
  if (!value) {
    return null;
  }
  try {
    const snapshot = JSON.parse(value) as Partial<LegacyThemeBootSnapshot>;
    if (!isValidLegacyThemeBootSnapshot(snapshot)) {
      return null;
    }
    const resolved = resolveTheme(snapshot, snapshot.effectiveScheme);
    return createThemeBootSnapshot(resolved);
  } catch {
    return null;
  }
}

export function bootstrapThemeFromSnapshot() {
  const snapshot = readThemeBootSnapshot();
  if (snapshot) {
    const resolved = resolveTheme(snapshot, snapshot.effectiveScheme);
    if (
      resolved.themeVersion === snapshot.themeVersion &&
      tokensMatch(resolved.tokens, snapshot.tokens) &&
      resolved.styleProfile === snapshot.styleProfile &&
      tokensMatch(resolved.styleTokens, snapshot.styleTokens)
    ) {
      return applyResolvedTheme(resolved);
    }
  }

  const fallback = resolveTheme(
    {
      themeMode: 'system',
      colorThemeId: DEFAULT_COLOR_THEME_ID,
      documentStyleId: DEFAULT_DOCUMENT_STYLE_ID,
    },
    readThemeBootSchemeHint() ?? getBrowserSystemScheme(),
  );
  return applyResolvedTheme(fallback);
}

function readThemeBootSchemeHint(): ColorScheme | null {
  if (typeof localStorage === 'undefined') {
    return null;
  }
  for (const key of [THEME_BOOT_SNAPSHOT_KEY, LEGACY_THEME_BOOT_SNAPSHOT_KEY]) {
    try {
      const value = JSON.parse(localStorage.getItem(key) ?? 'null') as {
        effectiveScheme?: unknown;
      } | null;
      if (isColorScheme(value?.effectiveScheme)) {
        return value.effectiveScheme;
      }
    } catch {
      // 继续读取另一版本快照。
    }
  }
  return null;
}

/**
 * 监听系统深浅色变化，并在变化时调用 `sync`。
 *
 * 桌面端同时订阅原生 `nomo://system-theme-changed` 和当前窗口 `onThemeChanged`，
 * 避免只靠后台 WKWebView 的 `matchMedia` 时事件被节流到 1～2 秒才到达。
 * 连续触发会合并成一次 in-flight 同步，后到的请求会再跑一轮以免丢掉最后状态。
 *
 * @param sync 系统深浅色可能已变时调用。若事件已带上 `light`/`dark`，参数为该值；
 *   仅有媒体查询/焦点变化时参数为空，调用方应先用 `getBrowserSystemScheme()` 上色。
 * @returns 取消全部监听的函数；重复调用安全。
 */
export function listenForSystemThemeChanges(
  sync: (scheme?: ColorScheme) => void | Promise<void>,
) {
  if (typeof window === 'undefined') {
    return () => undefined;
  }
  const mediaQuery = window.matchMedia?.('(prefers-color-scheme: dark)');
  let disposed = false;
  let syncRunning = false;
  let syncPending = false;
  let pendingScheme: ColorScheme | undefined;
  const unlisteners: Array<() => void> = [];
  const runSync = (scheme?: ColorScheme) => {
    if (disposed) return;
    if (scheme) {
      pendingScheme = scheme;
    }
    if (syncRunning) {
      syncPending = true;
      return;
    }

    syncRunning = true;
    void (async () => {
      try {
        do {
          syncPending = false;
          const nextScheme = pendingScheme;
          pendingScheme = undefined;
          try {
            await sync(nextScheme);
          } catch (error) {
            logError('ThemeManager', '同步系统主题失败', {
              error: error instanceof Error ? error.message : String(error),
            });
          }
        } while (syncPending && !disposed);
      } finally {
        syncRunning = false;
      }
    })();
  };
  const handleVisibility = () => {
    if (document.visibilityState === 'visible') {
      runSync();
    }
  };

  const handleMediaChange = () => {
    runSync(getBrowserSystemScheme());
  };
  const handleFocus = () => {
    runSync();
  };
  mediaQuery?.addEventListener?.('change', handleMediaChange);
  window.addEventListener('focus', handleFocus);
  document.addEventListener('visibilitychange', handleVisibility);
  bindDesktopSystemThemeListeners(
    (scheme) => {
      runSync(scheme);
    },
    (unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      unlisteners.push(unlisten);
    },
  );

  return () => {
    disposed = true;
    syncPending = false;
    mediaQuery?.removeEventListener?.('change', handleMediaChange);
    window.removeEventListener('focus', handleFocus);
    document.removeEventListener('visibilitychange', handleVisibility);
    for (const unlisten of unlisteners) {
      unlisten();
    }
    unlisteners.length = 0;
  };
}

/**
 * 订阅桌面壳广播的系统主题事件，以及当前窗口的原生 `onThemeChanged`。
 *
 * @param onScheme 收到明确深浅色时回调。
 * @param onReady 每成功挂上一个监听后回调，便于外层在 dispose 后立刻拆掉。
 */
function bindDesktopSystemThemeListeners(
  onScheme: (scheme: ColorScheme) => void,
  onReady: (unlisten: () => void) => void,
) {
  void import('@tauri-apps/api/event')
    .then(({ listen }) =>
      listen<unknown>(SYSTEM_THEME_CHANGED_EVENT, (event) => {
        if (isColorScheme(event.payload)) {
          onScheme(event.payload);
        }
      }),
    )
    .then(onReady)
    .catch(() => undefined);

  void import('@tauri-apps/api/window')
    .then(({ getCurrentWindow }) =>
      getCurrentWindow().onThemeChanged((event) => {
        onScheme(event.payload === 'dark' ? 'dark' : 'light');
      }),
    )
    .then(onReady)
    .catch(() => undefined);
}

function isValidThemeBootSnapshot(snapshot: Partial<ThemeBootSnapshot>) {
  if (
    snapshot.schemaVersion !== THEME_BOOT_SNAPSHOT_SCHEMA_VERSION ||
    !snapshot.themeVersion ||
    !isThemeMode(snapshot.themeMode) ||
    !isRegisteredThemeId(snapshot.colorThemeId) ||
    !isRegisteredDocumentStyleId(snapshot.documentStyleId) ||
    !isColorScheme(snapshot.effectiveScheme) ||
    !snapshot.tokens ||
    !isThemeStyleProfile(snapshot.styleProfile) ||
    !snapshot.styleTokens
  ) {
    return false;
  }
  const resolved = resolveTheme(snapshot, snapshot.effectiveScheme);
  return (
    resolved.themeVersion === snapshot.themeVersion &&
    tokensMatch(resolved.tokens, snapshot.tokens) &&
    resolved.styleProfile === snapshot.styleProfile &&
    tokensMatch(resolved.styleTokens, snapshot.styleTokens)
  );
}

function isValidLegacyThemeBootSnapshot(
  snapshot: Partial<LegacyThemeBootSnapshot>,
): snapshot is LegacyThemeBootSnapshot {
  if (
    snapshot.schemaVersion !== 1 ||
    !snapshot.themeVersion ||
    !isThemeMode(snapshot.themeMode) ||
    !isRegisteredThemeId(snapshot.colorThemeId) ||
    !isRegisteredDocumentStyleId(snapshot.documentStyleId) ||
    !isColorScheme(snapshot.effectiveScheme) ||
    !snapshot.tokens
  ) {
    return false;
  }
  const resolved = resolveTheme(snapshot, snapshot.effectiveScheme);
  return (
    resolved.themeVersion === snapshot.themeVersion && tokensMatch(resolved.tokens, snapshot.tokens)
  );
}

function isThemeStyleProfile(value: unknown): value is ThemeStyleProfile {
  return value === 'modern' || value === 'paper' || value === 'classic';
}

function tokensMatch<T extends Record<string, string>>(expected: T, actual: T) {
  const expectedEntries = Object.entries(expected);
  const actualKeys = Object.keys(actual);
  return (
    expectedEntries.length === actualKeys.length &&
    expectedEntries.every(([key, value]) => actual[key] === value)
  );
}

function startThemeTransition(root: HTMLElement, enabled: boolean) {
  if (!enabled || prefersReducedMotion() || typeof window === 'undefined') {
    return;
  }
  root.classList.add(THEME_TRANSITION_CLASS);
  if (themeTransitionTimer !== null) {
    window.clearTimeout(themeTransitionTimer);
  }
  themeTransitionTimer = window.setTimeout(() => {
    root.classList.remove(THEME_TRANSITION_CLASS);
    themeTransitionTimer = null;
  }, THEME_TRANSITION_MS + 40);
}

function prefersReducedMotion() {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

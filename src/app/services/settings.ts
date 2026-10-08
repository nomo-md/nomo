import {
  listAppSettings,
  updateAppSetting,
  updateAppSettings,
  type SettingRecord,
} from '../../lib/desktop/tauriStorage';
import type { DiagramType } from '../../lib/editor-core/diagramTemplates';
import { setTypographyOptions } from '../../lib/typography/options';
import { DEFAULT_TYPOGRAPHY } from '../../lib/typography/types';
import type { MarkdownLintRuleSet } from '../../lib/markdown-lint/types';
import type { AppearancePreferences, ColorScheme, ThemeMode } from '../../lib/theme/types';
import {
  DEFAULT_IMAGE_HANDLING_SETTINGS,
  type ImageDefaultAlign,
  type ImageHandlingSettings,
  type ImageInsertStrategy,
  type ImageUploadProvider,
} from '../../lib/services/render';
import { logDebug, logWarn, perfAsync } from '../../lib/services/logger';
import {
  DEFAULT_INTERFACE_LANGUAGE,
  isInterfaceLanguagePreference,
  type InterfaceLanguagePreference,
} from '../i18n';
import type { EditorViewMode, SplitViewLayout } from '../types';
import {
  CLASSIC_DOCUMENT_STYLE_ID,
  DEFAULT_COLOR_THEME_ID,
  DEFAULT_DOCUMENT_STYLE_ID,
  isRegisteredDocumentStyleId,
  isRegisteredThemeId,
} from './themeRegistry';

export type { AppearancePreferences, ColorScheme, ThemeMode };
export type EditorModePreference = EditorViewMode;
export type SplitViewLayoutPreference = SplitViewLayout;
export type OpenDefaultBehavior = 'current-window' | 'new-window' | 'ask-every-time';
export type CloseWindowBehavior = 'ask-every-time' | 'close-window' | 'close-to-tray';
export type ExternalFileChangeBehavior = 'reload-external' | 'overwrite-external' | 'ignore';
export type WritingStatsMetric = 'lines' | 'words' | 'visibleChars' | 'chars';
export type ImageDefaultAlignPreference = ImageDefaultAlign;
export type CodeBlockIndentPreference = 'spaces-2' | 'spaces-4' | 'tab';
export type RenderModePreference = 'hardware' | 'software';
export type { MarkdownLintRuleSet };
export type { InterfaceLanguagePreference };

export type ShortcutCommandId =
  | 'new-file'
  | 'open-file'
  | 'save-file'
  | 'export-html'
  | 'export-pdf'
  | 'toggle-source'
  | 'toggle-theme'
  | 'toggle-focus'
  | 'toggle-toolbar'
  | 'toggle-markdown-mini'
  | 'insert-code-block'
  | 'insert-table'
  | 'insert-math-block'
  | 'menu-link'
  | 'menu-clear-format';

export type ShortcutPreferences = Record<ShortcutCommandId, string>;

export interface PersistedEditorSettings {
  fontSize?: number;
  lineHeight?: number;
  contentWidthPercent?: number;
}

export interface AppPreferences {
  themeMode: ThemeMode;
  colorThemeId: string;
  documentStyleId: string;
  interfaceLanguage: InterfaceLanguagePreference;
  editorMode: EditorModePreference;
  splitViewLayout: SplitViewLayoutPreference;
  splitLeftPercent: number;
  autoSaveEnabled: boolean;
  autoSaveDelayMs: number;
  createSnapshotBeforeSave: boolean;
  fontSize: number;
  lineHeight: number;
  typographyEnabled: boolean;
  contentWidthPercent: number;
  largeDocumentLimit: number;
  openDefaultBehavior: OpenDefaultBehavior;
  filePreviewEnabled: boolean;
  closeWindowBehavior: CloseWindowBehavior;
  externalFileChangeBehavior: ExternalFileChangeBehavior;
  sidebarHidden: boolean;
  toolbarHidden: boolean;
  outlineVisible: boolean;
  writingStatsVisible: boolean;
  writingStatsMetric: WritingStatsMetric;
  readingTimeVisible: boolean;
  defaultCodeBlockLanguage: string;
  defaultDiagramType: DiagramType;
  zoomPercent: number;
  ctrlWheelZoomEnabled: boolean;
  outlineDefaultExpandLevel: number;
  codeBlockLineNumbersVisible: boolean;
  codeBlockIndent: CodeBlockIndentPreference;
  inlineCodeRenderingEnabled: boolean;
  copyMarkdownSyntaxEnabled: boolean;
  markdownLintEnabled: boolean;
  markdownLintRuleSet: MarkdownLintRuleSet;
  renderMode: RenderModePreference;
  shortcutPreferences: ShortcutPreferences;
  imageHandlingSettings: ImageHandlingSettings;
  developerMode: boolean;
  softwareUpdateAutoCheckEnabled: boolean;
}

export type AppPreferenceKey = keyof AppPreferences;
export type AppPreferencesPatch = Partial<AppPreferences>;

export interface SettingsUpdatedPayload {
  /**
   * 固定为设置窗，主窗只接受这个来源，避免把其它广播当成偏好补丁。
   */
  source: 'settings-window';
  /**
   * 设置窗刚确认的偏好补丁。
   * 缺省时主窗会整表重载；外观三项变更应带上 `effectiveScheme` 以免再查系统主题。
   */
  patch?: AppPreferencesPatch;
  /**
   * 设置窗已经解析好的有效深浅色。
   * 主窗应直接用来上色，不要再 `await` 桌面 IPC。
   */
  effectiveScheme?: 'light' | 'dark';
}

export const DEFAULT_SHORTCUT_PREFERENCES: ShortcutPreferences = {
  'new-file': 'Ctrl+N',
  'open-file': 'Ctrl+O',
  'save-file': 'Ctrl+S',
  'export-html': 'Ctrl+Shift+E',
  'export-pdf': 'Ctrl+Shift+P',
  'toggle-source': 'Ctrl+E',
  'toggle-theme': 'Ctrl+Shift+L',
  'toggle-focus': 'Ctrl+Shift+F',
  'toggle-toolbar': 'Ctrl+Shift+B',
  'toggle-markdown-mini': 'Ctrl+Alt+M',
  'insert-code-block': 'Ctrl+Shift+K',
  'insert-table': 'Ctrl+Shift+T',
  'insert-math-block': 'Ctrl+Shift+M',
  'menu-link': 'Ctrl+K',
  'menu-clear-format': 'Ctrl+\\',
};

export const DEFAULT_APP_PREFERENCES: AppPreferences = {
  themeMode: 'system',
  colorThemeId: DEFAULT_COLOR_THEME_ID,
  documentStyleId: DEFAULT_DOCUMENT_STYLE_ID,
  interfaceLanguage: DEFAULT_INTERFACE_LANGUAGE,
  editorMode: 'semantic',
  splitViewLayout: 'semantic-source',
  splitLeftPercent: 50,
  autoSaveEnabled: false,
  autoSaveDelayMs: 1000,
  createSnapshotBeforeSave: true,
  fontSize: 16,
  lineHeight: 1.75,
  typographyEnabled: DEFAULT_TYPOGRAPHY.enabled,
  contentWidthPercent: 60,
  largeDocumentLimit: 500_000,
  openDefaultBehavior: 'ask-every-time',
  filePreviewEnabled: true,
  closeWindowBehavior: 'ask-every-time',
  externalFileChangeBehavior: 'reload-external',
  sidebarHidden: false,
  toolbarHidden: false,
  outlineVisible: true,
  writingStatsVisible: true,
  writingStatsMetric: 'words',
  readingTimeVisible: false,
  defaultCodeBlockLanguage: 'ts',
  defaultDiagramType: 'flowchart',
  zoomPercent: 100,
  ctrlWheelZoomEnabled: true,
  outlineDefaultExpandLevel: 2,
  codeBlockLineNumbersVisible: true,
  codeBlockIndent: 'spaces-2',
  inlineCodeRenderingEnabled: true,
  copyMarkdownSyntaxEnabled: true,
  markdownLintEnabled: false,
  markdownLintRuleSet: 'relaxed',
  renderMode: 'hardware',
  shortcutPreferences: { ...DEFAULT_SHORTCUT_PREFERENCES },
  imageHandlingSettings: { ...DEFAULT_IMAGE_HANDLING_SETTINGS },
  developerMode: false,
  softwareUpdateAutoCheckEnabled: true,
};

export const SETTINGS_UPDATED_EVENT = 'nomo://settings-updated';

const LEGACY_CLOSE_TO_TRAY_PROMPT_ANSWERED_KEY = 'closeToTrayPromptAnswered';
const APPEARANCE_THEME_MODEL_MIGRATION_KEY = 'appearanceThemeModelV1';
const ZOOM_TRANSITION_MS = 150;
const ZOOM_REDUCED_TRANSITION_MS = 90;
let zoomAnimationFrame: number | null = null;

export async function loadPersistedEditorSettings(
  desktopEnabled: boolean,
  nativeSettings?: SettingRecord[],
): Promise<PersistedEditorSettings> {
  const settings = await perfAsync('settings', 'readNativeSettingsMap(editor)', () =>
    readNativeSettingsMap(desktopEnabled, nativeSettings),
  );
  const savedFontSize = Number(parseSetting<number>(settings, 'fontSize'));
  const savedLineHeight = Number(parseSetting<number>(settings, 'lineHeight'));
  const savedContentWidthPercent = Number(parseSetting<number>(settings, 'contentWidthPercent'));

  return {
    fontSize:
      Number.isFinite(savedFontSize) && savedFontSize >= 14 && savedFontSize <= 22
        ? savedFontSize
        : undefined,
    lineHeight:
      Number.isFinite(savedLineHeight) && savedLineHeight >= 1.4 && savedLineHeight <= 2.1
        ? savedLineHeight
        : undefined,
    contentWidthPercent:
      Number.isFinite(savedContentWidthPercent) &&
      savedContentWidthPercent >= 45 &&
      savedContentWidthPercent <= 90
        ? savedContentWidthPercent
        : undefined,
  };
}

export function persistEditorSetting(desktopEnabled: boolean, key: string, value: unknown) {
  if (!desktopEnabled) {
    return;
  }
  updateAppSetting(key, value).catch(() => undefined);
}

export async function loadPersistedImageSettings(
  desktopEnabled: boolean,
  nativeSettings?: SettingRecord[],
): Promise<ImageHandlingSettings> {
  const settings = await readNativeSettingsMap(desktopEnabled, nativeSettings);
  const saved = parseSetting<Partial<ImageHandlingSettings>>(settings, 'imageHandlingSettings');

  return normalizeImageSettings(saved);
}

function persistImageSettings(desktopEnabled: boolean, settings: ImageHandlingSettings) {
  const normalized = normalizeImageSettings(settings);
  if (!desktopEnabled) {
    return;
  }
  updateAppSetting('imageHandlingSettings', normalized).catch(() => undefined);
}

export async function loadAppPreferences(
  desktopEnabled: boolean,
  nativeSettings?: SettingRecord[],
): Promise<AppPreferences> {
  const settings = await perfAsync('settings', 'readNativeSettingsMap(prefs)', () =>
    readNativeSettingsMap(desktopEnabled, nativeSettings),
  );
  const appearance = await migrateAppearancePreferences(desktopEnabled, settings);

  return normalizeAppPreferences({
    ...appearance,
    interfaceLanguage: parseSetting<unknown>(settings, 'interfaceLanguage'),
    editorMode: parseSetting<unknown>(settings, 'editorMode'),
    splitViewLayout: parseSetting<unknown>(settings, 'splitViewLayout'),
    splitLeftPercent: parseSetting<unknown>(settings, 'splitLeftPercent'),
    autoSaveEnabled: parseSetting<unknown>(settings, 'autoSaveEnabled'),
    autoSaveDelayMs: parseSetting<unknown>(settings, 'autoSaveDelayMs'),
    createSnapshotBeforeSave: parseSetting<unknown>(settings, 'createSnapshotBeforeSave'),
    fontSize: parseSetting<unknown>(settings, 'fontSize'),
    lineHeight: parseSetting<unknown>(settings, 'lineHeight'),
    typographyEnabled: parseSetting<unknown>(settings, 'typographyEnabled'),
    contentWidthPercent: parseSetting<unknown>(settings, 'contentWidthPercent'),
    largeDocumentLimit: parseSetting<unknown>(settings, 'largeDocumentLimit'),
    openDefaultBehavior:
      parseSetting<unknown>(settings, 'openDefaultBehavior') ??
      parseSetting<unknown>(settings, 'folderOpenDefaultBehavior'),
    filePreviewEnabled: parseSetting<unknown>(settings, 'filePreviewEnabled'),
    closeWindowBehavior:
      parseSetting<unknown>(settings, 'closeWindowBehavior') ??
      resolveLegacyCloseWindowBehavior(settings),
    externalFileChangeBehavior: parseSetting<unknown>(settings, 'externalFileChangeBehavior'),
    sidebarHidden: parseSetting<unknown>(settings, 'sidebarHidden'),
    toolbarHidden: parseSetting<unknown>(settings, 'toolbarHidden'),
    outlineVisible: parseSetting<unknown>(settings, 'outlineVisible'),
    writingStatsVisible: parseSetting<unknown>(settings, 'writingStatsVisible'),
    writingStatsMetric: parseSetting<unknown>(settings, 'writingStatsMetric'),
    readingTimeVisible: parseSetting<unknown>(settings, 'readingTimeVisible'),
    defaultCodeBlockLanguage: parseSetting<unknown>(settings, 'defaultCodeBlockLanguage'),
    defaultDiagramType: parseSetting<unknown>(settings, 'defaultDiagramType'),
    zoomPercent: parseSetting<unknown>(settings, 'zoomPercent'),
    ctrlWheelZoomEnabled: parseSetting<unknown>(settings, 'ctrlWheelZoomEnabled'),
    outlineDefaultExpandLevel: parseSetting<unknown>(settings, 'outlineDefaultExpandLevel'),
    codeBlockLineNumbersVisible: parseSetting<unknown>(settings, 'codeBlockLineNumbersVisible'),
    codeBlockIndent: parseSetting<unknown>(settings, 'codeBlockIndent'),
    inlineCodeRenderingEnabled: parseSetting<unknown>(settings, 'inlineCodeRenderingEnabled'),
    copyMarkdownSyntaxEnabled: parseSetting<unknown>(settings, 'copyMarkdownSyntaxEnabled'),
    markdownLintEnabled: parseSetting<unknown>(settings, 'markdownLintEnabled'),
    markdownLintRuleSet: parseSetting<unknown>(settings, 'markdownLintRuleSet'),
    renderMode: parseSetting<unknown>(settings, 'renderMode'),
    shortcutPreferences: parseSetting<unknown>(settings, 'shortcutPreferences'),
    imageHandlingSettings: parseSetting<Partial<ImageHandlingSettings>>(
      settings,
      'imageHandlingSettings',
    ),
    developerMode: parseSetting<unknown>(settings, 'developerMode'),
    softwareUpdateAutoCheckEnabled: parseSetting<unknown>(
      settings,
      'softwareUpdateAutoCheckEnabled',
    ),
  });
}

async function readNativeSettingsMap(
  desktopEnabled: boolean,
  nativeSettings?: SettingRecord[],
): Promise<Map<string, string>> {
  const start = performance.now();
  const settingsRows =
    nativeSettings ?? (desktopEnabled ? await listAppSettings().catch(() => []) : []);
  logDebug('settings', `readNativeSettingsMap: 读取到 ${settingsRows.length} 条设置`, {
    desktopEnabled,
    hasNativeSettings: !!nativeSettings,
    elapsedMs: Math.round(performance.now() - start),
  });
  return new Map(settingsRows.map((setting) => [setting.key, setting.valueJson]));
}

export async function saveAppPreferences(
  desktopEnabled: boolean,
  preferences: AppPreferences,
  keys?: AppPreferenceKey[],
) {
  const normalized = normalizeAppPreferences(preferences);

  markAppearanceMigrationDone();

  if (!desktopEnabled) {
    return normalized;
  }

  const persistedEntries = {
    ...pickPersistedPreferenceEntries(normalized, keys),
    ...(keys ? {} : { [APPEARANCE_THEME_MODEL_MIGRATION_KEY]: true }),
  };
  await updateAppSettings(persistedEntries);

  return normalized;
}

export function normalizeAppPreferences(
  value: Partial<Record<keyof AppPreferences, unknown>>,
): AppPreferences {
  return {
    themeMode: isThemeMode(value.themeMode) ? value.themeMode : DEFAULT_APP_PREFERENCES.themeMode,
    colorThemeId: isRegisteredThemeId(value.colorThemeId)
      ? value.colorThemeId
      : DEFAULT_APP_PREFERENCES.colorThemeId,
    documentStyleId: isRegisteredDocumentStyleId(value.documentStyleId)
      ? value.documentStyleId
      : DEFAULT_APP_PREFERENCES.documentStyleId,
    interfaceLanguage: isInterfaceLanguagePreference(value.interfaceLanguage)
      ? value.interfaceLanguage
      : DEFAULT_APP_PREFERENCES.interfaceLanguage,
    editorMode: isEditorModePreference(value.editorMode)
      ? value.editorMode
      : DEFAULT_APP_PREFERENCES.editorMode,
    splitViewLayout: isSplitViewLayoutPreference(value.splitViewLayout)
      ? value.splitViewLayout
      : DEFAULT_APP_PREFERENCES.splitViewLayout,
    splitLeftPercent: clampNumber(
      value.splitLeftPercent,
      25,
      75,
      DEFAULT_APP_PREFERENCES.splitLeftPercent,
    ),
    autoSaveEnabled:
      typeof value.autoSaveEnabled === 'boolean'
        ? value.autoSaveEnabled
        : DEFAULT_APP_PREFERENCES.autoSaveEnabled,
    autoSaveDelayMs: clampNumber(
      value.autoSaveDelayMs,
      500,
      5000,
      DEFAULT_APP_PREFERENCES.autoSaveDelayMs,
    ),
    createSnapshotBeforeSave:
      typeof value.createSnapshotBeforeSave === 'boolean'
        ? value.createSnapshotBeforeSave
        : DEFAULT_APP_PREFERENCES.createSnapshotBeforeSave,
    fontSize: clampNumber(value.fontSize, 14, 22, DEFAULT_APP_PREFERENCES.fontSize),
    lineHeight: clampNumber(value.lineHeight, 1.4, 2.1, DEFAULT_APP_PREFERENCES.lineHeight),
    typographyEnabled:
      typeof value.typographyEnabled === 'boolean'
        ? value.typographyEnabled
        : DEFAULT_TYPOGRAPHY.enabled,
    contentWidthPercent: clampNumber(
      value.contentWidthPercent,
      45,
      90,
      DEFAULT_APP_PREFERENCES.contentWidthPercent,
    ),
    largeDocumentLimit: clampNumber(
      value.largeDocumentLimit,
      100_000,
      1_000_000,
      DEFAULT_APP_PREFERENCES.largeDocumentLimit,
    ),
    openDefaultBehavior: isOpenDefaultBehavior(value.openDefaultBehavior)
      ? value.openDefaultBehavior
      : DEFAULT_APP_PREFERENCES.openDefaultBehavior,
    filePreviewEnabled:
      typeof value.filePreviewEnabled === 'boolean'
        ? value.filePreviewEnabled
        : DEFAULT_APP_PREFERENCES.filePreviewEnabled,
    closeWindowBehavior: isCloseWindowBehavior(value.closeWindowBehavior)
      ? value.closeWindowBehavior
      : DEFAULT_APP_PREFERENCES.closeWindowBehavior,
    externalFileChangeBehavior: isExternalFileChangeBehavior(value.externalFileChangeBehavior)
      ? value.externalFileChangeBehavior
      : DEFAULT_APP_PREFERENCES.externalFileChangeBehavior,
    sidebarHidden:
      typeof value.sidebarHidden === 'boolean'
        ? value.sidebarHidden
        : DEFAULT_APP_PREFERENCES.sidebarHidden,
    toolbarHidden:
      typeof value.toolbarHidden === 'boolean'
        ? value.toolbarHidden
        : DEFAULT_APP_PREFERENCES.toolbarHidden,
    outlineVisible:
      typeof value.outlineVisible === 'boolean'
        ? value.outlineVisible
        : DEFAULT_APP_PREFERENCES.outlineVisible,
    writingStatsVisible:
      typeof value.writingStatsVisible === 'boolean'
        ? value.writingStatsVisible
        : DEFAULT_APP_PREFERENCES.writingStatsVisible,
    writingStatsMetric: isWritingStatsMetric(value.writingStatsMetric)
      ? value.writingStatsMetric
      : DEFAULT_APP_PREFERENCES.writingStatsMetric,
    readingTimeVisible:
      typeof value.readingTimeVisible === 'boolean'
        ? value.readingTimeVisible
        : DEFAULT_APP_PREFERENCES.readingTimeVisible,
    defaultCodeBlockLanguage:
      typeof value.defaultCodeBlockLanguage === 'string' &&
      /^[A-Za-z0-9_+#.-]{1,32}$/.test(value.defaultCodeBlockLanguage.trim())
        ? value.defaultCodeBlockLanguage.trim()
        : DEFAULT_APP_PREFERENCES.defaultCodeBlockLanguage,
    defaultDiagramType: isDiagramTypePreference(value.defaultDiagramType)
      ? value.defaultDiagramType
      : DEFAULT_APP_PREFERENCES.defaultDiagramType,
    zoomPercent: clampNumber(value.zoomPercent, 80, 160, DEFAULT_APP_PREFERENCES.zoomPercent),
    ctrlWheelZoomEnabled:
      typeof value.ctrlWheelZoomEnabled === 'boolean'
        ? value.ctrlWheelZoomEnabled
        : DEFAULT_APP_PREFERENCES.ctrlWheelZoomEnabled,
    outlineDefaultExpandLevel: clampNumber(
      value.outlineDefaultExpandLevel,
      1,
      6,
      DEFAULT_APP_PREFERENCES.outlineDefaultExpandLevel,
    ),
    codeBlockLineNumbersVisible:
      typeof value.codeBlockLineNumbersVisible === 'boolean'
        ? value.codeBlockLineNumbersVisible
        : DEFAULT_APP_PREFERENCES.codeBlockLineNumbersVisible,
    codeBlockIndent: isCodeBlockIndentPreference(value.codeBlockIndent)
      ? value.codeBlockIndent
      : DEFAULT_APP_PREFERENCES.codeBlockIndent,
    inlineCodeRenderingEnabled:
      typeof value.inlineCodeRenderingEnabled === 'boolean'
        ? value.inlineCodeRenderingEnabled
        : DEFAULT_APP_PREFERENCES.inlineCodeRenderingEnabled,
    copyMarkdownSyntaxEnabled:
      typeof value.copyMarkdownSyntaxEnabled === 'boolean'
        ? value.copyMarkdownSyntaxEnabled
        : DEFAULT_APP_PREFERENCES.copyMarkdownSyntaxEnabled,
    markdownLintEnabled:
      typeof value.markdownLintEnabled === 'boolean'
        ? value.markdownLintEnabled
        : DEFAULT_APP_PREFERENCES.markdownLintEnabled,
    markdownLintRuleSet: isMarkdownLintRuleSet(value.markdownLintRuleSet)
      ? value.markdownLintRuleSet
      : DEFAULT_APP_PREFERENCES.markdownLintRuleSet,
    renderMode: isRenderModePreference(value.renderMode)
      ? value.renderMode
      : DEFAULT_APP_PREFERENCES.renderMode,
    shortcutPreferences: normalizeShortcutPreferences(value.shortcutPreferences),
    imageHandlingSettings: normalizeImageSettings(
      value.imageHandlingSettings as Partial<ImageHandlingSettings> | null | undefined,
    ),
    developerMode:
      typeof value.developerMode === 'boolean'
        ? value.developerMode
        : DEFAULT_APP_PREFERENCES.developerMode,
    softwareUpdateAutoCheckEnabled:
      typeof value.softwareUpdateAutoCheckEnabled === 'boolean'
        ? value.softwareUpdateAutoCheckEnabled
        : DEFAULT_APP_PREFERENCES.softwareUpdateAutoCheckEnabled,
  };
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

export function applyTypographySettings(
  fontSize: number,
  lineHeight: number,
  preferences?: Pick<AppPreferences, 'typographyEnabled'>,
) {
  document.documentElement.style.setProperty('--md-editor-font-size', `${fontSize}px`);
  document.documentElement.style.setProperty('--md-editor-line-height', String(lineHeight));
  setTypographyOptions({
    enabled: preferences?.typographyEnabled,
    profile: DEFAULT_TYPOGRAPHY.profile,
    hanging: DEFAULT_TYPOGRAPHY.hanging,
  });
}

export function applyEditorLayoutSettings(contentWidthPercent: number) {
  document.documentElement.style.setProperty(
    '--md-editor-content-width-percent',
    String(contentWidthPercent),
  );
}

export function applyZoomSetting(
  zoomPercent: number,
  options?: { transition?: boolean; onFrame?: () => void },
) {
  const targetZoom = zoomPercent / 100;
  if (!options?.transition || typeof window === 'undefined') {
    cancelZoomAnimation();
    setZoomValue(targetZoom);
    options?.onFrame?.();
    return;
  }

  const raf = window.requestAnimationFrame?.bind(window);
  if (!raf) {
    cancelZoomAnimation();
    setZoomValue(targetZoom);
    options.onFrame?.();
    return;
  }

  cancelZoomAnimation();

  const startZoom = getCurrentZoomValue();
  const delta = targetZoom - startZoom;
  if (Math.abs(delta) < 0.001) {
    setZoomValue(targetZoom);
    options.onFrame?.();
    return;
  }

  const duration = prefersReducedMotion() ? ZOOM_REDUCED_TRANSITION_MS : ZOOM_TRANSITION_MS;
  const startedAt = Date.now();
  const tick = () => {
    const elapsed = Date.now() - startedAt;
    const progress = Math.min(1, Math.max(0, elapsed / duration));
    setZoomValue(startZoom + delta * easeOutCubic(progress));
    options.onFrame?.();

    if (progress < 1) {
      zoomAnimationFrame = raf(tick);
    } else {
      setZoomValue(targetZoom);
      options.onFrame?.();
      zoomAnimationFrame = null;
    }
  };

  zoomAnimationFrame = raf(tick);
}

export function applyCodeBlockLineNumberSetting(visible: boolean) {
  document.documentElement.dataset.codeLineNumbers = visible ? 'visible' : 'hidden';
}

function toPersistedPreferenceEntries(
  preferences: AppPreferences,
): Record<AppPreferenceKey, unknown> {
  return {
    themeMode: preferences.themeMode,
    colorThemeId: preferences.colorThemeId,
    documentStyleId: preferences.documentStyleId,
    interfaceLanguage: preferences.interfaceLanguage,
    editorMode: preferences.editorMode,
    splitViewLayout: preferences.splitViewLayout,
    splitLeftPercent: preferences.splitLeftPercent,
    autoSaveEnabled: preferences.autoSaveEnabled,
    autoSaveDelayMs: preferences.autoSaveDelayMs,
    createSnapshotBeforeSave: preferences.createSnapshotBeforeSave,
    fontSize: preferences.fontSize,
    lineHeight: preferences.lineHeight,
    typographyEnabled: preferences.typographyEnabled,
    contentWidthPercent: preferences.contentWidthPercent,
    largeDocumentLimit: preferences.largeDocumentLimit,
    openDefaultBehavior: preferences.openDefaultBehavior,
    filePreviewEnabled: preferences.filePreviewEnabled,
    closeWindowBehavior: preferences.closeWindowBehavior,
    externalFileChangeBehavior: preferences.externalFileChangeBehavior,
    sidebarHidden: preferences.sidebarHidden,
    toolbarHidden: preferences.toolbarHidden,
    outlineVisible: preferences.outlineVisible,
    writingStatsVisible: preferences.writingStatsVisible,
    writingStatsMetric: preferences.writingStatsMetric,
    readingTimeVisible: preferences.readingTimeVisible,
    defaultCodeBlockLanguage: preferences.defaultCodeBlockLanguage,
    defaultDiagramType: preferences.defaultDiagramType,
    zoomPercent: preferences.zoomPercent,
    ctrlWheelZoomEnabled: preferences.ctrlWheelZoomEnabled,
    outlineDefaultExpandLevel: preferences.outlineDefaultExpandLevel,
    codeBlockLineNumbersVisible: preferences.codeBlockLineNumbersVisible,
    codeBlockIndent: preferences.codeBlockIndent,
    inlineCodeRenderingEnabled: preferences.inlineCodeRenderingEnabled,
    copyMarkdownSyntaxEnabled: preferences.copyMarkdownSyntaxEnabled,
    markdownLintEnabled: preferences.markdownLintEnabled,
    markdownLintRuleSet: preferences.markdownLintRuleSet,
    renderMode: preferences.renderMode,
    shortcutPreferences: preferences.shortcutPreferences,
    imageHandlingSettings: preferences.imageHandlingSettings,
    developerMode: preferences.developerMode,
    softwareUpdateAutoCheckEnabled: preferences.softwareUpdateAutoCheckEnabled,
  };
}

function pickPersistedPreferenceEntries(preferences: AppPreferences, keys?: AppPreferenceKey[]) {
  const entries = toPersistedPreferenceEntries(preferences);
  if (!keys) {
    return entries;
  }

  const uniqueKeys = new Set(keys);
  return Object.fromEntries(
    Object.entries(entries).filter(([key]) => uniqueKeys.has(key as AppPreferenceKey)),
  );
}

async function migrateAppearancePreferences(
  desktopEnabled: boolean,
  settings: Map<string, string>,
): Promise<AppearancePreferences> {
  const migrationDone =
    parseSetting<boolean>(settings, APPEARANCE_THEME_MODEL_MIGRATION_KEY) === true;
  const storedThemeMode = parseSetting<unknown>(settings, 'themeMode');
  const storedColorThemeId = parseSetting<unknown>(settings, 'colorThemeId');
  const storedDocumentStyleId = parseSetting<unknown>(settings, 'documentStyleId');

  if (migrationDone) {
    const appearance = {
      themeMode: isThemeMode(storedThemeMode) ? storedThemeMode : 'system',
      colorThemeId: isRegisteredThemeId(storedColorThemeId)
        ? storedColorThemeId
        : DEFAULT_COLOR_THEME_ID,
      documentStyleId: isRegisteredDocumentStyleId(storedDocumentStyleId)
        ? storedDocumentStyleId
        : DEFAULT_DOCUMENT_STYLE_ID,
    } satisfies AppearancePreferences;
    const repairEntries: Record<string, unknown> = {};
    if (appearance.themeMode !== storedThemeMode) {
      repairEntries.themeMode = appearance.themeMode;
    }
    if (appearance.colorThemeId !== storedColorThemeId) {
      repairEntries.colorThemeId = appearance.colorThemeId;
    }
    if (appearance.documentStyleId !== storedDocumentStyleId) {
      repairEntries.documentStyleId = appearance.documentStyleId;
    }
    if (Object.keys(repairEntries).length > 0) {
      logWarn('settings', '外观设置无效，已回退到内置默认值', repairEntries);
      if (desktopEnabled) {
        await updateAppSettings(repairEntries).catch(() => undefined);
      }
    }
    markAppearanceMigrationDone();
    return appearance;
  }

  const legacyTheme = parseSetting<unknown>(settings, 'theme');
  const legacyBlockStyle = parseSetting<unknown>(settings, 'blockStyle');
  const appearance: AppearancePreferences = {
    themeMode: isThemeMode(legacyTheme) ? legacyTheme : 'system',
    colorThemeId: DEFAULT_COLOR_THEME_ID,
    documentStyleId:
      legacyBlockStyle === 'classic' ? CLASSIC_DOCUMENT_STYLE_ID : DEFAULT_DOCUMENT_STYLE_ID,
  };
  markAppearanceMigrationDone();
  if (desktopEnabled) {
    await updateAppSettings({
      ...appearance,
      [APPEARANCE_THEME_MODEL_MIGRATION_KEY]: true,
    }).catch((error) => {
      logWarn('settings', '外观设置迁移写入失败，将在下次启动重试', {
        error: error instanceof Error ? error.message : String(error),
      });
    });
  }
  return appearance;
}

function markAppearanceMigrationDone() {
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(APPEARANCE_THEME_MODEL_MIGRATION_KEY, 'true');
  }
}

function parseSetting<T>(settings: Map<string, string>, key: string): T | null {
  const value = settings.get(key);
  if (!value) {
    return null;
  }

  try {
    return JSON.parse(value) as T;
  } catch {
    return null;
  }
}

export function normalizeImageSettings(
  value: Partial<ImageHandlingSettings> | null | undefined,
): ImageHandlingSettings {
  const strategy = value?.imageInsertStrategy;
  const provider = value?.uploadProvider;

  return {
    imageInsertStrategy: isImageInsertStrategy(strategy)
      ? strategy
      : DEFAULT_IMAGE_HANDLING_SETTINGS.imageInsertStrategy,
    autoDeleteUnusedLocalImages:
      typeof value?.autoDeleteUnusedLocalImages === 'boolean'
        ? value.autoDeleteUnusedLocalImages
        : DEFAULT_IMAGE_HANDLING_SETTINGS.autoDeleteUnusedLocalImages,
    uploadProvider: isImageUploadProvider(provider)
      ? provider
      : DEFAULT_IMAGE_HANDLING_SETTINGS.uploadProvider,
    picgoServerUrl:
      typeof value?.picgoServerUrl === 'string' && value.picgoServerUrl.trim()
        ? value.picgoServerUrl.trim()
        : DEFAULT_IMAGE_HANDLING_SETTINGS.picgoServerUrl,
    picgoCoreCommand:
      typeof value?.picgoCoreCommand === 'string' && value.picgoCoreCommand.trim()
        ? value.picgoCoreCommand.trim()
        : DEFAULT_IMAGE_HANDLING_SETTINGS.picgoCoreCommand,
    picgoCoreConfigPath:
      typeof value?.picgoCoreConfigPath === 'string' ? value.picgoCoreConfigPath.trim() : '',
    defaultImageWidth: normalizeImageDefaultWidth(value?.defaultImageWidth),
    defaultImageAlign: isImageDefaultAlign(value?.defaultImageAlign)
      ? value.defaultImageAlign
      : DEFAULT_IMAGE_HANDLING_SETTINGS.defaultImageAlign,
  };
}

function isImageInsertStrategy(value: unknown): value is ImageInsertStrategy {
  return (
    value === 'copy-current-folder' ||
    value === 'copy-assets' ||
    value === 'copy-document-assets' ||
    value === 'upload'
  );
}

function isImageUploadProvider(value: unknown): value is ImageUploadProvider {
  return value === 'picgo' || value === 'picgo-core';
}

function isImageDefaultAlign(value: unknown): value is ImageDefaultAlignPreference {
  return value === 'none' || value === 'left' || value === 'center' || value === 'right';
}

function normalizeImageDefaultWidth(value: unknown): string {
  if (typeof value !== 'string') {
    return DEFAULT_IMAGE_HANDLING_SETTINGS.defaultImageWidth;
  }
  const width = value.trim();
  if (!width) {
    return '';
  }
  if (/^\d+$/.test(width)) {
    return `${Math.min(2400, Math.max(1, Number(width)))}px`;
  }
  if (/^\d+px$/.test(width)) {
    const numberValue = Number(width.slice(0, -2));
    return `${Math.min(2400, Math.max(1, numberValue))}px`;
  }
  if (/^\d+%$/.test(width)) {
    const numberValue = Number(width.slice(0, -1));
    return `${Math.min(100, Math.max(1, numberValue))}%`;
  }
  return DEFAULT_IMAGE_HANDLING_SETTINGS.defaultImageWidth;
}

function isCodeBlockIndentPreference(value: unknown): value is CodeBlockIndentPreference {
  return value === 'spaces-2' || value === 'spaces-4' || value === 'tab';
}

function isRenderModePreference(value: unknown): value is RenderModePreference {
  return value === 'hardware' || value === 'software';
}

function isMarkdownLintRuleSet(value: unknown): value is MarkdownLintRuleSet {
  return value === 'relaxed' || value === 'default';
}

function normalizeShortcutPreferences(value: unknown): ShortcutPreferences {
  const source =
    value && typeof value === 'object'
      ? (value as Partial<Record<ShortcutCommandId, unknown>>)
      : {};
  const normalized = { ...DEFAULT_SHORTCUT_PREFERENCES };
  for (const key of Object.keys(DEFAULT_SHORTCUT_PREFERENCES) as ShortcutCommandId[]) {
    const shortcut = source[key];
    if (typeof shortcut === 'string' && isValidShortcut(shortcut)) {
      normalized[key] = normalizeShortcutText(shortcut);
    }
  }
  return normalized;
}

function isValidShortcut(value: string): boolean {
  return /^[A-Za-z0-9+\\\-\[\]` ]{3,40}$/.test(value.trim());
}

function normalizeShortcutText(value: string): string {
  return value
    .split('+')
    .map((part) => part.trim())
    .filter(Boolean)
    .join('+');
}

function clampNumber(value: unknown, min: number, max: number, fallback: number): number {
  if (value === null || value === undefined || value === '') {
    return fallback;
  }
  const numberValue = typeof value === 'string' ? Number(value) : Number(value);
  if (!Number.isFinite(numberValue)) {
    return fallback;
  }
  return Math.min(max, Math.max(min, numberValue));
}

function getCurrentZoomValue() {
  const inlineValue = document.documentElement.style.getPropertyValue('--md-editor-zoom');
  const computedValue = getComputedStyle(document.documentElement).getPropertyValue(
    '--md-editor-zoom',
  );
  const parsed = Number.parseFloat(inlineValue || computedValue);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

function cancelZoomAnimation() {
  if (zoomAnimationFrame === null || typeof window === 'undefined') {
    zoomAnimationFrame = null;
    return;
  }
  window.cancelAnimationFrame?.(zoomAnimationFrame);
  zoomAnimationFrame = null;
}

function setZoomValue(value: number) {
  document.documentElement.style.setProperty('--md-editor-zoom', String(value));
}

function easeOutCubic(progress: number) {
  return 1 - Math.pow(1 - progress, 3);
}

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark' || value === 'system';
}

function isEditorModePreference(value: unknown): value is EditorModePreference {
  return value === 'semantic' || value === 'source' || value === 'split';
}

function isSplitViewLayoutPreference(value: unknown): value is SplitViewLayoutPreference {
  return value === 'semantic-source' || value === 'source-semantic';
}

function isOpenDefaultBehavior(value: unknown): value is OpenDefaultBehavior {
  return value === 'current-window' || value === 'new-window' || value === 'ask-every-time';
}

function isCloseWindowBehavior(value: unknown): value is CloseWindowBehavior {
  return value === 'ask-every-time' || value === 'close-window' || value === 'close-to-tray';
}

function isExternalFileChangeBehavior(value: unknown): value is ExternalFileChangeBehavior {
  return value === 'reload-external' || value === 'overwrite-external' || value === 'ignore';
}

function resolveLegacyCloseWindowBehavior(
  settings: Map<string, string>,
): CloseWindowBehavior | undefined {
  const closeToTrayEnabled = parseSetting<boolean>(settings, 'closeToTrayEnabled');
  if (closeToTrayEnabled === true) {
    return 'close-to-tray';
  }

  const promptAnswered = parseSetting<boolean>(settings, LEGACY_CLOSE_TO_TRAY_PROMPT_ANSWERED_KEY);
  if (closeToTrayEnabled === false && promptAnswered === true) {
    return 'close-window';
  }

  return undefined;
}

function isWritingStatsMetric(value: unknown): value is WritingStatsMetric {
  return (
    value === 'lines' || value === 'words' || value === 'visibleChars' || value === 'chars'
  );
}

function isDiagramTypePreference(value: unknown): value is DiagramType {
  return (
    value === 'flowchart' ||
    value === 'sequenceDiagram' ||
    value === 'classDiagram' ||
    value === 'stateDiagram' ||
    value === 'pie' ||
    value === 'gantt' ||
    value === 'erDiagram'
  );
}

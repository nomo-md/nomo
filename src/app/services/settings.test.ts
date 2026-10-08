import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_IMAGE_HANDLING_SETTINGS } from '../../lib/services/render';
import { getTypographyOptions, setTypographyOptions } from '../../lib/typography/options';

const storageMock = vi.hoisted(() => ({
  listAppSettings: vi.fn(),
  updateAppSetting: vi.fn(),
  updateAppSettings: vi.fn(),
}));

vi.mock('../../lib/desktop/tauriStorage', () => storageMock);

import {
  DEFAULT_APP_PREFERENCES,
  applyZoomSetting,
  applyTypographySettings,
  loadAppPreferences,
  loadPersistedEditorSettings,
  loadPersistedImageSettings,
  normalizeAppPreferences,
  normalizeImageSettings,
  saveAppPreferences,
} from './settings';

beforeEach(() => {
  storageMock.listAppSettings.mockReset();
  storageMock.updateAppSetting.mockReset();
  storageMock.updateAppSettings.mockReset();
  storageMock.updateAppSetting.mockResolvedValue(undefined);
  storageMock.updateAppSettings.mockResolvedValue(undefined);
  localStorage.clear();
});

describe('settings', () => {
  it('uses one typography switch and ignores retired secondary preferences', () => {
    const previous = getTypographyOptions();
    const legacyPreferences = {
      typographyEnabled: true,
      typographyHanging: false,
      typographyProfile: 'zh-TW',
    };
    const preferences = normalizeAppPreferences(legacyPreferences);
    try {
      applyTypographySettings(16, 1.75, preferences);
      expect(getTypographyOptions()).toEqual({ enabled: true, profile: 'zh-CN', hanging: true });
      applyTypographySettings(16, 1.75, normalizeAppPreferences({ typographyEnabled: false }));
      expect(getTypographyOptions()).toEqual({ enabled: false, profile: 'zh-CN', hanging: true });
    } finally {
      setTypographyOptions(previous);
    }
  });
  it.skip('keeps automatic local image cleanup enabled for existing image settings', () => {
    expect(normalizeImageSettings({}).autoDeleteUnusedLocalImages).toBe(true);
  });

  it('preserves an explicit automatic local image cleanup choice', () => {
    expect(
      normalizeImageSettings({
        autoDeleteUnusedLocalImages: false,
      }).autoDeleteUnusedLocalImages,
    ).toBe(false);
  });

  it('keeps autosave disabled by default and guarded by settings', () => {
    const appSource = readFileSync(resolve(__dirname, '../App.svelte'), 'utf-8');
    const settingsWindowSource = readFileSync(
      resolve(__dirname, '../components/SettingsWindow.svelte'),
      'utf-8',
    );
    const documentActionsSource = readFileSync(
      resolve(__dirname, 'documentActionsController.ts'),
      'utf-8',
    );

    expect(appSource).toContain('DEFAULT_APP_PREFERENCES.autoSaveEnabled');
    expect(settingsWindowSource).toContain('t.autoSave()');
    expect(settingsWindowSource).toContain('autoSaveDelayMs');
    expect(appSource).toContain('autoSaveEnabled && desktopEnabled && dirty && nativePath');
    expect(documentActionsSource).toContain('if (!options.getAutoSaveEnabled()) return;');
    expect(documentActionsSource).toContain('options.getAutoSaveDelayMs()');
    expect(documentActionsSource).toContain('cancelPendingAutoSaves');
  });

  it('persists view preferences from the settings window', () => {
    const appSource = readFileSync(resolve(__dirname, '../App.svelte'), 'utf-8');
    const settingsWindowSource = readFileSync(
      resolve(__dirname, '../components/SettingsWindow.svelte'),
      'utf-8',
    );

    expect(settingsWindowSource).toContain('t.editorModeDefault()');
    expect(settingsWindowSource).toContain('t.hideExplorerOnLaunch()');
    expect(settingsWindowSource).toContain('t.showDocumentOutline()');
    expect(settingsWindowSource).toContain('setEditorMode');
    expect(settingsWindowSource).toContain('externalFileChangeBehavior');
    expect(settingsWindowSource).toContain('setExternalFileChangeBehavior');
    expect(settingsWindowSource).toContain("activeCategory === 'files'");
    expect(settingsWindowSource).toContain('sidebarHidden');
    expect(settingsWindowSource).toContain('outlineVisible');
    expect(appSource).toContain("updateAppSetting('editorMode', nextMode)");
    expect(appSource).toContain("updateAppSetting('sidebarHidden', hidden)");
    expect(appSource).toContain("updateAppSetting('outlineVisible', visible)");
    expect(appSource).toContain('loadAppPreferences(desktopEnabled)');
    expect(appSource).toContain('loadAppPreferences(desktopEnabled, settings)');
    expect(appSource).toContain('applyAppPreferences');
  });

  it('reuses provided native settings rows without another desktop read', async () => {
    const preferences = await loadAppPreferences(true, [
      createSetting('appearanceThemeModelV1', true),
      createSetting('themeMode', 'system'),
      createSetting('colorThemeId', 'nomo-default'),
      createSetting('documentStyleId', 'nomo-modern'),
      createSetting('fontSize', 18),
      createSetting('interfaceLanguage', 'ja-JP'),
    ]);

    expect(preferences.fontSize).toBe(18);
    expect(preferences.interfaceLanguage).toBe('ja-JP');
    expect(storageMock.listAppSettings).not.toHaveBeenCalled();
  });

  it('reads native settings when no reusable rows are provided', async () => {
    storageMock.listAppSettings.mockResolvedValue([
      createSetting('appearanceThemeModelV1', true),
      createSetting('fontSize', 19),
    ]);

    const preferences = await loadAppPreferences(true);

    expect(preferences.fontSize).toBe(19);
    expect(storageMock.listAppSettings).toHaveBeenCalledTimes(1);
  });

  it('migrates legacy theme and block style into the appearance model once', async () => {
    const preferences = await loadAppPreferences(true, [
      createSetting('theme', 'dark'),
      createSetting('blockStyle', 'classic'),
    ]);

    expect(preferences).toMatchObject({
      themeMode: 'dark',
      colorThemeId: 'nomo-default',
      documentStyleId: 'nomo-classic',
    });
    expect(storageMock.updateAppSettings).toHaveBeenCalledWith({
      themeMode: 'dark',
      colorThemeId: 'nomo-default',
      documentStyleId: 'nomo-classic',
      appearanceThemeModelV1: true,
    });
  });

  it('repairs invalid persisted theme and document style identifiers', async () => {
    const preferences = await loadAppPreferences(true, [
      createSetting('appearanceThemeModelV1', true),
      createSetting('themeMode', 'sepia'),
      createSetting('colorThemeId', 'missing-theme'),
      createSetting('documentStyleId', 'missing-style'),
    ]);

    expect(preferences).toMatchObject({
      themeMode: 'system',
      colorThemeId: 'nomo-default',
      documentStyleId: 'nomo-modern',
    });
    expect(storageMock.updateAppSettings).toHaveBeenCalledWith({
      themeMode: 'system',
      colorThemeId: 'nomo-default',
      documentStyleId: 'nomo-modern',
    });
  });

  it('ignores legacy local storage preference fallbacks when native settings are missing', async () => {
    localStorage.setItem('nomo-theme', 'dark');
    localStorage.setItem('nomo-interface-language', 'ja-JP');
    localStorage.setItem('nomo-font-size', '21');
    localStorage.setItem('nomo-line-height', '2.1');
    localStorage.setItem('nomo-content-width-percent', '80');
    localStorage.setItem('nomo-block-style', 'classic');

    const preferences = await loadAppPreferences(true, [
      createSetting('appearanceThemeModelV1', true),
    ]);

    expect(preferences.themeMode).toBe(DEFAULT_APP_PREFERENCES.themeMode);
    expect(preferences.colorThemeId).toBe(DEFAULT_APP_PREFERENCES.colorThemeId);
    expect(preferences.documentStyleId).toBe(DEFAULT_APP_PREFERENCES.documentStyleId);
    expect(preferences.interfaceLanguage).toBe(DEFAULT_APP_PREFERENCES.interfaceLanguage);
    expect(preferences.fontSize).toBe(DEFAULT_APP_PREFERENCES.fontSize);
    expect(preferences.lineHeight).toBe(DEFAULT_APP_PREFERENCES.lineHeight);
    expect(preferences.contentWidthPercent).toBe(DEFAULT_APP_PREFERENCES.contentWidthPercent);
  });

  it('ignores legacy local image settings when native image preferences are missing', async () => {
    localStorage.setItem(
      'nomo-image-handling-settings',
      JSON.stringify({
        imageInsertStrategy: 'upload',
        uploadProvider: 'picgo-core',
        defaultImageWidth: '80%',
        defaultImageAlign: 'center',
      }),
    );

    const preferences = await loadAppPreferences(true, [
      createSetting('appearanceThemeModelV1', true),
    ]);
    const imageSettings = await loadPersistedImageSettings(true, []);

    expect(preferences.imageHandlingSettings).toEqual(DEFAULT_IMAGE_HANDLING_SETTINGS);
    expect(imageSettings).toEqual(DEFAULT_IMAGE_HANDLING_SETTINGS);
  });

  it('returns only native editor settings and ignores legacy local storage values', async () => {
    localStorage.setItem('nomo-theme', 'dark');
    localStorage.setItem('nomo-font-size', '21');
    localStorage.setItem('nomo-line-height', '2.1');
    localStorage.setItem('nomo-content-width-percent', '80');
    localStorage.setItem('nomo-block-style', 'classic');

    await expect(loadPersistedEditorSettings(true, [])).resolves.toEqual({
      fontSize: undefined,
      lineHeight: undefined,
      contentWidthPercent: undefined,
    });

    await expect(
      loadPersistedEditorSettings(true, [
        createSetting('theme', 'dark'),
        createSetting('fontSize', 18),
        createSetting('lineHeight', 1.8),
        createSetting('contentWidthPercent', 72),
        createSetting('blockStyle', 'classic'),
      ]),
    ).resolves.toEqual({
      fontSize: 18,
      lineHeight: 1.8,
      contentWidthPercent: 72,
    });
  });

  it('saves settings window changes as dirty preference patches', () => {
    const appSource = readFileSync(resolve(__dirname, '../App.svelte'), 'utf-8');
    const settingsWindowSource = readFileSync(
      resolve(__dirname, '../components/SettingsWindow.svelte'),
      'utf-8',
    );
    const settingsServiceSource = readFileSync(resolve(__dirname, 'settings.ts'), 'utf-8');
    const tauriStorageSource = readFileSync(
      resolve(__dirname, '../../lib/desktop/tauriStorage.ts'),
      'utf-8',
    );

    expect(settingsWindowSource).toContain('dirtyPreferenceKeys');
    expect(settingsWindowSource).toContain(
      'saveAppPreferences(desktopEnabled, settingsToSave, keysToSave)',
    );
    expect(settingsWindowSource).toContain(
      "{ source: 'settings-window' as const, patch, effectiveScheme }",
    );
    expect(settingsWindowSource).toContain("keysToSave.includes('interfaceLanguage')");
    const saveCallIndex = settingsWindowSource.indexOf(
      'saveAppPreferences(desktopEnabled, settingsToSave, keysToSave)',
    );
    const languageChromeRefreshIndex = settingsWindowSource.indexOf(
      "await invoke('refresh_interface_language_chrome')",
    );
    expect(languageChromeRefreshIndex).toBeGreaterThan(saveCallIndex);
    const emitSettingsUpdatedSource = settingsWindowSource.slice(
      settingsWindowSource.indexOf('async function emitSettingsUpdated'),
      settingsWindowSource.indexOf('async function closeCurrentWindow'),
    );
    expect(emitSettingsUpdatedSource).not.toContain('refresh_interface_language_chrome');
    expect(settingsServiceSource).toContain('updateAppSettings(persistedEntries)');
    expect(tauriStorageSource).toContain("invoke('update_app_settings'");
    expect(appSource).toContain('applyAppPreferencesPatch');
    expect(appSource).toContain('handleSettingsUpdated(event.payload)');
  });

  it.skip('uses a real updater entry in the settings about page', () => {
    const appSource = readFileSync(resolve(__dirname, '../App.svelte'), 'utf-8');
    const settingsWindowSource = readFileSync(
      resolve(__dirname, '../components/SettingsWindow.svelte'),
      'utf-8',
    );

    expect(settingsWindowSource).toContain('checkForSoftwareUpdate');
    expect(settingsWindowSource).toContain('installDownloadedSoftwareUpdate');
    expect(settingsWindowSource).toContain('getSoftwareUpdateButtonLabel');
    expect(settingsWindowSource).toContain('isSoftwareUpdateIntegrityFailure');
    expect(settingsWindowSource).toContain('softwareUpdateIntegrityFailed');
    expect(settingsWindowSource).toContain('nomo://request-update-install');
    expect(settingsWindowSource).not.toContain(
      '<span class="disabled-pill">{t.futureVersionSupport()}</span>\n            </div>\n          </div>\n        {/if}',
    );
    expect(appSource).toContain('nomo://request-update-install');
    expect(appSource).toContain('nomo://update-install-decision');
    expect(appSource).toContain('unsavedChangesBeforeUpdate');
  });

  it('normalizes new preference boundaries and invalid enum values', () => {
    const normalized = normalizeAppPreferences({
      interfaceLanguage: 'zh-TW',
      autoSaveDelayMs: 50,
      largeDocumentLimit: 5_000_000,
      defaultDiagramType: 'unknown' as never,
      defaultCodeBlockLanguage: 'ts script!' as never,
    });

    expect(normalized.interfaceLanguage).toBe('zh-TW');
    expect(normalized.autoSaveDelayMs).toBe(500);
    expect(normalized.largeDocumentLimit).toBe(1_000_000);
    expect(normalized.defaultDiagramType).toBe(DEFAULT_APP_PREFERENCES.defaultDiagramType);
    expect(normalized.defaultCodeBlockLanguage).toBe(
      DEFAULT_APP_PREFERENCES.defaultCodeBlockLanguage,
    );
  });

  it('normalizes first and second batch preference fields', () => {
    const normalized = normalizeAppPreferences({
      themeMode: 'system',
      colorThemeId: 'nomo-amber-paper',
      documentStyleId: 'nomo-classic',
      zoomPercent: 500,
      outlineDefaultExpandLevel: 0,
      codeBlockIndent: 'tab',
      inlineCodeRenderingEnabled: false,
      shortcutPreferences: {
        'toggle-source': 'Ctrl+Alt+E',
      },
      imageHandlingSettings: {
        defaultImageWidth: '2000%',
        defaultImageAlign: 'center',
      } as never,
    });

    expect(normalized.themeMode).toBe('system');
    expect(normalized.colorThemeId).toBe('nomo-amber-paper');
    expect(normalized.documentStyleId).toBe('nomo-classic');
    expect(normalized.zoomPercent).toBe(160);
    expect(normalized.outlineDefaultExpandLevel).toBe(1);
    expect(normalized.codeBlockIndent).toBe('tab');
    expect(DEFAULT_APP_PREFERENCES.inlineCodeRenderingEnabled).toBe(true);
    expect(normalized.inlineCodeRenderingEnabled).toBe(false);
    expect(normalized.shortcutPreferences['toggle-source']).toBe('Ctrl+Alt+E');
    expect(normalized.imageHandlingSettings.defaultImageWidth).toBe('100%');
    expect(normalized.imageHandlingSettings.defaultImageAlign).toBe('center');
  });

  it('falls back to system interface language for invalid preference values', () => {
    const normalized = normalizeAppPreferences({
      interfaceLanguage: 'xx-XX' as never,
    });

    expect(normalized.interfaceLanguage).toBe('system');
  });

  it('preserves the Japanese interface language preference', () => {
    const normalized = normalizeAppPreferences({
      interfaceLanguage: 'ja-JP',
    });

    expect(normalized.interfaceLanguage).toBe('ja-JP');
  });

  it('falls back to enabled inline code rendering for invalid preference values', () => {
    const normalized = normalizeAppPreferences({
      inlineCodeRenderingEnabled: 'false' as never,
    });

    expect(normalized.inlineCodeRenderingEnabled).toBe(true);
  });

  it('defaults Markdown clipboard syntax to enabled and preserves explicit choices', () => {
    expect(DEFAULT_APP_PREFERENCES.copyMarkdownSyntaxEnabled).toBe(true);
    expect(normalizeAppPreferences({}).copyMarkdownSyntaxEnabled).toBe(true);
    expect(
      normalizeAppPreferences({ copyMarkdownSyntaxEnabled: 'false' as never })
        .copyMarkdownSyntaxEnabled,
    ).toBe(true);
    expect(
      normalizeAppPreferences({ copyMarkdownSyntaxEnabled: false }).copyMarkdownSyntaxEnabled,
    ).toBe(false);
    expect(
      normalizeAppPreferences({ copyMarkdownSyntaxEnabled: true }).copyMarkdownSyntaxEnabled,
    ).toBe(true);
  });

  it('loads and saves the Markdown clipboard syntax preference', async () => {
    await expect(
      loadAppPreferences(true, [
        createSetting('appearanceThemeModelV1', true),
        createSetting('copyMarkdownSyntaxEnabled', false),
      ]),
    ).resolves.toMatchObject({ copyMarkdownSyntaxEnabled: false });

    await saveAppPreferences(
      true,
      { ...DEFAULT_APP_PREFERENCES, copyMarkdownSyntaxEnabled: false },
      ['copyMarkdownSyntaxEnabled'],
    );

    expect(storageMock.updateAppSettings).toHaveBeenCalledWith({
      copyMarkdownSyntaxEnabled: false,
    });
  });

  it('loads and saves the automatic software update preference', async () => {
    await expect(
      loadAppPreferences(true, [
        createSetting('appearanceThemeModelV1', true),
        createSetting('softwareUpdateAutoCheckEnabled', false),
      ]),
    ).resolves.toMatchObject({ softwareUpdateAutoCheckEnabled: false });

    await saveAppPreferences(
      true,
      { ...DEFAULT_APP_PREFERENCES, softwareUpdateAutoCheckEnabled: false },
      ['softwareUpdateAutoCheckEnabled'],
    );

    expect(storageMock.updateAppSettings).toHaveBeenCalledWith({
      softwareUpdateAutoCheckEnabled: false,
    });
  });

  it('normalizes close window behavior settings', () => {
    const normalized = normalizeAppPreferences({
      closeWindowBehavior: {} as never,
    });

    expect(DEFAULT_APP_PREFERENCES.closeWindowBehavior).toBe('ask-every-time');
    expect(normalized.closeWindowBehavior).toBe('ask-every-time');
    expect(
      normalizeAppPreferences({ closeWindowBehavior: 'close-to-tray' }).closeWindowBehavior,
    ).toBe('close-to-tray');
  });

  it('defaults external file change behavior to reload and normalizes invalid values', () => {
    expect(DEFAULT_APP_PREFERENCES.externalFileChangeBehavior).toBe('reload-external');
    expect(normalizeAppPreferences({}).externalFileChangeBehavior).toBe('reload-external');
    expect(
      normalizeAppPreferences({ externalFileChangeBehavior: 'ignore' }).externalFileChangeBehavior,
    ).toBe('ignore');
    expect(
      normalizeAppPreferences({ externalFileChangeBehavior: 'invalid' as never })
        .externalFileChangeBehavior,
    ).toBe('reload-external');
  });

  it('defaults developer mode to false and preserves explicit value', () => {
    expect(DEFAULT_APP_PREFERENCES.developerMode).toBe(false);
    expect(normalizeAppPreferences({}).developerMode).toBe(false);
    expect(normalizeAppPreferences({ developerMode: true }).developerMode).toBe(true);
    expect(normalizeAppPreferences({ developerMode: false }).developerMode).toBe(false);
  });

  it('defaults render mode to hardware and normalizes invalid values', () => {
    expect(DEFAULT_APP_PREFERENCES.renderMode).toBe('hardware');
    expect(normalizeAppPreferences({}).renderMode).toBe('hardware');
    expect(normalizeAppPreferences({ renderMode: 'software' }).renderMode).toBe('software');
    expect(normalizeAppPreferences({ renderMode: 'unknown' as never }).renderMode).toBe('hardware');
  });

  it('migrates legacy close-to-tray choices into close window behavior', async () => {
    await expect(
      loadAppPreferences(true, [
        createSetting('appearanceThemeModelV1', true),
        createSetting('closeToTrayEnabled', true),
      ]),
    ).resolves.toMatchObject({ closeWindowBehavior: 'close-to-tray' });

    await expect(
      loadAppPreferences(true, [
        createSetting('appearanceThemeModelV1', true),
        createSetting('closeToTrayEnabled', false),
        createSetting('closeToTrayPromptAnswered', true),
      ]),
    ).resolves.toMatchObject({ closeWindowBehavior: 'close-window' });

    await expect(
      loadAppPreferences(true, [
        createSetting('appearanceThemeModelV1', true),
        createSetting('closeToTrayEnabled', false),
      ]),
    ).resolves.toMatchObject({ closeWindowBehavior: 'ask-every-time' });
  });

  it('calls the zoom frame callback after applying an immediate zoom value', () => {
    const onFrame = vi.fn();

    applyZoomSetting(120, { onFrame });

    expect(document.documentElement.style.getPropertyValue('--md-editor-zoom')).toBe('1.2');
    expect(onFrame).toHaveBeenCalledTimes(1);
  });

  it('calls the zoom frame callback while animated zoom is progressing', () => {
    const originalRequestAnimationFrame = window.requestAnimationFrame;
    const originalCancelAnimationFrame = window.cancelAnimationFrame;
    const frames: FrameRequestCallback[] = [];
    let now = 0;
    const dateNow = vi.spyOn(Date, 'now').mockImplementation(() => now);
    const onFrame = vi.fn();

    window.requestAnimationFrame = ((callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    }) as typeof window.requestAnimationFrame;
    window.cancelAnimationFrame = vi.fn() as typeof window.cancelAnimationFrame;

    try {
      applyZoomSetting(130, { transition: true, onFrame });
      expect(onFrame).not.toHaveBeenCalled();

      now = 75;
      frames.shift()?.(75);
      expect(onFrame).toHaveBeenCalledTimes(1);

      now = 200;
      frames.shift()?.(200);
      expect(document.documentElement.style.getPropertyValue('--md-editor-zoom')).toBe('1.3');
      expect(onFrame).toHaveBeenCalledTimes(3);
    } finally {
      dateNow.mockRestore();
      window.requestAnimationFrame = originalRequestAnimationFrame;
      window.cancelAnimationFrame = originalCancelAnimationFrame;
      applyZoomSetting(100);
    }
  });
});

function createSetting(key: string, value: unknown) {
  return {
    key,
    valueJson: JSON.stringify(value),
    updatedAt: 1,
  };
}

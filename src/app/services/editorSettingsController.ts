import {
  applyEditorLayoutSettings,
  applyTypographySettings,
  loadPersistedEditorSettings,
  persistEditorSetting,
} from './settings';

interface EditorSettingsControllerOptions {
  getDesktopEnabled(): boolean;
  getFontSize(): number;
  setFontSize(value: number): void;
  getLineHeight(): number;
  setLineHeight(value: number): void;
  getContentWidthPercent(): number;
  setContentWidthPercent(value: number): void;
  refreshEditorViewportLayout(): void;
}

export function createEditorSettingsController(options: EditorSettingsControllerOptions) {
  async function loadPersistedSettings() {
    const settings = await loadPersistedEditorSettings(options.getDesktopEnabled());

    if (settings.fontSize) {
      options.setFontSize(settings.fontSize);
      applyTypographySettings(options.getFontSize(), options.getLineHeight());
      options.refreshEditorViewportLayout();
    }
    if (settings.lineHeight) {
      options.setLineHeight(settings.lineHeight);
      applyTypographySettings(options.getFontSize(), options.getLineHeight());
      options.refreshEditorViewportLayout();
    }
    if (settings.contentWidthPercent) {
      options.setContentWidthPercent(settings.contentWidthPercent);
      applyEditorLayoutSettings(options.getContentWidthPercent());
      options.refreshEditorViewportLayout();
    }
  }

  function updateFontSizeValue(value: number) {
    if (value === options.getFontSize()) return;
    options.setFontSize(value);
    localStorage.setItem('nomo-font-size', String(value));
    persistSetting('fontSize', value);
    applyTypographySettings(options.getFontSize(), options.getLineHeight());
    options.refreshEditorViewportLayout();
  }

  function updateFontSize(event: Event) {
    updateFontSizeValue(Number((event.currentTarget as HTMLInputElement).value));
  }

  function updateLineHeightValue(value: number) {
    if (value === options.getLineHeight()) return;
    options.setLineHeight(value);
    localStorage.setItem('nomo-line-height', String(value));
    persistSetting('lineHeight', value);
    applyTypographySettings(options.getFontSize(), options.getLineHeight());
    options.refreshEditorViewportLayout();
  }

  function updateLineHeight(event: Event) {
    updateLineHeightValue(Number((event.currentTarget as HTMLInputElement).value));
  }

  function updateContentWidth(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    updateContentWidthValue(value);
  }

  function updateContentWidthValue(value: number) {
    if (value === options.getContentWidthPercent()) return;
    options.setContentWidthPercent(value);
    localStorage.setItem('nomo-content-width-percent', String(value));
    persistSetting('contentWidthPercent', value);
    applyEditorLayoutSettings(options.getContentWidthPercent());
    options.refreshEditorViewportLayout();
  }

  function persistSetting(key: string, value: unknown) {
    persistEditorSetting(options.getDesktopEnabled(), key, value);
  }

  return {
    loadPersistedSettings,
    updateFontSize,
    updateFontSizeValue,
    updateLineHeight,
    updateLineHeightValue,
    updateContentWidth,
    updateContentWidthValue,
  };
}

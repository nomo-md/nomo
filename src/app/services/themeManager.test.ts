import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setDesktopIconTheme } from './desktopWindow';
import {
  LEGACY_THEME_BOOT_SNAPSHOT_KEY,
  THEME_BOOT_SNAPSHOT_KEY,
  applyResolvedTheme,
  applyThemeRuntime,
  bootstrapThemeFromSnapshot,
  cancelPendingThemeEffects,
  readThemeBootSnapshot,
  resolveTheme,
  resolveThemeMode,
  writeThemeBootSnapshot,
} from './themeManager';

vi.mock('./desktopWindow', () => ({
  getDesktopSystemTheme: vi.fn().mockResolvedValue('dark'),
  setDesktopIconTheme: vi.fn().mockResolvedValue(undefined),
}));

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('style');
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.removeAttribute('data-theme-preference');
  document.documentElement.removeAttribute('data-color-theme');
  document.documentElement.removeAttribute('data-theme-style');
  document.documentElement.removeAttribute('data-document-style');
  document.documentElement.removeAttribute('data-block-style');
  cancelPendingThemeEffects();
  vi.mocked(setDesktopIconTheme).mockClear();
});

afterEach(() => {
  cancelPendingThemeEffects();
  vi.unstubAllGlobals();
});

describe('themeManager', () => {
  it('resolves system mode and explicit modes independently from the color theme', () => {
    expect(resolveThemeMode('system', 'dark')).toBe('dark');
    expect(resolveThemeMode('light', 'dark')).toBe('light');
    expect(resolveThemeMode('dark', 'light')).toBe('dark');

    const resolved = resolveTheme(
      {
        themeMode: 'dark',
        colorThemeId: 'nomo-amber-paper',
        documentStyleId: 'nomo-classic',
      },
      'light',
    );
    expect(resolved.effectiveScheme).toBe('dark');
    expect(resolved.preferences.colorThemeId).toBe('nomo-amber-paper');
  });

  it('applies explicit root attributes and every required color token', () => {
    const resolved = resolveTheme(
      {
        themeMode: 'light',
        colorThemeId: 'nomo-amber-paper',
        documentStyleId: 'nomo-classic',
      },
      'dark',
    );
    applyResolvedTheme(resolved);

    expect(document.documentElement.dataset).toMatchObject({
      theme: 'light',
      themePreference: 'light',
      colorTheme: 'nomo-amber-paper',
      themeStyle: 'paper',
      documentStyle: 'nomo-classic',
      blockStyle: 'classic',
    });
    expect(document.documentElement.style.getPropertyValue('--md-editor-bg')).toBe('#F3F0E8');
    expect(document.documentElement.style.getPropertyValue('--md-editor-code-string')).not.toBe('');
    expect(document.documentElement.style.getPropertyValue('--md-editor-radius-md')).toBe('10px');
    expect(document.documentElement.style.getPropertyValue('--md-editor-font-document')).toBe(
      "'Segoe UI', 'Microsoft YaHei', sans-serif",
    );
  });

  it('round-trips a validated boot snapshot and ignores damaged snapshots', () => {
    const resolved = resolveTheme(
      {
        themeMode: 'dark',
        colorThemeId: 'nomo-amber-paper',
        documentStyleId: 'nomo-modern',
      },
      'light',
    );
    writeThemeBootSnapshot(resolved);
    expect(readThemeBootSnapshot()).toMatchObject({
      themeMode: 'dark',
      colorThemeId: 'nomo-amber-paper',
      effectiveScheme: 'dark',
      styleProfile: 'paper',
    });

    localStorage.setItem(
      THEME_BOOT_SNAPSHOT_KEY,
      '{"schemaVersion":2,"effectiveScheme":"dark","tokens":{},"styleTokens":{}}',
    );
    expect(readThemeBootSnapshot()).toBeNull();
    const fallback = bootstrapThemeFromSnapshot();
    expect(fallback.preferences.colorThemeId).toBe('nomo-default');
    expect(fallback.effectiveScheme).toBe('dark');
  });

  it('restores a valid v1 snapshot with registered v2 style tokens', () => {
    const resolved = resolveTheme(
      {
        themeMode: 'dark',
        colorThemeId: 'nomo-amber-paper',
        documentStyleId: 'nomo-modern',
      },
      'dark',
    );
    localStorage.setItem(
      LEGACY_THEME_BOOT_SNAPSHOT_KEY,
      JSON.stringify({
        schemaVersion: 1,
        themeVersion: resolved.themeVersion,
        themeMode: resolved.preferences.themeMode,
        colorThemeId: resolved.preferences.colorThemeId,
        documentStyleId: resolved.preferences.documentStyleId,
        effectiveScheme: resolved.effectiveScheme,
        tokens: resolved.tokens,
      }),
    );

    expect(readThemeBootSnapshot()).toMatchObject({
      schemaVersion: 2,
      colorThemeId: 'nomo-amber-paper',
      styleProfile: 'paper',
      styleTokens: resolved.styleTokens,
    });
    const restored = bootstrapThemeFromSnapshot();
    expect(restored.styleProfile).toBe('paper');
    expect(document.documentElement.dataset.themeStyle).toBe('paper');
  });

  it('updates the latest editor theme even when the background window never paints', async () => {
    // 模拟后台 WKWebView 不再执行 rAF，主题更新仍须在当前任务结束后完成。
    const pausedAnimationFrame = vi.fn(() => 1);
    vi.stubGlobal('requestAnimationFrame', pausedAnimationFrame);
    const editor = { updateTheme: vi.fn() };
    const withoutIcons = applyThemeRuntime(
      {
        themeMode: 'dark',
        colorThemeId: 'nomo-github',
        documentStyleId: 'nomo-modern',
      },
      { desktopEnabled: true, syncDesktopIcons: false, editor },
    );

    expect(withoutIcons.effectiveScheme).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(document.documentElement.dataset.colorTheme).toBe('nomo-github');
    expect(document.documentElement.style.colorScheme).toBe('dark');
    expect(document.documentElement.classList.contains('theme-transitioning')).toBe(false);
    expect(editor.updateTheme).not.toHaveBeenCalled();
    expect(setDesktopIconTheme).not.toHaveBeenCalled();

    applyThemeRuntime(
      {
        themeMode: 'light',
        colorThemeId: 'nomo-github',
        documentStyleId: 'nomo-modern',
      },
      { desktopEnabled: true, editor, transition: true },
    );

    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.style.colorScheme).toBe('light');
    expect(document.documentElement.classList.contains('theme-transitioning')).toBe(false);
    expect(editor.updateTheme).not.toHaveBeenCalled();
    expect(setDesktopIconTheme).not.toHaveBeenCalled();

    await Promise.resolve();

    expect(editor.updateTheme).toHaveBeenCalledTimes(1);
    expect(editor.updateTheme).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'light', colorThemeId: 'nomo-github' }),
    );
    expect(setDesktopIconTheme).toHaveBeenCalledWith(true, 'light', expect.any(String));
    expect(pausedAnimationFrame).not.toHaveBeenCalled();
  });

  it('does not apply queued effects after cancellation', async () => {
    const editor = { updateTheme: vi.fn() };
    applyThemeRuntime({ themeMode: 'dark' }, { editor, desktopEnabled: true });
    cancelPendingThemeEffects();
    await Promise.resolve();

    expect(editor.updateTheme).not.toHaveBeenCalled();
    expect(setDesktopIconTheme).not.toHaveBeenCalled();
  });

  it('falls back for invalid theme and document style identifiers', () => {
    const resolved = resolveTheme({
      themeMode: 'system',
      colorThemeId: 'not-installed',
      documentStyleId: 'not-installed',
    });

    expect(resolved.preferences).toMatchObject({
      colorThemeId: 'nomo-default',
      documentStyleId: 'nomo-modern',
    });
  });
});

import { cleanup, fireEvent, render } from '@testing-library/svelte/pure';
import { afterEach, describe, expect, it, vi } from 'vitest';

const desktop = vi.hoisted(() => ({ emit: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ emit: desktop.emit, listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../../lib/desktop/tauriStorage', async importOriginal => ({
  ...await importOriginal<typeof import('../../lib/desktop/tauriStorage')>(), isTauriRuntime: () => true,
}));
vi.mock('../services/settings', async importOriginal => {
  const actual = await importOriginal<typeof import('../services/settings')>();
  return { ...actual,
    loadAppPreferences: vi.fn().mockResolvedValue({ ...actual.DEFAULT_APP_PREFERENCES, themeMode: 'light' }),
    saveAppPreferences: vi.fn(async (_desktop, preferences) => preferences),
  };
});
vi.mock('../services/themeManager', async importOriginal => {
  const actual = await importOriginal<typeof import('../services/themeManager')>();
  return { ...actual, readEffectiveSystemScheme: async () => 'light',
    applyThemeRuntime: (preferences: Parameters<typeof actual.resolveTheme>[0]) => actual.resolveTheme(preferences, 'light'),
    writeThemeBootSnapshot() {}, listenForSystemThemeChanges: () => () => {},
  };
});
vi.mock('../services/userThemes', async importOriginal => ({
  ...await importOriginal<typeof import('../services/userThemes')>(),
  ensureUserThemesLoaded: async () => ({ failures: [] }),
}));
vi.mock('../services/softwareUpdate', async importOriginal => ({
  ...await importOriginal<typeof import('../services/softwareUpdate')>(),
  initializeSoftwareUpdateCoordinator: async () => {}, disposeSoftwareUpdateCoordinator: async () => {},
}));

import { saveAppPreferences, SETTINGS_UPDATED_EVENT } from '../services/settings';
import { t } from '../i18n';
import SettingsWindow from './SettingsWindow.svelte';

afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe('windowing percentage slider', () => {
  it('previews input without broadcasting or saving, then commits one percentage patch', async () => {
    const rendered = render(SettingsWindow);
    await fireEvent.click(rendered.getByRole('button', { name: t.settingsCategoryEditor() }));
    const slider = await rendered.findByRole('slider', { name: t.markdownWindowingThreshold() }) as HTMLInputElement;
    expect(slider.value).toBe('100');
    expect([slider.min, slider.max, slider.step]).toEqual(['75', '400', '25']);
    expect(rendered.container.querySelector('#largeDocumentLimit')).toBeNull();
    await fireEvent.input(slider, { target: { value: '200' } });
    expect(rendered.container.querySelector('output[for="markdownWindowingScalePercent"]')?.textContent).toBe('200%');
    await new Promise(resolve => setTimeout(resolve, 400));
    expect(desktop.emit).not.toHaveBeenCalled();
    expect(saveAppPreferences).not.toHaveBeenCalled();
    await fireEvent.change(slider);
    expect(desktop.emit).toHaveBeenCalledWith(SETTINGS_UPDATED_EVENT,
      expect.objectContaining({ patch: { markdownWindowingScalePercent: 200 } }));
    await vi.waitFor(() => expect(saveAppPreferences).toHaveBeenCalledWith(true,
      expect.objectContaining({ markdownWindowingScalePercent: 200 }), ['markdownWindowingScalePercent']));
    await fireEvent.change(slider, { target: { value: '175' } });
    expect(desktop.emit).toHaveBeenLastCalledWith(SETTINGS_UPDATED_EVENT,
      expect.objectContaining({ patch: { markdownWindowingScalePercent: 175 } }));
  });
});

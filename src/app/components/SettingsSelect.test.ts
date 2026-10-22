import { cleanup, fireEvent, render } from '@testing-library/svelte/pure';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SettingsSelect from './SettingsSelect.svelte';

const options = [
  { value: '', label: '跟随主题' },
  { value: 'Microsoft YaHei', label: 'Microsoft YaHei' },
  { value: 'Noto Sans SC', label: 'Noto Sans SC' },
];

function setup(editable = false, value = '') {
  const onValueChange = vi.fn();
  const onInputValue = vi.fn();
  const result = render(SettingsSelect, {
    id: 'font',
    label: '界面字体',
    value,
    options,
    editable,
    resetLabel: '恢复主题字体',
    onValueChange,
    onInputValue,
  });
  const control = result.getByRole('combobox', { name: '界面字体' });
  control.focus();
  return { ...result, control, onValueChange, onInputValue };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('settings dropdown interactions', () => {
  it('navigates without applying a value and cancels with Escape', async () => {
    const view = setup();
    await fireEvent.keyDown(view.control, { key: 'ArrowDown' });
    await fireEvent.keyDown(view.control, { key: 'ArrowDown' });
    expect(view.getByRole('option', { selected: true }).textContent).toContain('Microsoft YaHei');
    expect(view.onValueChange).not.toHaveBeenCalled();
    await fireEvent.keyDown(view.control, { key: 'Escape' });
    expect(view.queryByRole('listbox')).toBeNull();
    expect(document.activeElement).toBe(view.control);
    expect(view.onValueChange).not.toHaveBeenCalled();
  });

  it('commits a keyboard selection once and returns focus to the control', async () => {
    const view = setup();
    await fireEvent.keyDown(view.control, { key: 'End' });
    await fireEvent.keyDown(view.control, { key: 'Enter' });
    expect(view.onValueChange).toHaveBeenCalledTimes(1);
    expect(view.onValueChange).toHaveBeenCalledWith('Noto Sans SC');
    expect(view.queryByRole('listbox')).toBeNull();
    expect(document.activeElement).toBe(view.control);
  });

  it('filters font suggestions without applying partial input', async () => {
    const view = setup(true);
    await fireEvent.input(view.control, { target: { value: 'noto' } });
    expect(view.getAllByRole('option')).toHaveLength(1);
    expect(view.onInputValue).toHaveBeenCalledWith('noto');
    expect(view.onValueChange).not.toHaveBeenCalled();
    await fireEvent.click(view.getByRole('option', { name: 'Noto Sans SC' }));
    expect(view.onValueChange).toHaveBeenCalledTimes(1);
    expect(view.onValueChange).toHaveBeenCalledWith('Noto Sans SC');
    expect((view.control as HTMLInputElement).value).toBe('Noto Sans SC');
  });

  it('accepts a custom font on Enter and allows recovery after no suggestions match', async () => {
    const view = setup(true);
    await fireEvent.input(view.control, { target: { value: 'My Custom Font' } });
    expect(view.queryByRole('listbox')).toBeNull();
    expect(view.control.getAttribute('aria-expanded')).toBe('false');
    await fireEvent.keyDown(view.control, { key: 'Enter' });
    expect(view.onValueChange).toHaveBeenCalledTimes(1);
    expect(view.onValueChange).toHaveBeenCalledWith('My Custom Font');
    await fireEvent.input(view.control, { target: { value: 'Microsoft' } });
    expect(view.getByRole('option', { name: 'Microsoft YaHei' })).toBeTruthy();
  });

  it('does not intercept IME confirmation or apply composition input', async () => {
    const view = setup(true);
    await fireEvent.input(view.control, { target: { value: 'Microsoft' }, isComposing: true });
    await fireEvent.keyDown(view.control, { key: 'Enter', isComposing: true });
    expect(view.onValueChange).not.toHaveBeenCalled();
    expect(view.onInputValue).not.toHaveBeenCalled();
    await fireEvent.input(view.control, {
      target: { value: 'Microsoft YaHei' },
      isComposing: false,
    });
    expect(view.onInputValue).toHaveBeenCalledWith('Microsoft YaHei');
  });

  it('retains custom text editing keys and commits on blur', async () => {
    const view = setup(true);
    await fireEvent.input(view.control, { target: { value: 'Custom' } });
    expect(await fireEvent.keyDown(view.control, { key: 'Home' })).toBe(true);
    expect(await fireEvent.keyDown(view.control, { key: ' ' })).toBe(true);
    await fireEvent.blur(view.control);
    expect(view.onValueChange).toHaveBeenCalledTimes(1);
    expect(view.onValueChange).toHaveBeenCalledWith('Custom');
  });

  it('can restore the theme default from the same option list or the reset action', async () => {
    const view = setup(true, 'Microsoft YaHei');
    await fireEvent.click(view.control);
    await fireEvent.click(view.getByRole('option', { name: '跟随主题' }));
    expect(view.onValueChange).toHaveBeenCalledTimes(1);
    expect(view.onValueChange).toHaveBeenCalledWith('');
    await view.rerender({ value: 'Noto Sans SC' });
    await fireEvent.click(view.getByRole('button', { name: '恢复主题字体' }));
    expect(view.onValueChange).toHaveBeenCalledTimes(2);
    expect(view.onValueChange).toHaveBeenLastCalledWith('');
  });

  it('dismisses on Tab, outside interaction and parent scrolling, but not list scrolling', async () => {
    const view = setup();
    await fireEvent.click(view.control);
    await fireEvent.scroll(view.getByRole('listbox'));
    expect(view.getByRole('listbox')).toBeTruthy();
    await fireEvent.keyDown(view.control, { key: 'Tab' });
    expect(view.queryByRole('listbox')).toBeNull();
    await fireEvent.click(view.control);
    await fireEvent.pointerDown(document.body);
    expect(view.queryByRole('listbox')).toBeNull();
    await fireEvent.click(view.control);
    await fireEvent.scroll(document);
    expect(view.queryByRole('listbox')).toBeNull();
    expect(view.onValueChange).not.toHaveBeenCalled();
  });

  it('cleans up popup listeners when its settings category unmounts', async () => {
    const view = setup();
    const removeDocumentListener = vi.spyOn(document, 'removeEventListener');
    const removeWindowListener = vi.spyOn(window, 'removeEventListener');
    await fireEvent.click(view.control);
    view.unmount();
    expect(removeDocumentListener).toHaveBeenCalledWith('pointerdown', expect.any(Function), true);
    expect(removeDocumentListener).toHaveBeenCalledWith('scroll', expect.any(Function), true);
    expect(removeWindowListener).toHaveBeenCalledWith('resize', expect.any(Function));
  });
});

// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';

it('验收探针在组字未收到结束事件后仍记录非组字输入', async () => {
  vi.stubEnv('VITE_NOMO_ACCEPTANCE', '1');
  vi.resetModules();
  const editor = document.body.appendChild(document.createElement('div'));
  editor.className = 'ProseMirror';
  const { installAcceptanceProbe } = await import('./acceptanceProbe');
  installAcceptanceProbe();
  try {
    editor.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    editor.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, isComposing: true }));
    expect(window.__nomoAcceptance?.events.filter(event => event.kind === 'input')).toHaveLength(0);
    editor.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, isComposing: false }));
    editor.textContent = '字';
    await Promise.resolve();
    expect(window.__nomoAcceptance?.events.filter(event => event.kind === 'input')).toHaveLength(1);
    expect(window.__nomoAcceptance?.events.filter(event => event.kind === 'input-dom')).toHaveLength(1);
  } finally {
    window.dispatchEvent(new Event('beforeunload'));
    delete window.__nomoAcceptance;
    editor.remove();
    vi.unstubAllEnvs();
  }
});

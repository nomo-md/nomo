import { afterEach, expect, it } from 'vitest';
import { createEditorCore } from './createEditorCore';
import { getTypographyOptions, setTypographyOptions } from '../typography/options';
import { cleanEditorArtifacts } from '../../app/services/exportService';
import type { EditorChangeEvent } from './types';

let core: ReturnType<typeof createEditorCore> | undefined;
afterEach(() => {
  core?.destroy();
  core = undefined;
  document.body.replaceChildren();
});

it('exports current source even when the semantic view has not caught up', () => {
  const host = document.createElement('div');
  document.body.append(host);
  core = createEditorCore({ target: host, markdown: 'Old ![old](old.png)' });
  core.updateOptions({ mode: 'source' });
  core.setMarkdown('Latest ![new](new.png)', { sourceInput: true });
  const html = core.getExportHtml!();
  expect(html).toContain('Latest');
  expect(html).toContain('new.png');
  expect(html).not.toContain('old.png');
});

it('typography toggles preserve exact Markdown, dirty state and semantic export', () => {
  const original = 'hello\nworld\n\n中文\n换行 **粗体** 和 $x^2$。\n';
  const host = document.createElement('div');
  document.body.append(host);
  core = createEditorCore({ target: host, markdown: original });
  const before = core.getSnapshot();
  const events: EditorChangeEvent[] = [];
  const unsubscribe = core.subscribe((event) => events.push(event));
  const previous = getTypographyOptions();
  setTypographyOptions({ enabled: !previous.enabled });
  setTypographyOptions(previous);
  expect(core.flushMarkdown()).toBe(original);
  expect(core.getSnapshot().version).toBe(before.version);
  expect(events.every((event) => event.dirty === false)).toBe(true);
  expect(core.execute({ type: 'undo' })).toBe(false);
  expect(core.flushMarkdown()).toBe(original);
  unsubscribe();
  const exported = cleanEditorArtifacts(core.getExportHtml!());
  const doc = new DOMParser().parseFromString(exported, 'text/html');
  expect(doc.querySelectorAll('br[data-nomo-break="soft"]')).toHaveLength(2);
  expect(doc.querySelector('strong')?.textContent).toBe('粗体');
  expect(doc.querySelector('.math-inline .katex')).not.toBeNull();
  expect(doc.querySelector('.kp-unit,[data-kp-owned]')).toBeNull();
});

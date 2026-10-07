import { cleanup, fireEvent, render } from '@testing-library/svelte/pure';
import { afterEach, describe, expect, it } from 'vitest';
import { createMarkdownLintState } from '../../lib/markdown-lint/types';
import { t } from '../i18n';
import StatusBar from './StatusBar.svelte';

afterEach(cleanup);
const props = {
  interfaceLocale: 'zh-CN' as const,
  writingStatsVisible: true,
  readingTimeVisible: true,
  stats: {
    chars: 1234,
    words: 1234,
    visibleChars: 1234,
    lines: 1234,
    headings: 0,
    readingMinutes: 5,
  },
  zoomPercent: 100,
  markdownLintEnabled: false,
  markdownLintRuleSet: 'relaxed' as const,
  markdownLintState: createMarkdownLintState('disabled'),
};

describe('writing statistics availability', () => {
  it.each(['pending', 'error'] as const)(
    'hides old document numbers when statistics are %s',
    async (statsStatus) => {
      const rendered = render(StatusBar, { props: { ...props, statsStatus } });
      const trigger = rendered.container.querySelector('.statusbar-stats-trigger')!;
      expect(trigger.textContent?.trim()).toBe(
        statsStatus === 'error' ? t.writingStatsUnavailable() : `—${t.wordUnit()}`,
      );
      await fireEvent.click(trigger);
      expect(rendered.container.querySelector('.reading-time')).toBeNull();
      const values = [...rendered.container.querySelectorAll('.writing-stats-option strong')];
      expect(values).toHaveLength(4);
      expect(values.every((value) => value.textContent === '—')).toBe(true);
      await rendered.rerender({ ...props, statsStatus: 'ready' });
      expect(trigger.textContent).toContain('1234');
    },
  );
});

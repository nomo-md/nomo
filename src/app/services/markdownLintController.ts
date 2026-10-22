import type {
  MarkdownLintInput,
  MarkdownLintState,
  MarkdownLintWorkerRequest,
  MarkdownLintWorkerResponse,
} from '../../lib/markdown-lint/types';
import { createMarkdownLintState } from '../../lib/markdown-lint/types';

export const MARKDOWN_LINT_MAX_CHARACTERS = 500_000;

export interface MarkdownLintController {
  schedule(input: MarkdownLintInput, delayMs: number): void;
  checkNow(input: MarkdownLintInput): void;
  disable(): void;
  destroy(): void;
}

export function createMarkdownLintController(
  onStateChange: (state: MarkdownLintState) => void,
): MarkdownLintController {
  let worker: Worker | null = null;
  let timer: number | null = null;
  let requestId = 0;
  let latestRequestId = 0;
  let destroyed = false;
  let latestInput: MarkdownLintInput | null = null;

  function clearTimer() {
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
  }

  function terminateWorker() {
    worker?.terminate();
    worker = null;
  }

  function ensureWorker(): Worker {
    if (worker) return worker;
    const nextWorker = new Worker(new URL('../workers/markdownLint.worker.ts', import.meta.url), {
      type: 'module',
    });
    nextWorker.onmessage = (event: MessageEvent<MarkdownLintWorkerResponse>) => {
      if (destroyed || event.data.requestId !== latestRequestId) return;
      if ('error' in event.data) {
        terminateWorker();
        onStateChange({
          ...createMarkdownLintState('failed', event.data),
          failureMessage: event.data.error,
        });
        return;
      }
      onStateChange({
        ...createMarkdownLintState(event.data.total > 0 ? 'issues' : 'clean', event.data),
        issues: event.data.issues,
        total: event.data.total,
        checkedAt: event.data.checkedAt,
      });
    };
    nextWorker.onerror = (event) => {
      if (destroyed) return;
      event.preventDefault();
      terminateWorker();
      onStateChange({
        ...createMarkdownLintState('failed', latestInput ?? undefined),
        failureMessage: event.message || 'Markdown lint worker failed',
      });
    };
    worker = nextWorker;
    return nextWorker;
  }

  function prepare(input: MarkdownLintInput): number | null {
    clearTimer();
    latestInput = input;
    latestRequestId = ++requestId;
    if (input.largeDocumentMode || input.markdown.length > MARKDOWN_LINT_MAX_CHARACTERS) {
      onStateChange(createMarkdownLintState('skipped', input));
      return null;
    }
    onStateChange(createMarkdownLintState('checking', input));
    return latestRequestId;
  }

  function dispatch(input: MarkdownLintInput, preparedRequestId: number) {
    if (destroyed || preparedRequestId !== latestRequestId) return;
    const request: MarkdownLintWorkerRequest = {
      requestId: preparedRequestId,
      tabId: input.tabId,
      version: input.version,
      markdown: input.markdown,
      ruleSet: input.ruleSet,
    };
    ensureWorker().postMessage(request);
  }

  return {
    schedule(input, delayMs) {
      if (destroyed) return;
      const preparedRequestId = prepare(input);
      if (preparedRequestId === null) return;
      timer = window.setTimeout(() => {
        timer = null;
        dispatch(input, preparedRequestId);
      }, delayMs);
    },
    checkNow(input) {
      if (destroyed) return;
      const preparedRequestId = prepare(input);
      if (preparedRequestId !== null) dispatch(input, preparedRequestId);
    },
    disable() {
      clearTimer();
      latestRequestId = ++requestId;
      terminateWorker();
      onStateChange(createMarkdownLintState('disabled'));
    },
    destroy() {
      destroyed = true;
      clearTimer();
      terminateWorker();
    },
  };
}

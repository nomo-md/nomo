export type MarkdownLintRuleSet = 'relaxed' | 'default';

export type MarkdownLintStatus =
  | 'disabled'
  | 'checking'
  | 'clean'
  | 'issues'
  | 'skipped'
  | 'failed';

export interface MarkdownLintIssue {
  ruleId: string;
  ruleDescription: string;
  lineNumber: number;
  columnNumber: number | null;
  rangeLength: number | null;
  errorDetail: string | null;
  errorContext: string | null;
}

export interface MarkdownLintState {
  status: MarkdownLintStatus;
  ruleSet: MarkdownLintRuleSet;
  tabId: string;
  version: number;
  issues: MarkdownLintIssue[];
  total: number;
  checkedAt: number | null;
  failureMessage: string;
}

export interface MarkdownLintInput {
  tabId: string;
  version: number;
  markdown: string;
  ruleSet: MarkdownLintRuleSet;
  largeDocumentMode: boolean;
}

export interface MarkdownLintWorkerRequest {
  requestId: number;
  tabId: string;
  version: number;
  markdown: string;
  ruleSet: MarkdownLintRuleSet;
}

export type MarkdownLintWorkerResponse =
  | {
      requestId: number;
      tabId: string;
      version: number;
      ruleSet: MarkdownLintRuleSet;
      issues: MarkdownLintIssue[];
      total: number;
      checkedAt: number;
    }
  | {
      requestId: number;
      tabId: string;
      version: number;
      ruleSet: MarkdownLintRuleSet;
      error: string;
    };

export function createMarkdownLintState(
  status: MarkdownLintStatus,
  input?: Pick<MarkdownLintInput, 'tabId' | 'version' | 'ruleSet'>,
): MarkdownLintState {
  return {
    status,
    ruleSet: input?.ruleSet ?? 'relaxed',
    tabId: input?.tabId ?? '',
    version: input?.version ?? 0,
    issues: [],
    total: 0,
    checkedAt: null,
    failureMessage: '',
  };
}

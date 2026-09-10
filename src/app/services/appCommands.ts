import { isDiagramType, type EditorCommand } from '../../lib/editor-core';
import type { EditorViewMode } from '../types';
import {
  DEFAULT_SHORTCUT_PREFERENCES,
  type ShortcutCommandId,
  type ShortcutPreferences,
} from './settings';
import { t } from '../i18n';

export interface AppCommandHandlers {
  createNewFile: () => void;
  createNewWindow: () => void;
  openFileDialog: () => void;
  openFolderDialog: () => void;
  openRecentEntry: (path: string, entryType: 'file' | 'folder') => void;
  saveMarkdownFile: (saveAs?: boolean) => void;
  closeCurrentFile: () => void;
  closeCurrentWindow: () => void;
  runCommand: (command: EditorCommand) => void;
  openTablePicker: () => void;
  openLinkPicker: () => void;
  openSearchPanel: (replaceVisible?: boolean) => void;
  closeSearchPanel: () => void;
  getSearchState: () => { open: boolean; replaceVisible: boolean };
  openSettings: () => void;
  editFrontMatter: () => void;
  showUnavailableFeature: (featureName: string) => void;
  setMode: (mode: EditorViewMode) => void;
  getMode: () => EditorViewMode;
  toggleTheme: () => void;
  toggleFocusMode: () => void;
  toggleToolbar?: () => void;
  toggleMarkdownMini?: () => void;
  toggleOutlineVisible: () => void;
  switchToNextTab: () => void;
  switchToPrevTab: () => void;
  getDefaultCodeBlockLanguage: () => string;
  getDefaultDiagramType: () => Parameters<typeof isDiagramType>[0];
  exportHtml: () => void;
  exportPdf: () => void;
}

type ShortcutParts = {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
};

const shortcutCommandOrder = Object.keys(DEFAULT_SHORTCUT_PREFERENCES) as ShortcutCommandId[];

export function executeDesktopCommand(command: string, handlers: AppCommandHandlers) {
  if (command === 'new-file') {
    handlers.createNewFile();
  } else if (command === 'new-window') {
    handlers.createNewWindow();
  } else if (command === 'open-file') {
    handlers.openFileDialog();
  } else if (command === 'open-directory') {
    handlers.openFolderDialog();
  } else if (command.startsWith('open-recent:')) {
    const payload = command.slice('open-recent:'.length);
    if (payload.startsWith('file:')) {
      handlers.openRecentEntry(payload.slice('file:'.length), 'file');
    } else if (payload.startsWith('folder:')) {
      handlers.openRecentEntry(payload.slice('folder:'.length), 'folder');
    } else {
      // 兼容旧格式，默认当作文件处理
      handlers.openRecentEntry(payload, 'file');
    }
  } else if (command === 'save-file') {
    handlers.saveMarkdownFile();
  } else if (command === 'save-file-as') {
    handlers.saveMarkdownFile(true);
  } else if (command === 'close-current-file') {
    handlers.closeCurrentFile();
  } else if (command === 'close-current-window') {
    handlers.closeCurrentWindow();
  } else if (command === 'undo') {
    handlers.runCommand({ type: 'undo' });
  } else if (command === 'redo') {
    handlers.runCommand({ type: 'redo' });
  } else if (command === 'toggle-blockquote') {
    handlers.runCommand({ type: 'toggleBlockquote' });
  } else if (command === 'insert-callout') {
    handlers.runCommand({ type: 'insertCallout' });
  } else if (command === 'toggle-ordered-list') {
    handlers.runCommand({ type: 'toggleOrderedList' });
  } else if (command === 'toggle-bullet-list') {
    handlers.runCommand({ type: 'toggleBulletList' });
  } else if (command === 'toggle-task-list') {
    handlers.runCommand({ type: 'toggleTaskList' });
  } else if (command === 'insert-table') {
    handlers.openTablePicker();
  } else if (command === 'insert-math-block') {
    handlers.runCommand({ type: 'insertMathBlock', tex: '' });
  } else if (command === 'insert-code-block') {
    handlers.runCommand({
      type: 'insertCodeBlock',
      language: handlers.getDefaultCodeBlockLanguage(),
    });
  } else if (command.startsWith('insert-diagram:')) {
    const diagramType = command.slice('insert-diagram:'.length);
    if (isDiagramType(diagramType)) {
      handlers.runCommand({ type: 'insertDiagramBlock', diagramType });
    }
  } else if (command === 'set-heading-1') {
    handlers.runCommand({ type: 'setHeading', level: 1 });
  } else if (command === 'set-heading-2') {
    handlers.runCommand({ type: 'setHeading', level: 2 });
  } else if (command === 'set-heading-3') {
    handlers.runCommand({ type: 'setHeading', level: 3 });
  } else if (command === 'set-heading-4') {
    handlers.runCommand({ type: 'setHeading', level: 4 });
  } else if (command === 'set-heading-5') {
    handlers.runCommand({ type: 'setHeading', level: 5 });
  } else if (command === 'set-heading-6') {
    handlers.runCommand({ type: 'setHeading', level: 6 });
  } else if (command === 'set-paragraph') {
    handlers.runCommand({ type: 'setParagraph' });
  } else if (command === 'toggle-bold') {
    handlers.runCommand({ type: 'toggleBold' });
  } else if (command === 'toggle-italic') {
    handlers.runCommand({ type: 'toggleItalic' });
  } else if (command === 'toggle-inline-code') {
    handlers.runCommand({ type: 'toggleCode' });
  } else if (command === 'toggle-strikethrough') {
    handlers.runCommand({ type: 'toggleStrikethrough' });
  } else if (command === 'toggle-underline') {
    handlers.runCommand({ type: 'toggleUnderline' });
  } else if (command === 'menu-heading-up') {
    handlers.runCommand({ type: 'increaseHeadingLevel' });
  } else if (command === 'menu-heading-down') {
    handlers.runCommand({ type: 'decreaseHeadingLevel' });
  } else if (command === 'menu-insert-paragraph-before') {
    handlers.runCommand({ type: 'insertParagraphBefore' });
  } else if (command === 'menu-insert-paragraph-after') {
    handlers.runCommand({ type: 'insertParagraphAfter' });
  } else if (command === 'menu-chart') {
    handlers.runCommand({ type: 'insertMermaidBlock' });
  } else if (command.startsWith('menu-chart:')) {
    const diagramType = command.slice('menu-chart:'.length);
    if (isDiagramType(diagramType)) {
      handlers.runCommand({ type: 'insertDiagramBlock', diagramType });
    }
  } else if (command === 'menu-footnote') {
    handlers.runCommand({ type: 'insertFootnote' });
  } else if (command === 'menu-horizontal-rule') {
    handlers.runCommand({ type: 'insertHorizontalRule' });
  } else if (command === 'menu-content-directory') {
    handlers.runCommand({ type: 'insertToc' });
  } else if (command === 'menu-yaml-front-matter') {
    handlers.editFrontMatter();
  } else if (command === 'menu-underline') {
    handlers.runCommand({ type: 'toggleUnderline' });
  } else if (command === 'menu-inline-math') {
    handlers.showUnavailableFeature(t.inlineMath());
  } else if (command === 'menu-strikethrough') {
    handlers.runCommand({ type: 'toggleStrikethrough' });
  } else if (command === 'menu-highlight') {
    handlers.runCommand({ type: 'toggleHighlight' });
  } else if (command === 'menu-comment') {
    handlers.runCommand({ type: 'insertCommentInline' });
  } else if (command === 'menu-comment-block') {
    handlers.runCommand({ type: 'insertCommentBlock' });
  } else if (command === 'menu-link') {
    handlers.openLinkPicker();
  } else if (command === 'menu-image') {
    handlers.showUnavailableFeature(t.imageMenu());
  } else if (command === 'menu-clear-format') {
    handlers.runCommand({ type: 'clearInlineStyles' });
  } else if (command === 'toggle-source') {
    handlers.setMode(handlers.getMode() === 'source' ? 'semantic' : 'source');
  } else if (command === 'toggle-outline') {
    handlers.toggleOutlineVisible();
  } else if (command === 'toggle-theme') {
    handlers.toggleTheme();
  } else if (command === 'toggle-focus') {
    handlers.toggleFocusMode();
  } else if (command === 'toggle-toolbar') {
    handlers.toggleToolbar?.();
  } else if (command === 'toggle-markdown-mini') {
    handlers.toggleMarkdownMini?.();
  } else if (command === 'export-html') {
    handlers.exportHtml();
  } else if (command === 'export-pdf') {
    handlers.exportPdf();
  } else if (command === 'open-settings') {
    handlers.openSettings();
  }
}

export function handleGlobalShortcut(
  event: KeyboardEvent,
  handlers: AppCommandHandlers,
  shortcuts: Partial<ShortcutPreferences> = DEFAULT_SHORTCUT_PREFERENCES,
) {
  if (event.defaultPrevented) {
    return;
  }

  if (event.ctrlKey && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'w') {
    event.preventDefault();
    handlers.closeCurrentFile();
    return;
  }

  if (
    event.ctrlKey &&
    !event.shiftKey &&
    !event.altKey &&
    (event.code === 'Comma' || event.key === ',')
  ) {
    event.preventDefault();
    handlers.openSettings();
    return;
  }

  const customCommand = findShortcutCommand(event, shortcuts);
  if (customCommand) {
    event.preventDefault();
    executeDesktopCommand(customCommand, handlers);
    return;
  }

  if (!event.ctrlKey || event.altKey) {
    return;
  }

  const key = event.key.toLowerCase();
  if (key === 'n' && !event.shiftKey) {
    event.preventDefault();
    handlers.createNewFile();
  } else if (key === 'n' && event.shiftKey) {
    event.preventDefault();
    handlers.createNewWindow();
  } else if (key === 'o' && !event.shiftKey) {
    event.preventDefault();
    handlers.openFileDialog();
  } else if (key === 'o' && event.shiftKey) {
    event.preventDefault();
    handlers.openFolderDialog();
  } else if (key === 's') {
    event.preventDefault();
    handlers.saveMarkdownFile(event.shiftKey);
  } else if (key === 'k' && !event.shiftKey) {
    event.preventDefault();
    handlers.openLinkPicker();
  } else if (key === 'f' && !event.shiftKey) {
    event.preventDefault();
    const state = handlers.getSearchState();
    if (state.open && state.replaceVisible) {
      // 替换可见时切回纯搜索
      handlers.openSearchPanel(false);
    } else if (state.open && !state.replaceVisible) {
      handlers.closeSearchPanel();
    } else {
      handlers.openSearchPanel(false);
    }
  } else if (key === 'h' && !event.shiftKey) {
    event.preventDefault();
    const state = handlers.getSearchState();
    if (state.open && state.replaceVisible) {
      handlers.closeSearchPanel();
    } else {
      handlers.openSearchPanel(true);
    }
  } else if (key === 'e') {
    event.preventDefault();
    handlers.setMode(handlers.getMode() === 'source' ? 'semantic' : 'source');
  } else if (key === 'l' && event.shiftKey) {
    event.preventDefault();
    handlers.toggleTheme();
  } else if (key === 'f' && event.shiftKey) {
    event.preventDefault();
    handlers.toggleFocusMode();
  } else if (key === 'tab') {
    event.preventDefault();
    if (event.shiftKey) {
      handlers.switchToPrevTab();
    } else {
      handlers.switchToNextTab();
    }
  } else if (['1', '2', '3', '4', '5', '6'].includes(key)) {
    event.preventDefault();
    handlers.runCommand({ type: 'setHeading', level: Number(key) as 1 | 2 | 3 | 4 | 5 | 6 });
  } else if (key === '0') {
    event.preventDefault();
    handlers.runCommand({ type: 'setParagraph' });
  } else if (event.code === 'Backslash') {
    event.preventDefault();
    handlers.runCommand({ type: 'clearInlineStyles' });
  } else if (key === '=' || key === '+') {
    event.preventDefault();
    handlers.runCommand({ type: 'increaseHeadingLevel' });
  } else if (key === '-') {
    event.preventDefault();
    handlers.runCommand({ type: 'decreaseHeadingLevel' });
  }
  // Ctrl+Shift 快捷键：用 event.code 按物理按键匹配，避免 Shift 影响 event.key
  if (!event.shiftKey) return;

  const code = event.code;
  if (code === 'BracketLeft') {
    // Ctrl+Shift+[ → 有序列表
    event.preventDefault();
    handlers.runCommand({ type: 'toggleOrderedList' });
  } else if (code === 'BracketRight') {
    // Ctrl+Shift+] → 无序列表
    event.preventDefault();
    handlers.runCommand({ type: 'toggleBulletList' });
  } else if (key === 'x') {
    // Ctrl+Shift+X → 任务列表
    event.preventDefault();
    handlers.runCommand({ type: 'toggleTaskList' });
  } else if (key === 'q') {
    // Ctrl+Shift+Q → 引用块
    event.preventDefault();
    handlers.runCommand({ type: 'toggleBlockquote' });
  } else if (key === 't') {
    // Ctrl+Shift+T → 表格尺寸选择
    event.preventDefault();
    handlers.openTablePicker();
  } else if (key === 'm') {
    // Ctrl+Shift+M → 公式块
    event.preventDefault();
    handlers.runCommand({ type: 'insertMathBlock', tex: '' });
  } else if (key === 'k') {
    // Ctrl+Shift+K → 代码块
    event.preventDefault();
    handlers.runCommand({
      type: 'insertCodeBlock',
      language: handlers.getDefaultCodeBlockLanguage(),
    });
  } else if (code === 'KeyH') {
    // Ctrl+Shift+H → 水平分割线
    event.preventDefault();
    handlers.runCommand({ type: 'insertHorizontalRule' });
  }
}

function findShortcutCommand(
  event: KeyboardEvent,
  shortcuts: Partial<ShortcutPreferences>,
): ShortcutCommandId | null {
  for (const commandId of shortcutCommandOrder) {
    if (matchesShortcut(event, shortcuts[commandId] ?? DEFAULT_SHORTCUT_PREFERENCES[commandId])) {
      return commandId;
    }
  }
  return null;
}

function matchesShortcut(event: KeyboardEvent, shortcut: string): boolean {
  const parsed = parseShortcut(shortcut);
  if (!parsed) {
    return false;
  }
  return (
    event.ctrlKey === parsed.ctrl &&
    event.shiftKey === parsed.shift &&
    event.altKey === parsed.alt &&
    normalizeEventKey(event) === parsed.key
  );
}

function parseShortcut(shortcut: string): ShortcutParts | null {
  const parts = shortcut
    .split('+')
    .map((part) => part.trim())
    .filter(Boolean);
  const key = parts.pop();
  if (!key) {
    return null;
  }
  return {
    ctrl: parts.some((part) => /^ctrl$/i.test(part)),
    shift: parts.some((part) => /^shift$/i.test(part)),
    alt: parts.some((part) => /^alt$/i.test(part)),
    key: normalizeShortcutKey(key),
  };
}

function normalizeEventKey(event: KeyboardEvent): string {
  if (event.code === 'Backslash') {
    return '\\';
  }
  if (event.code === 'BracketLeft') {
    return '[';
  }
  if (event.code === 'BracketRight') {
    return ']';
  }
  if (event.key === ' ') {
    return 'space';
  }
  return normalizeShortcutKey(event.key);
}

function normalizeShortcutKey(key: string): string {
  return key.trim().toLowerCase().replace(/^key/, '');
}

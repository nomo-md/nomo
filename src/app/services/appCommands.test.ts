import { describe, expect, it, vi } from 'vitest';
import type { EditorCommand, EditorMode } from '../../lib/editor-core';
import {
  executeDesktopCommand,
  handleGlobalShortcut,
  type AppCommandHandlers,
} from './appCommands';

function createHandlers(): AppCommandHandlers & { commands: EditorCommand[] } {
  const commands: EditorCommand[] = [];
  return {
    commands,
    createNewFile: vi.fn(),
    createNewWindow: vi.fn(),
    openFileDialog: vi.fn(),
    openFolderDialog: vi.fn(),
    openRecentEntry: vi.fn(),
    saveMarkdownFile: vi.fn(),
    closeCurrentFile: vi.fn(),
    closeCurrentWindow: vi.fn(),
    runCommand: vi.fn((command: EditorCommand) => {
      commands.push(command);
    }),
    openTablePicker: vi.fn(),
    openLinkPicker: vi.fn(),
    openSearchPanel: vi.fn(),
    closeSearchPanel: vi.fn(),
    getSearchState: vi.fn(() => ({ open: false, replaceVisible: false })),
    openSettings: vi.fn(),
    editFrontMatter: vi.fn(),
    showUnavailableFeature: vi.fn(),
    setMode: vi.fn(),
    getMode: vi.fn((): EditorMode => 'semantic'),
    toggleTheme: vi.fn(),
    toggleFocusMode: vi.fn(),
    toggleOutlineVisible: vi.fn(),
    switchToNextTab: vi.fn(),
    switchToPrevTab: vi.fn(),
    getDefaultCodeBlockLanguage: vi.fn(() => 'ts'),
    getDefaultDiagramType: vi.fn(() => 'flowchart'),
    exportHtml: vi.fn(),
    exportPdf: vi.fn(),
  };
}

describe('appCommands', () => {
  it('通过桌面菜单命令新建窗口', () => {
    const handlers = createHandlers();

    executeDesktopCommand('new-window', handlers);

    expect(handlers.createNewWindow).toHaveBeenCalledTimes(1);
  });

  it('通过桌面菜单命令打开最近文件', () => {
    const handlers = createHandlers();

    executeDesktopCommand('open-recent:file:D:\\Docs\\demo.md', handlers);

    expect(handlers.openRecentEntry).toHaveBeenCalledWith('D:\\Docs\\demo.md', 'file');
  });

  it('通过桌面菜单命令打开最近文件夹', () => {
    const handlers = createHandlers();

    executeDesktopCommand('open-recent:folder:D:\\Docs', handlers);

    expect(handlers.openRecentEntry).toHaveBeenCalledWith('D:\\Docs', 'folder');
  });

  it('兼容旧格式打开最近文件', () => {
    const handlers = createHandlers();

    executeDesktopCommand('open-recent:D:\\Docs\\demo.md', handlers);

    expect(handlers.openRecentEntry).toHaveBeenCalledWith('D:\\Docs\\demo.md', 'file');
  });

  it('通过桌面菜单命令触发清除样式', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-clear-format', handlers);

    expect(handlers.commands).toEqual([{ type: 'clearInlineStyles' }]);
    expect(handlers.showUnavailableFeature).not.toHaveBeenCalled();
  });

  it('通过桌面菜单命令关闭当前文件和窗口', () => {
    const handlers = createHandlers();

    executeDesktopCommand('close-current-file', handlers);
    executeDesktopCommand('close-current-window', handlers);

    expect(handlers.closeCurrentFile).toHaveBeenCalledTimes(1);
    expect(handlers.closeCurrentWindow).toHaveBeenCalledTimes(1);
  });

  it('通过桌面菜单命令打开偏好设置', () => {
    const handlers = createHandlers();

    executeDesktopCommand('open-settings', handlers);

    expect(handlers.openSettings).toHaveBeenCalledTimes(1);
  });

  it('通过 Ctrl + , 打开偏好设置', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: ',',
      code: 'Comma',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openSettings).toHaveBeenCalledTimes(1);
  });

  it('通过桌面菜单命令触发段落列表和提示块', () => {
    const handlers = createHandlers();

    executeDesktopCommand('insert-callout', handlers);
    executeDesktopCommand('toggle-ordered-list', handlers);
    executeDesktopCommand('toggle-bullet-list', handlers);
    executeDesktopCommand('toggle-task-list', handlers);

    expect(handlers.commands).toEqual([
      { type: 'insertCallout' },
      { type: 'toggleOrderedList' },
      { type: 'toggleBulletList' },
      { type: 'toggleTaskList' },
    ]);
  });

  it('通过桌面图表菜单插入空白图表，模板命令才插入示例图表', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-chart', handlers);
    executeDesktopCommand('menu-chart:flowchart', handlers);

    expect(handlers.commands).toEqual([
      { type: 'insertMermaidBlock' },
      { type: 'insertDiagramBlock', diagramType: 'flowchart' },
    ]);
  });

  it('通过桌面菜单命令触发高亮', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-highlight', handlers);

    expect(handlers.commands).toEqual([{ type: 'toggleHighlight' }]);
    expect(handlers.showUnavailableFeature).not.toHaveBeenCalled();
  });

  it('通过桌面菜单命令打开超链接编辑器', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-link', handlers);

    expect(handlers.openLinkPicker).toHaveBeenCalledTimes(1);
    expect(handlers.showUnavailableFeature).not.toHaveBeenCalled();
  });

  it('通过桌面菜单命令插入行内注释', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-comment', handlers);

    expect(handlers.commands).toEqual([{ type: 'insertCommentInline' }]);
    expect(handlers.showUnavailableFeature).not.toHaveBeenCalled();
  });

  it('通过桌面菜单命令插入注释块', () => {
    const handlers = createHandlers();

    executeDesktopCommand('menu-comment-block', handlers);

    expect(handlers.commands).toEqual([{ type: 'insertCommentBlock' }]);
    expect(handlers.showUnavailableFeature).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + K 打开超链接编辑器', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'k',
      code: 'KeyK',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openLinkPicker).toHaveBeenCalledTimes(1);
    expect(handlers.commands).toEqual([]);
  });

  it('通过 Ctrl + F 打开搜索面板（面板关闭时）', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'f',
      code: 'KeyF',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openSearchPanel).toHaveBeenCalledWith(false);
    expect(handlers.closeSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + F 关闭搜索面板（纯搜索已打开时）', () => {
    const handlers = createHandlers();
    handlers.getSearchState = vi.fn(() => ({ open: true, replaceVisible: false }));
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'f',
      code: 'KeyF',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.closeSearchPanel).toHaveBeenCalledTimes(1);
    expect(handlers.openSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + F 从替换切回纯搜索（替换可见时）', () => {
    const handlers = createHandlers();
    handlers.getSearchState = vi.fn(() => ({ open: true, replaceVisible: true }));
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'f',
      code: 'KeyF',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openSearchPanel).toHaveBeenCalledWith(false);
    expect(handlers.closeSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + H 打开替换面板（面板关闭时）', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'h',
      code: 'KeyH',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openSearchPanel).toHaveBeenCalledWith(true);
    expect(handlers.closeSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + H 切换到替换模式（搜索已打开但替换不可见时）', () => {
    const handlers = createHandlers();
    handlers.getSearchState = vi.fn(() => ({ open: true, replaceVisible: false }));
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'h',
      code: 'KeyH',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.openSearchPanel).toHaveBeenCalledWith(true);
    expect(handlers.closeSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + H 关闭替换面板（替换已可见时）', () => {
    const handlers = createHandlers();
    handlers.getSearchState = vi.fn(() => ({ open: true, replaceVisible: true }));
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: 'h',
      code: 'KeyH',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.closeSearchPanel).toHaveBeenCalledTimes(1);
    expect(handlers.openSearchPanel).not.toHaveBeenCalled();
  });

  it('通过 Ctrl + \\ 触发清除样式快捷键', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      key: '\\',
      code: 'Backslash',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers);

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.commands).toEqual([{ type: 'clearInlineStyles' }]);
  });

  it('支持使用设置覆盖常用快捷键', () => {
    const handlers = createHandlers();
    const event = new KeyboardEvent('keydown', {
      ctrlKey: true,
      altKey: true,
      key: 'e',
      code: 'KeyE',
      bubbles: true,
      cancelable: true,
    });

    handleGlobalShortcut(event, handlers, {
      'new-file': 'Ctrl+N',
      'open-file': 'Ctrl+O',
      'save-file': 'Ctrl+S',
      'export-html': 'Ctrl+Shift+E',
      'export-pdf': 'Ctrl+Shift+P',
      'toggle-source': 'Ctrl+Alt+E',
      'toggle-theme': 'Ctrl+Shift+L',
      'toggle-focus': 'Ctrl+Shift+F',
      'insert-code-block': 'Ctrl+Shift+K',
      'insert-table': 'Ctrl+Shift+T',
      'insert-math-block': 'Ctrl+Shift+M',
      'menu-link': 'Ctrl+K',
      'menu-clear-format': 'Ctrl+\\',
    });

    expect(event.defaultPrevented).toBe(true);
    expect(handlers.setMode).toHaveBeenCalledWith('source');
  });
});

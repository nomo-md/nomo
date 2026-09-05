import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

describe('App outline layout', () => {
  const appSource = readFileSync(resolve(__dirname, 'App.svelte'), 'utf-8').replace(/\r\n/g, '\n');
  const editorSource = readFileSync(
    resolve(__dirname, 'components/EditorWorkspace.svelte'),
    'utf-8',
  );
  const toolbarSource = readFileSync(
    resolve(__dirname, 'components/EditorToolbar.svelte'),
    'utf-8',
  );
  const settingsWindowSource = readFileSync(
    resolve(__dirname, 'components/SettingsWindow.svelte'),
    'utf-8',
  );
  const softwareUpdateDialogSource = readFileSync(
    resolve(__dirname, 'components/SoftwareUpdateDialog.svelte'),
    'utf-8',
  );
  const linkQuickEditorSource = readFileSync(
    resolve(__dirname, 'components/LinkQuickEditor.svelte'),
    'utf-8',
  );
  const appShellSource = readFileSync(resolve(__dirname, 'components/AppShell.svelte'), 'utf-8');
  const emptyWorkspaceSource = readFileSync(
    resolve(__dirname, 'components/EmptyWorkspace.svelte'),
    'utf-8',
  );
  const titleBarSource = readFileSync(resolve(__dirname, 'components/AppTitleBar.svelte'), 'utf-8');
  const windowsCaptionControlsSource = readFileSync(
    resolve(__dirname, 'components/WindowsCaptionControls.svelte'),
    'utf-8',
  );
  const documentTabsSource = readFileSync(
    resolve(__dirname, 'components/DocumentTabs.svelte'),
    'utf-8',
  );
  const externalChangeDialogSource = readFileSync(
    resolve(__dirname, 'components/ExternalChangeDialog.svelte'),
    'utf-8',
  );
  const segmentedEditorCssSource = readFileSync(
    resolve(__dirname, 'styles/editor-segmented.css'),
    'utf-8',
  );
  const explorerSidebarSource = readFileSync(
    resolve(__dirname, 'components/ExplorerSidebar.svelte'),
    'utf-8',
  );
  const frontMatterCardSource = readFileSync(
    resolve(__dirname, 'components/FrontMatterCard.svelte'),
    'utf-8',
  );
  const appCommandsSource = readFileSync(resolve(__dirname, 'services/appCommands.ts'), 'utf-8');
  const documentActionsSource = readFileSync(
    resolve(__dirname, 'services/documentActionsController.ts'),
    'utf-8',
  );
  const settingsServiceSource = readFileSync(resolve(__dirname, 'services/settings.ts'), 'utf-8');
  const editorSettingsControllerSource = readFileSync(
    resolve(__dirname, 'services/editorSettingsController.ts'),
    'utf-8',
  );
  const desktopWindowSource = readFileSync(
    resolve(__dirname, 'services/desktopWindow.ts'),
    'utf-8',
  );
  const themeManagerSource = readFileSync(
    resolve(__dirname, 'services/themeManager.ts'),
    'utf-8',
  );
  const tauriMenuSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/menu.rs'),
    'utf-8',
  );
  const tauriLibSource = readFileSync(resolve(__dirname, '../../src-tauri/src/lib.rs'), 'utf-8');
  const tauriConfigSource = readFileSync(
    resolve(__dirname, '../../src-tauri/tauri.conf.json'),
    'utf-8',
  );
  const tauriWindowsConfigSource = readFileSync(
    resolve(__dirname, '../../src-tauri/tauri.windows.conf.json'),
    'utf-8',
  );
  const tauriMacosConfigSource = readFileSync(
    resolve(__dirname, '../../src-tauri/tauri.macos.conf.json'),
    'utf-8',
  );
  const releaseWorkflowSource = readFileSync(
    resolve(__dirname, '../../.github/workflows/release.yml'),
    'utf-8',
  );
  const windowsOpenWithInstallerHookSource = readFileSync(
    resolve(__dirname, '../../src-tauri/installer/windows-open-with.nsh'),
    'utf-8',
  );
  const simplifiedChineseInstallerLanguageSource = readFileSync(
    resolve(__dirname, '../../src-tauri/installer/SimpChinese.nsh'),
    'utf-8',
  );
  const tauriTraySource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/tray.rs'),
    'utf-8',
  );
  const tauriExternalOpenSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/external_open.rs'),
    'utf-8',
  );
  const tauriWindowCommandsSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/commands.rs'),
    'utf-8',
  );
  const tauriMacosSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/os/macos.rs'),
    'utf-8',
  );
  const tauriWindowsSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/window/os/windows.rs'),
    'utf-8',
  );
  const desktopCapabilitySource = readFileSync(
    resolve(__dirname, '../../src-tauri/capabilities/default.json'),
    'utf-8',
  );
  const tauriImageAssetsSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/file_system/image_assets.rs'),
    'utf-8',
  );
  const tauriFileSystemSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/file_system.rs'),
    'utf-8',
  );
  const tauriConfigCommandsSource = readFileSync(
    resolve(__dirname, '../../src-tauri/src/config/commands.rs'),
    'utf-8',
  );
  const tauriStorageSource = readFileSync(
    resolve(__dirname, '../lib/desktop/tauriStorage.ts'),
    'utf-8',
  );
  const outlineInteractionSource = readFileSync(
    resolve(__dirname, 'services/outlineInteractionController.ts'),
    'utf-8',
  );
  const editorInteractionSource = readFileSync(
    resolve(__dirname, 'services/editorInteractionController.ts'),
    'utf-8',
  );
  const tocNodeViewSource = readFileSync(
    resolve(__dirname, '../lib/editor-core/nodeViews/TocBlockNodeView.ts'),
    'utf-8',
  );
  const tableControlsSource = readFileSync(
    resolve(__dirname, '../lib/editor-core/plugins/tableControls.ts'),
    'utf-8',
  );
  const tableControlsStyles = readFileSync(
    resolve(__dirname, 'styles/editor-table-controls.css'),
    'utf-8',
  );
  const styles = [
    'styles/app.css',
    'styles/app-layout.css',
    'styles/app-chrome.css',
    'styles/app-responsive.css',
    'styles/editor-document.css',
    'styles/editor-outline.css',
    'styles/editor-table-controls.css',
  ]
    .map((path) => readFileSync(resolve(__dirname, path), 'utf-8'))
    .join('\n');
  const responsiveStyles = readFileSync(resolve(__dirname, 'styles/app-responsive.css'), 'utf-8');
  const documentStylesSource = readFileSync(
    resolve(__dirname, 'styles/editor-document.css'),
    'utf-8',
  );
  const outlineStylesSource = readFileSync(
    resolve(__dirname, 'styles/editor-outline.css'),
    'utf-8',
  );

  function extractCssBlock(source: string, selector: string, fromIndex = 0) {
    const selectorIndex = source.indexOf(selector, fromIndex);
    expect(selectorIndex).toBeGreaterThan(-1);
    const blockStart = source.indexOf('{', selectorIndex);
    expect(blockStart).toBeGreaterThan(-1);
    let depth = 0;
    for (let index = blockStart; index < source.length; index += 1) {
      if (source[index] === '{') {
        depth += 1;
      } else if (source[index] === '}') {
        depth -= 1;
        if (depth === 0) {
          return source.slice(blockStart + 1, index);
        }
      }
    }
    throw new Error(`CSS block not closed: ${selector}`);
  }

  it('keeps the document centered until it needs to avoid the floating outline', () => {
    const documentLayouts =
      editorSource.match(/<div class="document-layout">[\s\S]*?<\/div>/g) ?? [];
    const adaptiveDocumentStyles = extractCssBlock(
      documentStylesSource,
      '.editor-grid:has(> .content-outline) .document-layout',
    );
    const compactOutlineStyles = extractCssBlock(responsiveStyles, '@container (max-width: 900px)');
    const outlineStyles = extractCssBlock(outlineStylesSource, '.content-outline');

    expect(documentLayouts).toHaveLength(2);
    for (const layout of documentLayouts) {
      expect(layout).not.toContain('class="content-outline"');
    }
    expect(adaptiveDocumentStyles).toContain(
      '--md-editor-outline-safe-space: calc(224px + clamp(32px, 3.5cqw, 160px));',
    );
    expect(adaptiveDocumentStyles).toMatch(
      /grid-template-columns:\s*minmax\(0,\s*1fr\)\s*minmax\(0,\s*min\(100%,\s*calc\(var\(--md-editor-content-width-percent\) \* 1cqw\)\)\)\s*minmax\(var\(--md-editor-outline-safe-space\),\s*1fr\);/,
    );
    expect(adaptiveDocumentStyles).toMatch(/justify-content:\s*stretch;/);
    expect(documentStylesSource).toMatch(
      /\.editor-grid:has\(> \.content-outline\) \.document-layout > \.front-matter-card\s*\{[\s\S]*?grid-column:\s*2;/,
    );
    expect(outlineStyles).toMatch(/position:\s*fixed;/);
    expect(outlineStyles).toMatch(/width:\s*220px;/);
    expect(outlineStyles).toMatch(/right:\s*clamp\(32px,\s*3\.5cqw,\s*160px\);/);
    expect(compactOutlineStyles).toMatch(
      /\.editor-grid:has\(> \.content-outline\) \.document-layout\s*\{[\s\S]*?grid-template-columns:\s*1fr;[\s\S]*?justify-content:\s*center;/,
    );
    expect(compactOutlineStyles).toMatch(
      /\.editor-grid:has\(> \.content-outline\) \.document-layout > \.front-matter-card\s*\{[\s\S]*?grid-column:\s*1;/,
    );
    expect(compactOutlineStyles).toMatch(/\.content-outline\s*\{[\s\S]*?display:\s*none;/);
    expect(styles).toMatch(/\.editor-shell\s*\{[\s\S]*?container-type:\s*inline-size;/);
    expect(styles).toMatch(/\.image-node\.is-badge img\s*\{[\s\S]*?height:\s*20px;/);
    expect(styles).toMatch(/\.image-node\.is-badge img\s*\{[\s\S]*?min-width:\s*0;/);
    expect(styles).toMatch(
      /\.image-node\.is-badge \.image-node-fullscreen-button\s*\{[\s\S]*?display:\s*none;/,
    );
    expect(styles).toMatch(
      /\.prosemirror-host \.ProseMirror img\[src\*='img\.shields\.io'\][\s\S]*?\{\s*display:\s*inline-block;[\s\S]*?height:\s*20px;/,
    );
    expect(styles).toMatch(
      /\.prosemirror-host \.ProseMirror img\[width\][\s\S]*?\.prosemirror-host \.ProseMirror img\[src\*='img\.shields\.io'\][\s\S]*?min-width:\s*0;/,
    );
    expect(styles).toMatch(
      /\.prosemirror-host \.ProseMirror a:has\(img\[src\*='img\.shields\.io'\]\)[\s\S]*?\{\s*display:\s*inline-flex;[\s\S]*?text-decoration:\s*none;/,
    );
    expect(styles).toMatch(
      /\.prosemirror-host \.ProseMirror p:has\(img\[src\*='img\.shields\.io'\]\)[\s\S]*?white-space:\s*normal;/,
    );
    expect(styles).toMatch(
      /\.prosemirror-host \.ProseMirror \.html-widget:has\(img\[src\*='img\.shields\.io'\]\)[\s\S]*?display:\s*block;/,
    );
    expect(styles).not.toContain('7vw');
  });

  it('sizes the document content as a percentage of the editor shell', () => {
    expect(appSource).toContain(
      'contentWidthPercent = DEFAULT_APP_PREFERENCES.contentWidthPercent',
    );
    expect(appSource).toContain('{contentWidthPercent}');
    expect(styles).toMatch(
      /grid-template-columns:\s*minmax\(0,\s*calc\(var\(--md-editor-content-width-percent\) \* 1cqw\)\);/,
    );
  });

  it('refreshes editor viewport layout after content width and zoom changes', () => {
    const contentWidthStart = editorSettingsControllerSource.indexOf('function updateContentWidth');
    const contentWidthEnd = editorSettingsControllerSource.indexOf(
      'function updateBlockStyle',
      contentWidthStart,
    );
    const contentWidthSource = editorSettingsControllerSource.slice(
      contentWidthStart,
      contentWidthEnd,
    );
    const preferencesStart = appSource.indexOf('async function applyAppPreferences');
    const preferencesEnd = appSource.indexOf('function getCurrentAppPreferences', preferencesStart);
    const preferencesSource = appSource.slice(preferencesStart, preferencesEnd);
    const wheelStart = appSource.indexOf('function handleGlobalWheel');
    const wheelEnd = appSource.indexOf('function handleZoomChange', wheelStart);
    const wheelSource = appSource.slice(wheelStart, wheelEnd);
    const zoomChangeStart = appSource.indexOf('function handleZoomChange');
    const zoomChangeEnd = appSource.indexOf('function setupSystemThemeListener', zoomChangeStart);
    const zoomChangeSource = appSource.slice(zoomChangeStart, zoomChangeEnd);

    expect(editorSettingsControllerSource).toContain('refreshEditorViewportLayout(): void;');
    expect(contentWidthSource).toContain(
      'applyEditorLayoutSettings(options.getContentWidthPercent())',
    );
    expect(contentWidthSource).toContain('options.refreshEditorViewportLayout();');
    expect(appSource).toContain(
      'refreshEditorViewportLayout = editorInteraction.refreshEditorViewportLayout;',
    );
    expect(preferencesSource).toContain('applyTypographySettings(fontSize, lineHeight);');
    expect(preferencesSource).toContain('applyEditorLayoutSettings(contentWidthPercent);');
    expect(preferencesSource).toContain(
      'applyZoomSetting(zoomPercent, { onFrame: refreshEditorViewportLayout });',
    );
    expect(zoomChangeSource).toContain('applyZoomSetting(zoomPercent, {');
    expect(zoomChangeSource).toContain('transition: true,');
    expect(zoomChangeSource).toContain('refreshEditorViewportLayout');
  });

  it('keeps front matter aligned with the zoomed document body', () => {
    expect(styles).toMatch(
      /\.front-matter-card\s*\{[\s\S]*?max-width:\s*calc\(var\(--md-editor-content-width-percent\) \* 1cqw\);[\s\S]*?margin:\s*0 auto 22px;[\s\S]*?zoom:\s*var\(--md-editor-zoom\);/,
    );
  });

  it('keeps document stats as a floating editor card', () => {
    expect(styles).toMatch(
      /\.editor-shell\s*\{[\s\S]*?grid-template-rows:\s*auto auto minmax\(0,\s*1fr\);/,
    );
    expect(styles).toMatch(/\.statusbar\s*\{[\s\S]*?position:\s*absolute;/);
    expect(styles).toMatch(/\.statusbar\s*\{[\s\S]*?right:\s*16px;/);
    expect(styles).toMatch(/\.statusbar-stats-trigger\s*\{[\s\S]*?border:\s*1px solid/);
    expect(styles).not.toMatch(/\.statusbar\s*\{[\s\S]*?border-top:\s*1px solid/);
  });

  it.skip('shows a real no-document workspace instead of an empty editor surface', () => {
    expect(appShellSource).toContain(
      '$: hasOpenDocument = tabs.length > 0 && Boolean(activeTabId);',
    );
    expect(appShellSource).toContain("import EmptyWorkspace from './EmptyWorkspace.svelte';");
    expect(appShellSource).toContain('class:no-open-document={!hasOpenDocument}');

    const tabsIndex = appShellSource.indexOf('<DocumentTabs');
    const documentBranchIndex = appShellSource.indexOf('{#if hasOpenDocument}');
    const emptyBranchIndex = appShellSource.indexOf('{:else}', documentBranchIndex);
    const branchEndIndex = appShellSource.indexOf('      {/if}', emptyBranchIndex);
    const documentBranch = appShellSource.slice(documentBranchIndex, emptyBranchIndex);
    const emptyBranch = appShellSource.slice(emptyBranchIndex, branchEndIndex);
    const sectionEndIndex = appShellSource.indexOf('</section>');

    expect(tabsIndex).toBeGreaterThan(-1);
    expect(documentBranchIndex).toBeGreaterThan(tabsIndex);
    expect(documentBranch).toContain('<EditorToolbar');
    expect(documentBranch).toContain('<EditorWorkspace');
    expect(documentBranch).toContain('<LinkQuickEditor');
    expect(appShellSource.indexOf('<StatusBar')).toBeGreaterThan(sectionEndIndex);
    expect(emptyBranch).toContain('<EmptyWorkspace');
    expect(emptyBranch).toContain('{createNewFile}');
    expect(emptyBranch).toContain('{openFileDialog}');
    expect(emptyBranch).toContain('{openFolderDialog}');
    expect(styles).toMatch(
      /\.editor-shell\.no-open-document\s*\{[\s\S]*?grid-template-rows:\s*auto minmax\(0,\s*1fr\);/,
    );
    expect(styles).toContain('.empty-workspace');
    expect(styles).toContain('grid-row: 2;');
    expect(styles).toContain('align-self: stretch;');
    expect(styles).toContain('justify-self: stretch;');
  });

  it('wires the no-document workspace to existing document actions', () => {
    expect(emptyWorkspaceSource).toContain('export let interfaceLocale: string;');
    expect(emptyWorkspaceSource).toContain('export let createNewFile: () => void;');
    expect(emptyWorkspaceSource).toContain('export let openFileDialog: () => void;');
    expect(emptyWorkspaceSource).toContain('export let openFolderDialog: () => void;');
    expect(emptyWorkspaceSource).toContain('on:click={createNewFile}');
    expect(emptyWorkspaceSource).toContain('on:click={openFileDialog}');
    expect(emptyWorkspaceSource).toContain('on:click={openFolderDialog}');
    expect(emptyWorkspaceSource).toContain('t.noOpenDocument()');
    expect(emptyWorkspaceSource).toContain('t.noOpenDocumentDescription()');
    expect(emptyWorkspaceSource).toContain('FilePlus2');
    expect(emptyWorkspaceSource).toContain('FileText');
    expect(emptyWorkspaceSource).toContain('FolderOpen');
  });

  it('mounts the semantic editor only after an open document renders its host', () => {
    const mountStart = appSource.indexOf('function mountEditorHostIfReady()');
    const onMountStart = appSource.indexOf('onMount(async () => {');
    const onMountEnd = appSource.indexOf('  });', onMountStart);
    const mountSource = appSource.slice(mountStart, onMountStart);
    const onMountSource = appSource.slice(onMountStart, onMountEnd);

    expect(mountStart).toBeGreaterThan(-1);
    expect(mountSource).toContain('if (!hasOpenDocument() || !editorHost');
    expect(mountSource).toContain('editor.mount(editorHost);');
    expect(mountSource).toContain("editorHost.addEventListener('image-context-menu'");
    expect(appSource).toContain(
      '$: if (tabs.length > 0 && activeTabId && editorHost) mountEditorHostIfReady();',
    );
    expect(appSource).toContain(
      '$: if ((tabs.length === 0 || !activeTabId) && mountedEditorHost) detachMountedEditorHostEvents();',
    );
    expect(onMountSource).not.toContain('editor.mount(editorHost);');
    expect(onMountSource).not.toContain("editorHost.addEventListener('image-context-menu'");
  });

  it('keeps outline navigation in the current editor mode', () => {
    const jumpStart = outlineInteractionSource.indexOf('function jumpToOutlineItem');
    const jumpEnd = outlineInteractionSource.indexOf(
      'function updateActiveOutlineFromSourceScroll',
    );
    const jumpSource = outlineInteractionSource.slice(jumpStart, jumpEnd);

    expect(jumpSource).not.toContain("setMode('source')");
    expect(jumpSource).toContain('options.setActiveOutlineId(item.id);');
    expect(jumpSource).toContain(
      'scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane()',
    );
  });

  it('keeps source typing from normalizing content or resetting scroll', () => {
    const updateStart = editorInteractionSource.indexOf('function updateMarkdown');
    const updateEnd = editorInteractionSource.indexOf('function runCommand');
    const updateSource = editorInteractionSource.slice(updateStart, updateEnd);

    expect(updateSource).not.toContain('normalizeMarkdownForSave');
    expect(updateSource).toContain(
      'options.setPendingSourceScrollTop(options.getSplitView?.() ? null : options.getSourcePane()?.scrollTop ?? null);',
    );
    expect(updateSource).toContain("reason: 'source-input'");
    expect(updateSource).toContain('sourceInput: true');
    expect(appSource).toContain("if (event.reason === 'source-input') {");
    expect(editorInteractionSource).toContain('getSourceScrollTopWithVisibleCaret(');
    expect(editorInteractionSource).toContain('clampPaneScrollTop(sourcePane, nextScrollTop);');
    expect(editorInteractionSource).toContain('options.setPendingSourceScrollTop(null);');
  });

  it.skip('renders one shared outline panel with expandable items', () => {
    expect(editorSource.match(/<aside class="content-outline"/g)).toHaveLength(1);
    expect(editorSource).toContain('export let collapsedOutlineIds');
    expect(editorSource).toContain('export let visibleOutlineIds');
    expect(appSource).toContain('{collapsedOutlineIds}');
    expect(appSource).toContain('{visibleOutlineIds}');
    expect(editorSource).toContain('toggleOutlineItemExpanded');
    expect(editorSource).toContain('visibleOutlineIds.has(item.id)');
    expect(editorSource).toContain('handleOutlineToggle(event, item)');
    expect(editorSource).toContain('{#each outline as item, index (item.id)}');
  });

  it('exposes toc insertion and deletion as accessible UI actions', () => {
    expect(toolbarSource).toContain("runCommand({ type: 'insertToc' })");
    expect(toolbarSource).toContain('aria-label={t.insertToc()}');
    expect(toolbarSource).toContain('TableOfContents');
    expect(titleBarSource).toContain("runCommand({ type: 'insertToc' })");
    expect(titleBarSource).not.toContain("comingSoon('正文目录'");
    expect(appCommandsSource).toContain("command === 'menu-content-directory'");
    expect(appCommandsSource).toContain("handlers.runCommand({ type: 'insertToc' });");
    expect(tocNodeViewSource).toContain("this.dom.className = 'toc-block'");
    expect(tocNodeViewSource).toContain("deleteButton.setAttribute('aria-label', t.deleteToc())");
    expect(tocNodeViewSource).toContain('empty.textContent = t.documentHasNoHeadings()');
  });

  it('keeps table inline controls aligned and exposes compact table utility actions', () => {
    expect(tableControlsSource).toContain('const overlayHost = this.getOverlayHost(view);');
    expect(tableControlsSource).toContain('getOverlayScale(overlayHost)');
    expect(tableControlsSource).toContain('tableRect.width / scale.x');
    expect(tableControlsSource).toContain('resizeCurrentTable(rows, columns)');
    expect(tableControlsSource).toContain('t.resizeTable()');
    expect(tableControlsSource).toContain("setTableColumnAlignment('left')");
    expect(tableControlsSource).toContain("setTableColumnAlignment('center')");
    expect(tableControlsSource).toContain("setTableColumnAlignment('right')");
    expect(tableControlsSource).toContain('toggleFirstTableRowHeader()');
    expect(tableControlsSource).toContain('deleteCurrentTable()');
    expect(tableControlsStyles).toContain('.util-icon-align-left');
    expect(tableControlsStyles).toContain('.util-icon-resize-grid');
    expect(tableControlsStyles).toContain('.table-resize-popover');
    expect(tableControlsStyles).not.toContain("content: '≡'");
    expect(tableControlsStyles).not.toContain("content: '⌫'");
  });

  it('wires Mermaid diagram insertion through toolbar, titlebar and native menu', () => {
    expect(toolbarSource).toContain('DIAGRAM_TEMPLATES');
    expect(toolbarSource).toContain("type: 'insertMermaidBlock'");
    expect(toolbarSource).toContain('t.blankDiagram()');
    expect(toolbarSource).toContain("type: 'insertDiagramBlock'");
    expect(toolbarSource).toContain('aria-label={t.insertDiagram()}');
    expect(titleBarSource).toContain('DIAGRAM_TEMPLATES');
    expect(titleBarSource).toContain('insertBlankDiagram');
    expect(titleBarSource).toContain('insertDiagram(template.type');
    expect(titleBarSource).not.toContain("comingSoon('图表'");
    expect(appCommandsSource).toContain("command === 'menu-chart'");
    expect(appCommandsSource).toContain("type: 'insertMermaidBlock'");
    expect(appCommandsSource).toContain("command.startsWith('menu-chart:')");
    expect(appCommandsSource).toContain("type: 'insertDiagramBlock'");
    expect(tauriMenuSource).toContain('SubmenuBuilder::new(app, tr(locale, "menu_chart"))');
    expect(tauriMenuSource).toContain('"menu-chart"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_chart_blank")');
    expect(tauriMenuSource).toContain('menu-chart:flowchart');
    expect(tauriMenuSource).toContain('menu-chart:erDiagram');
  });

  it('wires highlight through toolbar, titlebar and native menu', () => {
    expect(toolbarSource).toContain('Highlighter');
    expect(toolbarSource).toContain("runCommand({ type: 'toggleHighlight' })");
    expect(titleBarSource).toContain("runCommand({ type: 'toggleHighlight' })");
    expect(titleBarSource).not.toContain("comingSoon('高亮'");
    expect(appCommandsSource).toContain("command === 'menu-highlight'");
    expect(appCommandsSource).toContain("handlers.runCommand({ type: 'toggleHighlight' });");
    expect(tauriMenuSource).toContain('"menu-highlight"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_highlight")');
  });

  it('wires link editing through toolbar, titlebar and shortcuts', () => {
    expect(toolbarSource).toContain('Link');
    expect(toolbarSource).toContain('aria-label={t.editLink()}');
    expect(linkQuickEditorSource).toContain('role="dialog"');
    expect(linkQuickEditorSource).toContain('role="alert"');
    expect(linkQuickEditorSource).toContain('placeholder={t.linkTitlePlaceholder()}');
    expect(linkQuickEditorSource).toContain('placeholder="https://example.com"');
    expect(appSource).toContain('getLinkPickerPositionStyle(editor.getSelectionAnchorRect())');
    expect(titleBarSource).toContain('finish(openLinkPicker,');
    expect(titleBarSource).not.toContain("comingSoon('超链接'");
    expect(appCommandsSource).toContain("command === 'menu-link'");
    expect(appCommandsSource).toContain('handlers.openLinkPicker();');
    expect(appCommandsSource).toContain("key === 'k' && !event.shiftKey");
    expect(tauriMenuSource).toContain('"menu-link"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_link")');
    expect(appSource).toContain('editor.getActiveLink()');
    expect(appSource).toContain("type: 'insertLink'");
    expect(appSource).toContain('text: linkText');
    expect(appSource).toContain("type: 'removeLink'");
    expect(appSource).toContain('updateLinkText');
  });

  it('wires Markdown comments through toolbar, titlebar and native menu', () => {
    expect(toolbarSource).toContain('MessageSquare');
    expect(toolbarSource).toContain("type: 'insertCommentInline'");
    expect(toolbarSource).toContain('aria-label={t.insertInlineComment()}');
    expect(titleBarSource).toContain("type: 'insertCommentInline'");
    expect(titleBarSource).toContain("type: 'insertCommentBlock'");
    expect(titleBarSource).not.toContain("comingSoon('注释'");
    expect(appCommandsSource).toContain("command === 'menu-comment'");
    expect(appCommandsSource).toContain("type: 'insertCommentInline'");
    expect(appCommandsSource).toContain("command === 'menu-comment-block'");
    expect(appCommandsSource).toContain("type: 'insertCommentBlock'");
    expect(tauriMenuSource).toContain('"menu-comment"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_comment")');
    expect(tauriMenuSource).toContain('"menu-comment-block"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_comment_block")');
  });

  it('forwards native menu events to desktop command handlers', () => {
    const tauriLibSource = readFileSync(resolve(__dirname, '../../src-tauri/src/lib.rs'), 'utf-8');
    const tauriCommandsSource = readFileSync(
      resolve(__dirname, '../../src-tauri/src/window/commands.rs'),
      'utf-8',
    );

    expect(tauriLibSource).toContain('install_window_menu(app.handle(), &window)');
    expect(tauriCommandsSource).toContain('install_window_menu(&app, &window)');
    expect(tauriMenuSource).toContain('app.set_menu(menu)');
    expect(tauriMenuSource).toContain('app.on_menu_event(|app, event|');
    expect(tauriMenuSource).toContain('focused_document_window(app)');
    expect(tauriMenuSource).toContain('app.webview_windows()');
    expect(tauriMenuSource).toContain('window.is_focused().unwrap_or(false)');
    expect(tauriMenuSource).toContain('app.get_webview_window("main")');
    expect(tauriMenuSource).toContain('window.on_menu_event(|window, event|');
    expect(tauriMenuSource).toContain('window.emit("nomo://menu-command", command)');
    expect(tauriMenuSource).toContain('emit_exit_request(window.app_handle())');
    expect(tauriMenuSource).toContain('emit_exit_request(app)');
    expect(tauriCommandsSource).toContain('app.emit("nomo://request-exit-app", ())');
    expect(appSource).toContain("listen('nomo://request-exit-app'");
    expect(appSource).toContain('requestExitApp()');
    expect(tauriMenuSource).toContain('format!("open-recent:{}:{}", entry.entry_type, entry.path)');
    expect(appCommandsSource).toContain("command === 'new-window'");
    expect(appCommandsSource).toContain("command.startsWith('open-recent:')");
  });

  it('routes external document open requests into existing document windows', () => {
    expect(tauriLibSource).toContain('tauri_plugin_single_instance::init');
    expect(tauriLibSource).toContain('collect_external_open_targets_from_args');
    expect(tauriLibSource).toContain('route_external_open_targets');
    expect(tauriLibSource).toContain('tauri::RunEvent::Opened');
    expect(tauriLibSource).toContain('collect_markdown_paths_from_urls');
    expect(tauriLibSource).toContain('persist_pending_external_open');
    expect(tauriLibSource).toContain('persist_pending_external_folder_open');
    expect(tauriLibSource).toContain('try_state::<crate::config::ConfigManager>()');
    expect(tauriLibSource).toContain('queue_early_external_open(paths)');
    expect(tauriLibSource).toContain('take_early_external_open_paths()');
    expect(tauriExternalOpenSource).toContain(
      'const OPEN_DOCUMENT_EVENT: &str = "nomo://open-document"',
    );
    expect(tauriExternalOpenSource).toContain(
      'const OPEN_FOLDER_EVENT: &str = "nomo://open-folder"',
    );
    expect(tauriExternalOpenSource).toContain('.emit(');
    expect(tauriExternalOpenSource).toContain('OPEN_DOCUMENT_EVENT');
    expect(tauriExternalOpenSource).toContain('OPEN_FOLDER_EVENT');
    expect(tauriExternalOpenSource).toContain('#[serde(rename = "windowLabel")]');
    expect(tauriExternalOpenSource).toContain('window_label: label.clone()');
    expect(tauriExternalOpenSource).toContain('is_document_window_label');
    expect(tauriStorageSource).toContain('listenDesktopOpenDocuments');
    expect(tauriStorageSource).toContain("listen<ExternalOpenPayload>('nomo://open-document'");
    expect(tauriStorageSource).toContain('listenDesktopOpenFolder');
    expect(tauriStorageSource).toContain("listen<ExternalOpenFolderPayload>('nomo://open-folder'");
    expect(tauriStorageSource).toContain(
      'const eventWindowLabel = normalizeEventWindowLabel(event.payload);',
    );
    expect(tauriStorageSource).toContain('handler(paths, eventWindowLabel);');
    expect(tauriStorageSource).toContain('handler(folderPath, eventWindowLabel);');
    expect(appSource).toContain('listenDesktopOpenDocuments');
    expect(appSource).toContain('listenDesktopOpenFolder');
    expect(appSource).toContain('listenDesktopOpenDocuments((paths, targetWindowLabel) =>');
    expect(appSource).toContain('listenDesktopOpenFolder((folderPath, targetWindowLabel) =>');
    expect(appSource).toContain('if (targetWindowLabel && targetWindowLabel !== windowLabel)');
    const criticalEventsSource = appSource.slice(
      appSource.indexOf('async function setupCriticalDesktopEvents()'),
      appSource.indexOf('async function setupDesktopEvents()'),
    );
    expect(criticalEventsSource).toContain('listenDesktopOpenDocuments');
    expect(criticalEventsSource).toContain("if (appBootState !== 'ready')");
    expect(criticalEventsSource).toContain('queuePendingExternalOpenPaths(paths)');
    expect(appSource).toContain('const startupExternalOpenPaths = pendingExternalOpenPaths');
    expect(appSource).toContain('while (pendingExternalOpenPaths.length > 0)');
    expect(appSource).toContain('const deferredExternalOpenPaths = pendingExternalOpenPaths');
    expect(appSource).toContain('pendingExternalOpen:${windowLabel}');
    expect(appSource).toContain('pendingFolder:${windowLabel}');
    expect(appSource).toContain('openStartupExternalMarkdownPaths(startupExternalOpenPaths)');
    expect(appSource).toContain('async function openStartupExternalMarkdownPaths(paths: string[])');
    expect(appSource).toContain('openExternalMarkdownPaths(paths)');
    expect(appSource).toContain('openFolderWithBehavior(folderPath)');
    expect(appSource).toContain("openRecentEntry(path, 'file')");
    expect(tauriConfigSource).toContain('"fileAssociations"');
    expect(tauriConfigSource).toContain('"md"');
    expect(tauriConfigSource).toContain('"markdown"');
    expect(tauriConfigSource).toContain('"role": "Editor"');
  });

  it('ships Windows releases as NSIS plus zip without MSI', () => {
    const tauriWindowsConfig = JSON.parse(tauriWindowsConfigSource);
    const tauriMacosConfig = JSON.parse(tauriMacosConfigSource);

    expect(tauriWindowsConfig.bundle.targets).toEqual(['nsis']);
    expect(tauriWindowsConfig.bundle.windows.nsis.languages).toEqual(['SimpChinese', 'English']);
    expect(tauriWindowsConfig.bundle.windows.nsis.displayLanguageSelector).toBe(true);
    expect(tauriWindowsConfig.bundle.windows.nsis.startMenuFolder).toBe('Nomo');
    expect(tauriWindowsConfig.bundle.windows.nsis.installerIcon).toBe('icons/icon.ico');
    expect(tauriWindowsConfig.bundle.windows.nsis.uninstallerIcon).toBe('icons/icon.ico');
    expect(tauriWindowsConfig.bundle.windows.nsis.customLanguageFiles.SimpChinese).toBe(
      'installer/SimpChinese.nsh',
    );
    expect(tauriWindowsConfig.bundle.windows.nsis.installerHooks).toBe(
      'installer/windows-open-with.nsh',
    );
    expect(tauriMacosConfig.bundle.targets).toEqual(['app', 'dmg']);
    expect(tauriMacosConfig.bundle.macOS.files['Resources/Assets.car']).toBe(
      'target/appicon/Assets.car',
    );
    expect(readFileSync(resolve(__dirname, '../../src-tauri/Info.plist'), 'utf-8')).toContain(
      '<string>AppIcon</string>',
    );
    expect(
      readFileSync(resolve(__dirname, '../../scripts/compile-macos-appicon.sh'), 'utf-8'),
    ).toContain('sips", "-z", "1024", "1024"');
    expect(readFileSync(resolve(__dirname, '../../package.json'), 'utf-8')).toContain(
      'bash scripts/compile-macos-appicon.sh',
    );

    expect(releaseWorkflowSource).toContain("args: '--bundles nsis'");
    expect(releaseWorkflowSource).toContain('tauri-apps/tauri-action@v0.6.2');
    expect(releaseWorkflowSource).toContain('Nomo_${version}_x64.zip');
    expect(releaseWorkflowSource).toContain('gh release upload');
    expect(releaseWorkflowSource).toContain('checksums.md5: MD5 校验清单');
    expect(releaseWorkflowSource).toContain('name: Publish MD5 checksums');
    expect(releaseWorkflowSource).toContain(
      "gh release upload '${{ github.ref_name }}' checksums.md5 --clobber",
    );
    expect(releaseWorkflowSource).toContain('x64.zip: 免安装版');
    expect(releaseWorkflowSource).not.toContain('TAURI_SIGNING_PRIVATE_KEY');
    expect(releaseWorkflowSource).not.toContain('includeUpdaterJson');
    expect(releaseWorkflowSource).not.toContain('x64_en-US.msi');
    expect(releaseWorkflowSource).not.toContain('portable');
    expect(tauriConfigSource).not.toContain('createUpdaterArtifacts');
    expect(tauriConfigSource).not.toContain('"pubkey"');
    expect(tauriLibSource).not.toContain('tauri_plugin_updater');
  });

  it('registers Nomo as an optional document open-with application', () => {
    expect(windowsOpenWithInstallerHookSource).toContain('NSIS_HOOK_POSTINSTALL');
    expect(windowsOpenWithInstallerHookSource).toContain(
      'Software\\Classes\\Applications\\${MAINBINARYNAME}.exe',
    );
    expect(windowsOpenWithInstallerHookSource).toContain('SupportedTypes');
    expect(windowsOpenWithInstallerHookSource).toContain('.md');
    expect(windowsOpenWithInstallerHookSource).toContain('.markdown');
    expect(windowsOpenWithInstallerHookSource).toContain('.txt');
    expect(windowsOpenWithInstallerHookSource).toContain('.json');
    expect(windowsOpenWithInstallerHookSource).toContain('OpenWithList');
    expect(windowsOpenWithInstallerHookSource).toContain('OpenWithProgids');
    expect(windowsOpenWithInstallerHookSource).toContain('Nomo.Markdown');
    expect(windowsOpenWithInstallerHookSource).toContain('Nomo.Text');
    expect(windowsOpenWithInstallerHookSource).toContain('Nomo.Json');
    expect(windowsOpenWithInstallerHookSource).not.toContain('Software\\Classes\\.md" ""');
    expect(windowsOpenWithInstallerHookSource).toContain('NSIS_HOOK_POSTUNINSTALL');
  });

  it('localizes custom NSIS installer messages in Simplified Chinese', () => {
    expect(simplifiedChineseInstallerLanguageSource).toContain(
      'LangString createDesktop ${LANG_SIMPCHINESE} "创建桌面快捷方式"',
    );
    expect(simplifiedChineseInstallerLanguageSource).toContain(
      'LangString installingWebview2 ${LANG_SIMPCHINESE} "正在安装 WebView2..."',
    );
    expect(simplifiedChineseInstallerLanguageSource).toContain(
      'LangString deleteAppData ${LANG_SIMPCHINESE} "删除应用数据"',
    );
  });

  it('keeps the explicit explorer root across restored workspace tabs', () => {
    expect(appSource).toContain('updateWorkspaceStateSettings(workspaceEntries)');
    expect(appSource).toContain('[`workspaceTabs:${windowLabel}`]: state');
    expect(appSource).toContain('workspaceTabs:folder:${currentFolderPath}');
    expect(appSource).toContain("typeof state.currentFolderPath === 'string'");
    expect(appSource).toContain('currentFolderPath = state.currentFolderPath');
    expect(appSource).toContain('startupFolderPath = state.currentFolderPath');
    expect(appSource).not.toContain(
      `const parentDir = getDirectoryLabel(filePath);
      if (parentDir && parentDir !== '当前文件夹') loadFolder(parentDir).catch(() => undefined);`,
    );
  });

  it('loads the restored explorer root in the background after desktop events are ready', () => {
    expect(appSource).toContain("let startupFolderPath = ''");
    expect(appSource).toContain('function scheduleStartupFolderLoad()');
    expect(appSource).toContain('async function restoreWindowWorkspaceState(');
    expect(appSource).toMatch(/startupFolderPath = (state\.currentFolderPath|pendingFolderPath);/);
    const mountStart = appSource.indexOf('onMount(async () =>');
    const setupDesktopEventsIndex = appSource.indexOf('await setupDesktopEvents();', mountStart);
    const scheduleStartupFolderLoadIndex = appSource.indexOf(
      'scheduleStartupFolderLoad();',
      mountStart,
    );

    expect(setupDesktopEventsIndex).toBeGreaterThan(mountStart);
    expect(scheduleStartupFolderLoadIndex).toBeGreaterThan(setupDesktopEventsIndex);
    expect(appSource).toContain('queueMicrotask(runStartupFolderLoad)');
    expect(appSource).toContain('window.setTimeout(runStartupFolderLoad, 0)');
    expect(appSource).toContain('await loadFolder(folderPath);');
    expect(appSource).toContain('await expandAncestors(nativePath, currentFolderPath);');
    expect(appSource).toContain('t.loadFolderTreeFailed()');
    expect(appSource).not.toContain('loadFolder(currentFolderPath).catch(() => undefined)');
  });

  it.skip('clears old tabs before opening a different folder in the current window', () => {
    expect(appSource).toContain(
      'function closeAllTabsWithConfirmation(options?: { skipPersist?: boolean })',
    );
    expect(appSource).toContain(
      'function clearAllTabsWithoutCreatingBlank(options?: { skipPersist?: boolean })',
    );
    expect(appSource).toMatch(
      /async function openFolderInCurrentWindow\(folderPath: string\) \{\s*if \(!currentFolderPath \|\| !sameFileSystemPath\(currentFolderPath, folderPath\)\) \{\s*\/\/ 切换文件夹前保存当前文件夹状态[\s\S]*?if \(!closeAllTabsWithConfirmation\(\{ skipPersist: true \}\)\) \{\s*return;\s*\}\s*\}\s*currentFolderPath = folderPath;\s*await loadFolder\(folderPath\);\s*await restoreFolderWorkspaceState\(folderPath\);/,
    );
  });

  it('keeps the manual explorer refresh on the existing folder loading path', () => {
    expect(appSource).toMatch(
      /async function handleRefreshFolder\(\) \{\s*if \(currentFolderPath\) \{\s*await loadFolder\(currentFolderPath\);\s*\}\s*\}/,
    );
  });

  it('does not mark ancestor folders as selected when a file is active', () => {
    expect(explorerSidebarSource).not.toContain('function isFolderActive');
    expect(explorerSidebarSource).not.toContain('class:active={isFolderActive(');
    expect(explorerSidebarSource).toContain('function isActiveFilePath(path: string)');
    expect(explorerSidebarSource).toContain('class:active={isActiveFilePath(node.path)}');
  });

  it('mirrors the app chrome menu into the native macOS menubar', () => {
    expect(appShellSource).toContain('{#if desktopEnabled}');
    expect(appShellSource).toContain('<AppTitleBar');
    expect(tauriMenuSource).toContain('static APP_MENU_EVENT_INSTALLED');
    expect(tauriMenuSource).toContain('SubmenuBuilder::new(app, tr(locale, "menu_paragraph"))');
    expect(tauriMenuSource).toContain('SubmenuBuilder::new(app, tr(locale, "menu_settings"))');
    expect(tauriMenuSource).toContain('"set-heading-1"');
    expect(tauriMenuSource).toContain('"set-heading-6"');
    expect(tauriMenuSource).toContain('"insert-callout",');
    expect(tauriMenuSource).toContain('"toggle-ordered-list",');
    expect(tauriMenuSource).toContain('"toggle-bullet-list",');
    expect(tauriMenuSource).toContain('"toggle-task-list",');
    expect(tauriMenuSource).toContain('"open-settings"');
    expect(tauriMenuSource).toContain('tr(locale, "menu_preferences")');
    expect(tauriMenuSource).toContain('Some("CmdOrCtrl+N")');
    expect(tauriMenuSource).toContain('"Cmd+Q"');
    expect(tauriMenuSource).toContain('"Alt+F4"');
    expect(appCommandsSource).toContain("command === 'close-current-file'");
    expect(appCommandsSource).toContain("command === 'close-current-window'");
    expect(appCommandsSource).toContain("command === 'open-settings'");
  });

  it('wires YAML Front Matter to the semantic metadata card flow', () => {
    expect(editorSource).toContain('FrontMatterCard');
    expect(editorSource).toContain('frontMatterEditing');
    expect(editorSource).toContain('focusRequest={frontMatterFocusRequest}');
    expect(editorSource).toContain('focusTarget={frontMatterFocusTarget}');
    expect(appShellSource).toContain('{frontMatterFocusRequest}');
    expect(appShellSource).toContain('{frontMatterFocusTarget}');
    expect(appSource).toContain('replaceFrontMatterContent');
    expect(appSource).toContain("editor.execute({ type: 'insertFrontMatter' })");
    expect(appSource).toContain('frontMatterFocusRequest');
    expect(appSource).toContain("frontMatterFocusTarget = 'title-value'");
    expect(titleBarSource).toContain('finish(editFrontMatter,');
    expect(titleBarSource).not.toContain("comingSoon('YAML Front Matter'");
    expect(titleBarSource).toContain('t.frontMatter()');
    expect(appCommandsSource).toContain("command === 'menu-yaml-front-matter'");
    expect(appCommandsSource).toContain('handlers.editFrontMatter();');
    expect(frontMatterCardSource).toContain('aria-label={t.documentMetadataEditing()}');
    expect(frontMatterCardSource).toContain('t.editDocumentMetadata()');
    expect(frontMatterCardSource).toContain('t.viewDocumentMetadata()');
    expect(frontMatterCardSource).not.toContain('YAML Front Matter');
    expect(frontMatterCardSource).toContain(
      "import { clickOutside } from '../actions/clickOutside';",
    );
    expect(frontMatterCardSource).toContain('readonly={readonly}');
    expect(frontMatterCardSource).toContain('use:clickOutside={leaveEdit}');
    expect(frontMatterCardSource).toContain('on:focus={enterEdit}');
    expect(frontMatterCardSource).toContain('on:focusout={handleFocusOut}');
    expect(frontMatterCardSource).toContain('on:input={handleInput}');
    expect(frontMatterCardSource).toContain('deleteFrontMatter');
    expect(frontMatterCardSource).toContain('t.confirmDeleteMetadata()');
    expect(frontMatterCardSource).toContain('frontMatter.fields.parseWarning');
  });

  it('keeps hidden Markdown editor events out of segmented documents', () => {
    expect(appSource).toMatch(
      /function syncFromEditor\(event: EditorChangeEvent\)[\s\S]*?const activeTab = tabs\.find\(\(tab\) => tab\.id === activeTabId\);[\s\S]*?if \(!isMarkdownTab\(activeTab\)\) \{[\s\S]*?return;/,
    );
    expect(appSource).toMatch(
      /const activeDocument = tabs\.find\(\(tab\) => tab\.id === activeTabId\);\s*if \(isMarkdownTab\(activeDocument\)\) \{[\s\S]*?refreshSearchMatches/,
    );
  });

  it('mounts exactly one workspace implementation for each document kind', () => {
    expect(appShellSource).toContain("{#if activeTab?.documentKind === 'markdown'}");
    expect(appShellSource).toContain('<EditorWorkspace');
    expect(appShellSource).toContain(
      "{:else if activeTab?.documentKind === 'text' || activeTab?.documentKind === 'json'}",
    );
    expect(appShellSource).toContain('{#key activeTab.sessionId}');
    expect(appShellSource).toContain('<SegmentedTextEditorWorkspace');
  });

  it('keeps the editor toolbar focused on editing and view controls', () => {
    expect(toolbarSource).not.toContain('FolderOpen');
    expect(toolbarSource).not.toContain('PanelLeftClose');
    expect(toolbarSource).not.toContain('PanelLeftOpen');
    expect(toolbarSource).not.toContain('资源管理器侧边栏');
    expect(toolbarSource).not.toContain('Save');
    expect(toolbarSource).not.toContain('Image');
    expect(toolbarSource).not.toContain('Palette');
    expect(toolbarSource).not.toContain('Pilcrow');
    expect(toolbarSource).not.toContain('title="打开 Markdown"');
    expect(toolbarSource).not.toContain('title="导出保存"');
    expect(toolbarSource).not.toContain('title="图片"');
    expect(toolbarSource).not.toContain('title="字号"');
    expect(toolbarSource).not.toContain('title="行高"');
    expect(toolbarSource).not.toContain('aria-label="切换引用和提示块样式"');
    expect(toolbarSource).toContain('width-control');
    expect(toolbarSource).toContain('AlignHorizontalSpaceAround');
    expect(toolbarSource).toContain('contentWidthPercent');
  });

  it('uses semantic icons for mode, outline and toc controls in the editor toolbar', () => {
    expect(toolbarSource).toContain('BookOpenText');
    expect(toolbarSource).toContain('CodeXml');
    expect(toolbarSource).toContain('title={t.semanticEditingTitle()}');
    expect(toolbarSource).toContain('title={t.sourceModeTitle()}');
    expect(toolbarSource).toContain('aria-label={t.semanticEditing()}');
    expect(toolbarSource).toContain('aria-label={t.sourceMode()}');
    expect(toolbarSource).toContain("setMode('semantic')");
    expect(toolbarSource).toContain("setMode('source')");
    expect(toolbarSource).toContain('ListTree size={18}');
    expect(toolbarSource).toContain('TableOfContents size={17}');
  });

  it('keeps global explorer controls and Windows caption controls in one titlebar', () => {
    expect(appShellSource).toContain('{#if desktopEnabled}');
    expect(titleBarSource).toContain('sidebar-toggle-btn');
    expect(titleBarSource).toContain('PanelLeftClose');
    expect(titleBarSource).toContain('PanelLeftOpen');
    expect(titleBarSource).toContain(
      'shouldShowWindowMenu = platformCapabilities.showsInAppWindowMenu',
    );
    expect(titleBarSource).toContain('{#if shouldShowWindowMenu}');
    expect(titleBarSource).not.toContain('nomoAppIcon');
    expect(titleBarSource).not.toContain('class="app-logo"');
    expect(titleBarSource).not.toContain('<img class="app-logo"');
    expect(titleBarSource).toContain('Nomo</span>');
    expect(titleBarSource).not.toContain('<span class="app-logo">M</span>');
    expect(titleBarSource).toContain('t.showExplorerSidebar()');
    expect(titleBarSource).toContain('t.hideExplorerSidebar()');
    expect(titleBarSource).toContain('export let focusMode: boolean');
    expect(titleBarSource).toContain(
      "import WindowsCaptionControls from './WindowsCaptionControls.svelte'",
    );
    expect(titleBarSource).toContain(
      'desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar',
    );
    expect(titleBarSource).toContain(
      '<WindowsCaptionControls variant="return" onClose={toggleMarkdownMini} />',
    );
    expect(titleBarSource).toContain('<WindowsCaptionControls onClose={closeCurrentWindow} />');
    expect(titleBarSource).toContain('syncWindowState');
    expect(windowsCaptionControlsSource).toContain('await currentWindow.minimize()');
    expect(windowsCaptionControlsSource).toContain('await currentWindow.toggleMaximize()');
    expect(windowsCaptionControlsSource).toContain('await currentWindow.isMaximized()');
    expect(windowsCaptionControlsSource).toContain(
      'await currentWindow.onResized(syncMaximizedState)',
    );
    expect(windowsCaptionControlsSource).toContain('await onClose()');
    expect(windowsCaptionControlsSource).not.toContain('currentWindow.close()');
    expect(windowsCaptionControlsSource).not.toContain('currentWindow.destroy()');
    expect(windowsCaptionControlsSource).toContain('width: 46px;');
    expect(windowsCaptionControlsSource).toContain('height: 100%;');
    expect(desktopWindowSource).toContain("title: 'Nomo'");
    expect(desktopWindowSource).toContain('} - Nomo');
    expect(desktopWindowSource).toContain('getNewWindowChromeOptions');
    expect(desktopWindowSource).toContain("titleBarStyle: 'overlay'");
    expect(desktopWindowSource).toContain('trafficLightPosition: new LogicalPosition(16, 24)');
    expect(desktopWindowSource).toContain('hiddenTitle: true');
    expect(desktopWindowSource).toContain(
      'visible: !platformCapabilities.usesCustomWindowsTitlebar',
    );
    expect(existsSync(resolve(__dirname, 'services/windowChrome.ts'))).toBe(false);
    expect(existsSync(resolve(__dirname, 'styles/window-chrome.css'))).toBe(false);
    expect(
      existsSync(resolve(__dirname, '../../src-tauri/src/window/os/windows/titlebar.rs')),
    ).toBe(false);
    expect(tauriWindowsSource).toMatch(/fn window_decorations\(\) -> bool \{\s*false\s*\}/);
    expect(tauriWindowsSource).toContain('.set_decorations(window_decorations())');
    expect(tauriWindowsSource).toContain('.set_shadow(true)');
    expect(tauriWindowsSource).not.toContain('SetWindowSubclass');
    expect(tauriWindowCommandsSource).not.toContain('get_window_chrome_metrics');
    expect(tauriLibSource).not.toContain('get_window_chrome_metrics');
    expect(desktopCapabilitySource).toContain('"core:window:allow-minimize"');
    expect(tauriMacosSource).toContain('uses_overlay_titlebar(window.label())');
    expect(tauriMacosSource).toContain('label != "window-settings"');
    expect(tauriConfigSource).toContain('"y": 24');
    expect(styles).toMatch(/\.titlebar\s*\{[\s\S]*?height:\s*42px;/);
    expect(styles).toMatch(/\.titlebar-row\.top-row\s*\{[\s\S]*?height:\s*42px;/);
    expect(styles).toMatch(
      /\.titlebar\.is-win \.titlebar-row\.top-row:not\(\.markdown-mini-titlebar-row\)\s*\{[\s\S]*?padding-right:\s*0;/,
    );
    expect(styles).not.toContain('border-bottom: 1px solid var(--md-titlebar-border);');
    expect(styles).toMatch(
      /\.titlebar\.is-mac:not\(\.is-fullscreen\) \.titlebar-row\.top-row\s*\{[\s\S]*?padding-left:\s*88px;/,
    );
    const titlebarIconStyles = styles.match(/\.titlebar \.icon-btn\s*\{[^}]*\}/)?.[0] ?? '';
    expect(titlebarIconStyles).toContain('height: 32px;');
    expect(settingsWindowSource).toContain(
      'desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar',
    );
    expect(settingsWindowSource).toContain(
      '<WindowsCaptionControls onClose={() => handleClose()} />',
    );
    expect(styles).toMatch(/\.sidebar-toggle-btn\s*\{[\s\S]*?flex-shrink:\s*0;/);
    expect(styles).toMatch(/\.titlebar-row\.bottom-row\s*\{[\s\S]*?display:\s*none;/);
    expect(styles).not.toContain('.app-logo');
    expect(styles).toMatch(/\.app-name\s*\{[\s\S]*?font-size:\s*13px;/);
  });

  it('keeps explorer rows constrained to the visible tree width', () => {
    expect(explorerSidebarSource).toContain('class="folder-actions"');
    expect(styles).toMatch(/\.file-tree\s*\{[\s\S]*?scrollbar-gutter:\s*stable;/);
    expect(styles).toMatch(/--explorer-scrollbar-safe-area:\s*8px;/);
    expect(styles).toMatch(/--explorer-bottom-padding:\s*18px;/);
    expect(styles).toContain(
      'padding: 8px var(--explorer-scrollbar-safe-area) var(--explorer-bottom-padding) 8px;',
    );
    expect(explorerSidebarSource).toContain('const TREE_BOTTOM_PADDING = 18;');
    expect(explorerSidebarSource).toContain(
      'flattenedRows.length * TREE_ROW_HEIGHT + TREE_BOTTOM_PADDING',
    );
    expect(explorerSidebarSource).toContain(
      'const rowBottomWithPadding = rowTop + TREE_ROW_HEIGHT + TREE_BOTTOM_PADDING;',
    );
    expect(styles).toMatch(
      /\.rail:not\(:hover\) \.file-tree\s*\{[\s\S]*?scrollbar-color:\s*var\(--md-scrollbar-thumb-idle\) var\(--md-scrollbar-track\);/,
    );
    expect(styles).toMatch(
      /\.rail:hover \.file-tree\s*\{[\s\S]*?scrollbar-color:\s*var\(--md-scrollbar-thumb\) var\(--md-scrollbar-track\);/,
    );
    expect(styles).toMatch(/\.tree-root,\s*\.recent-tree\s*\{[\s\S]*?min-width:\s*0;/);
    expect(styles).toMatch(/\.recursive-tree-container\s*\{[\s\S]*?min-width:\s*0;/);
    expect(styles).toMatch(/\.recursive-tree-container\s*\{[\s\S]*?max-width:\s*100%;/);
    expect(styles).toMatch(/\.tree-folder-wrapper\s*\{[\s\S]*?min-width:\s*0;/);
    expect(styles).toMatch(/\.tree-folder-wrapper\s*\{[\s\S]*?max-width:\s*100%;/);
    expect(styles).toMatch(/\.tree-folder\.nested-dir\s*\{[\s\S]*?max-width:\s*100%;/);
    expect(styles).toMatch(/\.tree-folder\.nested-dir\s*\{[\s\S]*?overflow:\s*hidden;/);
    expect(styles).toMatch(/\.file-tree button\.tree-file\s*\{[\s\S]*?max-width:\s*100%;/);
    expect(styles).toMatch(/\.file-tree button\.tree-file\s*\{[\s\S]*?margin:\s*1px 0;/);
  });

  it('keeps folder chevron double-clicks from starting rename mode', () => {
    expect(explorerSidebarSource).toContain('function handleFolderDoubleClick');
    expect(explorerSidebarSource).toContain("target?.closest('.chevron-icon')");
    expect(explorerSidebarSource).toContain(
      'on:dblclick={(event) => handleFolderDoubleClick(node, event)}',
    );
  });

  it('cancels explorer rename mode when focus leaves the rename input', () => {
    expect(explorerSidebarSource).toContain(
      "import { clickOutside } from '../actions/clickOutside';",
    );
    expect(explorerSidebarSource).toContain('on:blur={cancelRenaming}');
    expect(explorerSidebarSource).toContain('use:renamingClickOutside={cancelRenaming}');
    expect(explorerSidebarSource).toContain("if (event.key === 'Enter')");
    expect(explorerSidebarSource).toContain('commitRenaming();');
  });

  it('auto-selects explorer rename text after context menu rename starts', () => {
    expect(explorerSidebarSource).toContain(
      "import { getExplorerRenameSelectionRange } from '../services/explorerRename';",
    );
    expect(explorerSidebarSource).toContain('function renameAutoSelect');
    expect(explorerSidebarSource).toContain('function startRenamingFromContextMenu');
    expect(explorerSidebarSource).toContain('setTimeout(() => {');
    expect(explorerSidebarSource).toContain('node.focus({ preventScroll: true });');
    expect(explorerSidebarSource).toContain('node.setSelectionRange(range.start, range.end);');
    expect(explorerSidebarSource.match(/use:renameAutoSelect=/g)).toHaveLength(2);
  });

  it('cancels explorer create mode when the new node input loses focus or receives an outside click', () => {
    expect(explorerSidebarSource).toContain('on:blur={cancelCreating}');
    expect(explorerSidebarSource.match(/use:clickOutside=\{cancelCreating\}/g)).toHaveLength(2);
    expect(explorerSidebarSource).toContain('commitCreating();');
    expect(explorerSidebarSource).toContain("} else if (event.key === 'Escape') {");
    expect(explorerSidebarSource).toContain('cancelCreating();');
  });

  it('recomputes explorer virtual rows when folder expansion changes', () => {
    expect(explorerSidebarSource).toContain(
      "import { buildVisibleExplorerRows, type ExplorerTreeRow } from '../services/explorerRows';",
    );
    expect(explorerSidebarSource).toContain('$: flattenedRows = buildVisibleExplorerRows(');
    expect(explorerSidebarSource).toContain('expandedFolders,');
    expect(explorerSidebarSource).toContain('creatingParentPath,');
  });

  it('prevents accidental text selection while clicking explorer icons', () => {
    expect(styles).toMatch(/\.file-tree\s*\{[\s\S]*?user-select:\s*none;/);
    expect(styles).toMatch(/\.rename-input\s*\{[\s\S]*?user-select:\s*text;/);
  });

  it('places the new tab button after the last visible tab and hides it when tabs overflow', () => {
    const containerStart = documentTabsSource.indexOf('class="tabs-container"');
    const addButtonIndex = documentTabsSource.indexOf('class="tab-add"');
    const containerEnd = documentTabsSource.indexOf('</div>', addButtonIndex);

    expect(containerStart).toBeGreaterThan(-1);
    expect(addButtonIndex).toBeGreaterThan(containerStart);
    expect(containerEnd).toBeGreaterThan(addButtonIndex);
    expect(documentTabsSource).not.toContain('class="tab-actions"');
    expect(documentTabsSource).toContain('{#if showAddButton}');
    expect(documentTabsSource).toContain('measureAndComputeVisible');
    expect(documentTabsSource).toContain('showDropdown');
    expect(styles).not.toContain('.tab-actions');
    expect(styles).toMatch(/\.tab-add\s*\{[\s\S]*?flex-shrink:\s*0;/);
    expect(styles).toContain('.tab-overflow-dropdown');
    expect(styles).toContain('.tab-dropdown-menu');
  });

  it('keeps narrow desktop chrome single-row and clips the editor toolbar without horizontal scroll', () => {
    const narrowDesktopStart = responsiveStyles.indexOf('@media (max-width: 920px)');
    const narrowDesktopStyles = extractCssBlock(responsiveStyles, '@media (max-width: 920px)');
    const railStyles = extractCssBlock(responsiveStyles, '.rail', narrowDesktopStart);
    const topbarStyles = extractCssBlock(responsiveStyles, '.topbar', narrowDesktopStart);
    const toolbarStyles = extractCssBlock(responsiveStyles, '.toolbar', narrowDesktopStart);

    expect(styles).toMatch(/--md-editor-effective-sidebar-width:\s*min\(/);
    expect(styles).toMatch(
      /grid-template-columns:\s*minmax\(0,\s*var\(--md-editor-effective-sidebar-width\)\)\s*minmax\(0,\s*1fr\);/,
    );
    expect(narrowDesktopStyles).not.toContain('.workspace');
    expect(railStyles).toMatch(/display:\s*flex;/);
    expect(railStyles).not.toMatch(/display:\s*none;/);
    expect(topbarStyles).toMatch(/flex-wrap:\s*nowrap;/);
    expect(topbarStyles).toMatch(/height:\s*40px;/);
    expect(toolbarStyles).toMatch(/flex-wrap:\s*nowrap;/);
    expect(toolbarStyles).toMatch(/overflow-x:\s*clip;/);
    expect(toolbarStyles).toMatch(/overflow-y:\s*hidden;/);
    expect(toolbarStyles).not.toMatch(/overflow-x:\s*auto;/);
  });

  it('opens preferences in a dedicated settings window', () => {
    expect(appSource).not.toContain('SettingsDrawer');
    expect(desktopWindowSource).toContain('openSettingsWindow');
    expect(settingsWindowSource).toContain('t.settingsTitle()');
    expect(settingsWindowSource).toContain('settings-nav');
    expect(settingsWindowSource).toContain('t.createSnapshotBeforeSave()');
    expect(settingsWindowSource).toContain('autoSaveDelayMs');
    expect(settingsWindowSource).toContain('t.largeDocumentLimit()');
    expect(settingsWindowSource).toContain('t.folderOpenDefaultBehavior()');
    expect(settingsWindowSource).toContain('filePreviewEnabled');
    expect(settingsWindowSource).toContain('writingStatsVisible');
    expect(settingsWindowSource).toContain('writingStatsMetric');
    expect(settingsWindowSource).toContain('readingTimeVisible');
    expect(settingsWindowSource).toContain('closeWindowBehavior');
    expect(settingsWindowSource).toContain('defaultImageWidth');
    expect(settingsWindowSource).toContain('disabled-pill');
    expect(settingsWindowSource).toContain('t.autoCleanLocalImages()');
    expect(settingsWindowSource).toContain('t.defaultCodeBlockLanguage()');
    expect(settingsWindowSource).toContain('t.defaultDiagramType()');
    expect(settingsWindowSource).toMatch(
      /:global\(html\),\s*:global\(body\)\s*\{\s*overflow:\s*hidden;/,
    );
    expect(settingsWindowSource).toMatch(/\.toggle-row\s*\{[\s\S]*?position:\s*relative;/);
    expect(settingsWindowSource).toMatch(/\.toggle-row input\s*\{[\s\S]*?right:\s*0;/);
    expect(appSource).toContain('DEFAULT_APP_PREFERENCES.filePreviewEnabled');
    expect(appSource).toContain('DEFAULT_APP_PREFERENCES.autoSaveEnabled');
    expect(appSource).toContain('DEFAULT_APP_PREFERENCES.closeWindowBehavior');
    expect(appSource).toContain('SETTINGS_UPDATED_EVENT');
    expect(appSource).toContain('applyAppPreferences');
    expect(appSource).toContain('autoSaveEnabled && desktopEnabled && dirty && nativePath');
    expect(appSource).toContain('previewTabId = filePreviewEnabled ? targetTab.id : null');
  });

  it('keeps package-managed Windows settings and Store updates visible without installer actions', () => {
    expect(settingsWindowSource).toContain('mdAssociationStatus?.managedByPackage');
    expect(settingsWindowSource).toContain('t.openWindowsDefaultApps()');
    expect(settingsWindowSource).toContain('contextMenuStatus?.managedByPackage');
    expect(settingsWindowSource).toContain("softwareUpdateSnapshot.installationKind === 'store'");
    expect(softwareUpdateDialogSource).toContain("state.installationKind === 'store'");
    expect(softwareUpdateDialogSource).toContain('{#if !isStore || state.storeProductId}');
    expect(softwareUpdateDialogSource).toContain('on:click={handlePrimaryAction}');
    expect(softwareUpdateDialogSource).toContain('if (isStore)');
    expect(softwareUpdateDialogSource).toContain('onOpenStore();');
    expect(appSource).toContain("invoke<{ shouldPrompt: boolean }>('get_legacy_installer_notice')");
  });

  it('wires the first and second batch settings to runtime behavior instead of placeholders', () => {
    expect(settingsWindowSource).toContain("on:click={() => setThemeMode('light')}");
    expect(settingsWindowSource).toContain("on:click={() => setThemeMode('dark')}");
    expect(settingsWindowSource).toContain("on:click={() => setThemeMode('system')}");
    expect(settingsWindowSource).toContain('availableThemes');
    expect(settingsWindowSource).toContain('setColorTheme');
    expect(settingsWindowSource).toContain('effectiveScheme');
    expect(settingsWindowSource).toContain('syncDesktopIcons: appearanceChanged');
    expect(appSource).toContain('payload.effectiveScheme');
    expect(appSource).toContain('getBrowserSystemScheme');
    expect(themeManagerSource).toContain('nativeColorScheme');
    expect(themeManagerSource).toContain('applyNativeColorScheme');
    expect(appSource).not.toContain(
      "requestedPreferences.themeMode === 'system'\n        ? await readEffectiveSystemScheme(desktopEnabled)",
    );
    expect(appSource).toContain('await syncSystemThemeFromDesktop({ writeBootSnapshot: true })');
    expect(tauriLibSource).toContain('WindowEvent::ThemeChanged');
    expect(tauriWindowCommandsSource).toContain('broadcast_system_theme_changed');
    expect(tauriWindowCommandsSource).toContain('system_theme_event_payload');
    expect(tauriWindowCommandsSource).toContain('crate::window::os::system_theme()');
    expect(tauriWindowCommandsSource).toContain('run_on_main_thread');
    expect(tauriMacosSource).toContain('NSUserDefaults');
    expect(tauriMacosSource).not.toContain('Command::new("defaults")');
    expect(settingsWindowSource).toContain('softwareUpdateUnsupportedMacos');
    expect(settingsWindowSource).toContain('HOMEBREW_SETUP_COMMAND');
    expect(releaseWorkflowSource).toContain('update-homebrew-cask');
    expect(releaseWorkflowSource).toContain('Casks/nomo.rb');
    expect(existsSync(resolve(__dirname, '../../Casks/nomo.rb'))).toBe(true);
    expect(
      readFileSync(resolve(__dirname, 'components/ContextMenu.svelte'), 'utf-8'),
    ).toContain('formatShortcutLabel');
    expect(settingsWindowSource).toContain('grid-template-columns: repeat(2, minmax(0, 1fr))');
    expect(settingsWindowSource).toContain('@media (max-width: 520px)');
    expect(settingsWindowSource).toContain('styleTokens.radiusMd');
    expect(settingsWindowSource).toContain('styleTokens.shadowRaised');
    expect(settingsWindowSource).toContain('id="zoomPercent"');
    expect(settingsWindowSource).toContain('ctrlWheelZoomEnabled');
    expect(settingsWindowSource).toContain('codeBlockLineNumbersVisible');
    expect(settingsWindowSource).toContain('inlineCodeRenderingEnabled');
    expect(settingsWindowSource).toContain('inlineCodeRenderingEnabled');
    expect(settingsWindowSource).toContain('setCodeBlockIndent');
    expect(settingsWindowSource).toContain('id="defaultImageWidth"');
    expect(settingsWindowSource).toContain('setImageDefaultAlign');
    expect(settingsWindowSource).toContain('testPicgoConnection');
    expect(settingsWindowSource).toContain('bindMarkdownAssociation');
    expect(settingsWindowSource).toContain('id="outlineDefaultExpandLevel"');
    expect(settingsWindowSource).toContain('shortcutItems');
    expect(settingsWindowSource).toContain('updateShortcut');

    expect(settingsServiceSource).toContain('APPEARANCE_THEME_MODEL_MIGRATION_KEY');
    expect(appSource).toContain('setupSystemThemeListener');
    expect(appSource).toContain('handleGlobalWheel');
    expect(appSource).toContain(
      'applyZoomSetting(zoomPercent, { onFrame: refreshEditorViewportLayout })',
    );
    expect(appSource).toContain('applyCodeBlockLineNumberSetting(codeBlockLineNumbersVisible)');
    expect(appSource).toContain('editor.updateOptions({ inlineCodeRenderingEnabled })');
    expect(appSource).toContain(
      'document.documentElement.dataset.codeBlockIndent = codeBlockIndent',
    );
    expect(appSource).toContain('applyOutlineDefaultExpansion');
    expect(appSource).toContain('shortcutPreferences');
    expect(appSource).toContain('requestExitApp()');

    expect(tauriImageAssetsSource).toContain('pub(crate) fn test_picgo_connection');
    expect(tauriImageAssetsSource).toContain('create_picgo_core_command(command)');
    expect(tauriWindowCommandsSource).toContain('pub(crate) fn register_markdown_file_association');
    expect(tauriWindowCommandsSource).toContain('pub(crate) fn request_exit_app');
    expect(tauriTraySource).toContain('emit_exit_request(app)');
    expect(tauriMenuSource).toContain('emit_exit_request(window.app_handle())');
    expect(tauriMenuSource).toContain('emit_exit_request(app)');
  });

  it.skip('supports closing windows to the system tray when enabled', () => {
    expect(tauriLibSource).toContain('crate::window::tray::install_app_tray');
    expect(tauriLibSource).toContain('crate::window::commands::hide_window_to_tray');
    expect(tauriLibSource).toContain(
      'crate::window::tray::sync_tray_active_with_window_visibility',
    );
    expect(tauriLibSource).toContain('WindowEvent::CloseRequested');
    expect(tauriTraySource).toContain('TrayIconBuilder::with_id');
    expect(tauriTraySource).toContain('i18n::app_text(app, "tray_open")');
    expect(tauriTraySource).toContain('nomo-tray-dark-active-24-preview.png');
    expect(tauriTraySource).toContain('nomo-tray-dark-inactive-24-preview.png');
    expect(tauriTraySource).toContain('nomo-tray-light-active-24-preview.png');
    expect(tauriTraySource).toContain('nomo-tray-light-inactive-24-preview.png');
    expect(tauriTraySource).toContain('set_tray_active');
    expect(tauriTraySource).toContain('sync_tray_active_with_window_visibility');
    expect(tauriTraySource).toContain('set_desktop_icon_theme');
    expect(tauriTraySource).toContain('nomo/macos/nomo-app-light-256.png');
    expect(tauriTraySource).toContain('nomo/macos/nomo-app-dark-256.png');
    expect(tauriWindowCommandsSource).toContain('get_desktop_system_theme');
    expect(tauriLibSource).toContain('crate::window::commands::get_desktop_system_theme');
    expect(tauriTraySource).toContain('apply_window_icons');
    expect(tauriTraySource).toContain('.set_icon(icon.clone())');
    expect(tauriTraySource).toContain('apply_dock_icon(app, theme)');
    expect(tauriTraySource).toContain('run_on_main_thread');
    expect(tauriTraySource).toContain('setApplicationIconImage(Some(&icon))');
    expect(tauriLibSource).toContain('crate::window::commands::set_desktop_icon_theme');
    expect(desktopWindowSource).toContain("invoke('set_desktop_icon_theme'");
    expect(desktopWindowSource).toContain("invoke<'light' | 'dark'>('get_desktop_system_theme'");
    expect(appSource).toContain('getBrowserSystemScheme');
    expect(appSource).toContain('payload.effectiveScheme');
    expect(appSource).toContain('syncSystemThemeFromDesktop({ transition: true, systemScheme })');
    expect(tauriTraySource).toContain('i18n::app_text(app, "tray_exit")');
    expect(tauriTraySource).toContain('emit_exit_request(app)');
    expect(tauriTraySource).toContain('TrayIconEvent::DoubleClick');
    expect(tauriTraySource).toContain('closeWindowBehavior');
    const showMainWindowStart = tauriTraySource.indexOf('pub(crate) fn show_main_window');
    const showMainWindowEnd = tauriTraySource.indexOf(
      'fn show_window_by_label',
      showMainWindowStart,
    );
    const showMainWindowSource = tauriTraySource.slice(showMainWindowStart, showMainWindowEnd);
    expect(tauriLibSource).toContain('WindowEvent::Focused(true)');
    expect(tauriLibSource).toContain('record_last_active_window(window.app_handle(), label)');
    expect(tauriLibSource).toContain('WindowEvent::Destroyed');
    expect(tauriLibSource).toContain('forget_window(window.app_handle(), label)');
    expect(tauriTraySource).toContain('static LAST_ACTIVE_WINDOW');
    expect(tauriTraySource).toContain('static WINDOW_TITLES');
    expect(tauriTraySource).toContain('const TRAY_WINDOW_PREFIX');
    expect(tauriTraySource).toContain('choose_last_active_window(windows.iter().filter');
    expect(tauriTraySource).toContain('tray-window:');
    expect(tauriTraySource).toContain('show_window_by_label');
    expect(tauriTraySource).toContain('window_display_title');
    expect(showMainWindowSource).not.toContain('for (_label, window) in app.webview_windows()');
    expect(tauriWindowCommandsSource).toContain('pub(crate) fn report_window_title');
    expect(tauriLibSource).toContain('crate::window::commands::report_window_title');
    expect(desktopWindowSource).toContain("invoke('report_window_title'");
  });

  it.skip('registers desktop close and exit listeners before first-run sample work', () => {
    const mountStart = appSource.indexOf('onMount(async () =>');
    const setupCriticalEventsIndex = appSource.indexOf(
      'await setupCriticalDesktopEvents();',
      mountStart,
    );
    const listSettingsIndex = appSource.indexOf('settings = await listAppSettings()', mountStart);
    const firstRunIndex = appSource.indexOf('await maybeOpenFirstRunSample({', mountStart);

    expect(appSource).toContain('async function setupCriticalDesktopEvents()');
    expect(appSource).toContain("listen('nomo://request-exit-app'");
    expect(appSource).toMatch(/listen.*\('nomo:\/\/request-close-window'/);
    expect(setupCriticalEventsIndex).toBeGreaterThan(mountStart);
    expect(listSettingsIndex).toBeGreaterThan(setupCriticalEventsIndex);
    expect(firstRunIndex).toBeGreaterThan(setupCriticalEventsIndex);
  });

  it('uses a controlled IDEA-style close behavior choice instead of raw confirm for repeated closes', () => {
    const resolverStart = appSource.indexOf(
      'async function resolveCloseWindowBehaviorForCloseRequest',
    );
    const resolverEnd = appSource.indexOf('function requestCloseWindowChoice', resolverStart);
    const resolverSource = appSource.slice(resolverStart, resolverEnd);

    expect(appSource).toContain('let closeWindowChoiceDialogOpen = false;');
    expect(appSource).toContain('function requestCloseWindowChoice()');
    expect(appSource).toContain('function resolveCloseWindowChoice(behavior: CloseWindowAction)');
    expect(appSource).toContain(
      'if (choice.remember) {\n      await persistCloseWindowBehavior(choice.behavior);',
    );
    expect(appSource).toContain("updateAppSetting('closeWindowBehavior', behavior)");
    expect(resolverSource).not.toContain('confirm(');
    expect(appSource).not.toContain('closeToTrayFirstPrompt');
    expect(appSource).toContain('open={closeWindowChoiceDialogOpen}');
    expect(appSource).toContain('closeWindowLabel={t.closeWindowBehaviorCloseWindow()}');
    expect(appSource).toContain('closeToTrayLabel={t.closeWindowBehaviorCloseToTray()}');
    expect(settingsWindowSource).toContain("setCloseWindowBehavior('ask-every-time')");
    expect(settingsWindowSource).toContain("setCloseWindowBehavior('close-window')");
    expect(settingsWindowSource).toContain("setCloseWindowBehavior('close-to-tray')");
  });

  it.skip('prompts before closing the window when any tab has unsaved changes', () => {
    const closeCurrentWindowStart = appSource.indexOf('async function closeCurrentWindow()');
    const closeCurrentWindowEnd = appSource.indexOf(
      'async function resolveCloseWindowBehaviorForCloseRequest',
      closeCurrentWindowStart,
    );
    const closeCurrentWindowSource = appSource.slice(
      closeCurrentWindowStart,
      closeCurrentWindowEnd,
    );

    expect(appSource).toContain('function getDirtyTabs(candidateTabs: Tab[])');
    expect(appSource).toContain('const dirtyTabs = getDirtyTabs(tabs);');
    expect(appSource).toContain('t.unsavedChangesBeforeClosingWindow({ names })');
    expect(closeCurrentWindowSource.indexOf('const dirtyTabs = getDirtyTabs(tabs);')).toBeLessThan(
      closeCurrentWindowSource.indexOf(
        'const closeBehavior = await resolveCloseWindowBehaviorForCloseRequest();',
      ),
    );
    expect(appSource).toContain(
      'if (dirty && activeTab && !dirtyTabs.some((tab) => tab.id === activeTab.id))',
    );
    expect(appSource).not.toContain(
      "if (closeBehavior === 'close-window' && dirtyTabs.length > 0)",
    );
    expect(appSource).toContain(
      "listen<{ windowLabel?: string; window_label?: string }>('nomo://request-close-window'",
    );
    expect(appSource).toContain(
      'const requestedWindowLabel = event.payload?.windowLabel ?? event.payload?.window_label;',
    );
    expect(tauriLibSource).toContain('请求前端确认关闭');
    expect(tauriLibSource).not.toContain('窗口隐藏到托盘：{label}');
  });

  it('only skips close confirmation for clean preview tabs', () => {
    expect(appSource).toContain('const tabToClose = tabs.find((t) => t.id === tabId);');
    expect(appSource).toContain(
      'const dirtyTabToClose = getDirtyTabs([tabToClose]).find((tab) => tab.id === tabId);',
    );
    expect(appSource).toContain('if (dirtyTabToClose && !tabToClose.dirty)');
    expect(appSource).toContain('if (tabId === previewTabId && !dirtyTabToClose)');
    expect(documentActionsSource).toContain('t.confirmCloseModifiedFile()');
  });

  it('bundles and opens the first-run sample document through the normal document flow', () => {
    const tauriConfig = JSON.parse(tauriConfigSource);

    expect(tauriConfig.bundle.resources).toEqual({
      '../sample.md': 'samples/sample.md',
      '../assets/128x128.png': 'samples/assets/128x128.png',
    });
    expect(tauriLibSource).toContain('crate::file_system::install_sample_document');
    expect(tauriStorageSource).toContain(
      "invoke<NativeDocumentPayload>('install_sample_document')",
    );
    expect(appSource).toContain('maybeOpenFirstRunSample');
    expect(appSource).toContain('documentActions.applyNativeDocument(document,');
    expect(appSource).toContain('FIRST_RUN_SAMPLE_DOCUMENT_OPENED_KEY');
    expect(appSource).toContain('showToast(message, 3500)');
  });

  it('does not render a fake standalone explorer file without an open document', () => {
    expect(explorerSidebarSource).toContain(
      '$: hasStandaloneFile = fileName.trim().length > 0 && filePath.trim().length > 0;',
    );
    expect(explorerSidebarSource).toContain('{:else if hasStandaloneFile}');
  });

  it('promotes an explorer preview tab when the same file is formally opened', () => {
    expect(explorerSidebarSource).toContain('openPreviewFile(path)');
    expect(documentActionsSource).toContain('getPreviewTabId(): string | null;');
    expect(documentActionsSource).toContain('setPreviewTabId(value: string | null): void;');
    expect(documentActionsSource).toContain('existingTab.id === options.getPreviewTabId()');
    expect(documentActionsSource).toContain('options.setPreviewTabId(null)');
    expect(appSource).toContain('getPreviewTabId: () => previewTabId');
    expect(appSource).toContain('setPreviewTabId: (value) => {');
  });

  it('offers Save As as an explicit external-change resolution path', () => {
    expect(externalChangeDialogSource).toContain('export let onSaveAs: () => void;');
    expect(externalChangeDialogSource).toContain('on:click={onSaveAs}');
    expect(appSource).toContain('onSaveAs={handleExternalChangeSaveAs}');
  });

  it('does not surface the app own in-flight segmented save as an external conflict', () => {
    expect(appSource).toContain('if (result.saveInProgress) return;');
  });

  it('never applies an automatic external-change preference to dirty content', () => {
    expect(appSource).toContain("if (change.type !== 'modified' || change.dirtyAtDetection)");
  });

  it('binds segmented save and external-check results to their originating session', () => {
    expect(appSource).toContain('const preparedSave = await segmentedWorkspace?.prepareSave();');
    expect(appSource).not.toContain(
      'await segmentedDocumentPort.flushJournal(savingSessionId, frozenRevision);',
    );
    expect(appSource).toContain('const savingSessionId = activeTab.sessionId;');
    expect(appSource).toContain('segmentedWorkspace?.applySaveResult(savingSessionId, result)');
    expect(appSource).toContain('const checkingSessionId = activeTab.sessionId;');
    expect(appSource).toContain('if (result.sessionId !== savingSessionId)');
    expect(appSource).toContain('if (result.sessionId !== checkingSessionId) return;');
    expect(appSource).toContain('reconcileSegmentedExternalChangeCheck(result, {');
    expect(appSource).toContain('segmentedWorkspace?.hasPendingEdits()');
    expect(appSource).toContain('const { dirtyAtDetection } = reconciledCheck;');
    expect(appSource).toContain('if (activeTabId !== checkingTabId)');
  });

  it('restores the active segmented tab before opening inactive segmented sessions', () => {
    expect(appSource).toContain('partitionPersistedWorkspaceTabsForRestore(');
    expect(appSource).not.toContain(
      "(tab) => tab.documentKind === 'markdown' || tab.id === state.activeTabId",
    );
    expect(appSource).toContain('restoreDeferredWorkspaceTabs(deferredTabs, order, generation)');
    expect(appSource).toContain('if (workspaceRestorePreparation || deferredWorkspaceRestore)');
    expect(appSource).toContain('opened.documentKind !== persistedTab.documentKind');
  });

  it('guards the whole workspace restore against partial persistence and cleans stale sessions', () => {
    expect(appSource).toContain('let workspaceRestorePreparation: Promise<void> | null = null;');
    expect(appSource).toContain('await workspaceRestorePreparation;');
    expect(appSource).toMatch(
      /async function openFolderInCurrentWindow[\s\S]*?beginWorkspaceRestorePreparation\(\)[\s\S]*?await loadFolder\(folderPath\)[\s\S]*?await restoreFolderWorkspaceState\(folderPath\)/,
    );
    expect(appSource).toContain('const restoreBarrier = new Promise<void>');
    expect(appSource.indexOf('deferredWorkspaceRestore = restoreBarrier;')).toBeLessThan(
      appSource.indexOf('for (const persistedTab of immediateTabs)'),
    );
    expect(appSource).toContain('await discardRestoredSegmentedTabs(restoredTabs);');
    expect(appSource).toContain(
      'finishWorkspaceRestore(restoreBarrier, resolveRestore, generation)',
    );
  });

  it('cancels deferred restores before closing sessions and invalidates every preview teardown', () => {
    expect(appSource).toContain('async function cancelDeferredWorkspaceRestore()');
    expect(appSource).toMatch(
      /async function closeCurrentWindow\(\)[\s\S]*?await cancelDeferredWorkspaceRestore\(\);[\s\S]*?await closeAllSegmentedSessions/,
    );
    expect(appSource).toMatch(
      /async function closeAllTabsWithConfirmation[\s\S]*?invalidatePendingPreviewOpen\(\);/,
    );
    expect(appSource).toMatch(
      /function pinPreviewTab\(\)[\s\S]*?invalidatePendingPreviewOpen\(\);/,
    );
    expect(appSource).toMatch(
      /async function closeTab\(tabId: string, event\?: Event\)[\s\S]*?invalidatePendingPreviewOpen\(\);/,
    );
  });

  it('closes restored segmented sessions before replacing a workspace with a pending folder', () => {
    const switchStart = appSource.indexOf('// 若待打开文件夹与恢复的工作区不同');
    const cancelRestore = appSource.indexOf('await cancelDeferredWorkspaceRestore();', switchStart);
    const closeSessions = appSource.indexOf('await closeAllSegmentedSessions(false);', switchStart);
    const clearTabs = appSource.indexOf('clearAllTabsWithoutCreatingBlank();', switchStart);

    expect(switchStart).toBeGreaterThan(-1);
    expect(cancelRestore).toBeGreaterThan(switchStart);
    expect(closeSessions).toBeGreaterThan(cancelRestore);
    expect(clearTabs).toBeGreaterThan(closeSessions);
  });

  it('binds every external-change dialog action to the originating tab and session', () => {
    expect(appSource).toContain('function openExternalChangeDialog(');
    expect(appSource).toContain('function getValidExternalChangeDialogTarget()');
    expect(appSource).toContain('externalChangeDialogTargetSessionId');
    expect(appSource).toContain('const target = getValidExternalChangeDialogTarget();');
  });

  it('suppresses only the exact segmented external identity the user ignored', () => {
    expect(appSource).toContain(
      'const ignoredSegmentedExternalChanges = new Map<string, string>()',
    );
    expect(appSource).toContain('getSegmentedExternalChangeToken(result)');
    expect(appSource).toContain(
      'ignoredSegmentedExternalChanges.get(checkingSessionId) === changeToken',
    );
    expect(appSource).toContain('ignoredSegmentedExternalChanges.set(sessionId, changeToken)');
    expect(appSource).toContain(
      'ignoreSegmentedExternalChange(target.sessionId, target.changeToken)',
    );
    expect(appSource).toContain('openExternalChangeDialog(change, changeToken)');
    expect(appSource).toContain('保留 tab 上的冲突状态以继续暂停自动保存');
    expect(appSource).toContain('同一磁盘身份只是不再弹框');
  });

  it('keeps a segmented tab open and surfaces flush, save and close failures', () => {
    const closeStart = appSource.indexOf('async function closeSegmentedTab(');
    const closeEnd = appSource.indexOf('// 包装 closeTab', closeStart);
    const closeSource = appSource.slice(closeStart, closeEnd);

    expect(closeSource).toContain(
      'await segmentedDocumentPort.flushJournal(tabToClose.sessionId, tabToClose.revision);',
    );
    expect(closeSource).not.toContain('.catch(() => undefined)');
    expect(closeSource).toContain('.catch((error) => {');
    expect(closeSource).toContain(
      'await segmentedDocumentPort.closeSession(tabToClose.sessionId, discardChanges);',
    );
    expect(closeSource.match(/showVisibleError\(error, t\.saveFileFailed\(\)\)/g)?.length).toBe(3);
    expect(appSource).toMatch(/function showVisibleError[\s\S]*?showToast\(message, 3500\)/);
  });

  it('drops stale preview opens and closes their segmented sessions', () => {
    expect(appSource).toContain('const requestGeneration = invalidatePendingPreviewOpen();');
    expect(appSource).toContain('requestGeneration !== previewOpenGeneration');
    expect(appSource).toContain('segmentedDocumentPort.closeSession(opened.sessionId, false)');
  });

  it('reloads segmented external changes through the atomic backend seam', () => {
    expect(appSource).toContain('segmentedDocumentPort.reloadSession(oldSessionId)');
    expect(appSource).not.toContain(
      'await segmentedDocumentPort.closeSession(oldSessionId, true);',
    );
  });

  it('pins conditional segmented rows without reserving space for a toolbar', () => {
    expect(segmentedEditorCssSource).not.toContain('.segmented-toolbar');
    expect(segmentedEditorCssSource).toContain('.segmented-search {\n  grid-row: 1;');
    expect(segmentedEditorCssSource).toContain('.readonly-notice {\n  grid-row: 2;');
    expect(segmentedEditorCssSource).toContain('.segmented-scroll {\n  grid-row: 3;');
    expect(segmentedEditorCssSource).toContain('.segmented-status {\n  grid-row: 4;');
  });

  it('does not leave open sessions pointing at stale paths after explorer renames', () => {
    expect(appSource).toContain('getOpenDocumentRenameBlock(tabs, path, targetPath)');
    expect(appSource).toContain('statusMessage = t.renameOpenDocumentBlocked();');
    expect(appSource).toMatch(
      /try \{\s*await renameFile\(path, targetPath\);\s*\} catch \(err\) \{\s*statusMessage = t\.renameFailed\(\{ error: err \}\);\s*return;/,
    );
  });

  it('removes application-level workspace storage path configuration', () => {
    expect(settingsWindowSource).not.toContain('工作区存储路径');
    expect(settingsWindowSource).not.toContain('workspaceDir');
    expect(settingsWindowSource).not.toContain('browseFolder');
    expect(settingsWindowSource).not.toContain('selectedDir');

    expect(appSource).not.toContain("updateAppSetting('workspaceDir'");
    expect(appSource).not.toContain("settings.find((s) => s.key === 'workspaceDir')");
    expect(appSource).not.toContain('getDefaultWorkspaceDir');
    expect(tauriStorageSource).not.toContain('getDefaultWorkspaceDir');
    expect(tauriStorageSource).not.toContain('get_default_workspace_dir');
    expect(tauriLibSource).not.toContain('get_default_workspace_dir');
    expect(tauriFileSystemSource).not.toContain('get_default_workspace_dir');
    expect(tauriConfigCommandsSource).not.toContain('workspaceDir');
  });

  it.skip('keeps automatic local image cleanup behind an image setting toggle', () => {
    const imageSettingsSource = readFileSync(
      resolve(__dirname, '../lib/services/render.ts'),
      'utf-8',
    );
    const appSettingsSource = readFileSync(resolve(__dirname, 'services/settings.ts'), 'utf-8');

    expect(imageSettingsSource).toContain('autoDeleteUnusedLocalImages: boolean');
    expect(imageSettingsSource).toContain('autoDeleteUnusedLocalImages: true');
    expect(appSettingsSource).toContain('autoDeleteUnusedLocalImages');
    expect(settingsWindowSource).toContain('autoDeleteUnusedLocalImages');
    expect(settingsWindowSource).toContain('autoDeleteUnusedLocalImages');
    expect(appSource).toContain('!imageSettings.autoDeleteUnusedLocalImages');
  });
});

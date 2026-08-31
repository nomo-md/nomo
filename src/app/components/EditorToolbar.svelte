<script lang="ts">
  import {
    AlignHorizontalSpaceAround,
    Bold,
    BookOpenText,
    Braces,
    CheckSquare,
    ChevronUp,
    Code2,
    CodeXml,
    Columns2,
    Heading1,
    Highlighter,
    Italic,
    Link,
    List,
    ListTree,
    Menu,
    MessageSquare,
    Info,
    Quote,
    ScanLine,
    Search,
    Sigma,
    Strikethrough,
    Superscript,
    Table2,
    TableOfContents,
    Underline,
    WandSparkles,
  } from '@lucide/svelte';
  import {
    DIAGRAM_TEMPLATES,
    type DiagramType,
    type EditorCommand,
    type InlinePendingMarks,
  } from '../../lib/editor-core';
  import type { EditorViewMode } from '../types';
  import { clickOutside } from '../actions/clickOutside';
  import { modeSwitchIndicator } from '../actions/motion';
  import { getDiagramTypeLabel, t } from '../i18n';

  export let interfaceLocale: string;
  export let mode: EditorViewMode;
  export let largeDocumentMode = false;
  export let contentWidthPercent: number;
  export let outlineVisible: boolean;
  export let toolbarShortcut: string;
  export let runCommand: (command: EditorCommand) => void;
  export let pendingInlineMarks: InlinePendingMarks;
  export let tablePickerOpen: boolean;
  export let openTablePicker: () => void;
  export let closeTablePicker: () => void;
  export let openLinkPicker: () => void;
  export let insertTableWithSize: (rows: number, columns: number) => void;
  export let updateContentWidth: (event: Event) => void;
  export let setMode: (mode: EditorViewMode) => void;
  export let splitAlignmentGuideVisible = false;
  export let toggleSplitAlignmentGuide: () => void = () => undefined;
  export let toggleOutlineVisible: () => void;
  export let toggleToolbar: () => void;
  export let inactive = false;
  export let openSearchPanel: () => void;
  export let mobile = false;
  export let openMobileDocuments: () => void = () => undefined;

  const tableRows = [1, 2, 3, 4, 5];
  const tableColumns = [1, 2, 3, 4, 5, 6];
  let previewRows = 3;
  let previewColumns = 4;
  let diagramPickerOpen = false;
  let widthPickerOpen = false;
  let stylePickerOpen = false;
  const styleShortcuts: Array<{ label: string; hint: string; command: EditorCommand }> = [
    { label: 'H1', hint: '#', command: { type: 'setHeading', level: 1 } },
    { label: 'H2', hint: '##', command: { type: 'setHeading', level: 2 } },
    { label: 'H3', hint: '###', command: { type: 'setHeading', level: 3 } },
    { label: t.bold(), hint: '**', command: { type: 'toggleBold' } },
    { label: t.italic(), hint: '*', command: { type: 'toggleItalic' } },
    { label: t.strikethrough(), hint: '~~', command: { type: 'toggleStrikethrough' } },
    { label: t.underline(), hint: '<u>', command: { type: 'toggleUnderline' } },
    { label: t.highlight(), hint: '==', command: { type: 'toggleHighlight' } },
    { label: t.inlineCode(), hint: '`', command: { type: 'toggleCode' } },
    { label: t.quote(), hint: '>', command: { type: 'toggleBlockquote' } },
    { label: t.unorderedList(), hint: '-', command: { type: 'toggleBulletList' } },
    { label: t.orderedList(), hint: '1.', command: { type: 'toggleOrderedList' } },
    { label: t.taskList(), hint: '[ ]', command: { type: 'toggleTaskList' } },
  ];

  $: if (inactive) {
    diagramPickerOpen = false;
    widthPickerOpen = false;
    stylePickerOpen = false;
  }

  function closeStylePicker() {
    stylePickerOpen = false;
  }

  function toggleStylePicker() {
    closeTablePicker();
    closeDiagramPicker();
    closeWidthPicker();
    stylePickerOpen = !stylePickerOpen;
  }

  function applyStyle(command: EditorCommand) {
    runCommand(command);
    stylePickerOpen = false;
  }

  function handleStylePickerKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      stylePickerOpen = false;
    }
  }

  function toggleTablePicker() {
    if (tablePickerOpen) {
      closeTablePicker();
      return;
    }
    previewRows = 3;
    previewColumns = 4;
    closeDiagramPicker();
    closeWidthPicker();
    openTablePicker();
  }

  function handleTablePickerKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeTablePicker();
    }
  }

  function closeDiagramPicker() {
    diagramPickerOpen = false;
  }

  function toggleDiagramPicker() {
    closeTablePicker();
    closeWidthPicker();
    diagramPickerOpen = !diagramPickerOpen;
  }

  function closeWidthPicker() {
    widthPickerOpen = false;
  }

  function toggleWidthPicker() {
    closeTablePicker();
    closeDiagramPicker();
    widthPickerOpen = !widthPickerOpen;
  }

  function handleWidthPickerKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeWidthPicker();
    }
  }

  function openLinkEditor() {
    closeDiagramPicker();
    closeWidthPicker();
    closeStylePicker();
    openLinkPicker();
  }

  function openSearch() {
    closeTablePicker();
    closeDiagramPicker();
    closeWidthPicker();
    openSearchPanel();
  }

  function closeResponsivePickers() {
    closeTablePicker();
    closeDiagramPicker();
    closeWidthPicker();
  }

  function insertBlankDiagram() {
    runCommand({ type: 'insertMermaidBlock' });
    closeDiagramPicker();
  }

  function insertDiagram(diagramType: DiagramType) {
    runCommand({ type: 'insertDiagramBlock', diagramType });
    closeDiagramPicker();
  }

  function handleDiagramPickerKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeDiagramPicker();
    }
  }
</script>

<svelte:window on:resize={closeResponsivePickers} />

{#key interfaceLocale}
  <div class="toolbar" aria-label={t.formatToolbar()} data-interface-locale={interfaceLocale}>
    {#if mobile}
      <button
        class="mobile-toolbar-menu"
        type="button"
        title={t.file()}
        aria-label={t.file()}
        on:click={openMobileDocuments}
      >
        <Menu size={20} />
      </button>
      <span class="mobile-toolbar-divider" aria-hidden="true"></span>
    {/if}
    <div class="toolbar-group toolbar-group-core">
      <div class="style-picker-anchor" use:clickOutside={closeStylePicker}>
        <button
          class="style-picker-button"
          class:active={stylePickerOpen}
          title={t.formatToolbar()}
          aria-label={t.formatToolbar()}
          aria-haspopup="dialog"
          aria-expanded={stylePickerOpen}
          on:mousedown|preventDefault
          on:click|stopPropagation={toggleStylePicker}
        >
          <WandSparkles size={17} />
        </button>
        {#if stylePickerOpen}
          <div
            class="style-picker-popover"
            role="dialog"
            tabindex="-1"
            aria-label={t.formatToolbar()}
            on:keydown={handleStylePickerKeydown}
          >
            {#each styleShortcuts as shortcut (shortcut.label + shortcut.hint)}
              <button
                type="button"
                on:mousedown|preventDefault
                on:click={() => applyStyle(shortcut.command)}
              >
                <strong>{shortcut.label}</strong>
                <small>{shortcut.hint}</small>
              </button>
            {/each}
            <button
              type="button"
              on:mousedown|preventDefault
              on:click={() => applyStyle({ type: 'insertCodeBlock' })}
            >
              <strong>{t.codeBlock()}</strong>
              <small>```</small>
            </button>
          </div>
        {/if}
      </div>
      <button
        title={t.title()}
        aria-label={t.setHeadingOne()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'setHeading', level: 1 })}
      >
        <Heading1 size={17} />
      </button>
      <button
        title={t.bold()}
        aria-label={t.toggleBold()}
        class:active={pendingInlineMarks.strong}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleBold' })}
      >
        <Bold size={17} />
      </button>
      <button
        title={t.italic()}
        aria-label={t.toggleItalic()}
        class:active={pendingInlineMarks.em}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleItalic' })}
      >
        <Italic size={17} />
      </button>
      <button
        title={t.strikethrough()}
        aria-label={t.toggleStrikethrough()}
        class:active={pendingInlineMarks.strikethrough}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleStrikethrough' })}
      >
        <Strikethrough size={17} />
      </button>
      <button
        title={t.underline()}
        aria-label={t.toggleUnderline()}
        class:active={pendingInlineMarks.underline}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleUnderline' })}
      >
        <Underline size={17} />
      </button>
      <button
        title={t.highlight()}
        aria-label={t.toggleHighlight()}
        class:active={pendingInlineMarks.highlight}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleHighlight' })}
      >
        <Highlighter size={17} />
      </button>
      <button
        title={t.link()}
        aria-label={t.editLink()}
        on:mousedown|preventDefault
        on:click={openLinkEditor}
      >
        <Link size={17} />
      </button>
    </div>

    <div class="toolbar-group toolbar-group-medium">
      <button
        title={t.inlineComment()}
        aria-label={t.insertInlineComment()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'insertCommentInline' })}
      >
        <MessageSquare size={17} />
      </button>
      <button
        title={t.quote()}
        aria-label={t.toggleQuote()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleBlockquote' })}
      >
        <Quote size={17} />
      </button>
      <button
        title={t.callout()}
        aria-label={t.insertCallout()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'insertCallout' })}
      >
        <Info size={17} />
      </button>
      <button
        title={t.unorderedList()}
        aria-label={t.toggleList()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleBulletList' })}
      >
        <List size={17} />
      </button>
      <button
        title={t.taskList()}
        aria-label={t.toggleTaskList()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'toggleTaskList' })}
      >
        <CheckSquare size={17} />
      </button>
    </div>

    <div class="toolbar-group toolbar-group-low">
      <div class="table-picker-anchor" use:clickOutside={closeTablePicker}>
        <button
          title={t.table()}
          aria-haspopup="dialog"
          aria-expanded={tablePickerOpen}
          aria-label={t.insertTable()}
          class:active={tablePickerOpen}
          on:click|stopPropagation={toggleTablePicker}
        >
          <Table2 size={17} />
        </button>
        {#if tablePickerOpen}
          <div
            class="table-picker-popover"
            role="dialog"
            aria-label={t.chooseTableSize()}
            tabindex="-1"
            on:keydown={handleTablePickerKeydown}
          >
            <div class="table-picker-header">
              <span>{t.table()}</span>
              <strong>{previewRows} × {previewColumns}</strong>
            </div>
            <div class="table-picker-grid" aria-label={t.tableRowsColumns()}>
              {#each tableRows as row}
                {#each tableColumns as column}
                  <button
                    type="button"
                    class="table-picker-cell"
                    class:active={row <= previewRows && column <= previewColumns}
                    aria-label={t.insertTableSize({ rows: row, columns: column })}
                    on:mouseenter={() => {
                      previewRows = row;
                      previewColumns = column;
                    }}
                    on:focus={() => {
                      previewRows = row;
                      previewColumns = column;
                    }}
                    on:click={() => insertTableWithSize(row, column)}
                  ></button>
                {/each}
              {/each}
            </div>
          </div>
        {/if}
      </div>
      <button
        title={t.codeBlock()}
        aria-label={t.insertCodeBlock()}
        on:click={() => runCommand({ type: 'insertCodeBlock' })}
      >
        <Code2 size={17} />
      </button>
      <button
        title={t.mathFormula()}
        aria-label={t.insertMathFormula()}
        on:click={() => runCommand({ type: 'insertMathBlock', tex: 'E = mc^2' })}
      >
        <Sigma size={17} />
      </button>
      <div class="diagram-picker-anchor" use:clickOutside={closeDiagramPicker}>
        <button
          title={t.diagram()}
          aria-haspopup="menu"
          aria-expanded={diagramPickerOpen}
          aria-label={t.insertDiagram()}
          class:active={diagramPickerOpen}
          on:click|stopPropagation={toggleDiagramPicker}
        >
          <Braces size={17} />
        </button>
        {#if diagramPickerOpen}
          <div
            class="diagram-picker-popover"
            role="menu"
            aria-label={t.insertDiagram()}
            tabindex="-1"
            on:keydown={handleDiagramPickerKeydown}
          >
            <div class="diagram-picker-header">{t.diagram()}</div>
            <button type="button" role="menuitem" on:click={insertBlankDiagram}>
              <span>{t.blankDiagram()}</span>
              <small>mermaid</small>
            </button>
            <div class="diagram-picker-header">{t.template()}</div>
            {#each DIAGRAM_TEMPLATES as template}
              <button type="button" role="menuitem" on:click={() => insertDiagram(template.type)}>
                <span>{getDiagramTypeLabel(template.type)}</span>
                <small>{template.type}</small>
              </button>
            {/each}
          </div>
        {/if}
      </div>
      <button
        title={t.insertFootnote()}
        aria-label={t.insertFootnote()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'insertFootnote' })}
      >
        <Superscript size={17} />
      </button>
      <button
        title={t.insertToc()}
        aria-label={t.insertToc()}
        on:mousedown|preventDefault
        on:click={() => runCommand({ type: 'insertToc' })}
      >
        <TableOfContents size={17} />
      </button>
    </div>
    <span class="divider toolbar-actions-divider"></span>
    <span class="toolbar-spacer"></span>
    <div class="toolbar-utilities">
      <button
        class="icon-button"
        title={t.searchReplace()}
        aria-label={t.searchReplace()}
        on:click={openSearch}
      >
        <Search size={18} />
      </button>
      <label
        class="range-control width-control width-control-expanded"
        title={mode === 'split' ? t.splitAdaptiveWidth() : t.contentWidth()}
      >
        <AlignHorizontalSpaceAround size={16} aria-hidden="true" />
        <span>{contentWidthPercent}%</span>
        <input
          type="range"
          min="45"
          max="90"
          step="1"
          value={contentWidthPercent}
          disabled={mode === 'split'}
          on:input={updateContentWidth}
        />
      </label>
      <div class="width-control-compact-anchor" use:clickOutside={closeWidthPicker}>
        <button
          class="width-control-compact"
          class:active={widthPickerOpen}
          type="button"
          title={mode === 'split' ? t.splitAdaptiveWidth() : t.contentWidth()}
          aria-label={mode === 'split' ? t.splitAdaptiveWidth() : t.contentWidth()}
          disabled={mode === 'split'}
          aria-haspopup="dialog"
          aria-expanded={widthPickerOpen}
          on:click|stopPropagation={toggleWidthPicker}
          on:keydown={handleWidthPickerKeydown}
        >
          <AlignHorizontalSpaceAround size={15} aria-hidden="true" />
          <span>{contentWidthPercent}%</span>
        </button>
        {#if widthPickerOpen}
          <div class="width-control-popover" role="dialog" aria-label={t.contentWidth()}>
            <strong>{t.contentWidth()}</strong>
            <span>{contentWidthPercent}%</span>
            <input
              type="range"
              min="45"
              max="90"
              step="1"
              value={contentWidthPercent}
              disabled={mode === 'split'}
              aria-label={t.contentWidth()}
              on:input={updateContentWidth}
              on:keydown={handleWidthPickerKeydown}
            />
          </div>
        {/if}
      </div>
      <div class="mode-switch" aria-label={t.mode()} use:modeSwitchIndicator={{ mode }}>
        <button
          title={largeDocumentMode
            ? t.largeDocumentStayReadonlySource()
            : t.semanticEditingTitle()}
          aria-label={t.semanticEditing()}
          aria-pressed={mode === 'semantic'}
          class:active={mode === 'semantic'}
          disabled={largeDocumentMode}
          on:click={() => setMode('semantic')}
        >
          <BookOpenText size={17} />
        </button>
        <button
          title={t.sourceModeTitle()}
          aria-label={t.sourceMode()}
          aria-pressed={mode === 'source'}
          class:active={mode === 'source'}
          on:click={() => setMode('source')}
        >
          <CodeXml size={17} />
        </button>
        <button
          title={largeDocumentMode ? t.largeDocumentStayReadonlySource() : t.splitMode()}
          aria-label={t.splitMode()}
          aria-pressed={mode === 'split'}
          class:active={mode === 'split'}
          disabled={largeDocumentMode}
          on:click={() => setMode('split')}
        >
          <Columns2 size={17} />
        </button>
      </div>
      {#if mode === 'split' && !largeDocumentMode}
        <button
          class="icon-button"
          class:active={splitAlignmentGuideVisible}
          title={t.toggleSplitAlignmentGuide()}
          aria-label={t.toggleSplitAlignmentGuide()}
          aria-pressed={splitAlignmentGuideVisible}
          on:click={toggleSplitAlignmentGuide}
        >
          <ScanLine size={18} />
        </button>
      {/if}
      <button
        class="icon-button"
        class:active={outlineVisible}
        title={outlineVisible ? t.hideOutline() : t.showOutline()}
        aria-label={outlineVisible ? t.hideOutline() : t.showOutline()}
        aria-pressed={outlineVisible}
        on:click={toggleOutlineVisible}
      >
        <ListTree size={18} />
      </button>
      <button
        class="toolbar-collapse-button"
        type="button"
        title={`${t.hideToolbar()} (${toolbarShortcut})`}
        aria-label={t.hideToolbar()}
        on:click={toggleToolbar}
      >
        <ChevronUp size={16} />
      </button>
    </div>
  </div>
{/key}

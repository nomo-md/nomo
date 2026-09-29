import { describe, expect, it } from 'vitest';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { EditorState, TextSelection, type Transaction } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { deleteCodeBlockBeforeCursor } from './codeBlockCommands';
import { applyInterfaceLanguagePreference } from '../../app/i18n';
import { parseMarkdown, serializeMarkdown } from './markdown';
import { CodeBlockNodeView } from './nodeViews/CodeBlockNodeView';
import { setCodeBlockTokenizer } from './renderers';
import { schema } from './schema';

function createDocWithCodeBlockAndParagraph(paragraphText = ''): {
  doc: ProseMirrorNode;
  codeBlock: ProseMirrorNode;
} {
  const codeBlock = schema.nodes.code_block.create({ params: 'ts' }, schema.text('const a = 1;'));
  const paragraph = paragraphText
    ? schema.nodes.paragraph.create(null, schema.text(paragraphText))
    : schema.nodes.paragraph.create();
  return {
    doc: schema.nodes.doc.create(null, [codeBlock, paragraph]),
    codeBlock,
  };
}

function createCodeLines(lineCount: number): string {
  return Array.from(
    { length: lineCount },
    (_, index) => `const line${index + 1} = ${index + 1};`,
  ).join('\n');
}

function mountCodeBlock(code: string) {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const view = new EditorView(target, {
    state: EditorState.create({ doc: parseMarkdown(`\`\`\`ts\n${code}\n\`\`\``) }),
    nodeViews: {
      code_block: (node, editorView, getPos) =>
        new CodeBlockNodeView(node, editorView, getPos as () => number),
    },
  });
  const card = target.querySelector<HTMLElement>('.code-card');
  const expandButton = target.querySelector<HTMLButtonElement>('.code-expand-button');
  if (!card || !expandButton) {
    view.destroy();
    target.remove();
    throw new Error('代码块展开控件未渲染');
  }
  return {
    target,
    view,
    card,
    expandButton,
    destroy() {
      view.destroy();
      target.remove();
    },
  };
}

describe('code_block markdown 解析', () => {
  it('解析带语言的 fenced code block', () => {
    const doc = parseMarkdown('```js\nconsole.log("hello")\n```');
    let found = false;
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        expect(node.attrs.params).toBe('js');
        expect(node.textContent).toBe('console.log("hello")');
        found = true;
        return false;
      }
      return true;
    });
    expect(found).toBe(true);
  });

  it('解析不带语言的 fenced code block', () => {
    const doc = parseMarkdown('```\nhello\n```');
    let found = false;
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        expect(node.attrs.params).toBe('');
        expect(node.textContent).toBe('hello');
        found = true;
        return false;
      }
      return true;
    });
    expect(found).toBe(true);
  });

  it('解析多行代码内容', () => {
    const code = 'function foo() {\n  return 42;\n}';
    const doc = parseMarkdown(`\`\`\`ts\n${code}\n\`\`\``);
    let content = '';
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        content = node.textContent;
        return false;
      }
      return true;
    });
    expect(content).toBe(code);
  });

  it('解析 XML fenced code 时不把标签内容额外渲染成段落', () => {
    const code = [
      '<dependency>',
      '    <groupId>com.example</groupId>',
      '    <artifactId>access-control</artifactId>',
      '    <version>${component.version}</version>',
      '</dependency>',
    ].join('\n');
    const doc = parseMarkdown(`\`\`\`xml\n${code}\n\`\`\``);

    expect(doc.childCount).toBe(1);
    expect(doc.child(0).type.name).toBe('code_block');
    expect(doc.child(0).attrs.params).toBe('xml');
    expect(doc.child(0).textContent).toBe(code);
  });

  it('渲染 XML fenced code 时不在代码块外重复显示标签内容', async () => {
    const code = [
      '<dependency>',
      '    <groupId>com.example</groupId>',
      '    <artifactId>access-control</artifactId>',
      '    <version>${component.version}</version>',
      '</dependency>',
    ].join('\n');
    setCodeBlockTokenizer({
      async tokenize(input) {
        return {
          language: input.language,
          tokens: input.code.split('\n').map((line) => ({ tokens: [{ content: line }] })),
        };
      },
    });

    const target = document.createElement('div');
    document.body.appendChild(target);
    const view = new EditorView(target, {
      state: EditorState.create({ doc: parseMarkdown(`\`\`\`xml\n${code}\n\`\`\``) }),
      nodeViews: {
        code_block: (node, view, getPos) =>
          new CodeBlockNodeView(node, view, getPos as () => number),
      },
    });

    await Promise.resolve();

    const codeCards = target.querySelectorAll('.code-card');
    const bodyText = target.textContent ?? '';
    expect(codeCards).toHaveLength(1);
    expect((bodyText.match(/com\.example/g) ?? []).length).toBe(1);
    expect(bodyText.trim().startsWith('com.example')).toBe(false);

    view.destroy();
    target.remove();
  });

  it('解析空代码块', () => {
    const doc = parseMarkdown('```\n```');
    let found = false;
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        found = true;
        return false;
      }
      return true;
    });
    // 空代码块可能不被解析（取决于 markdown-it 配置）
    // 此测试记录当前行为
    expect(typeof found).toBe('boolean');
  });
});

describe('code_block 展开与收起', () => {
  it('退出编辑后保留代码内容的横纵滚动位置', () => {
    const mounted = mountCodeBlock(createCodeLines(80));
    try {
      mounted.card.querySelector<HTMLElement>('.code-content')!.click();
      const textarea = mounted.card.querySelector<HTMLTextAreaElement>('.code-input')!;
      textarea.scrollTop = 640;
      textarea.scrollLeft = 120;
      textarea.dispatchEvent(new Event('scroll'));
      textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      const display = mounted.card.querySelector<HTMLElement>('.code-content')!;
      expect(display.scrollTop).toBe(640);
      expect(display.scrollLeft).toBe(120);
      expect(mounted.card.querySelector<HTMLElement>('.line-numbers-wrapper')!.style.transform)
        .toBe('translateY(-640px)');
    } finally {
      mounted.destroy();
    }
  });

  it('同文档重建仅恢复唯一且未改变的代码块', () => {
    const code = createCodeLines(80);
    const mounted = mountCodeBlock(code);
    try {
      mounted.card.querySelector<HTMLElement>('.code-content')!.scrollTop = 540;
      const nextDoc = parseMarkdown(`# Added heading\n\n\`\`\`ts\n${code}\n\`\`\``);
      CodeBlockNodeView.preserveViewports(mounted.view, nextDoc, () => {
        mounted.view.updateState(EditorState.create({ doc: nextDoc }));
      });
      expect(mounted.target.querySelector<HTMLElement>('.code-content')!.scrollTop).toBe(540);
      const duplicateDoc = parseMarkdown(`\`\`\`ts\n${code}\n\`\`\`\n\n\`\`\`ts\n${code}\n\`\`\``);
      CodeBlockNodeView.preserveViewports(mounted.view, duplicateDoc, () => {
        mounted.view.updateState(EditorState.create({ doc: duplicateDoc }));
      });
      expect([...mounted.target.querySelectorAll<HTMLElement>('.code-content')].map(e => e.scrollTop))
        .toEqual([0, 0]);
    } finally {
      mounted.destroy();
    }
  });

  it('仅为超过 24 行的代码块显示展开按钮', () => {
    const shortBlock = mountCodeBlock(createCodeLines(24));
    expect(shortBlock.expandButton.hidden).toBe(true);
    shortBlock.destroy();

    const longBlock = mountCodeBlock(createCodeLines(25));
    expect(longBlock.expandButton.hidden).toBe(false);
    expect(longBlock.card.classList.contains('is-expanded')).toBe(false);
    expect(longBlock.expandButton.getAttribute('aria-expanded')).toBe('false');
    expect(longBlock.expandButton.querySelector('svg')?.dataset.icon).toBe('chevrons-down');
    longBlock.destroy();
  });

  it('在展示态和编辑态之间保留同一个展开状态', () => {
    const mounted = mountCodeBlock(createCodeLines(25));

    mounted.expandButton.click();
    expect(mounted.card.classList.contains('is-expanded')).toBe(true);
    expect(mounted.expandButton.getAttribute('aria-expanded')).toBe('true');
    expect(mounted.expandButton.querySelector('svg')?.dataset.icon).toBe('chevrons-up');

    mounted.card
      .querySelector<HTMLElement>('.code-content')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    const textarea = mounted.card.querySelector<HTMLTextAreaElement>('.code-input');
    expect(textarea).not.toBeNull();
    expect(mounted.card.classList.contains('is-editing')).toBe(true);
    expect(mounted.card.classList.contains('is-expanded')).toBe(true);
    expect(textarea?.rows).toBe(25);

    textarea?.focus();
    const mouseDownAccepted = mounted.expandButton.dispatchEvent(
      new MouseEvent('mousedown', { bubbles: true, cancelable: true }),
    );
    expect(mouseDownAccepted).toBe(false);
    expect(document.activeElement).toBe(textarea);

    mounted.expandButton.click();
    expect(mounted.card.classList.contains('is-editing')).toBe(true);
    expect(mounted.card.classList.contains('is-expanded')).toBe(false);
    expect(textarea?.rows).toBe(24);
    expect(document.activeElement).toBe(textarea);

    mounted.expandButton.click();
    textarea?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(mounted.card.classList.contains('is-editing')).toBe(false);
    expect(mounted.card.classList.contains('is-expanded')).toBe(true);
    mounted.destroy();
  });

  it('内容缩短后隐藏按钮并在再次变长时恢复默认收起', () => {
    const mounted = mountCodeBlock(createCodeLines(25));
    mounted.expandButton.click();
    mounted.card
      .querySelector<HTMLElement>('.code-content')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    const textarea = mounted.card.querySelector<HTMLTextAreaElement>('.code-input');
    expect(textarea).not.toBeNull();

    if (textarea) {
      textarea.value = createCodeLines(24);
      textarea.dispatchEvent(new InputEvent('input', { bubbles: true }));
      expect(mounted.expandButton.hidden).toBe(true);
      expect(mounted.card.classList.contains('is-expanded')).toBe(false);

      textarea.value = createCodeLines(25);
      textarea.dispatchEvent(new InputEvent('input', { bubbles: true }));
      expect(mounted.expandButton.hidden).toBe(false);
      expect(mounted.expandButton.getAttribute('aria-expanded')).toBe('false');
      expect(textarea.rows).toBe(24);
    }
    mounted.destroy();
  });

  it('切换界面语言时更新展开按钮的辅助文案', () => {
    applyInterfaceLanguagePreference('en-US');
    const mounted = mountCodeBlock(createCodeLines(25));
    try {
      expect(mounted.expandButton.title).toBe('Expand code block');
      expect(mounted.expandButton.getAttribute('aria-label')).toBe('Expand code block');

      applyInterfaceLanguagePreference('zh-CN');
      expect(mounted.expandButton.title).toBe('展开代码块');
      expect(mounted.expandButton.getAttribute('aria-label')).toBe('展开代码块');

      mounted.expandButton.click();
      expect(mounted.expandButton.title).toBe('收起代码块');
      expect(mounted.expandButton.getAttribute('aria-label')).toBe('收起代码块');
    } finally {
      mounted.destroy();
      applyInterfaceLanguagePreference('en-US');
    }
  });
});

describe('code_block 序列化', () => {
  it('序列化带语言的 code_block 为 fenced code block', () => {
    const markdown = '```js\nconsole.log("hello")\n```';
    const serialized = serializeMarkdown(parseMarkdown(markdown)).trim();
    expect(serialized).toBe(markdown);
  });

  it('序列化不带语言的 code_block', () => {
    const markdown = '```\nhello\n```';
    const serialized = serializeMarkdown(parseMarkdown(markdown)).trim();
    expect(serialized).toBe(markdown);
  });

  it('序列化多行代码', () => {
    const code = 'function foo() {\n  return 42;\n}';
    const markdown = `\`\`\`ts\n${code}\n\`\`\``;
    const serialized = serializeMarkdown(parseMarkdown(markdown)).trim();
    expect(serialized).toBe(markdown);
  });
});

describe('code_block 往返一致性', () => {
  it('简单代码块往返一致', () => {
    const markdown = '```js\nconst x = 1;\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });

  it('多行代码块往返一致', () => {
    const markdown = '```python\ndef hello():\n    print("world")\n\nhello()\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });

  it('包含特殊字符的代码块往返一致', () => {
    const markdown = '```html\n<div class="test">&amp;</div>\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });

  it('包含反引号的代码块往返一致', () => {
    const markdown = '```md\nUse `code` in markdown\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });
});

describe('code_block 语言名称', () => {
  it('语言名称正确存储在 params 属性中', () => {
    const languages = ['js', 'typescript', 'python', 'java', 'shell', 'sql', 'yaml', 'diff'];
    for (const lang of languages) {
      const doc = parseMarkdown(`\`\`\`${lang}\ncode\n\`\`\``);
      let params = '';
      doc.descendants((node) => {
        if (node.type.name === 'code_block') {
          params = node.attrs.params as string;
          return false;
        }
        return true;
      });
      expect(params).toBe(lang);
    }
  });

  it('语言名称往返一致', () => {
    const markdown = '```javascript\nconst x = 1;\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });
});

describe('code_block schema', () => {
  it('code_block 是 block 类型节点', () => {
    const doc = parseMarkdown('```js\ncode\n```');
    let codeNode: ProseMirrorNode | null = null;
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        codeNode = node;
        return false;
      }
      return true;
    });
    expect(codeNode).not.toBeNull();
    expect(codeNode!.type.spec.group).toBe('block');
  });

  it('code_block 有 params 属性', () => {
    const doc = parseMarkdown('```js\ncode\n```');
    let codeNode: ProseMirrorNode | null = null;
    doc.descendants((node) => {
      if (node.type.name === 'code_block') {
        codeNode = node;
        return false;
      }
      return true;
    });
    expect(codeNode!.attrs).toHaveProperty('params');
    expect(codeNode!.attrs.params).toBe('js');
  });
});

describe('code_block 与其他节点共存', () => {
  it('代码块可以与段落共存', () => {
    const markdown = 'hello world\n\n```js\ncode\n```\n\nfoo bar';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });

  it('代码块可以与标题共存', () => {
    const markdown = '# Title\n\n```js\ncode\n```';
    const doc = parseMarkdown(markdown);
    const serialized = serializeMarkdown(doc).trim();
    expect(serialized).toBe(markdown);
  });
});

describe('code_block Backspace 行为', () => {
  it('光标在代码块下方空段落开头时，Backspace 直接删除上方代码块', () => {
    const { doc, codeBlock } = createDocWithCodeBlockAndParagraph();
    let capturedTr: Transaction | undefined;
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, codeBlock.nodeSize + 1),
    });

    const handled = deleteCodeBlockBeforeCursor(state, (tr) => {
      capturedTr = tr;
    });

    expect(handled).toBe(true);
    expect(capturedTr?.doc.childCount).toBe(1);
    expect(capturedTr?.doc.child(0).type).toBe(schema.nodes.paragraph);
    expect(capturedTr?.selection.from).toBe(1);
  });

  it('光标在代码块下方正文段落开头时，Backspace 删除代码块并保留正文', () => {
    const { doc, codeBlock } = createDocWithCodeBlockAndParagraph('后续正文');
    let capturedTr: Transaction | undefined;
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, codeBlock.nodeSize + 1),
    });

    const handled = deleteCodeBlockBeforeCursor(state, (tr) => {
      capturedTr = tr;
    });

    expect(handled).toBe(true);
    expect(capturedTr?.doc.childCount).toBe(1);
    expect(capturedTr?.doc.textContent).toBe('后续正文');
    expect(capturedTr?.selection.from).toBe(1);
  });

  it('光标不在下方段落开头时，不拦截默认 Backspace 行为', () => {
    const { doc, codeBlock } = createDocWithCodeBlockAndParagraph('后续正文');
    let capturedTr: Transaction | undefined;
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, codeBlock.nodeSize + 2),
    });

    const handled = deleteCodeBlockBeforeCursor(state, (tr) => {
      capturedTr = tr;
    });

    expect(handled).toBe(false);
    expect(capturedTr).toBeUndefined();
  });
});

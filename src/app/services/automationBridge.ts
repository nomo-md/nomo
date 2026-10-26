import { tick } from 'svelte';
import type { EditorCore } from '../../lib/editor-core';
import type { EditorViewMode, Tab } from '../types';
import { getMarkdownApplicationSession } from './markdownApplicationSession';
import { invoke } from '@tauri-apps/api/core';
import { createAutomationTelemetry } from './automationTelemetry';
import { getRenderQueueDiagnostics } from '../../lib/services/renderTaskQueue';
import type { EditorCommand } from '../../lib/editor-core';

type Request = { id: string; method: string; params?: Record<string, unknown> };
type Response = { id: string; ok: boolean; result?: unknown; error?: string; durationMs?: number };

/** 只由测试构建动态加载。串行调用，不保留正文、DOM 或历史响应。 */
export function startAutomationBridge(app: AutomationApp): () => void {
  const telemetry = createAutomationTelemetry();
  const dispatch = createDispatcher(app);
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let response: Response | undefined = { id: 'ready', ok: true, result: { protocol: 2,
    environment: { userAgent: navigator.userAgent, width: innerWidth, height: innerHeight, dpr: devicePixelRatio,
      language: navigator.language, theme: document.documentElement.dataset.theme ?? null } } };
  async function poll() {
    if (stopped) return;
    try {
      const request = await invoke<Request | null>('automation_exchange', { response });
      response = undefined;
      if (stopped) return;
      if (request) {
        const start = performance.now();
        try {
          const result = request.method === 'telemetry' ? telemetry.read() : await dispatch(request.method, request.params ?? {});
          response = { id: request.id, ok: true, result, durationMs: performance.now() - start };
        } catch (error) {
          response = { id: request.id, ok: false, error: String(error), durationMs: performance.now() - start };
        }
      }
      if (!stopped) timer = setTimeout(() => void poll(), response ? 0 : 50);
    } catch (error) {
      // 传输故障不重试，避免未知结果的写入被重复执行；脚本会超时并记录失败。
      console.error('[Nomo automation] transport stopped', error);
    }
  }
  void poll();
  return () => { stopped = true; clearTimeout(timer); telemetry.dispose(); };
}

interface AutomationApp {
  getEditor(): EditorCore;
  getActiveTabId(): string;
  getMode(): EditorViewMode;
  getTabs(): Tab[];
  getDirty(): boolean;
  open(path: string): Promise<boolean>;
  switchTab(id: string): Promise<void>;
  changeMode(mode: EditorViewMode): Promise<boolean>;
  save(): Promise<boolean>;
  close(id: string): Promise<void>;
  create(): Promise<void>;
  checkpoint(): Promise<void>;
  exit(): Promise<void>;
  workspace(): unknown;
  combine(first: string, second: string): Promise<void>;
  ungroup(): Promise<void>;
  checkExternal(): Promise<void>;
  reload(): Promise<void>;
  autosave(enabled: boolean): void;
  resources(): unknown;
  folder(path: string): Promise<void>;
}

function createDispatcher(app: AutomationApp) {
  const automationTabs = new Set<string>();
  const nodes = new WeakMap<Element, number>();
  let nodeId = 0;
  const identity = (node: Element) => { if (!nodes.has(node)) nodes.set(node, ++nodeId); return nodes.get(node); };
  const snapshot = () => {
    const session = getMarkdownApplicationSession(app.getEditor());
    const state = session?.document?.state;
    const images = [...document.querySelectorAll<HTMLImageElement>('.markdown-session-window img')]
      .filter(image => { const r = image.getBoundingClientRect(); return r.bottom > 0 && r.top < innerHeight && r.width > 0; });
    return {
      activeTabId: app.getActiveTabId(), mode: app.getMode(), workspace: app.workspace(), runtimeResources: app.resources(),
      tabs: app.getTabs().map(tab => ({ id: tab.id, fileName: tab.fileName, nativePath: tab.nativePath, dirty: tab.dirty,
        externalFileChange: tab.externalFileChange, testOwned: automationTabs.has(tab.id) })),
      document: state ? { ...state.status, progress: state.progress, timings: state.timings, failure: state.failure ? String(state.failure) : null,
        mountMode: state.mountMode, restriction: state.mountRestriction } : null,
      presentation: session?.presentation ?? null, views: session?.viewDiagnostics ?? null,
      selection: session?.captureSelection() ?? null, readingByte: session?.readingByte ?? null,
      renderQueues: getRenderQueueDiagnostics(), images: { visible: images.length, pending: images.filter(i => !i.complete).length,
        failed: images.filter(i => i.complete && !i.naturalWidth).length,
        errorPlaceholders: document.querySelectorAll('.markdown-session-window .image-node-placeholder').length },
      dom: { nodes: document.getElementsByTagName('*').length, semanticEditors: [...document.querySelectorAll('.ProseMirror')].map(identity),
        restrictedViews: document.querySelectorAll('.markdown-session-restricted').length,
        sourceEditors: document.querySelectorAll('.cm-editor').length, activeEditor: document.activeElement?.closest('.ProseMirror,.cm-editor')?.className ?? null },
      viewports: [...document.querySelectorAll<HTMLElement>('.markdown-session-scroll')]
        .filter(node => { const bounds = node.getBoundingClientRect(); return bounds.width > 0 && bounds.height > 0; })
        .map(node => ({ source: Boolean(node.querySelector('.cm-editor')), scrollTop: node.scrollTop, clientHeight: node.clientHeight, scrollHeight: node.scrollHeight })),
    };
  };

  // 测试版才加载；适配器复用正常打开/切换/保存链路，不维护第二套文档状态。
  return async (method: string, params: Record<string, unknown>) => {
    const active = () => {
      if (!automationTabs.has(app.getActiveTabId())) throw new Error('请先打开本次测试文档');
      const session = getMarkdownApplicationSession(app.getEditor());
      if (!session) throw new Error('当前接口仅支持 Markdown 文档');
      return session;
    };
    const settle = async () => {
      await tick();
      const session = getMarkdownApplicationSession(app.getEditor());
      if (session) { await session.ready; await session.flush(); }
      await tick();
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    };
    const text = (key: string) => {
      const value = params[key];
      if (typeof value !== 'string' || value.length > 65536) throw new Error(`无效参数：${key}`);
      return value;
    };
    switch (method) {
      case 'diagnostics': return snapshot();
      case 'folder': await app.folder(text('path')); break;
      case 'wait': {
        const stage = text('stage');
        if (!['readable', 'editable', 'stable', 'indexed', 'resources'].includes(stage)) throw new Error('未知完成阶段');
        const timeout = Math.min(600000, Math.max(100, Number(params.timeoutMs) || 30000));
        const until = performance.now() + timeout;
        do {
          const s = snapshot();
          if (s.document?.failure || s.presentation?.phase === 'failed') throw new Error(s.document?.failure ?? '视图失败');
          const p = s.presentation;
          if ((stage === 'stable' || stage === 'editable') && p?.mode === 'semantic' && s.dom.restrictedViews > 0)
            throw new Error(`UNSUPPORTED_SEMANTIC: ${s.dom.restrictedViews} 个区域仅支持源码，不能计作完整语义通过`);
          const ready = stage === 'indexed' ? s.document?.progress.state === 'ready'
            : stage === 'readable' ? p && p.phase !== 'preparing'
            : stage === 'editable' ? p?.phase === 'editable'
            : stage === 'stable' ? p?.timings.stable !== undefined && p.coverage !== 'pending'
            : p && p.coverage !== 'pending' && !s.images.pending && s.renderQueues.every(q => !q.active && !q.queued);
          if (ready) return s;
          await new Promise(resolve => setTimeout(resolve, 25));
        } while (performance.now() < until);
        throw new Error(`阶段超时：${stage}`);
      }
      case 'new':
        await app.create(); automationTabs.add(app.getActiveTabId()); break;
      case 'adoptRestored':
        // 仅供同一运行沙箱重启后的已恢复标签；原生配置目录已按运行隔离。
        for (const tab of app.getTabs()) if (tab.nativePath || tab.dirty) automationTabs.add(tab.id);
        break;
      case 'digest': {
        const doc = await active().ready; await doc.session.flushEdits();
        return invoke('automation_digest', { documentId: doc.state.status.documentId });
      }
      case 'readRange': {
        const doc = await active().ready; await doc.session.flushEdits();
        const from = Number(params.fromByte ?? 0), to = Number(params.toByte ?? Math.min(doc.state.status.byteLength, from + 4096));
        if (!Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from < 0 || to < from || to - from > 65536) throw new Error('无效读取范围');
        return doc.port.readWindow(doc.state.status.documentId, doc.state.status.revision, from, to);
      }
      case 'select': {
        const session = active(), doc = await session.ready;
        const from = Number(params.fromByte), to = Number(params.toByte ?? from);
        if (![from, to].every(n => Number.isSafeInteger(n) && n >= 0 && n <= doc.state.status.byteLength)) throw new Error('无效选区');
        await session.selectRange(from, to); break;
      }
      case 'reveal': {
        const session = active(), doc = await session.ready;
        const byte = Number(params.byte ?? 0);
        if (!Number.isSafeInteger(byte) || byte < 0 || byte > doc.state.status.byteLength) throw new Error('无效位置');
        await session.reveal(byte, true); break;
      }
      case 'outlineNavigate': {
        const session = active(), doc = await session.ready;
        const byte = Number(params.byte), revision = Number(params.revision);
        if (!Number.isSafeInteger(byte) || byte < 0 || byte > doc.state.status.byteLength || revision !== doc.state.status.revision) throw new Error('大纲位置或修订无效');
        await session.revealOutlineHeading(byte, revision); break;
      }
      case 'focus': active(); app.getEditor().focus(); break;
      case 'command': {
        active();
        const allowed = ['toggleBold', 'toggleItalic', 'toggleCode', 'toggleStrikethrough', 'toggleUnderline', 'toggleHighlight',
          'setHeading', 'setParagraph', 'toggleBlockquote', 'toggleBulletList', 'toggleOrderedList', 'toggleTaskList',
          'insertHorizontalRule', 'insertParagraphAfter', 'insertDetails', 'insertToc', 'insertTable',
          'addTableRowBefore', 'addTableRowAfter', 'addTableColumnBefore', 'addTableColumnAfter',
          'deleteTableRow', 'deleteTableColumn', 'deleteTable', 'toggleTableHeader', 'setTableColumnAlignment'];
        const command = params.command as EditorCommand;
        if (!command || !allowed.includes(command.type)) throw new Error('不允许的编辑命令');
        if (!app.getEditor().canExecute(command)) throw new Error('命令在当前位置不可用');
        if (!app.getEditor().execute(command)) throw new Error('命令未执行');
        break;
      }
      case 'paste': await active().paste({ text: text('text') }, params.plain === true ? 'plain' : 'auto'); break;
      case 'search': {
        const doc = await active().ready;
        const results = await doc.searches.search(text('query'), { signal: AbortSignal.timeout(30000), caseSensitive: params.caseSensitive === true, wholeWord: params.wholeWord === true });
        try { return { count: results.totalCount, revision: results.snapshotRevision, page: await results.page(Number(params.fromIndex) || 0) }; }
        finally { await results.dispose(); }
      }
      case 'searchCancel': {
        const doc = await active().ready;
        const abort = new AbortController();
        const pending = doc.searches.search(text('query'), { signal: abort.signal }); abort.abort();
        try { const result = await pending; await result.dispose(); return { cancelled: false }; }
        catch (error) { if (error instanceof DOMException && error.name === 'AbortError') return { cancelled: true }; throw error; }
      }
      case 'replaceAll': return { count: await active().replaceAll(text('query'), text('replacement'), { signal: AbortSignal.timeout(30000), caseSensitive: params.caseSensitive === true, wholeWord: params.wholeWord === true }) };
      case 'outline': {
        const doc = await active().ready;
        if (!doc.nativeStructure) throw new Error('原生结构索引尚未就绪');
        return doc.nativeStructure.client.query({ kind: 'outline', cursor: params.cursor as import('../../lib/markdown-document/structureProtocol').StructureCursor | undefined });
      }
      case 'checkpoint': active(); await settle(); await app.checkpoint(); break;
      case 'autosave': active(); app.autosave(params.enabled === true); break;
      case 'externalCheck': active(); await app.checkExternal(); break;
      case 'reload': active(); await app.reload(); break;
      case 'combine': {
        const first = text('first'), second = text('second');
        if (!automationTabs.has(first) || !automationTabs.has(second)) throw new Error('不是测试标签');
        await app.combine(first, second); break;
      }
      case 'ungroup': active(); await app.ungroup(); break;
      case 'exit':
        await settle(); await app.checkpoint();
        // 响应先落盘，随后走正常退出流程；不把 child.kill 当退出验证。
        setTimeout(() => void app.exit(), 200); return { exiting: true };
      case 'open': {
        const path = text('path');
        if (!/\.(md|markdown)$/i.test(path)) throw new Error('仅支持 Markdown fixture');
        if (!await app.open(path)) throw new Error('文件未打开');
        automationTabs.add(app.getActiveTabId());
        break;
      }
      case 'switch': {
        const id = text('tabId');
        if (!automationTabs.has(id)) throw new Error('不是本次测试标签');
        await app.switchTab(id);
        if (app.getActiveTabId() !== id) throw new Error('标签切换未完成');
        break;
      }
      case 'mode': {
        active();
        const value = text('mode');
        if (value !== 'semantic' && value !== 'source' && value !== 'split') throw new Error('无效编辑模式');
        if (!await app.changeMode(value)) throw new Error('模式切换失败');
        break;
      }
      case 'insert': {
        const session = active();
        await settle();
        if (app.getMode() === 'source') await session.insertText(text('text'));
        else if (!app.getEditor().execute({ type: 'insertText', text: text('text') })) throw new Error('编辑命令未执行');
        break;
      }
      case 'undo':
      case 'redo': {
        const doc = await active().ready;
        await doc.session.flushEdits();
        if (method === 'undo' ? !doc.state.status.canUndo : !doc.state.status.canRedo) throw new Error('没有可用历史');
        await doc.session.undo(method === 'redo');
        break;
      }
      case 'save':
        active();
        if (!app.getTabs().find(tab => tab.id === app.getActiveTabId())?.nativePath || !await app.save()) throw new Error('保存失败');
        break;
      case 'close': {
        active();
        const id = app.getActiveTabId();
        await settle();
        if (app.getDirty() || app.getTabs().find(tab => tab.id === id)?.dirty) throw new Error('请先保存再关闭');
        await app.close(id);
        if (app.getTabs().some(tab => tab.id === id)) throw new Error('标签关闭未完成');
        automationTabs.delete(id);
        break;
      }
      case 'read': {
        const session = active();
        await settle();
        if ((await session.ready).state.status.byteLength > 65536) throw new Error('read 只支持不超过 64 KiB 的测试文档');
        return { markdown: await session.readMarkdown() };
      }
      case 'state': break;
      default: throw new Error(`未知测试方法：${method}`);
    }
    await settle();
    return snapshot();
  };
}

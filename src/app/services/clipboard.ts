import type { EditorClipboardPayload } from '../../lib/editor-core';
import type { PreparedClipboardImage } from '../../lib/services/render';

export type ClipboardReadResult =
  | { kind: 'html'; html: string; text: string }
  | { kind: 'image'; files: File[] }
  | { kind: 'native-image'; image: PreparedClipboardImage }
  | { kind: 'text'; text: string };

export type ClipboardReadPreference = 'rich' | 'text';

export async function writeFileClipboard(path: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('copy_file_to_clipboard', { path });
}

export async function writeEditorClipboard(
  payload: EditorClipboardPayload,
  desktopEnabled: boolean,
): Promise<void> {
  if (desktopEnabled) {
    const { writeHtml, writeText } = await import('@tauri-apps/plugin-clipboard-manager');
    if (payload.html) {
      try {
        await writeHtml(payload.html, payload.text);
        return;
      } catch {
        // 部分平台或剪贴板实现不接受 HTML，降级为纯文本。
      }
    }
    await writeText(payload.text);
    return;
  }

  if (navigator.clipboard?.write && typeof ClipboardItem !== 'undefined' && payload.html) {
    await navigator.clipboard.write([
      new ClipboardItem({
        'text/plain': new Blob([payload.text], { type: 'text/plain' }),
        'text/html': new Blob([payload.html], { type: 'text/html' }),
      }),
    ]);
    return;
  }
  await navigator.clipboard.writeText(payload.text);
}

export async function readEditorClipboard(
  desktopEnabled: boolean,
  preference: ClipboardReadPreference = 'rich',
): Promise<ClipboardReadResult> {
  const webResult = await tryReadWebClipboard(preference);
  if (webResult) return webResult;
  if (!desktopEnabled) {
    const text = await navigator.clipboard.readText();
    return { kind: 'text', text };
  }

  const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
  if (preference === 'text') {
    return { kind: 'text', text: await readText().catch(() => '') };
  }
  const { invoke } = await import('@tauri-apps/api/core');
  // 原生读取失败返回 null；编码/落盘错误继续向调用方报告，不能伪装成文本粘贴。
  const image = await invoke<PreparedClipboardImage | null>('prepare_clipboard_image');
  if (image) return { kind: 'native-image', image };
  return { kind: 'text', text: await readText() };
}

export async function releasePreparedImage(image: PreparedClipboardImage): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('release_prepared_image', { assetId: image.assetId });
}

async function tryReadWebClipboard(
  preference: ClipboardReadPreference,
): Promise<ClipboardReadResult | null> {
  if (!navigator.clipboard?.read) return null;
  try {
    const items = await navigator.clipboard.read();
    const imageFiles: File[] = [];
    let html = '';
    let text = '';
    for (const item of items) {
      const imageType = item.types.find((type) => type.startsWith('image/'));
      if (imageType && preference !== 'text') {
        const blob = await item.getType(imageType);
        imageFiles.push(
          new File([blob], `clipboard.${extensionForMime(imageType)}`, { type: imageType }),
        );
      }
      if (!html && item.types.includes('text/html')) {
        html = await (await item.getType('text/html')).text();
      }
      if (!text && item.types.includes('text/plain')) {
        text = await (await item.getType('text/plain')).text();
      }
    }
    if (preference === 'text') return { kind: 'text', text };
    if (imageFiles.length) return { kind: 'image', files: imageFiles };
    if (html) return { kind: 'html', html, text };
    if (text) return { kind: 'text', text };
  } catch {
    // WebView 可能拒绝 clipboard.read；由桌面插件完成降级读取。
  }
  return null;
}

function extensionForMime(mime: string): string {
  if (mime === 'image/jpeg') return 'jpg';
  if (mime === 'image/gif') return 'gif';
  if (mime === 'image/webp') return 'webp';
  return 'png';
}

import { isTauriRuntime } from '../../lib/desktop/tauriStorage';
import {
  DEFAULT_IMAGE_HANDLING_SETTINGS,
  type ImageContext,
  type ImageImportInput,
  type ImageImportResult,
  type ImageLoader,
  type ImageResolveResult,
} from '../../lib/services/render';
import { t } from '../i18n';

interface NativeImageAssetPayload {
  markdown_src: string;
  absolute_path: string;
  reused?: boolean;
}

interface NativeImageResolvePayload {
  src: string;
  display_src: string;
  exists: boolean;
  absolute_path?: string | null;
  error?: string | null;
}

interface NativeImageUploadPayload {
  url: string;
}

interface NativeImageRemovePayload {
  src: string;
  removed: boolean;
  skipped?: boolean;
  error?: string | null;
}

export function createDesktopImageLoader(): ImageLoader {
  return {
    async resolve(src: string, context: ImageContext): Promise<ImageResolveResult> {
      if (isRemoteImageSrc(src) || !isTauriRuntime()) {
        return {
          src,
          displaySrc: src,
          exists: true,
        };
      }

      const { invoke, convertFileSrc } = await import('@tauri-apps/api/core');
      const result = await invoke<NativeImageResolvePayload>('resolve_image_asset', {
        input: {
          document_path: context.documentPath ?? null,
          src,
        },
      });

      return {
        src: result.src,
        displaySrc: result.exists ? convertFileSrc(result.display_src) : result.display_src,
        exists: result.exists,
        absolutePath: result.absolute_path ?? undefined,
        error: result.error ?? undefined,
      };
    },

    async import(input: ImageImportInput, context: ImageContext): Promise<ImageImportResult> {
      const settings = {
        ...DEFAULT_IMAGE_HANDLING_SETTINGS,
        ...context.settings,
      };

      if (settings.imageInsertStrategy === 'upload') {
        return uploadImage(input, context);
      }

      if (!isTauriRuntime()) {
        return {
          markdownSrc: createFallbackMarkdownSrc(
            context.documentFileName ?? 'document.md',
            input.fileName,
          ),
        };
      }

      if (!context.documentPath) {
        throw new Error(t.desktopImageInsertRequiresSavedFile());
      }

      const result = await invokeImage<NativeImageAssetPayload>('import_image_asset', input, {
        document_path: context.documentPath,
        document_file_name: context.documentFileName ?? 'document.md',
        strategy: settings.imageInsertStrategy,
        file_name: input.fileName,
      });

      return {
        markdownSrc: result.markdown_src,
        absolutePath: result.absolute_path,
        reused: result.reused ?? false,
      };
    },

    async remove(src: string, context: ImageContext) {
      if (isRemoteImageSrc(src) || !isTauriRuntime()) {
        return {
          src,
          removed: false,
          skipped: true,
        };
      }

      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<NativeImageRemovePayload>('delete_image_asset', {
        input: {
          document_path: context.documentPath ?? null,
          src,
        },
      });

      return {
        src: result.src,
        removed: result.removed,
        skipped: result.skipped ?? false,
        error: result.error ?? undefined,
      };
    },
  };
}

async function uploadImage(
  input: ImageImportInput,
  context: ImageContext,
): Promise<ImageImportResult> {
  if (!isTauriRuntime()) {
    throw new Error(t.imageUploadRequiresDesktop());
  }

  const settings = {
    ...DEFAULT_IMAGE_HANDLING_SETTINGS,
    ...context.settings,
  };
  const command =
    settings.uploadProvider === 'picgo'
      ? 'upload_image_via_picgo_server'
      : 'upload_image_via_picgo_core';
  const inputPayload =
    settings.uploadProvider === 'picgo'
      ? {
          file_name: input.fileName,
          server_url: settings.picgoServerUrl,
        }
      : {
          file_name: input.fileName,
          command: settings.picgoCoreCommand,
          config_path: settings.picgoCoreConfigPath || null,
        };

  const result = await invokeImage<NativeImageUploadPayload>(command, input, inputPayload);
  return {
    markdownSrc: result.url,
  };
}

async function invokeImage<T>(
  command: string,
  input: ImageImportInput,
  metadata: Record<string, unknown>,
): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  if (input.nativeAssetId) {
    return invoke<T>(command, { input: metadata, nativeAssetId: input.nativeAssetId });
  }
  if (!input.bytes?.byteLength) throw new Error(t.imageContentEmpty());
  return invoke<T>(command, input.bytes, {
    headers: { 'x-nomo-image-meta': encodeURIComponent(JSON.stringify(metadata)) },
  });
}

function isRemoteImageSrc(src: string): boolean {
  return /^(https?:|data:|blob:)/i.test(src.trim());
}

function createFallbackMarkdownSrc(documentFileName: string, imageName: string): string {
  const baseName = documentFileName.replace(/\.(md|markdown)$/i, '') || 'document';
  const safeName = imageName
    .trim()
    .replace(/[\\/:*?"<>|]/g, '-')
    .replace(/\s+/g, '-');
  return `./assets/${safeName || `${baseName}.png`}`;
}

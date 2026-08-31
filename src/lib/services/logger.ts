/**
 * Nomo 全局日志工具。
 *
 * 前端日志统一输出到 DevTools，并在 Tauri 环境转发到 Rust，由 Rust 负责终端输出和 ./logs 文件落盘。
 */

const LOGGER_ENABLED_KEY = 'nomo-logger-enabled';
const MAX_BUFFER_SIZE = 500;

/** 日志级别 */
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

interface LogEntry {
  level: LogLevel;
  tag: string;
  message: string;
  timestamp: string;
  data?: unknown;
}

let logBuffer: LogEntry[] = [];
let loggerEnabled = readInitialEnabled();

function now(): string {
  const d = new Date();
  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}.${d.getMilliseconds().toString().padStart(3, '0')}`;
}

function readInitialEnabled(): boolean {
  if (typeof localStorage === 'undefined') {
    return false;
  }
  return localStorage.getItem(LOGGER_ENABLED_KEY) === 'true';
}

function persistEnabled(enabled: boolean): void {
  loggerEnabled = enabled;
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(LOGGER_ENABLED_KEY, String(enabled));
  }
  syncNativeLoggerEnabled(enabled);
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function syncNativeLoggerEnabled(enabled: boolean): void {
  if (!isTauriRuntime()) {
    return;
  }
  import('@tauri-apps/api/core')
    .then(({ invoke }) => {
      void invoke('set_logger_enabled', { enabled });
    })
    .catch(() => undefined);
}

function pushLog(entry: LogEntry): void {
  if (!loggerEnabled) {
    return;
  }

  if (/Android/i.test(globalThis.navigator?.userAgent ?? '')) {
    // Redact before buffering, DevTools and IPC, not only when the native log is written.
    entry = {
      ...entry,
      message: redactAndroidMessage(entry.message),
      data: redactAndroidData(entry.data),
    };
  }

  logBuffer.push(entry);
  if (logBuffer.length > MAX_BUFFER_SIZE) {
    logBuffer = logBuffer.slice(logBuffer.length - MAX_BUFFER_SIZE);
  }

  writeDevTools(entry);
  writeNative(entry);
}

function redactAndroidMessage(message: string): string {
  const match =
    /(?:https?:\/\/|content:\/\/|file:\/\/|data:|mailto:|\/(?:data|storage|sdcard|mnt)\/)/i.exec(
      message,
    );
  return match ? message.slice(0, match.index) + '[redacted]' : message;
}

function redactAndroidData(value: unknown, depth = 0): unknown {
  // Free-form strings can be a filename, provider URI, error stack or shared body.
  if (typeof value === 'string' || depth > 4) return '[redacted]';
  if (Array.isArray(value)) return value.map((item) => redactAndroidData(item, depth + 1));
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        redactAndroidMessage(key),
        redactAndroidData(item, depth + 1),
      ]),
    );
  }
  return value;
}

function writeDevTools(entry: LogEntry): void {
  const prefix = `[${entry.timestamp}][${entry.level.toUpperCase()}][${entry.tag}]`;
  switch (entry.level) {
    case 'debug':
      // eslint-disable-next-line no-console
      console.log(prefix, entry.message, entry.data ?? '');
      break;
    case 'info':
      // eslint-disable-next-line no-console
      console.info(prefix, entry.message, entry.data ?? '');
      break;
    case 'warn':
      // eslint-disable-next-line no-console
      console.warn(prefix, entry.message, entry.data ?? '');
      break;
    case 'error':
      // eslint-disable-next-line no-console
      console.error(prefix, entry.message, entry.data ?? '');
      break;
  }
}

function writeNative(entry: LogEntry): void {
  if (!isTauriRuntime()) {
    return;
  }

  const data = serializeData(entry.data);
  import('@tauri-apps/api/core')
    .then(({ invoke }) => {
      void invoke('log_message', {
        level: entry.level.toUpperCase(),
        tag: entry.tag,
        message: data ? `${entry.message} ${data}` : entry.message,
      });
    })
    .catch(() => undefined);
}

function serializeData(data: unknown): string {
  if (data === undefined || data === null || data === '') {
    return '';
  }
  if (typeof data === 'string') {
    return data;
  }
  try {
    return JSON.stringify(data);
  } catch {
    return String(data);
  }
}

export function initializeLogger(): void {
  exposeLoggerControls();
  syncNativeLoggerEnabled(loggerEnabled);
  logInfo('Logger', `日志输出已${loggerEnabled ? '开启' : '关闭'}`);
}

function exposeLoggerControls(): void {
  if (typeof window === 'undefined') {
    return;
  }
  Object.assign(window as unknown as { NomoLogger?: unknown }, {
    NomoLogger: {
      enable: enableLogger,
      disable: disableLogger,
      toggle: toggleLogger,
      isEnabled: isLoggerEnabled,
      buffer: getLogBuffer,
      clear: clearLogBuffer,
    },
  });
}

function isLoggerEnabled(): boolean {
  return loggerEnabled;
}

export function enableLogger(): void {
  persistEnabled(true);
  logInfo('Logger', '日志输出已开启');
}

export function disableLogger(): void {
  // logInfo('Logger', '日志输出已关闭');
  persistEnabled(false);
}

function toggleLogger(): boolean {
  if (loggerEnabled) {
    disableLogger();
  } else {
    enableLogger();
  }
  return loggerEnabled;
}

export function logDebug(tag: string, message: string, data?: unknown): void {
  pushLog({ level: 'debug', tag, message, timestamp: now(), data });
}

export function logInfo(tag: string, message: string, data?: unknown): void {
  pushLog({ level: 'info', tag, message, timestamp: now(), data });
}

export function logWarn(tag: string, message: string, data?: unknown): void {
  pushLog({ level: 'warn', tag, message, timestamp: now(), data });
}

export function logError(tag: string, message: string, data?: unknown): void {
  pushLog({ level: 'error', tag, message, timestamp: now(), data });
}

export function logToTerminal(
  level: LogLevel,
  tag: string,
  message: string,
  data?: unknown,
): void {
  pushLog({ level, tag, message, timestamp: now(), data });
}

/**
 * 创建一个计时器，用于测量异步操作耗时。
 */
export function createPerfTimer(tag: string, operation: string) {
  const start = performance.now();
  const startTime = now();

  return {
    end(extraData?: Record<string, unknown>): void {
      const elapsed = Math.round(performance.now() - start);
      pushLog({
        level: 'debug',
        tag,
        message: `${operation} 完成，耗时 ${elapsed}ms`,
        timestamp: now(),
        data: {
          startTime,
          elapsedMs: elapsed,
          ...extraData,
        },
      });
    },
  };
}

/** 测量并记录同步操作耗时。 */
function perf<T>(tag: string, operation: string, fn: () => T): T {
  const timer = createPerfTimer(tag, operation);
  try {
    return fn();
  } finally {
    timer.end();
  }
}

/** 测量并记录异步操作耗时。 */
export async function perfAsync<T>(tag: string, operation: string, fn: () => Promise<T>): Promise<T> {
  const timer = createPerfTimer(tag, operation);
  try {
    return await fn();
  } finally {
    timer.end();
  }
}

/** 获取当前日志缓冲区。 */
function getLogBuffer(): readonly LogEntry[] {
  return logBuffer;
}

/** 清空日志缓冲区。 */
function clearLogBuffer(): void {
  logBuffer = [];
}

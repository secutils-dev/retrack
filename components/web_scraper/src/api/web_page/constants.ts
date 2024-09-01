/**
 * Default timeout for the extractor script, in ms.
 */
export const DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS = 30000;

// Every extractor script is represented as an ES module that is prefixed with this string.
export const EXTRACTOR_MODULE_PREFIX = 'data:text/javascript,void("retrack");';

/**
 * Represents the type of message that can be sent from the worker to the main thread.
 */
export enum WorkerMessageType {
  LOG = 'log',
  RESULT = 'error',
}

/**
 * Represents a log message that can be sent from the worker to the main thread.
 */
export interface WorkerLogMessage {
  type: WorkerMessageType.LOG;
  message: string;
  level?: string;
  screenshots?: Map<string, Uint8Array>;
  args?: ReadonlyArray<object>;
}

/**
 * Represents a result message that can be sent from the worker to the main thread.
 */
export interface WorkerResultMessage {
  type: WorkerMessageType.RESULT;
  content: { type: WorkerStringResultType; value: string };
}

export type WorkerStringResultType = 'html' | 'text' | 'json';

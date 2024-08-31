/**
 * Default timeout for the user scenario, in ms.
 */
export const DEFAULT_USER_SCRIPT_TIMEOUT_MS = 30000;

// Every user scenario is represented as an ES module that is prefixed with this string.
export const USER_MODULE_PREFIX = 'data:text/javascript,void("retrack");';

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

import type { BrowserEndpoint } from '../../utilities/browser.js';

/**
 * Default timeout for the extractor script, in ms.
 */
export const DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS = 60000;

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
  args?: ReadonlyArray<object>;
}

/**
 * Represents a result message that can be sent from the worker to the main thread.
 */
export interface WorkerResultMessage {
  type: WorkerMessageType.RESULT;
  content: unknown;
}

/**
 * Represents the data that is passed to the worker thread.
 */
export interface WorkerData {
  // The browser endpoint that the worker should connect Playwright to.
  endpoint: BrowserEndpoint;
  // The extractor script that the worker should execute.
  extractor: string;
  // The parameters that should be passed to the extractor script.
  extractorParams?: unknown;
  // Tags associated with the tracker.
  tags: string[];
  // The previous content that the extractor script extracted previously.
  previousContent?: unknown;
  // The user agent that should be used for all requests.
  userAgent?: string;
  // Whether to ignore HTTPS errors when sending network requests.
  ignoreHTTPSErrors?: boolean;
  // Path to a folder where to save screenshots.
  screenshotsPath?: string;
}

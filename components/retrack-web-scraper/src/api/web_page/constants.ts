import type { ExtractorSandboxConfig, RemoteBrowserConfig } from '../../config.js';

/**
 * Default timeout for the extractor script, in ms.
 */
export const DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS = 120000;

/**
 * Default maximum cumulative size (in bytes) of debug screenshots that can be captured per run.
 */
export const DEFAULT_MAX_DEBUG_SCREENSHOTS_TOTAL_SIZE = 5 * 1024 * 1024;

/**
 * Maximum number of debug screenshots that can be captured per run.
 */
export const MAX_DEBUG_SCREENSHOTS_COUNT = 10;

/**
 * Represents the type of message that can be sent from the worker to the main thread.
 */
export enum WorkerMessageType {
  LOG = 'log',
  RESULT = 'error',
  SCREENSHOT = 'screenshot',
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
 * Represents a screenshot message that can be sent from the worker to the main thread.
 */
export interface WorkerScreenshotMessage {
  type: WorkerMessageType.SCREENSHOT;
  label: string;
  data: string;
  mimeType: string;
}

/**
 * Debug options controlling screenshot capture and other debug behavior in the worker.
 */
export interface WorkerDebugOptions {
  enabled: boolean;
  maxScreenshotsTotalSize: number;
  autoScreenshots: boolean;
}

/**
 * Represents the data passed to the worker thread.
 */
export interface WorkerData {
  // The browser config that the worker should connect Playwright to.
  browserConfig: RemoteBrowserConfig;
  // The configuration for the extractor sandbox.
  extractorSandboxConfig: ExtractorSandboxConfig;
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
  // Whether to accept invalid server certificates when sending network requests.
  acceptInvalidCertificates?: boolean;
  // Path to a folder where to save screenshots (used in non-debug mode on failure).
  screenshotsPath?: string;
  // Debug options controlling screenshot capture behavior.
  debug?: WorkerDebugOptions;
}

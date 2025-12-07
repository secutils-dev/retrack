import type { ExtractorSandboxConfig, RemoteBrowserConfig } from '../../config.js';

/**
 * Default timeout for the extractor script, in ms.
 */
export const DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS = 120000;

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
 * Represents proxy configuration.
 */
export interface ProxyConfig {
  url: string;
  credentials?: {
    scheme: string;
    value: string;
  };
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
  // Path to a folder where to save screenshots.
  screenshotsPath?: string;
  // Optional proxy configuration.
  proxy?: ProxyConfig;
}

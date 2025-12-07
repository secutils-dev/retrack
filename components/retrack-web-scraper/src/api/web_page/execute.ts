import { resolve } from 'node:path';
import * as process from 'node:process';
import { Worker } from 'node:worker_threads';
import type { BrowserConfig, RemoteBrowserConfig } from '../../config.js';
import type { ApiRouteParams } from '../api_route_params.js';
import { Diagnostics } from '../diagnostics.js';
import type { WorkerData, WorkerLogMessage, WorkerResultMessage } from './constants.js';
import { DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS } from './constants.js';
import { WorkerMessageType } from './constants.js';

/**
 * Defines type of the input parameters.
 */
interface RequestBodyType {
  /**
   * Playwright script to extract content from a web page to track. It should be represented as an ES module that
   * exports a function named "execute" that accepts a previously saved web page "content", if available and returns a
   * new one. The function is supposed to return any JSON-serializable value or `ExecutionResult` instance that will be
   * used as a new web page "content".
   */
  extractor: string;

  /**
   * Specifies the backend (browser) to be used for content extraction.
   */
  extractorBackend?: 'chromium' | 'firefox';

  /**
   * Optional parameters that are passed to the extractor script.
   */
  extractorParams?: unknown;

  /**
   * Tags associated with the tracker.
   */
  tags: string[];

  /**
   * Optional web page content that has been extracted previously.
   */
  previousContent?: unknown;

  /**
   * Number of milliseconds to wait until extractor script finishes processing. Default is 30000ms.
   */
  timeout?: number;

  /**
   * Optional user agent string to use for every request at the web page.
   */
  userAgent?: string;

  /**
   * Whether to accept invalid server certificates when sending network requests.
   * Defaults to false.
   */
  acceptInvalidCertificates?: boolean;
}

export function registerExecuteRoutes({ config, server, getLocalBrowserServer }: ApiRouteParams) {
  return server.post<{ Body: RequestBodyType }>(
    '/api/web_page/execute',
    {
      schema: {
        body: {
          type: 'object',
          properties: {
            extractor: { type: 'string' },
            extractorParams: {},
            extractorBackend: { type: 'string' },
            tags: { type: 'array', items: { type: 'string' } },
            previousContent: {},
            timeout: { type: 'number' },
            userAgent: { type: 'string' },
            acceptInvalidCertificates: { type: 'boolean' },
          },
          required: ['extractor', 'tags'],
        },
      },
    },
    async (request, reply) => {
      const logger = server.log.child({ provider: 'web_page_execute' });
      const requestedBackend = request.body.extractorBackend;
      logger.debug(
        `Executing extractor script with the following parameters: ${JSON.stringify({
          extractorParams: request.body.extractorParams,
          backend: requestedBackend,
          tags: request.body.tags,
          timeout: request.body.timeout,
          userAgent: request.body.userAgent,
          acceptInvalidCertificates: request.body.acceptInvalidCertificates,
        })}`,
      );

      // Check if requested backend that's supported and configured.
      let browserConfig: BrowserConfig | undefined;
      if (requestedBackend === 'firefox') {
        browserConfig = config.browser.firefox;
      } else if (requestedBackend === 'chromium') {
        browserConfig = config.browser.chromium;
      } else if (!requestedBackend) {
        browserConfig = config.browser.chromium ?? config.browser.firefox;
      }

      if (!browserConfig) {
        logger.error(`The backend "${requestedBackend}" is not supported.`);
        return reply.code(400).send({ message: `The backend "${requestedBackend}" is not supported.` });
      }

      // If a local browser is requested, launch it before spawning a worker.
      let remoteBrowserConfig: RemoteBrowserConfig;
      if ('executablePath' in browserConfig) {
        remoteBrowserConfig = {
          backend: browserConfig.backend,
          protocol: browserConfig.protocol,
          wsEndpoint: (await getLocalBrowserServer(browserConfig)).wsEndpoint(),
        };
      } else {
        remoteBrowserConfig = browserConfig;
      }

      const workerLog = logger.child({ provider: 'worker' });
      const workerData: WorkerData = {
        browserConfig: remoteBrowserConfig,
        extractorSandboxConfig: config.extractorSandbox,
        extractor: request.body.extractor,
        extractorParams: request.body.extractorParams,
        tags: request.body.tags,
        previousContent: request.body.previousContent,
        userAgent: request.body.userAgent,
        acceptInvalidCertificates: request.body.acceptInvalidCertificates,
        screenshotsPath: config.browser.screenshotsPath,
        proxy: request.body.proxy,
      };

      try {
        // The extractor script is executed in a separate thread to isolate it from the main thread. We filter the
        // environment variables to only include the ones that are necessary for the extractor script to run.
        const worker = new Worker(resolve(import.meta.dirname, 'worker.js'), {
          eval: false,
          env: Object.fromEntries(Object.entries(process.env).filter(([k]) => k === 'NODE' || k === 'NODE_OPTIONS')),
          execArgv: ['--no-experimental-global-webcrypto'],
          workerData,
        });

        return await new Promise((resolve, reject) => {
          let errorResult: Error | undefined;
          let successfulResult: unknown = undefined;

          // It's intentional that 0 is treated as a fallback to the default timeout value.
          const timeout = request.body.timeout || DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS;
          const forcedWorkerTimeout = setTimeout(() => {
            errorResult = new Error(`The execution was terminated due to timeout ${timeout}ms.`);
            void worker.terminate();
          }, timeout);

          worker.on('message', (message: WorkerLogMessage | WorkerResultMessage) => {
            if (message.type === WorkerMessageType.LOG) {
              if (message.level === 'error') {
                workerLog.error(`${message.message}: ${JSON.stringify(message.args)}`);
              } else {
                workerLog.info(`${message.message}: ${JSON.stringify(message.args)}`);
              }
            } else {
              workerLog.debug(`Successfully executed extractor script.`);
              successfulResult = JSON.stringify(message.content);
            }
          });

          worker.on('error', async (err) => {
            errorResult = err;
            void worker.terminate();
          });

          worker.on('exit', async (code) => {
            clearTimeout(forcedWorkerTimeout);

            if (errorResult) {
              reject(errorResult);
            } else if (successfulResult !== undefined) {
              resolve(successfulResult);
            } else {
              reject(new Error(`Unexpected error occurred (${code}).`));
            }
          });
        });
      } catch (err) {
        logger.error(`Failed to execute extractor script: ${Diagnostics.errorMessage(err)}`);
        return reply.code(500).send({
          message: `Failed to execute extractor script: ${Diagnostics.errorMessage(err)}`,
        });
      }
    },
  );
}

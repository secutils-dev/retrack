import { resolve } from 'node:path';
import * as process from 'node:process';
import { Worker } from 'node:worker_threads';
import type { ApiRouteParams } from '../api_route_params.js';
import { Diagnostics } from '../diagnostics.js';
import type { WorkerLogMessage, WorkerResultMessage } from './constants.js';
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
   * Optional web page content that has been extracted previously.
   */
  previousContent?: { type: string; value: string };

  /**
   * Number of milliseconds to wait until extractor script finishes processing. Default is 30000ms.
   */
  timeout?: number;
}

export function registerExecuteRoutes({ server, getBrowserEndpoint }: ApiRouteParams) {
  return server.post<{ Body: RequestBodyType }>(
    '/api/web_page/execute',
    {
      schema: {
        body: {
          extractor: { type: 'string' },
          previousContent: {
            type: 'object',
            properties: { timestamp: { type: 'number' }, content: { type: 'string' } },
            nullable: true,
          },
          timeout: { type: 'number', nullable: true },
        },
        response: {
          200: { type: 'object', properties: { timestamp: { type: 'number' }, content: { type: 'string' } } },
        },
      },
    },
    async (request, reply) => {
      const log = server.log.child({ provider: 'web_page_execute' });
      const workerLog = log.child({ provider: 'worker' });
      const timeout = request.body.timeout ?? DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS;

      try {
        // The extractor script is executed in a separate thread to isolate it from the main thread. We filter the
        // environment variables to only include the ones that are necessary for the extractor script to run.
        const worker = new Worker(resolve(import.meta.dirname, 'worker.js'), {
          eval: false,
          env: Object.fromEntries(Object.entries(process.env).filter(([k]) => k === 'NODE' || k === 'NODE_OPTIONS')),
          execArgv: ['--no-experimental-global-webcrypto'],
          workerData: {
            endpoint: await getBrowserEndpoint(),
            extractor: request.body.extractor,
            previousContent: request.body.previousContent,
          },
        });

        return await new Promise((resolve, reject) => {
          let errorResult: Error | undefined;
          let successfulResult: WorkerResultMessage['content'] | undefined;

          const forcedWorkerTimeout = setTimeout(() => {
            errorResult = new Error(`The execution was terminated due to timeout ${timeout}ms.`);
            void worker.terminate();
          }, request.body.timeout ?? DEFAULT_EXTRACTOR_SCRIPT_TIMEOUT_MS);

          worker.on('message', (message: WorkerLogMessage | WorkerResultMessage) => {
            if (message.type === WorkerMessageType.LOG) {
              if (message.level === 'error') {
                workerLog.error(message.message, message.args);
                for (const [url, screenshot] of message.screenshots ?? []) {
                  workerLog.error({ screenshot }, `Screenshot for ${url}.`);
                }
              } else {
                workerLog.info(message.message, message.args);
              }
            } else {
              workerLog.debug(`Successfully executed extractor script.`);
              successfulResult = message.content;
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
            } else if (successfulResult) {
              resolve({ timestamp: Date.now(), content: JSON.stringify(successfulResult) });
            } else {
              reject(new Error(`Unexpected error occurred (${code}).`));
            }
          });
        });
      } catch (err) {
        log.error(`Failed to execute extractor script: ${Diagnostics.errorMessage(err)}`);
        return reply.code(500).send({
          message: `Failed to execute extractor script: ${Diagnostics.errorMessage(err)}`,
        });
      }
    },
  );
}

import { parentPort, workerData } from 'node:worker_threads';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import type { Browser, Page } from 'playwright-core';

import { Diagnostics } from '../diagnostics.js';
import type { WorkerData, WorkerStringResultType } from './constants.js';
import { EXTRACTOR_MODULE_PREFIX, WorkerMessageType } from './constants.js';
import { ExecutionResult } from './execution_result.js';

// We need parent port to communicate the errors and result of extractor script to the main thread.
if (!parentPort) {
  throw new Error('This worker parent port is not available.');
}

// Load the extractor script as an ES module.
const { endpoint, extractor, previousContent, userAgent, ignoreHTTPSErrors } = workerData as WorkerData;

// SECURITY: Basic prototype pollution protection against the most common vectors until we can use Playwright with
// `--frozen-intrinsics`. It DOES NOT protect against all prototype pollution vectors.
for (const Class of [
  Object,
  Array,
  Number,
  String,
  Boolean,
  Map,
  Set,
  MessagePort,
  Buffer,
  Blob,
  Uint8Array,
  ArrayBuffer,
  Response,
  Request,
  WebSocket,
  URL,
]) {
  Object.seal(Class.prototype);
}

// SECURITY: We load custom hooks to prevent extractor scripts from importing sensitive native and playwright modules.
// See https://github.com/nodejs/node/issues/47747 for more details.
register(resolve(import.meta.dirname, './extractor_module_hooks.js'), pathToFileURL('./'));
const extractorModule = (await import(`${EXTRACTOR_MODULE_PREFIX}${extractor}`)) as {
  execute: (
    page: Page,
    executionResult: typeof ExecutionResult,
    previousContent?: { type: WorkerStringResultType; value: string },
  ) => Promise<ExecutionResult | unknown>;
};
if (typeof extractorModule?.execute !== 'function') {
  throw new Error('The extractor script must export a function named "execute".');
}

// Logger to post messages to the main thread.
const log = {
  info: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, message, args }),
  error: (message: string, args?: ReadonlyArray<object>, screenshots?: Map<string, Uint8Array>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, level: 'error', message, args, screenshots }),
};

const { connectToBrowser } = await import('../../utilities/browser.js');

let browser: Browser | undefined;
try {
  log.info(`Connecting to a browser at ${endpoint.url} (protocol: ${endpoint.protocol})…`);
  browser = await connectToBrowser(endpoint, {
    isEnabled: () => true,
    // Forward browser logs to the main log sink.
    log: (context, level, message, args) =>
      level === 'error' ? log.error(`${context}: ${message}`, args) : log.info(`${context}: ${message}`, args),
  });
  log.info(`Successfully connected to a browser at ${endpoint.url} (protocol: ${endpoint.protocol}).`);
} catch (err) {
  log.error(
    `Failed to connect to a browser at ${endpoint.url} (protocol: ${endpoint.protocol}): ${Diagnostics.errorMessage(err)}.`,
  );
  throw new Error('Failed to connect to a browser.');
}

const context = await browser.newContext({ ignoreHTTPSErrors, userAgent });

// SECURITY: Ideally, the extractor script shouldn't have access to the browser instance, as it could close the browser
// and access other contexts. Unfortunately, the browser instance and context are accessible through various Playwright
// APIs (e.g., Locator -> Page -> Context -> Browser), making it infeasible to completely prevent this. Instead,
// extractor scripts should be closely monitored for potentially malicious behavior (see `logger`), and responsible
// actors should be penalized accordingly. Nevertheless, it's still valuable to remove methods that aren't meant to be
// used from the API to make this intention clearer even though this obstacle can be bypassed by the sufficiently
// motivated adversary. If it becomes a problem, it'd be easier to fork Playwright and remove the methods from the
// source code directly.
const browserPrototype = Object.getPrototypeOf(browser);
delete browserPrototype.newBrowserCDPSession;
delete browserPrototype.newContext;

// We need to preserve the original `browser.close` method to close the browser after the extractor execution.
const originalBrowserClose = browser.close.bind(browser);
delete browserPrototype.close;

const contextPrototype = Object.getPrototypeOf(context);
delete contextPrototype.newCDPSession;
delete contextPrototype.constructor;

const page = await context.newPage();
try {
  const executionResult = await extractorModule.execute(
    page,
    ExecutionResult,
    previousContent?.type === 'json' ? JSON.parse(previousContent.value) : previousContent?.value,
  );
  parentPort?.postMessage({
    type: WorkerMessageType.RESULT,
    content: (executionResult instanceof ExecutionResult
      ? executionResult
      : ExecutionResult.json(executionResult)
    ).toContent(),
  });
} catch (err) {
  // Capture screenshots.
  try {
    const pages = browser?.contexts().flatMap((context) => context.pages()) ?? [];
    log.error(
      'Diagnostics screenshots.',
      pages.map((page) => ({ url: page.url() })),
      new Map(
        await Promise.all(
          pages.map(async (page) => [page.url(), await page.screenshot({ fullPage: true })] as [string, Uint8Array]),
        ),
      ),
    );
  } catch (err) {
    log.error(
      `Failed to capture browser screenshots (protocol: ${endpoint.protocol}): ${Diagnostics.errorMessage(err)}.`,
    );
  }
  throw err;
} finally {
  await page.close();
  await context.close();
  await originalBrowserClose();
}

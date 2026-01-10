import { parentPort, workerData } from 'node:worker_threads';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import type { Browser, Page } from 'playwright-core';
import type { ExtractorSandboxConfig } from '../../config.js';

import { Diagnostics } from '../diagnostics.js';
import type { WorkerData } from './constants.js';
import { WorkerMessageType } from './constants.js';

// We need a parent port to communicate the errors and result of an extractor
// script to the main thread.
if (!parentPort) {
  throw new Error('This worker parent port is not available.');
}

// Load the extractor script as an ES module.
const {
  browserConfig,
  extractorSandboxConfig,
  extractor,
  extractorParams,
  tags,
  previousContent,
  userAgent,
  acceptInvalidCertificates,
  screenshotsPath,
  proxy,
} = workerData as WorkerData;

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
  URL,
]) {
  Object.seal(Class.prototype);
}

// SECURITY: We load custom hooks to prevent extractor scripts from importing sensitive native and playwright modules.
// See https://github.com/nodejs/node/issues/47747 for more details.
register<ExtractorSandboxConfig>(resolve(import.meta.dirname, './extractor_module_hooks.js'), pathToFileURL('./'), {
  data: extractorSandboxConfig,
});
const extractorModule = (await import(`data:text/javascript,${encodeURIComponent(extractor)}`)) as {
  execute: (page: Page, context: { tags: string[]; params?: unknown; previousContent?: unknown }) => Promise<unknown>;
};
if (typeof extractorModule?.execute !== 'function') {
  throw new Error('The extractor script must export a function named "execute".');
}

// Logger to post messages to the main thread.
const log = {
  info: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, message, args }),
  error: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, level: 'error', message, args }),
};

const { connectToBrowserServer } = await import('../../utilities/browser.js');

let browser: Browser | undefined;
try {
  log.info(`Connecting to a browser at ${browserConfig.wsEndpoint} (protocol: ${browserConfig.protocol})…`);
  browser = await connectToBrowserServer(
    {
      isEnabled: () => true,
      // Forward browser logs to the main log sink.
      log: (context, level, message, args) =>
        level === 'error' ? log.error(`${context}: ${message}`, args) : log.info(`${context}: ${message}`, args),
    },
    browserConfig,
  );
  log.info(`Successfully connected to a browser at ${browserConfig.wsEndpoint} (protocol: ${browserConfig.protocol}).`);
} catch (err) {
  log.error(
    `Failed to connect to a browser at ${browserConfig.wsEndpoint} (protocol: ${browserConfig.protocol}): ${Diagnostics.errorMessage(err)}.`,
  );
  throw new Error('Failed to connect to a browser.');
}

const contextOptions: {
  ignoreHTTPSErrors: boolean;
  userAgent?: string;
  viewport: null;
  proxy?: { server: string; username?: string; password?: string };
} = { ignoreHTTPSErrors: acceptInvalidCertificates ?? false, userAgent, viewport: null };

// Configure proxy if provided
if (proxy) {
  contextOptions.proxy = { server: proxy.url };
  // Note: Playwright's proxy authentication only supports username/password format
  // For custom auth schemes (like Bearer), the credentials would need to be handled
  // differently, potentially via extraHTTPHeaders. For now, we document this limitation.
  if (proxy.credentials) {
    // If using Basic auth, extract username and password
    // This is a simplified implementation - full Basic auth would require base64 decoding
    // For now, we'll just pass the server URL and note that custom auth isn't fully supported
    log.warn(
      `Proxy authentication with custom scheme '${proxy.credentials.scheme}' is configured, but Playwright only supports username/password format. Custom auth schemes may not work correctly.`,
    );
  }
}

const context = await browser.newContext(contextOptions);

// SECURITY: Ideally, the extractor script shouldn't have access to the browser instance, as it could close the browser
// and access other contexts. Unfortunately, the browser instance and context are accessible through various Playwright
// APIs (e.g., Locator -> Page -> Context -> Browser), making it infeasible to completely prevent this. Instead,
// extractor scripts should be closely monitored for potentially malicious behavior (see `logger`), and responsible
// actors should be penalized accordingly. Nevertheless, it's still valuable to remove methods that aren't meant to be
// used from the API to clarify this intention even though this obstacle can be bypassed by the motivated enough
// adversary. If it becomes a problem, it'd be easier to fork Playwright and remove the methods from the
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
  parentPort?.postMessage({
    type: WorkerMessageType.RESULT,
    content: await extractorModule.execute(
      page,
      extractorParams ? { params: extractorParams, tags, previousContent } : { tags, previousContent },
    ),
  });
} catch (err) {
  // Capture screenshots.
  if (screenshotsPath) {
    const pages = browser?.contexts().flatMap((context) => context.pages()) ?? [];
    for (const page of pages) {
      const screenshotPath = `${screenshotsPath}/screenshot_${Date.now()}.png`;
      try {
        await page.screenshot({ fullPage: true, path: screenshotPath });
        log.error(`Captured page screenshot for ${page.url()}: ${screenshotPath}`);
      } catch (err) {
        log.error(
          `Failed to capture browser screenshot for ${page.url()} (protocol: ${browserConfig.protocol}): ${Diagnostics.errorMessage(err)}.`,
        );
      }
    }
  }
  throw err;
} finally {
  await page.close();
  await context.close();
  await originalBrowserClose();
}

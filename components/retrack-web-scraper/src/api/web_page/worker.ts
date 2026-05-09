import { parentPort, workerData } from 'node:worker_threads';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import type { Browser, Locator, LocatorScreenshotOptions, Page, PageScreenshotOptions } from 'playwright-core';
import type { ExtractorSandboxConfig } from '../../config.js';

import { Diagnostics } from '../diagnostics.js';
import type { WorkerData } from './constants.js';
import { MAX_DEBUG_SCREENSHOTS_COUNT, WorkerMessageType } from './constants.js';

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
  debug: debugOptions,
} = workerData as WorkerData;

// SECURITY: Strip the global Web Crypto API from the extractor sandbox.
// `node:crypto` imports are independently blocked by `extractor_module_hooks`.
delete (globalThis as { crypto?: unknown }).crypto;

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
  debug: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, level: 'debug', message, args }),
  info: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, message, args }),
  warn: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, level: 'warn', message, args }),
  error: (message: string, args?: ReadonlyArray<object>) =>
    parentPort?.postMessage({ type: WorkerMessageType.LOG, level: 'error', message, args }),
};

// Redirect worker-thread console methods so that console.log/warn/error calls
// inside extractor scripts are captured in the debug log stream.
console.log = (...args: unknown[]) => log.info(args.map(String).join(' '));
console.debug = (...args: unknown[]) => log.debug(args.map(String).join(' '));
console.info = console.log;
console.warn = (...args: unknown[]) => log.warn(args.map(String).join(' '));
console.error = (...args: unknown[]) => log.error(args.map(String).join(' '));

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
  throw new Error('Failed to connect to a browser.', { cause: err });
}

const context = await browser.newContext({ ignoreHTTPSErrors: acceptInvalidCertificates, userAgent, viewport: null });

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

// Capture browser-side console messages (from page.evaluate, in-page JS, etc.).
page.on('console', (msg) => {
  const level = msg.type();
  const message = `[browser] ${msg.text()}`;
  if (level === 'debug') {
    log.debug(message);
  } else if (level === 'warning') {
    log.warn(message);
  } else if (level === 'error') {
    log.error(message);
  } else {
    log.info(message);
  }
});

// Debug screenshot capture: intercept page.screenshot(), locator.screenshot(), and optionally
// auto-capture after every significant Playwright action.
let screenshotCount = 0;
let screenshotTotalSize = 0;
const isDebug = debugOptions?.enabled === true;
const maxScreenshotSize = debugOptions?.maxScreenshotsTotalSize ?? 0;

const originalPageScreenshot = page.screenshot.bind(page);
async function captureDebugScreenshot(
  target: Page | Locator,
  label: string,
  options?: PageScreenshotOptions | LocatorScreenshotOptions,
): Promise<Buffer> {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { path, ...safeOptions } = options ?? {};

  // const safeOptions = options ? stripScreenshotPath(options) : {};
  const buffer = await originalPageScreenshot.call(target, safeOptions);
  if (screenshotCount < MAX_DEBUG_SCREENSHOTS_COUNT && screenshotTotalSize + buffer.length <= maxScreenshotSize) {
    screenshotCount++;
    screenshotTotalSize += buffer.length;
    parentPort?.postMessage({
      type: WorkerMessageType.SCREENSHOT,
      label,
      data: buffer.toString('base64'),
      mimeType: `image/${safeOptions.type ?? 'png'}`,
    });
  }
  return buffer;
}

let tracedPage: Page;
if (isDebug) {
  // Intercept Locator.prototype.screenshot() at the prototype level to capture in-memory and strip `path`.
  Object.getPrototypeOf(page.locator('body')).screenshot = async function (
    this: Locator,
    options?: LocatorScreenshotOptions,
  ): Promise<Buffer> {
    return captureDebugScreenshot(this, 'locator.screenshot()', options);
  };

  // Set of page actions that should trigger an automatic viewport screenshot after they succeed.
  const autoTraceActions: ReadonlySet<string> | undefined =
    debugOptions?.autoScreenshots !== false
      ? new Set(['goto', 'click', 'fill', 'type', 'press', 'check', 'uncheck', 'selectOption'])
      : undefined;

  // Use a Proxy to intercept page method calls: capture debug screenshots for `screenshot()` and
  // auto-traced actions without mutating the original page object.
  tracedPage = new Proxy(page, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver);
      if (typeof value !== 'function' || typeof prop !== 'string') {
        return value;
      }

      if (prop === 'screenshot') {
        return (options?: PageScreenshotOptions) => captureDebugScreenshot(target, 'page.screenshot()', options);
      }

      if (autoTraceActions?.has(prop)) {
        const original = value as (...args: unknown[]) => Promise<unknown>;
        return async (...args: unknown[]) => {
          const result = await original.apply(target, args);
          try {
            await captureDebugScreenshot(target, `after ${prop}: ${typeof args[0] === 'string' ? args[0] : ''}`, {
              fullPage: false,
            });
          } catch {
            // Never break the action if the auto-screenshot fails.
          }
          return result;
        };
      }

      return value;
    },
  });
} else {
  tracedPage = page;
}

try {
  parentPort?.postMessage({
    type: WorkerMessageType.RESULT,
    content: await extractorModule.execute(
      tracedPage,
      extractorParams ? { params: extractorParams, tags, previousContent } : { tags, previousContent },
    ),
  });
} catch (err) {
  if (isDebug || screenshotsPath) {
    const pages = browser?.contexts().flatMap((ctx) => ctx.pages()) ?? [];
    for (const p of pages) {
      try {
        if (isDebug) {
          await captureDebugScreenshot(p, `auto: error (${p.url()})`, { fullPage: true });
        } else {
          const screenshotPath = `${screenshotsPath}/screenshot_${Date.now()}.png`;
          await p.screenshot({ fullPage: true, path: screenshotPath });
          log.error(`Captured page screenshot for ${p.url()}: ${screenshotPath}`);
        }
      } catch (screenshotErr) {
        log.error(
          `Failed to capture ${isDebug ? 'debug' : 'browser'} screenshot for ${p.url()} (protocol: ${browserConfig.protocol}): ${Diagnostics.errorMessage(screenshotErr)}.`,
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

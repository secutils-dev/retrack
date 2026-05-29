import { setTimeout as delay } from 'node:timers/promises';
import type { FastifyBaseLogger } from 'fastify';
import type { BrowserServer, Logger } from 'playwright-core';
import { Diagnostics } from '../api/diagnostics.js';
import type { LocalBrowserConfig, RemoteBrowserConfig } from '../config.js';

/**
 * Represents supported browser backend.
 */
export type BrowserBackend = 'chromium' | 'firefox';

/**
 * Represents supported browser communication protocol.
 */
export type BrowserProtocol = 'cdp' | 'playwright';

// Timeout for connecting to a browser.
const BROWSER_CONNECT_TIMEOUT_MS = 30000;

// Number of attempts to launch a local browser server before giving up. The browser process can
// occasionally crash during startup (e.g. a transient `SIGSEGV` under momentary resource pressure
// when many trackers fire at the same time), and such failures usually clear on an immediate retry.
const BROWSER_LAUNCH_MAX_ATTEMPTS = 3;

// Delay between consecutive browser launch attempts.
const BROWSER_LAUNCH_RETRY_DELAY_MS = 500;

/**
 * Connects to a remote browser over Playwright or CDP protocol:
 *   * `browserType.connectOverCDP` is used to connect to a remote Chromium CDP session. In this case, Playwright
 *   doesn't even need to be installed where Chromium is running (e.g., in a Docker container with the following
 *   switches `--remote-debugging-port=9222 --remote-allow-origins="*"`). Both Playwright server and client will be
 *   running locally and talking to a remote browser over CDP.
 *   * `browserType.connect` is used to connect to a remote Playwright Server launched via `browserType.launchServer`.
 *   In this case communication between Playwright client and server will be done over the special Playwright protocol,
 *   and then the Playwright Server would be talking to the browser over normal CDP.
 *   See https://github.com/microsoft/playwright/issues/15265#issuecomment-1172860134 for more details.
 * @param logger
 * @param config
 */
export async function connectToBrowserServer(logger: Logger, config: RemoteBrowserConfig) {
  const { chromium, firefox } = await import('playwright-core');
  const backend = config.backend === 'chromium' ? chromium : firefox;
  return config.protocol === 'playwright'
    ? backend.connect(config.wsEndpoint, { timeout: BROWSER_CONNECT_TIMEOUT_MS, logger })
    : backend.connectOverCDP(config.wsEndpoint, { timeout: BROWSER_CONNECT_TIMEOUT_MS, logger });
}

export async function launchBrowserServer(logger: FastifyBaseLogger, config: LocalBrowserConfig) {
  logger.info(`Browser server (config: ${JSON.stringify(config)} will be run locally.`);

  const { chromium, firefox } = await import('playwright-core');
  const backend = config.backend === 'chromium' ? chromium : firefox;

  let lastError: unknown;
  for (let attempt = 1; attempt <= BROWSER_LAUNCH_MAX_ATTEMPTS; attempt++) {
    try {
      const localServer = await backend.launchServer({
        executablePath: config.executablePath,
        headless: config.headless,
        args: ['--disable-web-security', '--disable-blink-features=AutomationControlled'],
        ignoreDefaultArgs: ['--enable-automation'],
        ...(config.backend === 'chromium' ? { channel: 'chromium', chromiumSandbox: config.chromiumSandbox } : {}),
      });
      logger.info(`Browser server is running locally at ${localServer.wsEndpoint()}.`);
      return localServer;
    } catch (err) {
      lastError = err;
      logger.error(
        `Failed to run browser server locally (attempt ${attempt}/${BROWSER_LAUNCH_MAX_ATTEMPTS}): ${Diagnostics.errorMessage(
          err,
        )}`,
      );
      if (attempt < BROWSER_LAUNCH_MAX_ATTEMPTS) {
        await delay(BROWSER_LAUNCH_RETRY_DELAY_MS);
      }
    }
  }

  throw lastError;
}

export async function stopBrowserServer(logger: FastifyBaseLogger, server: BrowserServer) {
  logger.info('Stopping local browser server...');

  try {
    await server.close();
    logger.info('Successfully stopped local browser server.');
  } catch (err) {
    logger.error(`Failed to stop local browser server: ${Diagnostics.errorMessage(err)}`);
  }
}

/**
 * Dependencies of {@link createBrowserServerManager}. Primarily exists to allow tests to inject
 * fakes for the launch/stop primitives without spinning up a real browser.
 */
export interface BrowserServerManagerDeps {
  launch?: (logger: FastifyBaseLogger, config: LocalBrowserConfig) => Promise<BrowserServer>;
  stop?: (logger: FastifyBaseLogger, server: BrowserServer) => Promise<void>;
}

/**
 * Manages a single, lazily-launched local browser server that is shared across all in-flight
 * extraction requests and torn down after an idle TTL.
 */
export interface BrowserServerManager {
  /**
   * Returns the shared browser server, launching it on first use. Concurrent callers share the
   * same in-flight launch. If the launch fails, the cached (rejected) launch is discarded so the
   * next caller retries from scratch instead of replaying the failure.
   */
  get: (config: LocalBrowserConfig) => Promise<BrowserServer>;
  /** Whether a browser server is currently launched (or being launched). */
  isRunning: () => boolean;
  /** Stops the shared browser server (if any) and cancels the pending idle shutdown. */
  close: () => Promise<void>;
}

export function createBrowserServerManager(
  logger: FastifyBaseLogger,
  deps: BrowserServerManagerDeps = {},
): BrowserServerManager {
  const launch = deps.launch ?? launchBrowserServer;
  const stop = deps.stop ?? stopBrowserServer;

  let serverPromise: Promise<BrowserServer> | undefined;
  let shutdownInProgress: Promise<void> | undefined;
  let shutdownTimer: NodeJS.Timeout | undefined;

  const clearShutdownTimer = () => {
    if (shutdownTimer) {
      clearTimeout(shutdownTimer);
      shutdownTimer = undefined;
    }
  };

  const scheduleShutdown = (ttlSec: number) => {
    clearShutdownTimer();
    shutdownTimer = setTimeout(() => {
      shutdownTimer = undefined;

      const instance = serverPromise;
      if (!instance) {
        return;
      }

      serverPromise = undefined;
      shutdownInProgress = instance
        .then((server) => stop(logger, server))
        .catch(() => {})
        .finally(() => {
          shutdownInProgress = undefined;
        });
    }, ttlSec * 1000);

    // The idle shutdown timer must not, by itself, keep the process alive.
    shutdownTimer.unref?.();
  };

  return {
    isRunning: () => !!serverPromise,
    get: (config: LocalBrowserConfig) => {
      if (!serverPromise) {
        const launchPromise = (shutdownInProgress ?? Promise.resolve()).then(() => launch(logger, config));
        serverPromise = launchPromise;

        // A failed launch must not be cached: drop the rejected promise (and any pending idle
        // shutdown scheduled for it) so the next request attempts a fresh launch instead of
        // synchronously replaying the cached failure for the whole TTL window.
        launchPromise.catch(() => {
          if (serverPromise === launchPromise) {
            serverPromise = undefined;
            clearShutdownTimer();
          }
        });
      }

      scheduleShutdown(config.ttlSec);

      return serverPromise;
    },
    close: async () => {
      clearShutdownTimer();

      const instance = serverPromise;
      serverPromise = undefined;
      if (instance) {
        await instance.then((server) => stop(logger, server)).catch(() => {});
      }
    },
  };
}

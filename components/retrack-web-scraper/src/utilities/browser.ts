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
    logger.error(`Failed to run browser server locally: ${Diagnostics.errorMessage(err)}`);
    throw err;
  }
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

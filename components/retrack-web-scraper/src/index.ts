import { fastify } from 'fastify';
import type { BrowserServer } from 'playwright-core';
import { chromium } from 'playwright-core';

import { Diagnostics } from './api/diagnostics.js';
import { registerRoutes } from './api/index.js';
import { configure } from './config.js';
import type { BrowserEndpoint } from './utilities/browser.js';

const config = configure();
const server = fastify({
  bodyLimit: config.server.bodyLimit,
  logger: config.isDev
    ? {
        level: config.logLevel,
        transport: { target: 'pino-pretty', options: { translateTime: 'HH:MM:ss Z', ignore: 'pid,hostname' } },
      }
    : { level: config.logLevel },
}).addHook('onClose', () => stopBrowserServer());

server.log.debug(`Configuration: ${JSON.stringify(config, null, 2)}.`);

await server.register(import('@fastify/compress'));

const browserServer: {
  cachedEndpoint: BrowserEndpoint;
  pendingEndpoint?: Promise<BrowserEndpoint>;
  shutdownTimer?: NodeJS.Timeout;
  server?: BrowserServer;
} = {
  // The scraper can connect to a remote browser or run a local one:
  // * `browserType.connectOverCDP` is used to connect to a remote Chromium CDP session. In this case, Playwright
  // doesn't even need to be installed where Chromium is running (e.g., in a Docker container with the following
  // switches `--remote-debugging-port=9222 --remote-allow-origins="*"`). Both Playwright server and client will be
  // running locally and talking to remote browser over CDP.
  // * `browserType.connect` is used to connect to a remote Playwright Server launched via `browserType.launchServer`.
  // In this case communication between Playwright client and server will be done over the special Playwright protocol,
  // and then the Playwright Server would be talking to the browser over normal CDP.
  // See https://github.com/microsoft/playwright/issues/15265#issuecomment-1172860134 for more details.
  cachedEndpoint: config.browser.cdpEndpoint
    ? { protocol: 'cdp', url: config.browser.cdpEndpoint }
    : { protocol: 'playwright', url: '' },
};

async function launchBrowserServer() {
  server.log.info(`Browser server will be run locally (config: ${JSON.stringify(config.browser)}).`);

  try {
    const localServer = await chromium.launchServer({
      executablePath: config.browser.executablePath,
      headless: config.browser.headless,
      channel: 'chromium',
      chromiumSandbox: config.browser.sandbox,
      args: ['--disable-web-security', '--disable-blink-features=AutomationControlled'],
      ignoreDefaultArgs: ['--enable-automation'],
    });
    server.log.info(
      `Browser server is running locally at ${browserServer.cachedEndpoint.url} (config: ${JSON.stringify(config.browser)}).`,
    );
    return localServer;
  } catch (err) {
    server.log.error(
      `Failed to run browser server locally (config: ${JSON.stringify(config.browser)}): ${Diagnostics.errorMessage(err)}`,
    );
    throw err;
  }
}

async function stopBrowserServer() {
  const localServer = browserServer.server;
  if (!localServer) {
    return;
  }

  server.log.info('Stopping local browser server...');

  browserServer.server = undefined;
  browserServer.cachedEndpoint.url = '';
  clearTimeout(browserServer.shutdownTimer);
  browserServer.shutdownTimer = undefined;

  try {
    await localServer.close();
    server.log.info('Successfully stopped local browser server.');
  } catch (err) {
    server.log.error(`Failed to stop local browser server: ${Diagnostics.errorMessage(err)}`);
  }
}

registerRoutes({
  server,
  config,
  getBrowserEndpoint: async ({ launchServer = true }: { launchServer?: boolean } = {}) => {
    // For local browser server, we will stop it after a certain period of inactivity to free up resources.
    if (browserServer.cachedEndpoint.protocol === 'playwright') {
      if (browserServer.shutdownTimer) {
        clearTimeout(browserServer.shutdownTimer);
      }
      browserServer.shutdownTimer = setTimeout(() => stopBrowserServer().catch(() => {}), config.browser.ttlSec * 1000);
    }

    if (browserServer.cachedEndpoint.url || !launchServer) {
      return browserServer.cachedEndpoint;
    }

    if (!browserServer.pendingEndpoint) {
      browserServer.pendingEndpoint = launchBrowserServer()
        .then((localServer) => {
          browserServer.server = localServer;
          browserServer.cachedEndpoint.url = localServer.wsEndpoint();
          return browserServer.cachedEndpoint;
        })
        .finally(() => {
          browserServer.pendingEndpoint = undefined;
        });
    }

    return browserServer.pendingEndpoint;
  },
});

server.listen({ port: config.port, host: '0.0.0.0' }, (err, address) => {
  if (err) {
    server.log.error(`Failed to run server: ${err.message}.`);
    throw err;
  }

  server.log.info(`Server is listening on ${address}.`);
});

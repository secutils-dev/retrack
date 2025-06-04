import { fastify } from 'fastify';
import type { BrowserServer } from 'playwright-core';

import { registerRoutes } from './api/index.js';
import type { LocalBrowserConfig } from './config.js';
import { configure } from './config.js';
import { stopBrowserServer } from './utilities/browser.js';
import { launchBrowserServer } from './utilities/browser.js';

const config = configure();
if (!config.browser.chromium && !config.browser.firefox) {
  throw new Error('At least one browser (Chromium or Firefox) should be configured.');
}

const browserServer: {
  server?: Promise<BrowserServer>;
  shutdownInProgress?: Promise<void>;
  shutdownTimer?: NodeJS.Timeout;
} = {};

const server = fastify({
  bodyLimit: config.server.bodyLimit,
  logger: config.isDev
    ? {
        level: config.logLevel,
        transport: { target: 'pino-pretty', options: { translateTime: 'HH:MM:ss Z', ignore: 'pid,hostname' } },
      }
    : { level: config.logLevel },
}).addHook('onClose', async () => {
  if (browserServer.server) {
    await browserServer.server.then((localServer) => stopBrowserServer(logger, localServer)).catch(() => {});
  }
});

const logger = server.log;
logger.debug(`Configuration: ${JSON.stringify(config, null, 2)}.`);

await server.register(import('@fastify/compress'));

registerRoutes({
  server,
  config,
  isLocalBrowserServerRunning: () => !!browserServer.server,
  getLocalBrowserServer: async (locaConfig: LocalBrowserConfig) => {
    if (!browserServer.server) {
      browserServer.server = (browserServer.shutdownInProgress ?? Promise.resolve()).then(() =>
        launchBrowserServer(logger, locaConfig),
      );
    }

    if (browserServer.shutdownTimer) {
      clearTimeout(browserServer.shutdownTimer);
    }
    browserServer.shutdownTimer = setTimeout(() => {
      if (!browserServer.server) {
        return;
      }

      const browserServerInstance = browserServer.server;
      browserServer.server = undefined;

      clearTimeout(browserServer.shutdownTimer);
      browserServer.shutdownTimer = undefined;

      browserServer.shutdownInProgress = browserServerInstance
        .then((server) => stopBrowserServer(logger, server))
        .catch(() => {})
        .finally(() => {
          browserServer.shutdownInProgress = undefined;
        });
    }, locaConfig.ttlSec * 1000);

    return browserServer.server;
  },
});

server.listen({ port: config.port, host: '0.0.0.0' }, (err, address) => {
  if (err) {
    logger.error(`Failed to run server: ${err.message}.`);
    throw err;
  }

  logger.info(`Server is listening on ${address}.`);
});

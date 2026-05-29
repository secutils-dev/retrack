import { fastify } from 'fastify';

import { registerRoutes } from './api/index.js';
import { configure } from './config.js';
import { createBrowserServerManager } from './utilities/browser.js';

const config = configure();
if (!config.browser.chromium && !config.browser.firefox) {
  throw new Error('At least one browser (Chromium or Firefox) should be configured.');
}

const server = fastify({
  bodyLimit: config.server.bodyLimit,
  logger: config.isDev
    ? {
        level: config.logLevel,
        transport: { target: 'pino-pretty', options: { translateTime: 'HH:MM:ss Z', ignore: 'pid,hostname' } },
      }
    : { level: config.logLevel },
});

const logger = server.log;
logger.debug(`Configuration: ${JSON.stringify(config, null, 2)}.`);

const browserServerManager = createBrowserServerManager(logger);
server.addHook('onClose', async () => {
  await browserServerManager.close();
});

await server.register(import('@fastify/compress'));

registerRoutes({
  server,
  config,
  isLocalBrowserServerRunning: () => browserServerManager.isRunning(),
  getLocalBrowserServer: (locaConfig) => browserServerManager.get(locaConfig),
});

server.listen({ port: config.port, host: '0.0.0.0' }, (err, address) => {
  if (err) {
    logger.error(`Failed to run server: ${err.message}.`);
    throw err;
  }

  logger.info(`Server is listening on ${address}.`);
});

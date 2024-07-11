import * as process from 'process';

import { fastifyCompress } from '@fastify/compress';
import type { FastifyInstance } from 'fastify';
import { fastify } from 'fastify';
import NodeCache from 'node-cache';
import type { Browser } from 'playwright';
import { chromium } from 'playwright';

import { Diagnostics } from './api/diagnostics.js';
import { registerRoutes } from './api/index.js';
import { configure } from './config.js';

const config = configure();

let browser: Browser | undefined;
let browserShutdownTimer: NodeJS.Timeout | undefined;

const cache = new NodeCache({ stdTTL: config.cacheTTLSec });
const server = fastify({
  logger:
    process.env.NODE_ENV === 'production'
      ? { level: process.env.RETRACK_WEB_SCRAPER_LOG_LEVEL ?? 'debug' }
      : {
          level: process.env.RETRACK_WEB_SCRAPER_LOG_LEVEL ?? 'debug',
          transport: { target: 'pino-pretty', options: { translateTime: 'HH:MM:ss Z', ignore: 'pid,hostname' } },
        },
})
  .register(fastifyCompress)
  .addHook('onClose', (instance) => stopBrowser(instance));

async function runBrowser(serverInstance: FastifyInstance) {
  const headless = true;
  const chromiumSandbox = !(process.env.RETRACK_WEB_SCRAPER_BROWSER_NO_SANDBOX === 'true');
  serverInstance.log.info(`Running browser (headless: ${headless}, sandbox: ${chromiumSandbox})...`);
  try {
    const browserToRun = await chromium.launch({
      executablePath: process.env.RETRACK_WEB_SCRAPER_BROWSER_EXECUTABLE_PATH || undefined,
      headless,
      chromiumSandbox,
      args: ['--disable-web-security'],
    });
    serverInstance.log.info(`Successfully run browser (headless: ${headless}, sandbox: ${chromiumSandbox}).`);
    return browserToRun;
  } catch (err) {
    serverInstance.log.error(
      `Failed to run browser (headless: ${headless}, sandbox: ${chromiumSandbox}): ${Diagnostics.errorMessage(err)}`,
    );
    throw err;
  }
}

async function stopBrowser(serverInstance: FastifyInstance) {
  if (!browser) {
    return;
  }

  try {
    serverInstance.log.info('Stopping browser...');
    await browser.close();
    browser = undefined;
    serverInstance.log.info('Successfully stopped browser.');
  } catch (err) {
    serverInstance.log.error(`Failed to stop browser: ${Diagnostics.errorMessage(err)}`);
  }
}

let browserIsLaunching: Promise<Browser> | undefined;
registerRoutes({
  server,
  cache,
  config,
  acquireBrowser: async () => {
    if (browserIsLaunching) {
      server.log.info('Requested browser while it is still launching, waiting...');
      return browserIsLaunching;
    }

    if (browserShutdownTimer) {
      clearTimeout(browserShutdownTimer);
      browserShutdownTimer = undefined;
    }

    if (browser?.isConnected()) {
      browserShutdownTimer = setTimeout(() => {
        stopBrowser(server).catch((err: Error) => {
          server.log.error(`Failed to stop browser: ${err?.message}`);
        });
      }, config.browserTTLSec * 1000);
      return browser;
    }

    return (browserIsLaunching = (browser ? stopBrowser(server).then(() => runBrowser(server)) : runBrowser(server))
      .then(
        (newBrowser) => {
          browser = newBrowser;
          browserShutdownTimer = setTimeout(() => {
            stopBrowser(server).catch((err: Error) => {
              server.log.error(`Failed to stop browser: ${err?.message}`);
            });
          }, config.browserTTLSec * 1000);
          return newBrowser;
        },
        (err) => {
          browser = undefined;
          throw err;
        },
      )
      .finally(() => {
        browserIsLaunching = undefined;
      }));
  },
});

server.listen({ port: config.port, host: '0.0.0.0' }, (err, address) => {
  if (err) {
    server.log.error(`Failed to run server: ${err.message}.`);
    throw err;
  }

  server.log.info(`Server is listening on ${address}.`);
});

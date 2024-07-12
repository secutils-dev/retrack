import { mock } from 'node:test';

import { fastify } from 'fastify';
import NodeCache from 'node-cache';
import type { Browser } from 'playwright';

import type { Config } from '../config.js';
import { configure } from '../config.js';
import type { BrowserInfo } from '../index.js';
import { createBrowserMock } from '../mocks.js';

interface MockOptions {
  browser?: Browser;
  browserInfo?: BrowserInfo;
  config?: Config;
}

export function createMock({
  browser = createBrowserMock() as unknown as Browser,
  config = configure(),
  browserInfo = { running: false, contexts: [] },
}: MockOptions = {}) {
  return {
    server: fastify({ logger: { level: 'warn' } }),
    cache: new NodeCache({ stdTTL: 0 }),
    config,
    acquireBrowser: mock.fn(() => Promise.resolve(browser)),
    browserInfo: mock.fn(() => browserInfo),
  };
}

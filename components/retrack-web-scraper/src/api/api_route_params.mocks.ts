import { mock } from 'node:test';

import { fastify } from 'fastify';

import type { Config } from '../config.js';
import { configure } from '../config.js';
import type { BrowserEndpoint } from '../utilities/browser.js';

interface MockOptions {
  browserEndpoint?: BrowserEndpoint;
  config?: Config;
}

export function createMock({
  config = configure(),
  browserEndpoint = { url: 'ws://localhost:3000', protocol: 'playwright' },
}: MockOptions = {}) {
  return {
    server: fastify({ logger: { level: 'warn' } }),
    config,
    getBrowserEndpoint: mock.fn(() => Promise.resolve(browserEndpoint)),
  };
}

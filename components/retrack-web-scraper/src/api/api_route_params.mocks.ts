import { mock } from 'node:test';

import { fastify } from 'fastify';

import type { Config } from '../config.js';
import { configure } from '../config.js';
import type { ApiRouteParams } from './api_route_params.js';

interface MockOptions {
  config?: Config;
  wsEndpoint?: string;
}

export function createMock(options: MockOptions = {}) {
  const config = options.config ?? configure();
  return {
    server: fastify({ logger: { level: 'warn' } }),
    config: options.config
      ? config
      : {
          ...config,
          browser: {
            chromium: { protocol: 'cdp', backend: 'chromium', wsEndpoint: options.wsEndpoint ?? 'ws://localhost:3000' },
          },
        },
    isLocalBrowserServerRunning: mock.fn(() => false),
    getLocalBrowserServer: () => mock.fn(() => Promise.reject("Local browser shouldn't be requested in tests")),
  } as unknown as ApiRouteParams;
}

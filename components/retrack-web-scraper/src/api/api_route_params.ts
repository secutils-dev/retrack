import type { FastifyInstance } from 'fastify';
import type { BrowserServer } from 'playwright-core';

import type { Config, LocalBrowserConfig } from '../config.js';

export interface ApiRouteParams {
  server: FastifyInstance;
  config: Config;
  isLocalBrowserServerRunning: () => boolean;
  getLocalBrowserServer: (config: LocalBrowserConfig) => Promise<BrowserServer>;
}

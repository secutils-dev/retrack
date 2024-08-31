import type { FastifyInstance } from 'fastify';
import type NodeCache from 'node-cache';

import type { Config } from '../config.js';
import type { BrowserEndpoint } from '../utilities/browser.js';

export interface ApiRouteParams {
  server: FastifyInstance;
  cache: NodeCache;
  config: Config;
  getBrowserEndpoint: (options?: { launchServer?: boolean }) => Promise<BrowserEndpoint>;
}

import type { FastifyInstance } from 'fastify';

import type { Config } from '../config.js';
import type { BrowserEndpoint } from '../utilities/browser.js';

export interface ApiRouteParams {
  server: FastifyInstance;
  config: Config;
  getBrowserEndpoint: (options?: { launchServer?: boolean }) => Promise<BrowserEndpoint>;
}

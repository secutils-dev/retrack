import type { FastifyInstance } from 'fastify';
import type NodeCache from 'node-cache';
import type { Browser } from 'playwright';

import type { Config } from '../config.js';
import type { BrowserInfo } from '../index.js';

export interface ApiRouteParams {
  server: FastifyInstance;
  cache: NodeCache;
  config: Config;
  acquireBrowser: () => Promise<Browser>;
  browserInfo: () => BrowserInfo;
}

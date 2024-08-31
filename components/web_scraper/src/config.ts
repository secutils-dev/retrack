import * as dotenv from 'dotenv';

import pkg from '../package.json' with { type: 'json' };

export interface Config {
  version: string;
  port: number;
  cacheTTLSec: number;
  browserTTLSec: number;
  userAgent?: string;
}

export function configure(): Config {
  dotenv.config({ path: process.env.RETRACK_WEB_SCRAPER_ENV_PATH });

  return {
    version: pkg.version,
    port: +(process.env.RETRACK_WEB_SCRAPER_PORT ?? 0) || 7272,
    cacheTTLSec: +(process.env.RETRACK_WEB_SCRAPER_CACHE_TTL_SEC ?? 0) || 20 * 60,
    browserTTLSec: +(process.env.RETRACK_WEB_SCRAPER_BROWSER_TTL_SEC ?? 0) || 10 * 60,
    userAgent: process.env.RETRACK_WEB_SCRAPER_USER_AGENT,
  };
}

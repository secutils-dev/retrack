import process from 'process';
import pkg from '../package.json' with { type: 'json' };

export interface Config {
  version: string;
  port: number;
  isDev: boolean;
  logLevel: string;
  browser: {
    ttlSec: number;
    screenshotsPath?: string;
    headless: boolean;
    sandbox: boolean;
    cdpEndpoint?: string;
    executablePath?: string;
  };
  userAgent?: string;
  server: {
    bodyLimit: number;
  };
}

export function configure(): Config {
  return {
    version: pkg.version,
    port: +(process.env.RETRACK_WEB_SCRAPER_PORT ?? 0) || 7272,
    logLevel: process.env.RETRACK_WEB_SCRAPER_LOG_LEVEL ?? 'debug',
    isDev: process.env.NODE_ENV !== 'production',
    browser: {
      ttlSec: +(process.env.RETRACK_WEB_SCRAPER_BROWSER_TTL_SEC ?? 0) || 10 * 60,
      headless: process.env.RETRACK_WEB_SCRAPER_BROWSER_NO_HEADLESS !== 'true',
      sandbox: !(process.env.RETRACK_WEB_SCRAPER_BROWSER_NO_SANDBOX === 'true'),
      executablePath: process.env.RETRACK_WEB_SCRAPER_BROWSER_EXECUTABLE_PATH || undefined,
      screenshotsPath: process.env.RETRACK_WEB_SCRAPER_BROWSER_SCREENSHOTS_PATH,
      cdpEndpoint: process.env.RETRACK_WEB_SCRAPER_BROWSER_CDP_WS_ENDPOINT || undefined,
    },
    server: {
      // Default body limit is 5MB.
      bodyLimit: +(process.env.RETRACK_WEB_SCRAPER_SERVER_BODY_LIMIT ?? 0) || 5 * 1024 * 1024,
    },
    userAgent: process.env.RETRACK_WEB_SCRAPER_USER_AGENT,
  };
}

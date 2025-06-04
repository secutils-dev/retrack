import process from 'process';
import pkg from '../package.json' with { type: 'json' };
import type { BrowserBackend, BrowserProtocol } from './utilities/browser.js';

export interface Config {
  version: string;
  port: number;
  isDev: boolean;
  logLevel: string;
  browser: { screenshotsPath?: string; chromium?: BrowserConfig; firefox?: BrowserConfig };
  userAgent?: string;
  server: { bodyLimit: number };
}

/**
 * Represents browser config.
 */
export type BrowserConfig = RemoteBrowserConfig | LocalBrowserConfig;

/**
 * Represents remote browser config.
 */
export interface RemoteBrowserConfig {
  protocol: BrowserProtocol;
  backend: BrowserBackend;
  wsEndpoint: string;
}

/**
 * Represents local browser config.
 */
export interface LocalBrowserConfig {
  protocol: 'playwright';
  backend: BrowserBackend;
  executablePath: string;
  ttlSec: number;
  headless: boolean;
  // Chromium specific configuration.
  chromiumSandbox: boolean;
}

export function configure(): Config {
  return {
    version: pkg.version,
    port: +(process.env.RETRACK_WEB_SCRAPER_PORT ?? 0) || 7272,
    logLevel: process.env.RETRACK_WEB_SCRAPER_LOG_LEVEL ?? 'debug',
    isDev: process.env.NODE_ENV !== 'production',
    browser: {
      screenshotsPath: process.env.RETRACK_WEB_SCRAPER_BROWSER_SCREENSHOTS_PATH,
      chromium: process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_EXECUTABLE_PATH
        ? {
            protocol: 'playwright',
            backend: 'chromium',
            executablePath: process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_EXECUTABLE_PATH,
            ttlSec: +(process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_TTL_SEC ?? 0) || 10 * 60,
            headless: process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_NO_HEADLESS !== 'true',
            chromiumSandbox: !(process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_NO_SANDBOX === 'true'),
          }
        : process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_WS_ENDPOINT
          ? {
              protocol: 'cdp',
              backend: 'chromium',
              wsEndpoint: process.env.RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_WS_ENDPOINT,
            }
          : undefined,
      firefox: process.env.RETRACK_WEB_SCRAPER_BROWSER_FIREFOX_WS_ENDPOINT
        ? {
            protocol: 'playwright',
            backend: 'firefox',
            wsEndpoint: process.env.RETRACK_WEB_SCRAPER_BROWSER_FIREFOX_WS_ENDPOINT,
          }
        : undefined,
    },
    server: {
      // The default body limit is 5MB.
      bodyLimit: +(process.env.RETRACK_WEB_SCRAPER_SERVER_BODY_LIMIT ?? 0) || 5 * 1024 * 1024,
    },
    userAgent: process.env.RETRACK_WEB_SCRAPER_USER_AGENT,
  };
}

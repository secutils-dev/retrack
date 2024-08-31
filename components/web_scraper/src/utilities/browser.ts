import type { Logger } from 'playwright-core';

/**
 * Represents a browser endpoint.
 */
export interface BrowserEndpoint {
  protocol: 'cdp' | 'playwright';
  url: string;
}

// Timeout for connecting to a browser.
const BROWSER_CONNECT_TIMEOUT_MS = 30000;

export async function connectToBrowser(endpoint: BrowserEndpoint, logger: Logger) {
  const { chromium } = await import('playwright-core');
  return endpoint.protocol === 'cdp'
    ? chromium.connectOverCDP(endpoint.url, { timeout: BROWSER_CONNECT_TIMEOUT_MS, logger })
    : chromium.connect(endpoint.url, { timeout: BROWSER_CONNECT_TIMEOUT_MS, logger });
}

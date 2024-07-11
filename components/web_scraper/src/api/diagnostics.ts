import type { FastifyBaseLogger } from 'fastify';
import type { Browser } from 'playwright';

export class Diagnostics {
  public static async screenshot(log: FastifyBaseLogger, browser: Browser) {
    log.info('Capturing screenshots...');
    if (!browser.isConnected()) {
      log.error('Browser is not connected, bailing out...');
      return;
    }

    try {
      const pages = browser.contexts().flatMap((context) => context.pages());
      log.debug(`Retrieved ${pages.length} pages.`);
      for (const page of pages) {
        if (page.isClosed()) {
          log.debug(`Page is closed: ${page.url()}.`);
        } else if (page.url() === 'about:blank') {
          log.debug(`Skipping page: ${page.url()}.`);
        } else {
          log.info(`Making screenshot ${page.url()}.`);
          log.error(
            {
              screenshot: Buffer.from((await page.screenshot({ fullPage: true })).toString('base64'), 'base64'),
            },
            `Screenshot is made ${page.url()}.`,
          );
        }
      }
    } catch (err) {
      log.error('Failed to capture screenshots', err);
    }
  }

  public static errorMessage(err: unknown): string {
    if (typeof err === 'string') {
      return err;
    }

    if (err && typeof err === 'object') {
      return (err as { message?: string }).message ?? 'Unknown error';
    }

    return 'UNKNOWN';
  }
}

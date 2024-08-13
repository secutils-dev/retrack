import type { FastifyBaseLogger } from 'fastify/fastify.js';
import type { CDPSession } from 'playwright';

import { Diagnostics } from '../diagnostics.js';

export interface FetchedResource {
  url: string;
  data: string;
  type: 'script' | 'stylesheet';
}

interface FetchInterceptorOptions {
  log: FastifyBaseLogger;
  pageUrl: string;
  session: CDPSession;
}

/**
 * Intercept and collects all fetch request made by the page.
 */
export class FetchInterceptor {
  private readonly interceptedResources: FetchedResource[] = [];
  constructor(private readonly options: FetchInterceptorOptions) {}

  public async start() {
    this.options.session.on('Fetch.requestPaused', async (event) => {
      if (event.responseStatusCode && event.responseStatusCode > 199 && event.responseStatusCode < 300) {
        try {
          const response = await this.options.session.send('Fetch.getResponseBody', { requestId: event.requestId });
          const responseContent = response.base64Encoded
            ? Buffer.from(response.body, 'base64').toString('utf8')
            : response.body;
          this.interceptedResources.push({
            url: event.request.url,
            data: responseContent,
            type: event.resourceType === 'Script' ? 'script' : 'stylesheet',
          });
          this.options.log.debug(
            `Page loaded resource (${Buffer.byteLength(responseContent, 'utf8')} bytes): ${event.request.url}.`,
          );
        } catch (err) {
          this.options.log.error(
            `Failed to fetch external resource "${event.request.url}" body for page "${
              this.options.pageUrl
            }": ${Diagnostics.errorMessage(err)}`,
          );
        }
      }

      await this.options.session.send('Fetch.continueRequest', { requestId: event.requestId }).catch(() => {
        this.options.log.error(`Failed to continue request: ${event.request.url}`);
      });
    });
    await this.options.session.send('Fetch.enable', {
      patterns: [
        { resourceType: 'Script', requestStage: 'Response' },
        { resourceType: 'Stylesheet', requestStage: 'Response' },
      ],
    });
  }

  public async stop() {
    await this.options.session.send('Fetch.disable');
    return this.interceptedResources;
  }
}

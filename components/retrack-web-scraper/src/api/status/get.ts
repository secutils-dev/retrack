import type { ApiRouteParams } from '../api_route_params.js';

export function registerStatusGetRoutes({ server, config, isLocalBrowserServerRunning }: ApiRouteParams) {
  // Register a route that returns the status of the Web Scraper component.
  return server.get(
    '/api/status',
    {
      schema: {
        response: {
          200: {
            type: 'object',
            properties: {
              version: { type: 'string' },
              browser: {
                type: 'object',
                properties: {
                  chromium: {
                    type: 'object',
                    properties: {
                      configured: { type: 'boolean' },
                    },
                  },
                  firefox: {
                    type: 'object',
                    properties: {
                      configured: { type: 'boolean' },
                    },
                  },
                  isServerRunning: { type: 'boolean' },
                },
              },
            },
          },
        },
      },
    },
    async () => {
      return {
        version: config.version,
        browser: {
          isServerRunning: isLocalBrowserServerRunning(),
          chromium: { configured: !!config.browser.chromium },
          firefox: { configured: !!config.browser.firefox },
        },
      };
    },
  );
}

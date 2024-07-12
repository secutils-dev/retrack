import type { ApiRouteParams } from '../api_route_params.js';

export function registerStatusGetRoutes({ server, config, browserInfo }: ApiRouteParams) {
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
                  running: { type: 'boolean' },
                  name: { type: 'string', nullable: true },
                  version: { type: 'string', nullable: true },
                  contexts: {
                    type: 'array',
                    items: { type: 'object', properties: { pages: { type: 'array', items: { type: 'string' } } } },
                  },
                },
              },
            },
          },
        },
      },
    },
    () => {
      return {
        version: config.version,
        browser: browserInfo(),
      };
    },
  );
}

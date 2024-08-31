import type { ApiRouteParams } from '../api_route_params.js';

export function registerStatusGetRoutes({ server, config, getBrowserEndpoint }: ApiRouteParams) {
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
                  protocol: { type: 'string' },
                  url: { type: 'string', nullable: true },
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
        browser: await getBrowserEndpoint({ launchServer: false }),
      };
    },
  );
}

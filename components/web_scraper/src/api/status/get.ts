import type { ApiRouteParams } from '../api_route_params.js';

export function registerStatusGetRoutes({ server, config }: ApiRouteParams) {
  return server.get(
    '/api/status',
    {
      schema: {
        response: {
          200: { type: 'object', properties: { version: { type: 'string' } } },
        },
      },
    },
    () => ({ version: config.version }),
  );
}

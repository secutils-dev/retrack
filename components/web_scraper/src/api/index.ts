import type { ApiRouteParams } from './api_route_params.js';
import * as status from './status/index.js';
import * as webPage from './web_page/index.js';

export function registerRoutes(params: ApiRouteParams) {
  webPage.registerRoutes(params);
  status.registerRoutes(params);
}

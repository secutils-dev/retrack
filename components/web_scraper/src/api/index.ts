import type { ApiRouteParams } from './api_route_params.js';
import * as status from './status/index.js';
import * as web_page from './web_page/index.js';

export function registerRoutes(params: ApiRouteParams) {
  web_page.registerRoutes(params);
  status.registerRoutes(params);
}

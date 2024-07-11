import { registerStatusGetRoutes } from './get.js';
import type { ApiRouteParams } from '../api_route_params.js';

export function registerRoutes(params: ApiRouteParams) {
  registerStatusGetRoutes(params);
}

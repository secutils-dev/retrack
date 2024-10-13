import { registerExecuteRoutes } from './execute.js';
import type { ApiRouteParams } from '../api_route_params.js';

export function registerRoutes(params: ApiRouteParams) {
  registerExecuteRoutes(params);
}

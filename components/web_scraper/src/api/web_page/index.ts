import { registerWebPageContentGetRoutes } from './content/index.js';
import type { WebPageContext } from './content/index.js';
import type { ApiRouteParams } from '../api_route_params.js';

export interface RetrackWindow extends Window {
  __retrack?: {
    extractContent?: (context: WebPageContext) => Promise<unknown>;
  };
}

export function registerRoutes(params: ApiRouteParams) {
  registerWebPageContentGetRoutes(params);
}

import { registerGetContentRoutes } from './get_content.js';
import type { WebPageContext } from './web_page_context.js';
import type { ApiRouteParams } from '../api_route_params.js';

export interface RetrackWindow extends Window {
  __retrack?: {
    extractContent?: (context: WebPageContext) => Promise<unknown>;
  };
}

export function registerRoutes(params: ApiRouteParams) {
  registerGetContentRoutes(params);
}

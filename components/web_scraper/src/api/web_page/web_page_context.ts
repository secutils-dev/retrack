import type { FetchedResource } from './fetch_interceptor.js';

/**
 * Represents the context of a web page the content is being extracted from.
 */
export interface WebPageContext<T = unknown> {
  /**
   * Previous content extracted from the web page.
   */
  previous?: T;

  /**
   * Response headers returned by the web page request.
   */
  responseHeaders: Record<string, string>;

  /**
   * All external resources fetched by the web page.
   */
  externalResources: FetchedResource[];
}

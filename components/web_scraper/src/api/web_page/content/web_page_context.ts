import type { FetchedResource } from '../fetch_interceptor.js';

export interface WebPageContext<T = unknown> {
  previous?: T;
  responseHeaders: Record<string, string>;
  externalResources: FetchedResource[];
}

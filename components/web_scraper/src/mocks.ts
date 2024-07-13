import { mock } from 'node:test';

import type { RetrackWindow } from './api/web_page/index.js';

export function createBrowserMock(browserContextMock?: BrowserContextMock) {
  return {
    isConnected: mock.fn(() => false),
    newContext: mock.fn(() => Promise.resolve(browserContextMock ?? createBrowserContextMock())),
  };
}

export type BrowserContextMock = ReturnType<typeof createBrowserContextMock>;

export function createBrowserContextMock(
  pageMock?: ReturnType<typeof createPageMock>,
  cdpSessionMock?: ReturnType<typeof createCDPSessionMock>,
) {
  return {
    newCDPSession: mock.fn(() => Promise.resolve(cdpSessionMock ?? createCDPSessionMock())),
    newPage: mock.fn(() => Promise.resolve(pageMock ?? createPageMock())),
    close: mock.fn(),
  };
}

export interface CDPResourceMock {
  url: string;
  resourceType: unknown;
  body: string;
}

export function createCDPSessionMock(resources: CDPResourceMock[] = []) {
  let requestPausedHandler: (response: unknown) => Promise<void>;
  return {
    on: mock.fn((eventName: string, handler: (response: unknown) => Promise<void>) => {
      if (eventName === 'Fetch.requestPaused') {
        requestPausedHandler = handler;
      }
    }),
    send: mock.fn(async (methodName: string, args: unknown) => {
      if (methodName === 'Fetch.getResponseBody') {
        const getResponseBodyArgs = args as { requestId: string };
        return Promise.resolve({
          body: Buffer.from(resources[Number(getResponseBodyArgs.requestId)].body).toString('base64'),
          base64Encoded: true,
        });
      }

      if (requestPausedHandler && methodName === 'Fetch.disable') {
        let index = 0;
        for (const resource of resources) {
          await requestPausedHandler({
            requestId: (index++).toString(),
            responseStatusCode: 200,
            request: { url: resource.url, resourceType: resource.resourceType },
          });
        }
      }

      return Promise.resolve();
    }),
  };
}

interface PageMockOptions {
  window?: WindowMock;
  responses?: Array<ResponseMock>;
  content?: string;
}

export function createLocatorMock() {
  return {
    waitFor: mock.fn(),
  };
}

export function createPageMock({ window = createWindowMock(), responses = [], content = '' }: PageMockOptions = {}) {
  return {
    on: mock.fn((eventName: string, handler: (response: ResponseMock) => void) => {
      if (eventName === 'response') {
        for (const response of responses) {
          handler(response);
        }
      }
    }),
    close: mock.fn(),
    goto: mock.fn(() => Promise.resolve(createResponseMock({ url: 'https://retrack.dev', type: 'document' }))),
    content: mock.fn(() => Promise.resolve(content)),
    addInitScript: mock.fn(),
    route: mock.fn(),
    locator: mock.fn(() => createLocatorMock()),
    evaluateHandle: mock.fn(() => window),
    evaluate: mock.fn((fn: (args: unknown) => Promise<unknown>, args: unknown) => fn(args)),
  };
}

export type WindowMock = ReturnType<typeof createWindowMock>;

export function createWindowMock(
  { __retrack }: Pick<RetrackWindow, '__retrack'> = {},
  documentProperties: Record<string, unknown> = {},
) {
  return {
    document: {
      querySelectorAll: mock.fn(() => []),
      ...documentProperties,
    },
    __retrack,
  };
}

export interface ResponseMockOptions {
  url: string;
  type: 'script' | 'stylesheet' | 'document';
  body?: unknown;
}

export type ResponseMock = ReturnType<typeof createResponseMock>;

export function createResponseMock({ url, body, type }: ResponseMockOptions) {
  return {
    url: () => url,
    request: () => ({
      resourceType: () => type,
      isNavigationRequest: () => false,
      method: () => 'GET',
    }),
    body: () => {
      return Promise.resolve(
        Buffer.isBuffer(body)
          ? body
          : body
            ? Buffer.from(typeof body === 'string' ? body : JSON.stringify(body))
            : body,
      );
    },
    allHeaders: mock.fn(() => Promise.resolve({})),
  };
}

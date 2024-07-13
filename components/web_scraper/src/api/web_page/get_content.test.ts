import * as assert from 'node:assert';
import { mock, test } from 'node:test';

import type { Browser } from 'playwright';

import { registerGetContentRoutes } from './get_content.js';
import type { WebPageContext } from './web_page_context.js';
import { configure } from '../../config.js';
import {
  createBrowserContextMock,
  createBrowserMock,
  createCDPSessionMock,
  createLocatorMock,
  createPageMock,
  createWindowMock,
} from '../../mocks.js';
import { createMock } from '../api_route_params.mocks.js';

await test('[/api/web_page/content] can successfully create route', () => {
  assert.doesNotThrow(() => registerGetContentRoutes(createMock()));
});

await test('[/api/web_page/content] can extract content', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const windowMock = createWindowMock();
  const pageMock = createPageMock({
    window: windowMock,
    responses: [],
    content: '<body><div>Hello Retrack and world!</div><div>Hello World</div></body>',
  });
  const cdpSessionMock = createCDPSessionMock();
  const browserContextMock = createBrowserContextMock(pageMock, cdpSessionMock);

  const response = await registerGetContentRoutes(
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    createMock({ browser: createBrowserMock(browserContextMock) as unknown as Browser }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/content',
    payload: { url: 'https://retrack.dev', delay: 0 },
  });

  assert.strictEqual(response.statusCode, 200);

  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123,
      content: '"<body>\\n    <div>Hello Retrack and world!</div>\\n    <div>Hello World</div>\\n</body>"',
    }),
  );

  // Make sure we cleared the cache.
  assert.strictEqual(cdpSessionMock.send.mock.callCount(), 4);
  assert.deepEqual(cdpSessionMock.send.mock.calls[0].arguments, ['Network.clearBrowserCache']);
  assert.deepEqual(cdpSessionMock.send.mock.calls[1].arguments, ['Network.setCacheDisabled', { cacheDisabled: true }]);
  assert.deepEqual(cdpSessionMock.send.mock.calls[2].arguments, [
    'Fetch.enable',
    {
      patterns: [
        { resourceType: 'Script', requestStage: 'Response' },
        { resourceType: 'Stylesheet', requestStage: 'Response' },
      ],
    },
  ]);
  assert.deepEqual(cdpSessionMock.send.mock.calls[3].arguments, ['Fetch.disable']);

  // Maure we set up a proxy URL to load resources bypassing CORS and CSP.
  assert.strictEqual(pageMock.route.mock.callCount(), 1);
  assert.deepEqual(pageMock.route.mock.calls[0].arguments[0], '**/proxy.retrack.dev/*');

  // Make sure we loaded correct page.
  assert.strictEqual(pageMock.goto.mock.callCount(), 1);
  assert.deepEqual(pageMock.goto.mock.calls[0].arguments, ['https://retrack.dev', { timeout: 10000 }]);

  // Make sure we didn't wait for a selector since it wasn't specified.
  assert.strictEqual(pageMock.locator.mock.callCount(), 0);
});

await test('[/api/web_page/content] can wait for element', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const windowMock = createWindowMock();
  const pageMock = createPageMock({
    window: windowMock,
    responses: [],
    content: '<body><div>Hello Retrack and world!</div><div>Hello World</div></body>',
  });
  const locatorMock = createLocatorMock();
  pageMock.locator.mock.mockImplementation(() => locatorMock);

  const cdpSessionMock = createCDPSessionMock();
  const browserContextMock = createBrowserContextMock(pageMock, cdpSessionMock);

  const response = await registerGetContentRoutes(
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    createMock({ browser: createBrowserMock(browserContextMock) as unknown as Browser }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/content',
    payload: { url: 'https://retrack.dev', delay: 0, waitFor: { selector: 'div', state: 'attached', timeout: 10000 } },
  });

  assert.strictEqual(response.statusCode, 200);

  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123,
      content: '"<body>\\n    <div>Hello Retrack and world!</div>\\n    <div>Hello World</div>\\n</body>"',
    }),
  );

  // Make sure we loaded correct page.
  assert.strictEqual(pageMock.goto.mock.callCount(), 1);
  assert.deepEqual(pageMock.goto.mock.calls[0].arguments, ['https://retrack.dev', { timeout: 10000 }]);

  // Make sure we waited for a selector.
  assert.strictEqual(pageMock.locator.mock.callCount(), 1);
  assert.deepEqual(pageMock.locator.mock.calls[0].arguments, ['div']);
  assert.strictEqual(locatorMock.waitFor.mock.callCount(), 1);
  assert.deepEqual(locatorMock.waitFor.mock.calls[0].arguments, [{ state: 'attached', timeout: 10000 }]);
});

await test('[/api/web_page/content] can proxy requests', async () => {
  const windowMock = createWindowMock();
  const pageMock = createPageMock({
    window: windowMock,
    responses: [],
    content: '<body><div>Hello Retrack and world!</div><div>Hello World</div></body>',
  });
  const browserContextMock = createBrowserContextMock(pageMock);

  const response = await registerGetContentRoutes(
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    createMock({ browser: createBrowserMock(browserContextMock) as unknown as Browser }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/content',
    payload: { url: 'https://retrack.dev', delay: 0 },
  });
  assert.strictEqual(response.statusCode, 200);

  // Maure we set up a proxy URL to load resources bypassing CORS and CSP.
  assert.strictEqual(pageMock.route.mock.callCount(), 1);
  assert.deepEqual(pageMock.route.mock.calls[0].arguments[0], '**/proxy.retrack.dev/*');

  const proxyHandler = pageMock.route.mock.calls[0].arguments[1] as (route: unknown) => Promise<void>;
  const mockProxyResponse = Symbol('response');
  const routeMock = {
    fetch: mock.fn(() => Promise.resolve(mockProxyResponse)),
    request: () => ({
      url: () =>
        'https://retrack.dev/proxy.retrack.dev/https%3A%2F%2Fsecutils-dev.github.io%2Fretrack-sandbox%2Fmodule.js',
    }),
    fulfill: mock.fn(),
  };
  await proxyHandler(routeMock);

  assert.strictEqual(routeMock.fetch.mock.callCount(), 1);
  assert.deepEqual(routeMock.fetch.mock.calls[0].arguments, [
    { url: 'https://secutils-dev.github.io/retrack-sandbox/module.js' },
  ]);
  assert.strictEqual(routeMock.fulfill.mock.callCount(), 1);
  assert.deepEqual(routeMock.fulfill.mock.calls[0].arguments, [{ response: mockProxyResponse }]);
});

await test('[/api/web_page/content] can inject content extractor', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const extractContentMock = mock.fn((context: WebPageContext) => {
    return Promise.resolve({ message: (context.previous as { message: string }).message.toUpperCase() });
  });

  const windowMock = createWindowMock({ __retrack: { extractContent: extractContentMock } });
  const pageMock = createPageMock({ window: windowMock, responses: [] });
  const browserContextMock = createBrowserContextMock(pageMock);

  const browserMock = createBrowserMock(browserContextMock);
  const response = await registerGetContentRoutes(
    createMock({
      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
      browser: browserMock as unknown as Browser,
      config: { ...configure(), userAgent: 'retrack/1.0.0' },
    }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/content',
    payload: {
      url: 'https://retrack.dev',
      delay: 0,
      previousContent: '{ "message": "hello" }',
      headers: { Cookie: 'my-cookie' },
      extractor: 'script',
    },
  });

  assert.strictEqual(response.statusCode, 200);

  assert.strictEqual(response.body, JSON.stringify({ timestamp: 123, content: '{"message":"HELLO"}' }));

  // Make sure we loaded correct page.
  assert.strictEqual(pageMock.goto.mock.callCount(), 1);
  assert.deepEqual(pageMock.goto.mock.calls[0].arguments, ['https://retrack.dev', { timeout: 10000 }]);

  assert.strictEqual(browserMock.newContext.mock.callCount(), 1);
  assert.deepEqual(browserMock.newContext.mock.calls[0].arguments, [
    { extraHTTPHeaders: { Cookie: 'my-cookie' }, bypassCSP: false, userAgent: 'retrack/1.0.0' },
  ]);
  assert.strictEqual(browserContextMock.newPage.mock.callCount(), 1);

  // Make sure we didn't wait for a selector since it wasn't specified.
  assert.strictEqual(pageMock.locator.mock.callCount(), 0);

  // Make sure we called includeResource.
  assert.strictEqual(extractContentMock.mock.callCount(), 1);
  assert.deepEqual(extractContentMock.mock.calls[0].arguments, [
    { previous: { message: 'hello' }, externalResources: [], responseHeaders: {} },
  ]);
});

await test('[/api/web_page/content] reports errors in content extractor', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const extractContentMapMock = mock.fn(() => {
    throw new Error('something went wrong');
  });

  const windowMock = createWindowMock({ __retrack: { extractContent: extractContentMapMock } });
  const pageMock = createPageMock({ window: windowMock, responses: [] });
  const browserContextMock = createBrowserContextMock(pageMock);

  const response = await registerGetContentRoutes(
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    createMock({ browser: createBrowserMock(browserContextMock) as unknown as Browser }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/content',
    payload: {
      url: 'https://retrack.dev',
      delay: 0,
      previousContent: '"previous"',
      extractor: 'script',
    },
  });

  assert.strictEqual(response.statusCode, 400);

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: 'Content extractor script has thrown an exception: something went wrong.',
    }),
  );

  // Make sure we called includeResource.
  assert.strictEqual(extractContentMapMock.mock.callCount(), 1);
  assert.deepEqual(extractContentMapMock.mock.calls[0].arguments, [
    { previous: 'previous', externalResources: [], responseHeaders: {} },
  ]);
});

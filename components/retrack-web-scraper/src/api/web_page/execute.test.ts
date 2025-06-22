import * as assert from 'node:assert/strict';
import { test, beforeEach, afterEach } from 'node:test';
import { configure } from '../../config.js';
import { createBrowserServerMock } from '../../mocks.js';

import { registerExecuteRoutes } from './execute.js';
import { createMock } from '../api_route_params.mocks.js';

let browserServerMock: ReturnType<typeof createBrowserServerMock>;
beforeEach(() => (browserServerMock = createBrowserServerMock()));
afterEach(async () => await browserServerMock?.cleanup());

await test('[/api/web_page/execute] can successfully create route', () => {
  assert.doesNotThrow(() => registerExecuteRoutes(createMock()));
});

await test('[/api/web_page/execute] can run extractor scripts', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  browserServerMock.runtimeCallFunctionOn.mock.mockImplementation((params) => {
    if (browserServerMock.isBuiltInPageContent(params)) {
      return {
        type: 'string',
        value:
          '<html lang="en"><head><title>Retrack.dev</title></head><body><div>Hello Retrack and world!</div></body></html>',
      };
    }

    return { type: 'undefined' };
  });

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  await page.goto('https://retrack.dev');
  return await page.content();
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  // Doesn't ignore HTTPS errors by default.
  assert.ok(!browserServerMock.messages.some((message) => message.method === 'Security.setIgnoreCertificateErrors'));

  // Doesn't override the user agent by default.
  assert.ok(!browserServerMock.messages.some((message) => message.method === 'Emulation.setUserAgentOverride'));

  assert.strictEqual(
    response.body,
    JSON.stringify(
      '<html lang="en"><head><title>Retrack.dev</title></head><body><div>Hello Retrack and world!</div></body></html>',
    ),
  );
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] accepts context overrides', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `export async function execute(page) { return 'success'; };`,
      tags: [],
      userAgent: 'Retrack/1.0.0',
      acceptInvalidCertificates: true,
    },
  });

  const ignoreHTTPSErrorsMessage = browserServerMock.messages.find(
    (message) => message.method === 'Security.setIgnoreCertificateErrors',
  );
  assert.ok(ignoreHTTPSErrorsMessage);
  assert.deepStrictEqual(ignoreHTTPSErrorsMessage.params, { ignore: true });

  const userAgentOverrideMessage = browserServerMock.messages.find(
    (message) => message.method === 'Emulation.setUserAgentOverride',
  );
  assert.ok(userAgentOverrideMessage);
  assert.equal((userAgentOverrideMessage.params as { userAgent: string }).userAgent, 'Retrack/1.0.0');

  assert.strictEqual(response.body, JSON.stringify('success'));
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] can provide context', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const mockRoute = registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint }));
  let response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      previousContent: 'some previous content',
      extractor: `
export async function execute(page, context) {
  return context;
};
  `
        .replaceAll('\n', '')
        .trim(),
      extractorParams: { param: 'value' },
      tags: ['tag1', 'tag2'],
    },
  });

  assert.deepEqual(JSON.parse(response.body), {
    tags: ['tag1', 'tag2'],
    previousContent: 'some previous content',
    params: { param: 'value' },
  });
  assert.strictEqual(response.statusCode, 200);

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      previousContent: { a: 1 },
      extractor: `
export async function execute(page, context) {
  return Object.fromEntries(Object.entries(context));
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: ['tag1', 'tag2'],
    },
  });

  assert.deepEqual(JSON.parse(response.body), {
    previousContent: { a: 1 },
    tags: ['tag1', 'tag2'],
  });
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] allows extractor scripts to import selected modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  return (await import('node:util')).inspect(new Map([['one', 1], ['two', 2]]));
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(response.body, JSON.stringify("Map(2) { 'one' => 1, 'two' => 2 }"));
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] allows extractor scripts to import configured extra modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(
    createMock({
      wsEndpoint: browserServerMock.endpoint,
      config: {
        ...configure(),
        extractorSandbox: { extraAllowedModules: ['node:fs'] },
        browser: { chromium: { protocol: 'cdp', backend: 'chromium', wsEndpoint: browserServerMock.endpoint } },
      },
    }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute() {
  await import('timers');
  await import('node:fs');
  return 'OK';
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(response.body, JSON.stringify('OK'));
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] allows extractor scripts to import `data:` modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  return (await import('data:text/javascript,export function add(a, b) { return a + b; }')).add(1, 1);
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(response.body, JSON.stringify(2));
  assert.strictEqual(response.statusCode, 200);
});

await test('[/api/web_page/execute] prevents extractor scripts from importing restricted built-in modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute() {
  await import('node:fs');
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Extractor script is not allowed to import "node:fs" module.`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);
});

await test('[/api/web_page/execute] prevents extractor scripts from importing restricted custom modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute() {
  await import('../../utilities/browser.js');
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Extractor script is not allowed to import "../../utilities/browser.js" module.`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);
});

await test('[/api/web_page/execute] prevents extractor scripts from importing restricted custom modules from `data:` modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute() {
  return (await import('data:text/javascript,import crypto from "node:crypto"; export function add(a, b) { return a + b; }')).add(1, 1);
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Extractor script is not allowed to import "node:crypto" module.`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);
});

await test('[/api/web_page/execute] protects runtime from most common prototype pollution cases', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const mockRoute = registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint }));
  let response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  Object.getPrototypeOf({}).polluted = 'polluted';
  return ({}).polluted || 'Prototype pollution free!';
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  ({}).__proto__.polluted = 'polluted';
  return ({}).polluted || 'Prototype pollution free!';
};
    `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  ([]).__proto__.polluted = 'polluted';
  return ([]).polluted || 'Prototype pollution free!';
};
    `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);
});

await test('[/api/web_page/execute] terminates extractor scripts if it takes too long to execute', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(createMock({ wsEndpoint: browserServerMock.endpoint })).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(page) {
  const delay = (time) => new Promise((resolve) => setTimeout(resolve, time));
  await delay(10000);
  return 'some text';
};
  `
        .replaceAll('\n', '')
        .trim(),
      tags: [],
      timeout: 5000,
    },
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: The execution was terminated due to timeout 5000ms.`,
    }),
  );
  assert.strictEqual(response.statusCode, 500);
});

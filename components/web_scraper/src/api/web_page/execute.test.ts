import * as assert from 'node:assert';
import { test, beforeEach, afterEach } from 'node:test';
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

    throw new Error(`Unexpected objectId: ${params.objectId}`);
  });

  const response = await registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  const page = await context.newPage(); 
  await page.goto('https://retrack.dev');
  return result.html(await page.content());
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 200);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123000,
      content: JSON.stringify({
        type: 'html',
        value:
          '<html lang="en"><head><title>Retrack.dev</title></head><body><div>Hello Retrack and world!</div></body></html>',
      }),
    }),
  );
});

await test('[/api/web_page/execute] can provide previous content', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const mockRoute = registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  );
  let response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      previousContent: { type: 'html', value: 'some previous content' },
      extractor: `
export async function execute(context, result, previousContent) {
  return result.text(previousContent);
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 200);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123000,
      content: JSON.stringify({ type: 'text', value: 'some previous content' }),
    }),
  );

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      previousContent: { type: 'json', value: JSON.stringify({ a: 1 }) },
      extractor: `
export async function execute(context, result, previousContent) {
  return Object.entries(previousContent);
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 200);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123000,
      content: JSON.stringify({ type: 'json', value: JSON.stringify([['a', 1]]) }),
    }),
  );
});

await test('[/api/web_page/execute] allows extractor scripts to import selected modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  return result.text((await import('node:util')).inspect(new Map([['one', 1], ['two', 2]])));
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 200);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      timestamp: 123000,
      content: JSON.stringify({ type: 'text', value: "Map(2) { 'one' => 1, 'two' => 2 }" }),
    }),
  );
});

await test('[/api/web_page/execute] prevents extractor scripts from importing restricted built-in modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  await import('node:fs');
  return result.text('some text');
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Extractor script is not allowed to import "node:fs" module.`,
    }),
  );
});

await test('[/api/web_page/execute] prevents extractor scripts from importing restricted custom modules', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  await import('../../utilities/browser.js');
  return result.text('some text');
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Extractor script is not allowed to import "../../utilities/browser.js" module.`,
    }),
  );
});

await test('[/api/web_page/execute] protects runtime from most common prototype pollution cases', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const mockRoute = registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  );
  let response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  Object.getPrototypeOf({}).polluted = 'polluted';
  return result.text(({}).polluted || 'Prototype pollution free!');
};
  `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  ({}).__proto__.polluted = 'polluted';
  return result.text(({}).polluted || 'Prototype pollution free!');
};
    `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );

  response = await mockRoute.inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  ([]).__proto__.polluted = 'polluted';
  return result.text(([]).polluted || 'Prototype pollution free!');
};
    `
        .replaceAll('\n', '')
        .trim(),
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: Cannot add property polluted, object is not extensible`,
    }),
  );
});

await test('[/api/web_page/execute] terminates extractor scripts if it takes too long to execute', async (t) => {
  t.mock.method(Date, 'now', () => 123000);

  const response = await registerExecuteRoutes(
    createMock({ browserEndpoint: { protocol: 'cdp', url: browserServerMock.endpoint } }),
  ).inject({
    method: 'POST',
    url: '/api/web_page/execute',
    payload: {
      extractor: `
export async function execute(context, result) {
  const delay = (time) => new Promise((resolve) => setTimeout(resolve, time));
  await delay(10000);
  return result.text('some text');
};
  `
        .replaceAll('\n', '')
        .trim(),
      timeout: 5000,
    },
  });

  assert.strictEqual(response.statusCode, 500);
  assert.strictEqual(
    response.body,
    JSON.stringify({
      message: `Failed to execute extractor script: The execution was terminated due to timeout 5000ms.`,
    }),
  );
});

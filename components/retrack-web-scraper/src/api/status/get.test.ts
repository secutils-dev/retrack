import * as assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Config } from '../../config.js';

import { registerStatusGetRoutes } from './get.js';
import { createMock } from '../api_route_params.mocks.js';

await test('[/api/status] can successfully create route', () => {
  assert.doesNotThrow(() => registerStatusGetRoutes(createMock()));
});

await test('[/api/status] returns version from the config', async () => {
  const configMock: Config = {
    version: '1.0.0-rc.100',
    isDev: false,
    logLevel: 'debug',
    browser: {
      chromium: {
        backend: 'chromium' as const,
        wsEndpoint: 'ws://localhost:3000',
        protocol: 'cdp' as const,
      },
    },
    server: { bodyLimit: 5 * 1024 * 1024 },

    port: 3,
  };
  const response = await registerStatusGetRoutes(createMock({ config: configMock })).inject({
    method: 'GET',
    url: '/api/status',
  });

  assert.strictEqual(
    response.body,
    JSON.stringify({
      version: configMock.version,
      browser: { chromium: { configured: true }, firefox: { configured: false }, isServerRunning: false },
    }),
  );
  assert.strictEqual(response.statusCode, 200);
});

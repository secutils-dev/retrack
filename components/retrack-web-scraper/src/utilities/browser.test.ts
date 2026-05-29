import * as assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import type { FastifyBaseLogger } from 'fastify';
import type { BrowserServer } from 'playwright-core';

import type { LocalBrowserConfig } from '../config.js';
import { createBrowserServerManager } from './browser.js';

function createLoggerMock(): FastifyBaseLogger {
  const logger = {
    info: () => {},
    error: () => {},
    warn: () => {},
    debug: () => {},
    trace: () => {},
    fatal: () => {},
    silent: () => {},
    level: 'silent',
  } as unknown as FastifyBaseLogger;
  (logger as unknown as { child: () => FastifyBaseLogger }).child = () => logger;
  return logger;
}

function createServerMock(wsEndpoint: string) {
  return {
    wsEndpoint: () => wsEndpoint,
    close: mock.fn(() => Promise.resolve()),
  } as unknown as BrowserServer;
}

const LOCAL_CONFIG: LocalBrowserConfig = {
  protocol: 'playwright',
  backend: 'chromium',
  executablePath: '/usr/local/bin/chromium',
  // Large TTL so the idle shutdown never fires during the test.
  ttlSec: 3600,
  headless: true,
  chromiumSandbox: false,
};

await test('[browser server manager] launches a single shared server for concurrent callers', async () => {
  const server = createServerMock('ws://localhost/shared');
  const launch = mock.fn(() => Promise.resolve(server));
  const manager = createBrowserServerManager(createLoggerMock(), { launch, stop: () => Promise.resolve() });

  const [first, second] = await Promise.all([manager.get(LOCAL_CONFIG), manager.get(LOCAL_CONFIG)]);

  assert.strictEqual(first, server);
  assert.strictEqual(second, server);
  assert.strictEqual(launch.mock.callCount(), 1, 'expected a single launch shared across concurrent callers');

  await manager.close();
});

await test('[browser server manager] does not cache a failed launch and retries on the next request', async () => {
  const server = createServerMock('ws://localhost/recovered');
  let attempts = 0;
  const launch = mock.fn(() => {
    // First launch fails (simulating a transient browser crash), the second succeeds.
    attempts += 1;
    return attempts === 1 ? Promise.reject(new Error('Failed to launch browser.')) : Promise.resolve(server);
  });
  const manager = createBrowserServerManager(createLoggerMock(), { launch, stop: () => Promise.resolve() });

  await assert.rejects(manager.get(LOCAL_CONFIG), /Failed to launch browser/);

  // The previous failure must not be cached, so the manager reports no running server...
  assert.strictEqual(manager.isRunning(), false, 'a failed launch must not be cached');

  // ...and the next request launches a fresh server successfully instead of replaying the failure.
  const recovered = await manager.get(LOCAL_CONFIG);
  assert.strictEqual(recovered, server);
  assert.strictEqual(launch.mock.callCount(), 2, 'expected the failed launch to be retried on the next request');

  await manager.close();
});

await test('[browser server manager] close stops the running server and resets state', async () => {
  const server = createServerMock('ws://localhost/closing');
  const stop = mock.fn(() => Promise.resolve());
  const manager = createBrowserServerManager(createLoggerMock(), { launch: () => Promise.resolve(server), stop });

  await manager.get(LOCAL_CONFIG);
  assert.strictEqual(manager.isRunning(), true);

  await manager.close();
  assert.strictEqual(stop.mock.callCount(), 1, 'expected the running server to be stopped on close');
  assert.strictEqual(manager.isRunning(), false);
});

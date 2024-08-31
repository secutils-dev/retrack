import * as assert from 'node:assert';
import { test } from 'node:test';

import { ExecutionResult } from './execution_result.js';

await test('[execution_result] can properly convert result to content', () => {
  assert.deepStrictEqual(ExecutionResult.json({ a: 1 }).toContent(), { type: 'json', value: '{"a":1}' });
  assert.deepStrictEqual(ExecutionResult.html('<div>1</div>').toContent(), { type: 'html', value: '<div>1</div>' });
  assert.deepStrictEqual(ExecutionResult.text('some text').toContent(), { type: 'text', value: 'some text' });
});

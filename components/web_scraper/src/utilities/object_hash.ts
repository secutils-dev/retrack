import { createHash } from 'node:crypto';

import jsonStableStringify from 'fast-json-stable-stringify';

export function createObjectHash<T extends object>(data: T) {
  return createHash('sha256').update(jsonStableStringify(data)).digest('hex');
}

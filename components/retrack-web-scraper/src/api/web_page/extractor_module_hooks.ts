import type { ResolveHookSync } from 'node:module';
import type { ExtractorSandboxConfig } from '../../config.js';

// This set contains the modules that are allowed to be imported by extractor scripts.
const EXTRACTOR_MODULE_ALLOWLIST = [
  'node:stream',
  'node:stream/promises',
  'stream',
  'stream/promises',

  'node:timers',
  'node:timers/promises',
  'timers',
  'timers/promises',

  'node:util',
  'util',
];

// Creates a hook that is called whenever a module is resolved, allowing you to intercept the resolution process and
// prevent extractor scripts from importing modules. This is useful for preventing extractor scripts from accessing
// sensitive data, e.g., the `workerData` from `worker_threads` module or `fs`.
// For more details, refer to https://nodejs.org/api/module.html#resolvespecifier-context-nextresolve
export function createExtractorResolveHook({ extraAllowedModules }: ExtractorSandboxConfig): ResolveHookSync {
  const allowlist = new Set([...EXTRACTOR_MODULE_ALLOWLIST, ...extraAllowedModules]);
  return (specifier, context, nextResolve) => {
    if (context.parentURL?.startsWith('data:') && !specifier.startsWith('data:') && !allowlist.has(specifier)) {
      throw new Error(`Extractor script is not allowed to import "${specifier}" module.`);
    }
    return nextResolve(specifier, context);
  };
}

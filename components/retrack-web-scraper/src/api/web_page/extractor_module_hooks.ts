import type { ResolveFnOutput, ResolveHookContext } from 'module';
import { EXTRACTOR_MODULE_PREFIX } from './constants.js';

// This set contains the modules that are allowed to be imported by extractor scripts.
const EXTRACTOR_MODULE_ALLOWLIST = new Set(['node:util', 'stream', 'stream/promises']);

// This hook is called whenever a module is resolved, allowing you to intercept the resolution process and prevent
// extractor scripts from importing modules. This is useful for preventing extractor scripts from accessing sensitive
// data, e.g.,`workerData` from `worker_threads` module or `fs`.
// For more details, refer to https://nodejs.org/api/module.html#resolvespecifier-context-nextresolve
export function resolve(
  specifier: string,
  context: ResolveHookContext,
  nextResolve: (specifier: string, context?: ResolveHookContext) => ResolveFnOutput | Promise<ResolveFnOutput>,
): ResolveFnOutput | Promise<ResolveFnOutput> {
  if (context.parentURL?.startsWith(EXTRACTOR_MODULE_PREFIX) && !EXTRACTOR_MODULE_ALLOWLIST.has(specifier)) {
    throw new Error(`Extractor script is not allowed to import "${specifier}" module.`);
  }
  return nextResolve(specifier, context);
}

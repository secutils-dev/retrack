import type { ResolveFnOutput, ResolveHookContext } from 'module';
import { USER_MODULE_PREFIX } from './constants.js';

// This set contains the modules that are allowed to be imported by user scenarios.
const USER_MODULE_ALLOWLIST = new Set(['node:util']);

// This hook is called whenever a module is resolved, allowing you to intercept the resolution process and prevent
// user scripts from importing modules. This is useful for preventing user scripts from accessing sensitive data, e.g.,
// `workerData` from `worker_threads` module or `fs`.
// For more details, refer to https://nodejs.org/api/module.html#resolvespecifier-context-nextresolve
export function resolve(
  specifier: string,
  context: ResolveHookContext,
  nextResolve: (specifier: string, context?: ResolveHookContext) => ResolveFnOutput | Promise<ResolveFnOutput>,
): ResolveFnOutput | Promise<ResolveFnOutput> {
  if (context.parentURL?.startsWith(USER_MODULE_PREFIX) && !USER_MODULE_ALLOWLIST.has(specifier)) {
    throw new Error(`Scenario is not allowed to import "${specifier}" module.`);
  }
  return nextResolve(specifier, context);
}

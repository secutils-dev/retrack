import type { ResolveFnOutput, ResolveHookContext, InitializeHook } from 'module';
import type { ExtractorSandboxConfig } from '../../config.js';

// This set contains the modules that are allowed to be imported by extractor scripts.
const EXTRACTOR_MODULE_ALLOWLIST = new Set([
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
]);

// The initialize hook provides a way to define a custom function that runs in
// the hooks thread when the hooks module is initialized. Initialization happens
// when the hooks module is registered via `register`.
export const initialize: InitializeHook<ExtractorSandboxConfig> = async ({ extraAllowedModules }) => {
  // Add extra allowed modules to the allowlist.
  for (const module of extraAllowedModules) {
    EXTRACTOR_MODULE_ALLOWLIST.add(module);
  }
};

// This hook is called whenever a module is resolved, allowing you to intercept the resolution process and prevent
// extractor scripts from importing modules. This is useful for preventing extractor scripts from accessing sensitive
// data, e.g., the `workerData` from `worker_threads` module or `fs`.
// For more details, refer to https://nodejs.org/api/module.html#resolvespecifier-context-nextresolve
export function resolve(
  specifier: string,
  context: ResolveHookContext,
  nextResolve: (specifier: string, context?: ResolveHookContext) => ResolveFnOutput | Promise<ResolveFnOutput>,
): ResolveFnOutput | Promise<ResolveFnOutput> {
  if (
    context.parentURL?.startsWith('data:') &&
    !specifier.startsWith('data:') &&
    !EXTRACTOR_MODULE_ALLOWLIST.has(specifier)
  ) {
    throw new Error(`Extractor script is not allowed to import "${specifier}" module.`);
  }
  return nextResolve(specifier, context);
}

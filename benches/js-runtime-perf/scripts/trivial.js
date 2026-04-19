// Minimal extractor-shaped script: returns an empty body envelope, just
// enough for ExtractorScriptResult to deserialise. Used by the
// cold-start and steady-state trivial scenarios.
(() => ({ body: Deno.core.encode("{}") }))();

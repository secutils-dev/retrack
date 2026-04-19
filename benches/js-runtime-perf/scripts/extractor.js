// Realistic extractor: decodes the first response body (Uint8Array), parses
// JSON, filters items, and re-encodes the result. Exercises the same code
// paths a production extractor script would.
(() => {
  const response = context.responses[0];
  const payload = JSON.parse(Deno.core.decode(new Uint8Array(response.body)));
  const filtered = (payload.items || [])
    .filter((item) => item.value > 10)
    .map((item) => ({ id: item.id, value: item.value * 2 }));
  return { body: Deno.core.encode(JSON.stringify({ status: response.status, filtered })) };
})();

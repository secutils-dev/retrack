# Retrack dependency upgrades

Run commands from the Retrack root.
For a product-wide Secutils upgrade, use its root dependency guide for cross-repository stages and Playwright coordination.

## Sequence and gates

Use Rust → Node → NPM → Docker order for a full upgrade.
For a targeted upgrade, preserve applicable prerequisites.
Node and lockfile updates precede Docker rebuilds so `npm ci` consumes the selected runtime's dependency tree.
Each stage must pass its gate before the next stage starts.
Keep stages individually committable. Commit only when the user requests it.

### 1. Rust

Update `Cargo.toml`, `components/retrack-types/Cargo.toml`, and `benches/js-runtime-perf/Cargo.toml` as applicable.
For semver-compatible upgrades, update the selected dependencies and refresh the lockfile.

```bash
cargo update
cargo +nightly fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
make db-prepare-check
make perf ANALYZE=1 PERF_ITERATIONS=20 PERF_WARMUP=5
```

Read [performance](performance.md) before the harness run.
Regressions are advisory. Failed harness execution still requires investigation.
Use workspace tests too when shared types or other workspace members change, matching CI's workspace coverage.

After `deno_core` changes, review the `can_access_deno_apis` snapshot for intentional op additions/removals.
Use `cargo insta review` to inspect changes before acceptance.
After a SQLx bump or macro query change, run `make dev-up` and `make db-prepare` before the cache gate.
Include regenerated `.sqlx/` artifacts with the change.

### 2. Node

Select the intended LTS major in `.nvmrc`.
Match every declared `engines.node` and `@types/node` major.
For Node 24 or later, omit `--experimental-global-webcrypto` from worker `execArgv`.
Keep `delete (globalThis as { crypto?: unknown }).crypto;` in the sandboxed user-script worker.
The host's always-present WebCrypto API must remain inaccessible to user scripts.
Then refresh and run the workspace gate:

```bash
npm install
npm run lint --ws --if-present
npm test --ws --if-present
npm run build --ws --if-present
```

### 3. NPM

Update root and `components/retrack-web-scraper/package.json` dependencies.
Run the same workspace gate as stage 2 from the Retrack root.

Pin scraper `playwright-core` exactly.
For Secutils integration, inspect Web UI/E2E/Camoufox pins and keep all four consumers on the same minor.
Record the selected version for the parent upgrade stages. Match patches where compatible.
After `@commitlint/*` upgrades, run `npx commitlint --from HEAD~1` to exercise the existing configuration.

For remote browser diagnostics, use `DEBUG=pw:*` or Playwright tracing.
Keep `logger` out of `connect` and `connectOverCDP` options. Those APIs do not accept it.
Read `components/retrack-web-scraper/src/utilities/browser.ts` for the current connection adapter.

### 4. Docker

Update the three Dockerfiles, language/tool versions, and UPX as applicable.
For Camoufox changes, read [Camoufox](camoufox.md) before edits or builds.
Re-pin digests even for unchanged rolling tags:

```bash
./dev/scripts/docker-pin-digests.sh
make docker-api
make docker-scraper
make docker-scraper-camoufox
```

Smoke-check each built image's entrypoint and requested runtime version.
The scraper entrypoint is `components/retrack-web-scraper/src/index.js`.
Camoufox's entrypoint is `/app/camoufox_launcher.py` and also requires a usable browser cache.
Complete the Camoufox integration gate when that image changes.

## Scraper workspace layout

Preserve `/app` workspace structure in the scraper runtime image.
The shared lockfile stores scraper dependencies under `packages."components/retrack-web-scraper"`.
A flattened install can fail with missing-package or invalid-lockfile errors during a fresh `npm ci`.
Read `Dockerfile.web-scraper` for the copy paths and install command:

```bash
npm ci --omit=dev --workspace=retrack-web-scraper --include-workspace-root=false
```

Completion requires successful fresh image installation, rather than reliance on the host NPM cache.

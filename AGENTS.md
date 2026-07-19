# AGENTS.md

## Dependency upgrades

### Recommended upgrade order

Retrack has a hard dependency surface (Rust crates → SQLx schema cache → Node runtime →
NPM packages → Docker base images). Upgrade in this order so each stage builds on a
green previous one and a single failure does not contaminate later stages:

1. **Rust crates** in `Cargo.toml` (workspace root), `components/retrack-types/Cargo.toml`,
   and `benches/js-runtime-perf/Cargo.toml`. Bump everything to the latest semver-compatible
   release in one pass; refresh `Cargo.lock` with `cargo update`.
2. **`.nvmrc`** to the next Node LTS (the same major must be reflected in `engines.node` of
   every `package.json` in the workspace).
3. **NPM packages** in workspace-root `package.json` and `components/retrack-web-scraper/package.json`.
   Refresh `package-lock.json` with `npm install` (run from the workspace root so all
   workspaces resolve through the same tree).
4. **Dockerfile base images** (`Dockerfile`, `Dockerfile.web-scraper`,
   `Dockerfile.web-scraper-camoufox`), UPX, Camoufox, `playwright-python`. Re-pin SHA256
   manifest digests with `./dev/scripts/docker-pin-digests.sh`.

Do **not** reorder — bumping Node before bumping NPM deps means the host `npm install`
runs against the new Node and may produce a lockfile the older Docker base cannot
consume; bumping the Docker bases before the NPM lock means the runtime stage's
`npm ci` validates against a stale lock.

### Stage 1 — Rust crates

```bash
# Edit Cargo.toml files, then:
cargo update
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
make perf ANALYZE=1 PERF_ITERATIONS=20 PERF_WARMUP=5    # smoke; never fails the build
```

What to watch for:

- **`deno_core`** bumps almost always invalidate the `js_runtime::tests::can_access_deno_apis`
  inline snapshot because `Deno.core.*` exposes new ops or removes deprecated ones (e.g.
  `__processTimers`/`__resolveOps` were removed in favour of `__eventLoopTick` /
  `__setTimerExpiry`). Run `cargo insta accept -p retrack` and review the diff to confirm
  the new API surface is intentional rather than a regression.
- **`sqlx`** macros validate queries against `.sqlx/` cached query plans. After a `sqlx`
  bump or a query change, `cargo check` will fail offline with `Connection refused` /
  `SQLX_OFFLINE` errors. Regenerate the cache:

  ```bash
  make dev-up        # starts the dev Postgres
  make db-prepare    # runs `cargo sqlx prepare` against the live DB
  ```

  Commit the regenerated `.sqlx/` directory alongside the bump. CI runs `make
  db-prepare-check` and fails if the cache is out of date.

### Stage 2 — Node major bump (`.nvmrc`)

```bash
echo 24 > .nvmrc
# Update engines.node in every package.json (workspace root + each leaf) to "24.x"
# Update @types/node to ^24.x in every package.json that declares it (root, web-scraper).
npm install                                                     # refreshes package-lock.json
npm run lint --ws --if-present
npm test --ws --if-present
npm run build --ws --if-present
```

### Stage 3 — NPM packages

```bash
# Edit package.json files (root + leaves), then from components/retrack/:
npm install
npm run lint --ws --if-present
npm test --ws --if-present
npm run build --ws --if-present
```

What to watch for:

- **`playwright-core` must be pinned to a single exact version** across the whole product
  (web-scraper, secutils-webui, e2e harness, and the `playwright==<x.y.z>` pip pin in
  `Dockerfile.web-scraper-camoufox`). Mismatches cause subtle protocol-level
  incompatibilities. When bumping it here, record the pinned version and bump the other
  three locations in their respective stages — do not let them drift between PRs. All four
  are currently `1.61.0`.
- **`@commitlint/*` majors** sometimes change their config schema; rerun `npx commitlint
  --from HEAD~1` after bumping to confirm the existing `commitlint.config.cjs` still
  parses.

### Stage 4 — Docker base images

```bash
# Edit FROM lines (image + tag, no digest) and language/tool versions.
./dev/scripts/docker-pin-digests.sh    # rewrites @sha256:... to current manifest digests
make docker-api
make docker-scraper
make docker-scraper-camoufox
```

The pin script reads every `FROM` line in the three Dockerfiles, calls `docker buildx
imagetools inspect <image>:<tag>` to resolve the current manifest-list digest, and
rewrites the line in place. It always re-pins, even when the tag did not change — running
it on every upgrade keeps the digest fresh against the rolling tag.

What to watch for:

- **`Dockerfile.web-scraper` runtime stage requires the workspace layout.** The runtime
  image is a flat install, but the workspace lockfile records the web-scraper's deps
  under `packages."components/retrack-web-scraper"`, not under the top-level
  `packages.""` key. With npm ≥ 11 (which ships with Node ≥ 22), `npm ci --production`
  in a flattened layout fails with `Missing: <pkg> from lock file` /
  `Invalid: lock file's <pkg>@x does not satisfy <pkg>@y`. The Dockerfile preserves the
  workspace structure under `/app` and installs with:

  ```dockerfile
  COPY --from=builder ["/app/package.json", "/app/package-lock.json", "./"]
  COPY --from=builder ["/app/components/retrack-web-scraper/package.json", "./components/retrack-web-scraper/"]
  COPY --from=builder ["/app/components/retrack-web-scraper/dist/", "./components/retrack-web-scraper/"]
  RUN npm ci --omit=dev --workspace=retrack-web-scraper --include-workspace-root=false && ...
  CMD ["node", "components/retrack-web-scraper/src/index.js"]
  ```

  Do not "simplify" by flattening — it works against the host npm cache but breaks the
  fresh `npm ci` inside the runtime stage.
- **Camoufox is a coupled triple.** The Camoufox wrapper (PyPI), `playwright-python`, and
  the Camoufox Firefox build ID (`python -m camoufox fetch official/<id>`) must move
  together. The current set is the **official** `camoufox==0.5.4` (daijro) +
  `playwright==1.61.0` + Firefox `152.0.4`, pinned via the `CAMOUFOX_VERSION` ARG in
  `Dockerfile.web-scraper-camoufox`. **Pin the version, not the full build label** — see the
  next two bullets for why.
  - **Use the official `camoufox` package, not the `cloverlabs-camoufox` fork.** The fork's
    `launchServer.js` does `require(${cwd}/lib/browserServerImpl.js)`, a driver internal
    that current Playwright does not ship, so it crash-loops (`MODULE_NOT_FOUND` → "Server
    process terminated unexpectedly"). The official package resolves Playwright through the
    driver's **public `index.js` entrypoint** and calls the public `firefox.launchServer()`
    API, which is stable across current Playwright versions.
  - **Override the official package's conservative `playwright<1.61` cap.** `pip install
    camoufox==0.5.4` pulls `playwright 1.60.0`; the Dockerfile then runs `pip install
    playwright==1.61.0` to match the rest of the stack. pip prints a harmless
    dependency-conflict warning and proceeds — the private import
    `playwright._impl._driver.compute_driver_executable` and `driver/package/index.js`
    both resolve under 1.61. Re-check this when you bump `camoufox`: a future release may
    drop the cap or change how it launches the server.
  - **Install `playwright-python` from PyPI** with `pip install playwright==<x.y.z>` — no
    git ref, and therefore no `git`/`curl` build deps or purge layer in the Dockerfile.
- **The Camoufox build ID is the *asset* label, and it differs per architecture.**
  daijro publishes releases under tags like `v150.0.2-beta.25`, but the build id
  camoufox resolves comes from the *asset filename*, which is both `-alpha` (not the
  tag's `-beta`) **and architecture-specific**: `v150.0.2-beta.25` ships
  `camoufox-150.0.2-alpha.25-lin.arm64.zip` on arm64 but
  `camoufox-150.0.2-alpha.26-lin.x86_64.zip` on x86_64. Hardcoding a full
  `<version>-<label>` id therefore only builds on one arch — a local arm64 build can
  pass while CI's amd64 `docker bake` fails on the exact same id. The Dockerfile
  avoids this by pinning only `CAMOUFOX_VERSION` and resolving the arch-correct
  `<version>-<label>` from the platform-filtered `repo_cache.json` after
  `camoufox sync`, then fetching that. When bumping, set `CAMOUFOX_VERSION` to the
  Firefox version (e.g. `150.0.2`), never the build label.
- **`camoufox fetch` fails silently; the Dockerfile guards against it.** `camoufox
  fetch official/<id>` exits 0 even when `<id>` is absent from the synced repo cache,
  which would bake a *browser-less* image that crash-loops at runtime with
  `CamoufoxNotInstalled: official/stable is not installed`. The fetch RUN layer asserts
  the browser dir (`/root/.cache/camoufox/browsers/official/<id>`) exists and is
  non-empty so a dead pin breaks the build instead of e2e-up. If you hit
  `CamoufoxNotInstalled` after `make e2e-up`, the camoufox image was built against an
  id that no longer resolves — re-pin and rebuild (`make e2e-up BUILD=1`).
- **`camoufox sync` dedups builds by label, which expires older pins.** The sync cache
  keys available builds by their build label alone (`seen_builds`), so when two daijro
  releases ship the same label (e.g. both `146.0.1` and `150.0.2` carry `alpha.25`),
  only the newest survives in the cache and the older `<version>-<label>` id stops
  resolving. This is exactly what retired the previous `official/146.0.1-alpha.25` pin
  once `v150.0.2` was published. Camoufox also sorts builds by label (`alpha` < `beta`),
  so the newest Firefox can rank *below* stale `*-beta` builds in `official/stable` —
  do not rely on `stable`. This is why the Dockerfile resolves the build by matching
  the pinned `CAMOUFOX_VERSION` against the synced cache (per the previous bullet)
  rather than fetching `official/stable` or trusting "latest".
- **`playwright-python` minor** must match the `playwright-core` minor pinned in stage 3
  (currently `1.61`). The Camoufox image's Playwright driver speaks the protocol of the
  matching Node-side Playwright; mismatches surface as cryptic "browser closed
  unexpectedly" errors at runtime. The whole product is now on a single Playwright
  version — `playwright-core` (web-scraper, secutils-webui), `@playwright/test` (e2e), and
  `playwright-python` (camoufox) are all `1.61.0`.
- **Smoke-test each image after build:** check that the entrypoint path exists
  (`components/retrack-web-scraper/src/index.js` for the scraper, `/app/camoufox_launcher.py`
  for camoufox), the language runtime version is what was requested
  (`node --version` / `python3 --version`), and for camoufox that the Firefox build cache
  populated under `/root/.cache/camoufox/browsers`. The full app stack lives one level up
  in the parent `secutils` repo's `dev/docker/docker-compose.yml`; the local
  `dev/docker/docker-compose.yml` here only spins up Postgres for `make dev-up`.

### Cross-cutting reminders

- **Commitlint.** The repo enforces conventional commits via husky pre-commit. Use
  `chore(deps): ...` for dependency-only commits and `chore(docker): ...` for image
  re-pins; mixing dep code changes (e.g. an API adapter for `hickory-resolver`) with the
  bump is fine and should still go under `chore(deps)`.
- **Performance harness regressions are advisory.** A perf delta after a `deno_core`,
  `tokio`, or `reqwest` bump is informational; the CI job never fails on it. If a
  regression is large, decide whether to roll back the specific crate or accept it
  before commit — but do not block the upgrade chain on the harness.

## JS Runtime Performance Harness (`benches/js-runtime-perf/`)

### Overview

Retrack embeds a Deno/V8 runtime to execute user-supplied extractor and formatter scripts.
Retrack keeps a single long-lived worker thread that owns one V8 isolate and receives work
over an `mpsc` channel. This harness measures the latency, throughput, and peak RSS delta 
of that runtime so changes to the architecture (context-per-call, pooling, shared HTTP client,
startup snapshots, etc.) can be evaluated with real numbers.

The harness is self-contained: it lives inside the `retrack` workspace, links against the
real `retrack::js_runtime::JsRuntime`.

The harness is **advisory / warn-only**. CI records a new history entry on every push to
`main` and prints a table with per-metric deltas, but it never fails a build on
regressions. Thresholds in `.perf/config.json` only control when warnings are emitted.

### Scenario catalogue

All scenarios use a default `JsRuntimeConfig` with a 10 MiB heap and a 10s execution
budget, matching production settings.

| Scenario                   | What it measures                                                                                                       |
|----------------------------|------------------------------------------------------------------------------------------------------------------------|
| `cold_start_trivial`       | Full worker-thread startup: `JsRuntime::init()` + fresh V8 isolate + first script execution, trivial script.           |
| `steady_state_trivial`     | Serial executions of a trivial script through a single long-lived `JsRuntime`.                                         |
| `steady_state_extractor`   | Realistic extractor: decodes a `Uint8Array` response body, parses JSON, filters/maps, re-encodes the result.           |
| `concurrent_extractors_8x` | `tokio::spawn` burst of `N` extractor calls sharing one `Arc<JsRuntime>`; exposes the single-worker-thread bottleneck. |

The last scenario is deliberately designed to show that Retrack's current mpsc-based
architecture serialises concurrent work onto one worker thread, which is the exact shape
of the bottleneck we want any future optimisation to address.

### Running locally

```bash
# Full run + comparison table + history append (from components/retrack/)
make perf ANALYZE=1

# Run only, no history touch (useful when iterating locally and discarding results)
make perf

# Re-analyze an existing /tmp/perf.json (e.g. downloaded from CI) without rerunning
make perf-analyze

# Smoke test (fast)
make perf ANALYZE=1 PERF_ITERATIONS=20 PERF_WARMUP=5

# Single scenario
make perf ANALYZE=1 PERF_SCENARIOS=steady_state_extractor

# Custom output path
make perf PERF_OUTPUT=/tmp/perf-baseline.json

# View HTML report (opens scripts/perf-report.html, then load .perf/history.jsonl)
make perf-report
```

`make perf` produces `/tmp/perf.json` and prints a one-line summary per scenario. When
`ANALYZE=1` is set it then invokes `scripts/analyze-perf.ts`, which compares the fresh
report to the last entry in `.perf/history.jsonl`, prints a table with Δp50/Δp99/Δops/Δrss
columns, and appends to history **only when at least one tracked metric moved by more
than 0.1 %** (see "History append gating" below). `make perf-analyze` is the same
analyze-only tail, exposed separately for re-analyzing a file without rerunning the
harness.

### Interpreting the output

The printed table uses the last recorded history entry as the baseline:

```
Scenario                             p50       p99    throughput       rss      Δp50      Δp99      Δops      Δrss
steady_state_extractor            1.45ms    1.82ms       688.9/s     512KB     -2.1%     -3.0%     +1.4%      0.0%
```

- **Δp50 / Δp99**: percentage change in latency vs the previous run. Warnings fire when
  these exceed the thresholds in `.perf/config.json` (`p50`, `p99`).
- **Δops**: percentage change in throughput. Warnings fire on a _decrease_ below
  `-thresholds.throughput` (i.e. getting slower).
- **Δrss**: percentage change in peak RSS delta. Warnings fire above
  `thresholds.peakRssDeltaKb`.

A first run prints "First run recorded - no comparison available." and establishes the
baseline.

### History append gating

`scripts/analyze-perf.ts` does not append unconditionally. It diffs the fresh report
against the last entry in `.perf/history.jsonl` across a whitelist of tracked metrics
(`p50_us`, `p90_us`, `p99_us`, `max_us`, `throughput_ops_per_sec`, `peak_rss_delta_kb`).
If every tracked metric on every scenario is within ±0.1 % of the previous entry, the
file is left untouched and the CLI prints `All tracked metrics within ±0.1% of the
previous run; history not updated.` When something moves, the append happens and the
output names the scenario/metric that tripped the threshold.

This matters for the CI commit step: because `history.jsonl` is modified only on
material movement, the `git diff --cached --quiet || git commit` check becomes an
effective "commit only if something changed" — pushes with steady-state numbers no
longer produce noisy chore commits on `main`.

The threshold is hard-coded at `HISTORY_APPEND_THRESHOLD_PCT = 0.1` in
`scripts/analyze-perf.ts`. Adjust there if it proves too tight or too loose.
Scenario additions/removals are treated as unconditionally material (always appended).
Structural zero-valued metrics (e.g. `peak_rss_delta_kb = 0`) are handled explicitly —
`0 → 0` is unchanged, `0 → anything` or `anything → 0` triggers an append.

### CI contract

- `.github/workflows/ci.yml` has a `ci-perf` job that runs on every push to `main`.
- It builds the harness in release mode, runs `make perf ANALYZE=1` (which produces
  the report, prints the delta table, and appends to history only on material
  movement), uploads `/tmp/perf.json` as an artefact, and commits the updated
  `.perf/history.jsonl` back to `main` with `[skip ci]` in the commit message.
- The commit step is a no-op when nothing moved — `history.jsonl` is unmodified, so
  `git diff --cached --quiet` is true.
- The job **never fails on regressions**. Warnings are visible in the job log; acting on
  them is a human decision.

### File locations

```
benches/js-runtime-perf/Cargo.toml               # Workspace member, depends on `retrack` + `retrack-types`
benches/js-runtime-perf/src/main.rs              # CLI driver
benches/js-runtime-perf/src/measure.rs           # hdrhistogram recorder, peak RSS probe
benches/js-runtime-perf/src/report.rs            # JSON output shape (camelCase top-level)
benches/js-runtime-perf/src/scenarios/*.rs       # One scenario per file
benches/js-runtime-perf/scripts/*.js             # JS fixtures loaded via `include_str!`
src/lib.rs                                       # Minimal library target exposing `js_runtime` + `config`
.perf/config.json                                # Scenario list + warning thresholds
.perf/history.jsonl                              # Append-only history (one JSON per run)
scripts/analyze-perf.ts                          # Node 22 analyzer (reads /tmp/perf.json)
scripts/perf-report.html                         # Standalone HTML viewer for history.jsonl
```

### Tuning

- To relax or tighten warnings, edit `.perf/config.json`. Values are percentages.
- To add a scenario: create a module under `benches/js-runtime-perf/src/scenarios/`,
  register it in `scenarios.rs` (both the `ALL` slice and the `run` dispatcher), and add
  its name to `.perf/config.json`.
- Benchmark results are platform-sensitive. History entries include `env.os`, `env.arch`,
  and `env.cpuModel` for this reason; absolute numbers from a laptop are not directly
  comparable to those from a CI runner.

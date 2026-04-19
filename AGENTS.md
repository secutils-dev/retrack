# AGENTS.md

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

# Retrack JS runtime performance

Run commands from the Retrack root.
The harness in `benches/js-runtime-perf/` links the real `retrack::js_runtime::JsRuntime`.
Read `src/js_runtime.rs` and scenario source before interpreting results.
The runtime dispatches round-robin to multiple long-lived workers, with a fresh V8 isolate per script.
The concurrency scenario measures that current implementation, rather than a historical single-worker bottleneck.

## Run and compare

Use `make help` or harness CLI help for current defaults and flags.

```bash
make perf
make perf ANALYZE=1 PERF_ITERATIONS=20 PERF_WARMUP=5
make perf ANALYZE=1 PERF_SCENARIOS=steady_state_extractor
make perf PERF_OUTPUT=/tmp/perf-baseline.json
make perf-analyze
make perf-report
```

`make perf` produces the report without history changes.
`ANALYZE=1` compares against `.perf/history.jsonl` and can append a material change.
When comparison/history recording is intended, use `ANALYZE=1`.
Regressions are advisory. Failed benchmark or analyzer execution still requires investigation.
Compare platform-matched results through `env.os`, `env.arch`, and `env.cpuModel`.
Positive latency/RSS deltas and negative throughput deltas indicate regressions.

## Scenarios and history

Read `benches/js-runtime-perf/src/scenarios.rs` for the scenario inventory and dispatcher.
Read scenario source for workloads, heap limits, and execution budgets.
For a new scenario, add its module, dispatcher case, `ALL` entry, and `.perf/config.json` entry.
Completion requires the new scenario to run and produce the expected report fields.

Read `.perf/config.json` for advisory warning thresholds.
Read `scripts/analyze-perf.ts` for tracked metrics and `HISTORY_APPEND_THRESHOLD_PCT`.
The analyzer appends only on material movement, scenario changes, or zero/nonzero transitions.
Keep those definitions in the source rather than a second version inventory here.

CI uploads reports and can commit material history changes with `[skip ci]`.
Read `.github/workflows/ci.yml` for that job's actual gate and commit behavior.
Completion requires a reviewed report and an accounted-for history diff if analysis modifies history.

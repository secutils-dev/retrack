//! Scenario catalogue for the Retrack harness. Every scenario returns a
//! [`ScenarioResult`]; the driver collects them into a single JSON report.
//!
//! Scenarios intentionally use the real [`retrack::js_runtime::JsRuntime`]
//! entry points so any change to that module (e.g. introducing a worker
//! pool, sharing V8 isolates, or adding a startup snapshot) is reflected in
//! the measured numbers.

mod cold_start;
mod common;
mod concurrent_extractors;
mod steady_state_extractor;
mod steady_state_trivial;

use crate::measure::ScenarioResult;

/// Canonical ordering for scenarios, mirrored in `.perf/config.json`.
pub const ALL: &[&str] = &[
    "cold_start_trivial",
    "steady_state_trivial",
    "steady_state_extractor",
    "concurrent_extractors_8x",
];

pub async fn run(
    name: &str,
    iterations: u64,
    warmup: u64,
    concurrency: u64,
) -> anyhow::Result<ScenarioResult> {
    match name {
        "cold_start_trivial" => cold_start::run(iterations, warmup).await,
        "steady_state_trivial" => steady_state_trivial::run(iterations, warmup).await,
        "steady_state_extractor" => steady_state_extractor::run(iterations, warmup).await,
        "concurrent_extractors_8x" => {
            concurrent_extractors::run(iterations, warmup, concurrency).await
        }
        other => anyhow::bail!("unknown scenario `{other}`"),
    }
}

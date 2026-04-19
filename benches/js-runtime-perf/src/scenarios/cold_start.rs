//! `cold_start_trivial`: rebuilds the [`JsRuntime`] from scratch every
//! iteration, so each sample includes:
//!
//! - `deno_core::JsRuntime::init_platform` bookkeeping (idempotent but still
//!   visited)
//! - A dedicated OS thread spawn for the worker
//! - A fresh `tokio::runtime::Builder::new_current_thread` + `LocalSet`
//! - A fresh V8 isolate (no snapshot)
//! - Watchdog thread spawn + heap-limit callback registration
//!
//! Any future improvement that caches the isolate or ships a V8 startup
//! snapshot should produce a large drop in this scenario's p50/p99.

use crate::{
    measure::{Recorder, ScenarioResult, now},
    scenarios::common::{TRIVIAL_JS, extractor_args, runtime_config, script_config},
};
use retrack::js_runtime::JsRuntime;

pub async fn run(iterations: u64, warmup: u64) -> anyhow::Result<ScenarioResult> {
    for _ in 0..warmup {
        execute_once().await?;
    }

    let mut recorder = Recorder::new(iterations, warmup)?;
    for _ in 0..iterations {
        let start = now();
        execute_once().await?;
        recorder.observe(start.elapsed())?;
    }

    Ok(recorder.finalise())
}

async fn execute_once() -> anyhow::Result<()> {
    let runtime = JsRuntime::init_platform(&runtime_config())?;
    runtime
        .execute_script(TRIVIAL_JS, extractor_args(), script_config())
        .await?;
    Ok(())
}

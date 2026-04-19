//! `steady_state_trivial`: builds the [`JsRuntime`] once and sends a trivial
//! extractor script through the mpsc channel for every measured iteration.
//! The startup and thread-spawn costs are amortised; the per-iteration
//! number reflects the raw cost of creating a fresh V8 isolate per task on
//! the worker thread plus the Rust ↔ JS plumbing.

use crate::{
    measure::{Recorder, ScenarioResult, now},
    scenarios::common::{TRIVIAL_JS, extractor_args, runtime_config, script_config},
};
use retrack::js_runtime::JsRuntime;

pub async fn run(iterations: u64, warmup: u64) -> anyhow::Result<ScenarioResult> {
    let runtime = JsRuntime::init_platform(&runtime_config())?;

    for _ in 0..warmup {
        runtime
            .execute_script(TRIVIAL_JS, extractor_args(), script_config())
            .await?;
    }

    let mut recorder = Recorder::new(iterations, warmup)?;
    for _ in 0..iterations {
        let start = now();
        runtime
            .execute_script(TRIVIAL_JS, extractor_args(), script_config())
            .await?;
        recorder.observe(start.elapsed())?;
    }

    Ok(recorder.finalise())
}

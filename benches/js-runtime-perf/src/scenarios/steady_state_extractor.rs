//! `steady_state_extractor`: builds the [`JsRuntime`] once and runs a
//! realistic extractor script that parses a JSON response body, filters its
//! items, and re-encodes the result. This is the most production-like path
//! and is the best signal when evaluating extractor-specific changes (e.g.
//! sharing prototypes, reusing isolates, or caching compiled scripts).

use crate::{
    measure::{Recorder, ScenarioResult, now},
    scenarios::common::{EXTRACTOR_JS, extractor_args, runtime_config, script_config},
};
use retrack::js_runtime::JsRuntime;

pub async fn run(iterations: u64, warmup: u64) -> anyhow::Result<ScenarioResult> {
    let runtime = JsRuntime::init_platform(&runtime_config())?;

    for _ in 0..warmup {
        runtime
            .execute_script(EXTRACTOR_JS, extractor_args(), script_config())
            .await?;
    }

    let mut recorder = Recorder::new(iterations, warmup)?;
    for _ in 0..iterations {
        let start = now();
        runtime
            .execute_script(EXTRACTOR_JS, extractor_args(), script_config())
            .await?;
        recorder.observe(start.elapsed())?;
    }

    Ok(recorder.finalise())
}

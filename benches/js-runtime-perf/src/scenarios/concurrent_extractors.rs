//! `concurrent_extractors_8x`: shares a single [`JsRuntime`] across
//! `concurrency` concurrent tasks to approximate a burst of tracker runs.
//!
//! Because today's design exposes the runtime as a single mpsc channel into
//! one worker thread, every task is effectively serialised: the recorded
//! per-task latency should grow roughly linearly with `concurrency`. Any
//! future worker pool / multi-isolate change should compress p50/p99 here
//! without changing the serial scenarios.

use crate::{
    measure::{Recorder, ScenarioResult, now},
    scenarios::common::{EXTRACTOR_JS, extractor_args, runtime_config, script_config},
};
use anyhow::Context;
use futures::future::try_join_all;
use retrack::js_runtime::JsRuntime;
use std::{sync::Arc, time::Duration};

pub async fn run(iterations: u64, warmup: u64, concurrency: u64) -> anyhow::Result<ScenarioResult> {
    assert!(concurrency >= 1, "concurrency must be ≥ 1");

    let runtime = Arc::new(JsRuntime::init_platform(&runtime_config())?);

    for _ in 0..warmup {
        execute_batch(&runtime, concurrency).await?;
    }

    // `iterations` is the number of individual executions we want to measure,
    // split into batches of `concurrency`. Round up so we never miss samples.
    let batches = iterations.div_ceil(concurrency);
    let total = batches * concurrency;
    let mut recorder = Recorder::new(total, warmup)?;

    for _ in 0..batches {
        let durations = execute_batch(&runtime, concurrency).await?;
        for duration in durations {
            recorder.observe(duration)?;
        }
    }

    Ok(recorder.finalise())
}

async fn execute_batch(
    runtime: &Arc<JsRuntime>,
    concurrency: u64,
) -> anyhow::Result<Vec<Duration>> {
    let handles: Vec<_> = (0..concurrency)
        .map(|_| {
            let runtime = Arc::clone(runtime);
            tokio::spawn(async move {
                let start = now();
                runtime
                    .execute_script(EXTRACTOR_JS, extractor_args(), script_config())
                    .await?;
                Ok::<_, anyhow::Error>(start.elapsed())
            })
        })
        .collect();

    let results = try_join_all(handles)
        .await
        .context("concurrent script task panicked")?;

    results.into_iter().collect()
}

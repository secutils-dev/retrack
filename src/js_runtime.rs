mod script;
mod script_config;
mod script_execution_status;
mod script_task;

use self::script_execution_status::ScriptExecutionStatus;
pub use self::{
    script::{Script, ScriptBuilder},
    script_config::ScriptConfig,
    script_task::ScriptTask,
};
use crate::{config::JsRuntimeConfig, js_runtime::script::ScriptDefinition};
use anyhow::{anyhow, Context};
use deno_core::{serde_v8, v8, Extension, RuntimeOptions};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    runtime::Builder,
    sync::{mpsc, oneshot},
    task::LocalSet,
};
use tracing::error;

/// Defines a maximum interval on which script is checked for timeout.
const SCRIPT_TIMEOUT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Defines the name of the global variable available to the scripts that stores script arguments.
const SCRIPT_CONTEXT_KEY: &str = "context";

/// A list of Deno Core operations that aren't available to user scripts.
const SCRIPT_EXCLUDED_OPS: [&str; 6] = [
    "op_resources",
    "op_shutdown",
    "op_panic",
    "op_import_sync",
    "op_lazy_load_esm",
    "op_eval_context",
];

/// An abstraction over the V8/Deno runtime that allows any utilities to execute custom user
/// JavaScript scripts.
pub struct JsRuntime {
    tx: mpsc::Sender<ScriptTask>,
}

impl JsRuntime {
    /// Initializes the JS runtime platform, should be called only once and in the main thread.
    pub fn init_platform(config: &JsRuntimeConfig) -> anyhow::Result<Self> {
        deno_core::JsRuntime::init_platform(None, false);

        // JsRuntime will be initialized in the dedicated thread.
        let (tx, mut rx) = mpsc::channel::<ScriptTask>(config.channel_buffer_size);
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .context("Unable to initialize JS runtime worker thread.")?;
        std::thread::spawn(move || {
            let local = LocalSet::new();
            local.spawn_local(async move {
                while let Some(task) = rx.recv().await {
                    match task.script {
                        Script::ApiTargetConfigurator(def) => {
                            JsRuntime::handle_script(&task.config, def).await;
                        }
                        Script::ApiTargetExtractor(def) => {
                            JsRuntime::handle_script(&task.config, def).await;
                        }
                        Script::Custom(def) => {
                            JsRuntime::handle_script(&task.config, def).await;
                        }
                    }
                }
            });
            rt.block_on(local);
        });

        Ok(Self { tx })
    }

    /// Executes a user script and returns the result.
    pub async fn execute_script<ScriptArgs, ScriptResult>(
        &self,
        script_src: impl Into<String>,
        script_args: impl ScriptBuilder<ScriptArgs, ScriptResult>,
        script_config: ScriptConfig,
    ) -> Result<Option<ScriptResult>, anyhow::Error> {
        let (script_result_tx, script_result_rx) = oneshot::channel();

        self.tx
            .send(ScriptTask {
                config: script_config,
                script: script_args.build(script_src, script_result_tx).0,
            })
            .await
            .context("Failed to schedule script execute task")?;

        script_result_rx
            .await
            .context("Failed to receive script execute task result")?
    }

    /// Executes a user script and sends result over oneshot channel. This method doesn't fail, and
    /// only logs error if sending result over channel fails.
    async fn handle_script<ScriptArgs: Serialize, ScriptResult: DeserializeOwned>(
        config: &ScriptConfig,
        script: ScriptDefinition<ScriptArgs, ScriptResult>,
    ) {
        let execute_result = JsRuntime::execute_script_internal(config, &script).await;
        if script.result.send(execute_result).is_err() {
            error!("Failed to send script result.");
        }
    }

    async fn execute_script_internal<ScriptArgs: Serialize, ScriptResult: DeserializeOwned>(
        config: &ScriptConfig,
        script: &ScriptDefinition<ScriptArgs, ScriptResult>,
    ) -> Result<Option<ScriptResult>, anyhow::Error> {
        let mut runtime = deno_core::JsRuntime::new(RuntimeOptions {
            create_params: Some(
                v8::Isolate::create_params().heap_limits(1_048_576, config.max_heap_size),
            ),
            // Disable certain built-in operations.
            extensions: vec![Extension {
                name: "retrack_ext",
                middleware_fn: Some(Box::new(|op| {
                    if SCRIPT_EXCLUDED_OPS.contains(&op.name) {
                        op.disable()
                    } else {
                        op
                    }
                })),
                ..Default::default()
            }],
            ..Default::default()
        });

        let script_status = Arc::new(AtomicUsize::new(ScriptExecutionStatus::Running as usize));

        // Track memory usage and terminate execution if threshold is exceeded.
        let script_status_clone = script_status.clone();
        let isolate_handle = runtime.v8_isolate().thread_safe_handle();
        runtime.add_near_heap_limit_callback(move |current_value, _| {
            error!("Approaching the memory limit of ({current_value}), terminating execution.");

            // Define termination reason and terminate execution.
            isolate_handle.terminate_execution();

            script_status_clone.store(
                ScriptExecutionStatus::ReachedMemoryLimit as usize,
                Ordering::Relaxed,
            );

            // Give the runtime enough heap to terminate without crashing the process.
            5 * current_value
        });

        // Set script args as a global variable, if provided.
        if let Some(ref args) = script.args {
            Self::set_script_args(&mut runtime, args)?;
        }

        // Track the time the script takes to execute, and terminate execution if threshold is exceeded.
        let max_script_execution_time = config.max_execution_time;
        let isolate_handle = runtime.v8_isolate().thread_safe_handle();
        let script_status_clone = script_status.clone();
        std::thread::spawn(move || {
            let now = Instant::now();
            loop {
                // If script is no longer running, return immediately.
                let script_status = script_status_clone.load(Ordering::Relaxed);
                if ScriptExecutionStatus::from(script_status) != ScriptExecutionStatus::Running {
                    return;
                }

                // Otherwise, terminate execution if time is out, or sleep for max `SCRIPT_TIMEOUT_CHECK_INTERVAL`.
                let Some(time_left) = max_script_execution_time.checked_sub(now.elapsed()) else {
                    script_status_clone.store(
                        ScriptExecutionStatus::ReachedTimeLimit as usize,
                        Ordering::Relaxed,
                    );
                    isolate_handle.terminate_execution();
                    return;
                };

                std::thread::sleep(std::cmp::min(time_left, SCRIPT_TIMEOUT_CHECK_INTERVAL));
            }
        });

        let handle_error = |err: deno_core::error::CoreError| match ScriptExecutionStatus::from(
            script_status.load(Ordering::Relaxed),
        ) {
            ScriptExecutionStatus::ReachedMemoryLimit => {
                anyhow!(err).context("Script exceeded memory limit.")
            }
            ScriptExecutionStatus::ReachedTimeLimit => {
                anyhow!(err).context("Script exceeded time limit.")
            }
            ScriptExecutionStatus::Running => {
                script_status.store(
                    ScriptExecutionStatus::ExecutionCompleted as usize,
                    Ordering::Relaxed,
                );
                anyhow!(err).context("Script was running.")
            }
            ScriptExecutionStatus::ExecutionCompleted => {
                anyhow!(err).context("Script execution completed.")
            }
        };

        // Retrieve the result `Promise`.
        let script_src = script.src.trim();
        let script_result_promise = runtime
            .execute_script("<anon>", script_src.to_string())
            .map(|script_result| runtime.resolve(script_result))
            .map_err(handle_error)?;

        // Wait for the promise to resolve.
        let script_result = runtime
            .with_event_loop_promise(script_result_promise, Default::default())
            .await
            .map_err(handle_error)?;

        // Abort termination thread, if script managed to complete.
        script_status.store(
            ScriptExecutionStatus::ExecutionCompleted as usize,
            Ordering::Relaxed,
        );

        let scope = &mut runtime.handle_scope();
        let local = v8::Local::new(scope, script_result);
        serde_v8::from_v8(scope, local).context("Error deserializing script result")
    }

    fn set_script_args<ScriptArgs: Serialize>(
        runtime: &mut deno_core::JsRuntime,
        args: ScriptArgs,
    ) -> anyhow::Result<()> {
        let scope = &mut runtime.handle_scope();
        let context = scope.get_current_context();
        let context_scope = &mut v8::ContextScope::new(scope, context);

        let script_context_key = v8::String::new(context_scope, SCRIPT_CONTEXT_KEY)
            .expect("Cannot create script context key.");
        let script_context_value = serde_v8::to_v8(context_scope, args)
            .context("Cannot serialize script context value")?;
        context.global(context_scope).set(
            context_scope,
            script_context_key.into(),
            script_context_value,
        );

        Ok(())
    }
}
#[cfg(test)]
pub mod tests {
    use super::{JsRuntime, ScriptConfig};
    use crate::config::JsRuntimeConfig;
    use deno_core::error::CoreError;
    use http::{header::CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, Method};
    use retrack_types::trackers::{
        ConfiguratorScriptArgs, ConfiguratorScriptRequest, ConfiguratorScriptResult,
        ExtractorScriptArgs, ExtractorScriptResult, TrackerDataValue,
    };
    use serde::{Deserialize, Serialize};
    use serde_bytes::ByteBuf;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_execute_scripts() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        #[derive(Deserialize, Serialize, Debug, PartialEq, Eq, Clone)]
        struct ScriptParams {
            arg_num: usize,
            arg_str: String,
            arg_array: Vec<String>,
            arg_buf: Vec<u8>,
        }
        let script_params = ScriptParams {
            arg_num: 115,
            arg_str: "Hello, world!".to_string(),
            arg_array: vec!["one".to_string(), "two".to_string()],
            arg_buf: vec![1, 2, 3],
        };

        // Can access script context.
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"(() => {{ return context; }})();"#,
                Some(ByteBuf::from(serde_json::to_vec(&script_params)?)),
                config,
            )
            .await?
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<ScriptParams>(&result)?,
            script_params.clone()
        );

        // Supports known scripts.
        let result = js_runtime
            .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                r#"(() => {{ return { requests: [{ url: "https://retrack.dev/x-url", method: "POST", mediaType: "application/json", headers: { "x-key": "x-value" }, body: Deno.core.encode(JSON.stringify({ ...context, requests: [{...context.requests[0], body: JSON.parse(Deno.core.decode(context.requests[0].body))}] })) }] }; } })();"#,
                ConfiguratorScriptArgs {
                    tags: vec!["tag1".to_string(), "tag2".to_string()],
                    previous_content: Some(TrackerDataValue::new(json!({ "key": "content" }))),
                    requests: vec![ConfiguratorScriptRequest {
                        url: "https://retrack.dev".parse()?,
                        method: Some(Method::PUT),
                        headers: Some(
                            (&[
                                (CONTENT_TYPE, "application/json".to_string()),
                            ]
                                .into_iter()
                                .collect::<HashMap<_, _>>())
                                .try_into()?,
                        ),
                        body: Some(serde_json::to_vec(&json!({ "key": "body" }))?),
                        media_type: Some("text/plain; charset=UTF-8".parse()?),
                    }],
                },
                config,
            )
            .await?
            .unwrap();
        assert_eq!(
            result,
            ConfiguratorScriptResult::Requests(vec![ConfiguratorScriptRequest {
                url: "https://retrack.dev/x-url".parse()?,
                method: Some(Method::POST),
                headers: Some(HeaderMap::from_iter([(
                    HeaderName::from_static("x-key"),
                    HeaderValue::from_static("x-value")
                )])),
                body: Some(serde_json::to_vec(&json!({
                    "tags": ["tag1", "tag2"],
                    "previousContent": { "original": { "key": "content" } },
                    "requests": [{ "url": "https://retrack.dev/", "method": "PUT", "headers": { "content-type": "application/json" }, "mediaType": "text/plain; charset=UTF-8", "body": { "key": "body" } }]
                }))?),
                media_type: Some("application/json".parse()?),
            }])
        );

        // Can do basic math.
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"(() => {{
                  return Deno.core.encode(
                    JSON.stringify(
                      JSON.parse(Deno.core.decode(context)).arg_num * 2
                    )
                  );
                }})(context);"#,
                Some(ByteBuf::from(serde_json::to_vec(&script_params)?)),
                config,
            )
            .await?
            .unwrap();
        assert_eq!(Some(serde_json::from_slice::<usize>(&result)?), Some(230));

        // Returns error from scripts
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"(() => {{ throw new Error("Uh oh."); }})();"#,
                None,
                config,
            )
            .await
            .unwrap_err()
            .downcast::<CoreError>()?;
        if let CoreError::Js(js_err) = result {
            assert_eq!(
                js_err.exception_message,
                "Uncaught Error: Uh oh.".to_string()
            );
        } else {
            panic!("Expected CoreError::Js, got: {:?}", result);
        }

        // Can access script context (async).
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"(async () => {{ return new Promise((resolve) => resolve(context)); }})();"#,
                Some(ByteBuf::from(serde_json::to_vec(&script_params)?)),
                config,
            )
            .await?
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<ScriptParams>(&result)?,
            script_params
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_execute_api_target_scripts() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        // Supports extractor scripts.
        let ExtractorScriptResult { body, ..} = js_runtime
            .execute_script::<ExtractorScriptArgs, ExtractorScriptResult>(
                r#"(() => {{ return { body: Deno.core.encode(Deno.core.decode(new Uint8Array(context.responses[0]))) }; }})();"#,
                ExtractorScriptArgs {
                    responses: Some(vec![serde_json::to_vec(&json!({ "key": "value" }))?]),
                    ..Default::default()
                },
                config,
            )
            .await?
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body.unwrap())?,
            json!({ "key": "value" })
        );

        // Supports configurator (overrides request) scripts.
        let ConfiguratorScriptResult::Requests(requests) = js_runtime
            .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                r#"(() => {{ return { requests: [{ url: "https://retrack.dev/one", body: Deno.core.encode(JSON.stringify({ key: "value" })) }, { url: "https://retrack.dev/two", body: Deno.core.encode(JSON.stringify({ key: "value_2" })) }] }; }})();"#,
                ConfiguratorScriptArgs::default(),
                config,
            )
            .await?
            .unwrap() else {
            panic!("Expected ConfiguratorScriptResult::Request");
        };
        assert_eq!(
            requests,
            vec![
                ConfiguratorScriptRequest {
                    url: "https://retrack.dev/one".parse()?,
                    method: None,
                    headers: None,
                    media_type: None,
                    body: Some(serde_json::to_vec(&json!({ "key": "value" }))?),
                },
                ConfiguratorScriptRequest {
                    url: "https://retrack.dev/two".parse()?,
                    method: None,
                    headers: None,
                    media_type: None,
                    body: Some(serde_json::to_vec(&json!({ "key": "value_2" }))?),
                }
            ]
        );

        // Supports configurator (overrides response) scripts.
        let ConfiguratorScriptResult::Response { body, ..} = js_runtime
            .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                r#"(() => {{ return { response: { body: Deno.core.encode(JSON.stringify({ key: "value" })) } }; }})();"#,
                ConfiguratorScriptArgs::default(),
                config,
            )
            .await?
            .unwrap() else {
            panic!("Expected ConfiguratorScriptResult::Response");
        };
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)?,
            json!({ "key": "value" })
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_limit_execution_time() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        // Limit execution time (async).
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"
        (async () => {{
            return new Promise((resolve) => {
                Deno.core.queueUserTimer(
                    Deno.core.getTimerDepth() + 1,
                    false,
                    10 * 1000,
                    () => resolve(Deno.core.encode("Done"))
                );
            });
        }})();
        "#,
                None,
                config,
            )
            .await
            .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded time limit.".to_string()
        );

        // Limit execution time (sync).
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"
        (() => {{
            while (true) {}
        }})();
        "#,
                None,
                config,
            )
            .await
            .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded time limit.".to_string()
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_limit_execution_memory() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        // Limit memory usage.
        let result = js_runtime
            .execute_script::<ByteBuf, ByteBuf>(
                r#"
        (() => {{
           let s = "";
           while(true) { s += "Hello"; }
           return "Done";
        }})();
        "#,
                None,
                config,
            )
            .await
            .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded memory limit.".to_string()
        );

        Ok(())
    }
}

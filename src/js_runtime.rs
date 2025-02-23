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
                        Script::ActionFormatter(def) => {
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
                anyhow!(err)
            }
            ScriptExecutionStatus::ExecutionCompleted => {
                anyhow!(err)
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
    use insta::assert_json_snapshot;
    use retrack_types::trackers::{
        ConfiguratorScriptArgs, ConfiguratorScriptRequest, ConfiguratorScriptResult,
        ExtractorScriptArgs, ExtractorScriptResult, FormatterScriptArgs, FormatterScriptResult,
        TrackerDataValue,
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

        // Supports _empty_ configurator scripts.
        let configurator_result = js_runtime
            .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                r#"(() => new Promise((resolve) => Deno.core.queueUserTimer(Deno.core.getTimerDepth() + 1, false, 1000, resolve)))();"#,
                ConfiguratorScriptArgs::default(),
                config,
            )
            .await?;
        assert!(configurator_result.is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_execute_action_formatter_script() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        // Supports formatter scripts.
        let FormatterScriptResult { content } = js_runtime
            .execute_script::<FormatterScriptArgs, FormatterScriptResult>(
                r#"(() => {{ return { content: { key: `${context.newContent.key}_${context.previousContent.key}_result_${context.action}` } }; }})();"#,
                FormatterScriptArgs {
                    action: "log",
                    previous_content: Some(json!({ "key": "old-value" })),
                    new_content: json!({ "key": "value" }),
                },
                config,
            )
            .await?
            .unwrap();
        assert_eq!(
            content,
            Some(json!({ "key": "value_old-value_result_log" }))
        );

        // Supports formatter scripts returning empty value.
        let FormatterScriptResult { content } = js_runtime
            .execute_script::<FormatterScriptArgs, FormatterScriptResult>(
                r#"(() => {{ return {}; }})();"#,
                FormatterScriptArgs {
                    action: "log",
                    previous_content: None,
                    new_content: json!({ "key": "value" }),
                },
                config,
            )
            .await?
            .unwrap();
        assert!(content.is_none());

        // Supports formatter scripts that don't return anything.
        let result = js_runtime
            .execute_script::<FormatterScriptArgs, FormatterScriptResult>(
                r#"(() => {{ return; }})();"#,
                FormatterScriptArgs {
                    action: "log",
                    previous_content: None,
                    new_content: json!({ "key": "value" }),
                },
                config,
            )
            .await?;
        assert!(result.is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_access_deno_apis() -> anyhow::Result<()> {
        let js_runtime = JsRuntime::init_platform(&JsRuntimeConfig::default())?;
        let config = ScriptConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_execution_time: std::time::Duration::from_secs(5),
        };

        let ConfiguratorScriptResult::Response { body, .. } = js_runtime
            .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                r#"(() => {{
                    function collectMembers(prefix = '', obj, depth = Infinity) {
                        if (depth < 0) {
                            return [];
                        }

                        return Object.entries(obj).map(([key, value]) => {
                            const type = typeof value;
                            if (type === 'function') {
                                return `${prefix}${key}()`;
                            }

                            if (value != null && type === 'object' && key !== 'globalThis') {
                                return collectMembers(`${prefix}${key}.`, value, depth - 1);
                            }

                            return `${prefix}${key}=${type !== 'symbol' ? value : 'symbol'}`;
                        }).flat();
                    }

                    const members = [
                      ...collectMembers('Deno.', globalThis.Deno),
                      ...collectMembers('', globalThis.__bootstrap.primordials)
                    ];
                    members.sort();

                    return { response: { body: Deno.core.encode(JSON.stringify(members)) } };
                 }})();"#,
                ConfiguratorScriptArgs::default(),
                config,
            )
            .await?
            .unwrap()
        else {
            panic!("Expected ConfiguratorScriptResult::Response");
        };

        assert_json_snapshot!(serde_json::from_slice::<serde_json::Value>(&body)?, @r###"
        [
          "AggregateError()",
          "Array()",
          "ArrayBuffer()",
          "BigInt()",
          "BigInt64Array()",
          "BigUint64Array()",
          "Boolean()",
          "DataView()",
          "Date()",
          "Deno.core.AsyncVariable()",
          "Deno.core.BadResource()",
          "Deno.core.Interrupted()",
          "Deno.core.NotCapable()",
          "Deno.core.abortWasmStreaming()",
          "Deno.core.addMainModuleHandler()",
          "Deno.core.build.arch=unknown",
          "Deno.core.build.env=undefined",
          "Deno.core.build.os=unknown",
          "Deno.core.build.target=unknown",
          "Deno.core.build.vendor=unknown",
          "Deno.core.buildCustomError()",
          "Deno.core.byteLength()",
          "Deno.core.callConsole()",
          "Deno.core.cancelTimer()",
          "Deno.core.close()",
          "Deno.core.console.assert()",
          "Deno.core.console.clear()",
          "Deno.core.console.context()",
          "Deno.core.console.count()",
          "Deno.core.console.countReset()",
          "Deno.core.console.debug()",
          "Deno.core.console.dir()",
          "Deno.core.console.dirxml()",
          "Deno.core.console.error()",
          "Deno.core.console.group()",
          "Deno.core.console.groupCollapsed()",
          "Deno.core.console.groupEnd()",
          "Deno.core.console.info()",
          "Deno.core.console.log()",
          "Deno.core.console.profile()",
          "Deno.core.console.profileEnd()",
          "Deno.core.console.table()",
          "Deno.core.console.time()",
          "Deno.core.console.timeEnd()",
          "Deno.core.console.timeLog()",
          "Deno.core.console.timeStamp()",
          "Deno.core.console.trace()",
          "Deno.core.console.warn()",
          "Deno.core.consoleStringify()",
          "Deno.core.createCancelHandle()",
          "Deno.core.createLazyLoader()",
          "Deno.core.currentUserCallSite()",
          "Deno.core.decode()",
          "Deno.core.deserialize()",
          "Deno.core.destructureError()",
          "Deno.core.encode()",
          "Deno.core.encodeBinaryString()",
          "Deno.core.evalContext()",
          "Deno.core.eventLoopHasMoreWork()",
          "Deno.core.eventLoopTick()",
          "Deno.core.getAllLeakTraces()",
          "Deno.core.getAsyncContext()",
          "Deno.core.getLeakTraceForPromise()",
          "Deno.core.getPromiseDetails()",
          "Deno.core.getProxyDetails()",
          "Deno.core.getTimerDepth()",
          "Deno.core.hasPromise()",
          "Deno.core.hasTickScheduled()",
          "Deno.core.hostObjectBrand=symbol",
          "Deno.core.internalFdSymbol=symbol",
          "Deno.core.internalRidSymbol=symbol",
          "Deno.core.isAnyArrayBuffer()",
          "Deno.core.isArgumentsObject()",
          "Deno.core.isArrayBuffer()",
          "Deno.core.isArrayBufferView()",
          "Deno.core.isAsyncFunction()",
          "Deno.core.isBigIntObject()",
          "Deno.core.isBooleanObject()",
          "Deno.core.isBoxedPrimitive()",
          "Deno.core.isDataView()",
          "Deno.core.isDate()",
          "Deno.core.isGeneratorFunction()",
          "Deno.core.isGeneratorObject()",
          "Deno.core.isLeakTracingEnabled()",
          "Deno.core.isMap()",
          "Deno.core.isMapIterator()",
          "Deno.core.isModuleNamespaceObject()",
          "Deno.core.isNativeError()",
          "Deno.core.isNumberObject()",
          "Deno.core.isPromise()",
          "Deno.core.isProxy()",
          "Deno.core.isRegExp()",
          "Deno.core.isSet()",
          "Deno.core.isSetIterator()",
          "Deno.core.isSharedArrayBuffer()",
          "Deno.core.isStringObject()",
          "Deno.core.isSymbolObject()",
          "Deno.core.isTerminal()",
          "Deno.core.isTypedArray()",
          "Deno.core.isWeakMap()",
          "Deno.core.isWeakSet()",
          "Deno.core.memoryUsage()",
          "Deno.core.opNames()",
          "Deno.core.ops.op_abort_wasm_streaming()",
          "Deno.core.ops.op_add()",
          "Deno.core.ops.op_add_async()",
          "Deno.core.ops.op_add_main_module_handler()",
          "Deno.core.ops.op_cancel_handle()",
          "Deno.core.ops.op_close()",
          "Deno.core.ops.op_current_user_call_site()",
          "Deno.core.ops.op_decode()",
          "Deno.core.ops.op_deserialize()",
          "Deno.core.ops.op_destructure_error()",
          "Deno.core.ops.op_dispatch_exception()",
          "Deno.core.ops.op_encode()",
          "Deno.core.ops.op_encode_binary_string()",
          "Deno.core.ops.op_error_async()",
          "Deno.core.ops.op_error_async_deferred()",
          "Deno.core.ops.op_eval_context()",
          "Deno.core.ops.op_event_loop_has_more_work()",
          "Deno.core.ops.op_format_file_name()",
          "Deno.core.ops.op_get_constructor_name()",
          "Deno.core.ops.op_get_extras_binding_object()",
          "Deno.core.ops.op_get_non_index_property_names()",
          "Deno.core.ops.op_get_promise_details()",
          "Deno.core.ops.op_get_proxy_details()",
          "Deno.core.ops.op_has_tick_scheduled()",
          "Deno.core.ops.op_import_sync()",
          "Deno.core.ops.op_is_any_array_buffer()",
          "Deno.core.ops.op_is_arguments_object()",
          "Deno.core.ops.op_is_array_buffer()",
          "Deno.core.ops.op_is_array_buffer_view()",
          "Deno.core.ops.op_is_async_function()",
          "Deno.core.ops.op_is_big_int_object()",
          "Deno.core.ops.op_is_boolean_object()",
          "Deno.core.ops.op_is_boxed_primitive()",
          "Deno.core.ops.op_is_data_view()",
          "Deno.core.ops.op_is_date()",
          "Deno.core.ops.op_is_generator_function()",
          "Deno.core.ops.op_is_generator_object()",
          "Deno.core.ops.op_is_map()",
          "Deno.core.ops.op_is_map_iterator()",
          "Deno.core.ops.op_is_module_namespace_object()",
          "Deno.core.ops.op_is_native_error()",
          "Deno.core.ops.op_is_number_object()",
          "Deno.core.ops.op_is_promise()",
          "Deno.core.ops.op_is_proxy()",
          "Deno.core.ops.op_is_reg_exp()",
          "Deno.core.ops.op_is_set()",
          "Deno.core.ops.op_is_set_iterator()",
          "Deno.core.ops.op_is_shared_array_buffer()",
          "Deno.core.ops.op_is_string_object()",
          "Deno.core.ops.op_is_symbol_object()",
          "Deno.core.ops.op_is_terminal()",
          "Deno.core.ops.op_is_typed_array()",
          "Deno.core.ops.op_is_weak_map()",
          "Deno.core.ops.op_is_weak_set()",
          "Deno.core.ops.op_lazy_load_esm()",
          "Deno.core.ops.op_leak_tracing_enable()",
          "Deno.core.ops.op_leak_tracing_get()",
          "Deno.core.ops.op_leak_tracing_get_all()",
          "Deno.core.ops.op_leak_tracing_submit()",
          "Deno.core.ops.op_memory_usage()",
          "Deno.core.ops.op_op_names()",
          "Deno.core.ops.op_panic()",
          "Deno.core.ops.op_print()",
          "Deno.core.ops.op_queue_microtask()",
          "Deno.core.ops.op_read()",
          "Deno.core.ops.op_read_all()",
          "Deno.core.ops.op_read_sync()",
          "Deno.core.ops.op_ref_op()",
          "Deno.core.ops.op_resources()",
          "Deno.core.ops.op_run_microtasks()",
          "Deno.core.ops.op_serialize()",
          "Deno.core.ops.op_set_format_exception_callback()",
          "Deno.core.ops.op_set_handled_promise_rejection_handler()",
          "Deno.core.ops.op_set_has_tick_scheduled()",
          "Deno.core.ops.op_set_promise_hooks()",
          "Deno.core.ops.op_set_wasm_streaming_callback()",
          "Deno.core.ops.op_shutdown()",
          "Deno.core.ops.op_str_byte_length()",
          "Deno.core.ops.op_timer_cancel()",
          "Deno.core.ops.op_timer_queue()",
          "Deno.core.ops.op_timer_queue_immediate()",
          "Deno.core.ops.op_timer_queue_system()",
          "Deno.core.ops.op_timer_ref()",
          "Deno.core.ops.op_timer_unref()",
          "Deno.core.ops.op_try_close()",
          "Deno.core.ops.op_unref_op()",
          "Deno.core.ops.op_void_async()",
          "Deno.core.ops.op_void_async_deferred()",
          "Deno.core.ops.op_void_sync()",
          "Deno.core.ops.op_wasm_streaming_feed()",
          "Deno.core.ops.op_wasm_streaming_set_url()",
          "Deno.core.ops.op_write()",
          "Deno.core.ops.op_write_all()",
          "Deno.core.ops.op_write_sync()",
          "Deno.core.ops.op_write_type_error()",
          "Deno.core.print()",
          "Deno.core.promiseIdSymbol=symbol",
          "Deno.core.propGetterOnly()",
          "Deno.core.propNonEnumerable()",
          "Deno.core.propNonEnumerableLazyLoaded()",
          "Deno.core.propReadOnly()",
          "Deno.core.propWritable()",
          "Deno.core.propWritableLazyLoaded()",
          "Deno.core.queueImmediate()",
          "Deno.core.queueSystemTimer()",
          "Deno.core.queueUserTimer()",
          "Deno.core.read()",
          "Deno.core.readAll()",
          "Deno.core.readSync()",
          "Deno.core.refOpPromise()",
          "Deno.core.refTimer()",
          "Deno.core.registerErrorBuilder()",
          "Deno.core.registerErrorClass()",
          "Deno.core.reportUnhandledException()",
          "Deno.core.reportUnhandledPromiseRejection()",
          "Deno.core.resources()",
          "Deno.core.runMicrotasks()",
          "Deno.core.scopeAsyncContext()",
          "Deno.core.serialize()",
          "Deno.core.setAsyncContext()",
          "Deno.core.setBuildInfo()",
          "Deno.core.setHandledPromiseRejectionHandler()",
          "Deno.core.setHasTickScheduled()",
          "Deno.core.setLeakTracingEnabled()",
          "Deno.core.setMacrotaskCallback()",
          "Deno.core.setNextTickCallback()",
          "Deno.core.setPromiseHooks()",
          "Deno.core.setReportExceptionCallback()",
          "Deno.core.setUnhandledPromiseRejectionHandler()",
          "Deno.core.setUpAsyncStub()",
          "Deno.core.setWasmStreamingCallback()",
          "Deno.core.shutdown()",
          "Deno.core.tryClose()",
          "Deno.core.unrefOpPromise()",
          "Deno.core.unrefTimer()",
          "Deno.core.v8Console.assert()",
          "Deno.core.v8Console.clear()",
          "Deno.core.v8Console.context()",
          "Deno.core.v8Console.count()",
          "Deno.core.v8Console.countReset()",
          "Deno.core.v8Console.debug()",
          "Deno.core.v8Console.dir()",
          "Deno.core.v8Console.dirxml()",
          "Deno.core.v8Console.error()",
          "Deno.core.v8Console.group()",
          "Deno.core.v8Console.groupCollapsed()",
          "Deno.core.v8Console.groupEnd()",
          "Deno.core.v8Console.info()",
          "Deno.core.v8Console.log()",
          "Deno.core.v8Console.profile()",
          "Deno.core.v8Console.profileEnd()",
          "Deno.core.v8Console.table()",
          "Deno.core.v8Console.time()",
          "Deno.core.v8Console.timeEnd()",
          "Deno.core.v8Console.timeLog()",
          "Deno.core.v8Console.timeStamp()",
          "Deno.core.v8Console.trace()",
          "Deno.core.v8Console.warn()",
          "Deno.core.wrapConsole()",
          "Deno.core.write()",
          "Deno.core.writeAll()",
          "Deno.core.writeSync()",
          "Deno.core.writeTypeError()",
          "Error()",
          "ErrorStackTraceLimit=10",
          "EvalError()",
          "FinalizationRegistry()",
          "Float32Array()",
          "Float64Array()",
          "Function()",
          "Int16Array()",
          "Int32Array()",
          "Int8Array()",
          "Map()",
          "Number()",
          "Object()",
          "Promise()",
          "Proxy()",
          "RangeError()",
          "ReferenceError()",
          "RegExp()",
          "SafeArrayIterator()",
          "SafeFinalizationRegistry()",
          "SafeMap()",
          "SafeMapIterator()",
          "SafePromiseAll()",
          "SafePromisePrototypeFinally()",
          "SafeRegExp()",
          "SafeSet()",
          "SafeSetIterator()",
          "SafeStringIterator()",
          "SafeWeakMap()",
          "SafeWeakRef()",
          "SafeWeakSet()",
          "Set()",
          "String()",
          "Symbol()",
          "SyntaxError()",
          "TypeError()",
          "TypedArray()",
          "URIError()",
          "Uint16Array()",
          "Uint32Array()",
          "Uint8Array()",
          "Uint8ClampedArray()",
          "WeakMap()",
          "WeakRef()",
          "WeakSet()",
          "applyBind()",
          "decodeURI()",
          "decodeURIComponent()",
          "encodeURI()",
          "encodeURIComponent()",
          "globalThis=[object Object]",
          "indirectEval()",
          "isNaN()",
          "makeSafe()",
          "setQueueMicrotask()",
          "uncurryThis()"
        ]
        "###);

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

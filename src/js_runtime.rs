mod js_runtime_config;
mod script_termination_reason;

pub use self::js_runtime_config::JsRuntimeConfig;
use self::script_termination_reason::ScriptTerminationReason;
use anyhow::{bail, Context};
use deno_core::{serde_v8, v8, RuntimeOptions};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tracing::error;

/// Defines a maximum interval on which script is checked for timeout.
const SCRIPT_TIMEOUT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// An abstraction over the V8/Deno runtime that allows any utilities to execute custom user
/// JavaScript scripts.
pub struct JsRuntime;
impl JsRuntime {
    /// Initializes the JS runtime platform, should be called only once and in the main thread.
    pub fn init_platform() {
        deno_core::JsRuntime::init_platform(None, false);
    }

    /// Executes a user script and returns the result.
    pub fn execute_script_sync<R: for<'de> Deserialize<'de>>(
        config: &JsRuntimeConfig,
        js_code: impl Into<String>,
        js_script_context: Option<impl Serialize>,
    ) -> Result<(R, Duration), anyhow::Error> {
        let now = Instant::now();

        let termination_reason = Arc::new(AtomicUsize::new(
            ScriptTerminationReason::NotTerminated as usize,
        ));
        let timeout_token = Arc::new(AtomicBool::new(false));

        let mut runtime = deno_core::JsRuntime::new(RuntimeOptions {
            create_params: Some(v8::Isolate::create_params().heap_limits(0, config.max_heap_size)),
            ..Default::default()
        });
        let isolate_handle = runtime.v8_isolate().thread_safe_handle();

        // Track memory usage and terminate execution if threshold is exceeded.
        let isolate_handle_clone = isolate_handle.clone();
        let termination_reason_clone = termination_reason.clone();
        let timeout_token_clone = timeout_token.clone();
        runtime.add_near_heap_limit_callback(move |current_value, _| {
            error!("Approaching the memory limit of ({current_value}), terminating execution.");

            // Define termination reason and terminate execution.
            isolate_handle_clone.terminate_execution();

            timeout_token_clone.swap(true, Ordering::Relaxed);
            termination_reason_clone.store(
                ScriptTerminationReason::MemoryLimit as usize,
                Ordering::Relaxed,
            );

            // Give the runtime enough heap to terminate without crashing the process.
            5 * current_value
        });

        // Set script context on a global scope if provided.
        if let Some(script_context) = js_script_context {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let scope = &mut v8::ContextScope::new(scope, context);

            let Some(context_key) = v8::String::new(scope, "context") else {
                bail!("Cannot create script context key.");
            };
            let context_value = serde_v8::to_v8(scope, script_context)
                .with_context(|| "Cannot serialize script context")?;
            context
                .global(scope)
                .set(scope, context_key.into(), context_value);
        }

        // Track the time the script takes to execute, and terminate execution if threshold is exceeded.
        let termination_reason_clone = termination_reason.clone();
        let timeout_token_clone = timeout_token.clone();
        let max_script_execution_time = config.max_script_execution_time;
        std::thread::spawn(move || {
            let now = Instant::now();
            loop {
                // If task is cancelled, return immediately.
                if timeout_token_clone.load(Ordering::Relaxed) {
                    return;
                }

                // Otherwise, terminate execution if time is out, or sleep for max `SCRIPT_TIMEOUT_CHECK_INTERVAL`.
                let Some(time_left) = max_script_execution_time.checked_sub(now.elapsed()) else {
                    termination_reason_clone.store(
                        ScriptTerminationReason::TimeLimit as usize,
                        Ordering::Relaxed,
                    );
                    isolate_handle.terminate_execution();
                    return;
                };

                std::thread::sleep(std::cmp::min(time_left, SCRIPT_TIMEOUT_CHECK_INTERVAL));
            }
        });

        let handle_error = |err: anyhow::Error| match ScriptTerminationReason::from(
            termination_reason.load(Ordering::Relaxed),
        ) {
            ScriptTerminationReason::MemoryLimit => err.context("Script exceeded memory limit."),
            ScriptTerminationReason::TimeLimit => err.context("Script exceeded time limit."),
            ScriptTerminationReason::NotTerminated => err,
        };

        // Retrieve the result.
        let script_result_or_promise =
            runtime
                .execute_script("<anon>", js_code.into())
                .map_err(|err| {
                    timeout_token.swap(true, Ordering::Relaxed);
                    runtime.v8_isolate().cancel_terminate_execution();
                    handle_error(err)
                })?;

        let scope = &mut runtime.handle_scope();
        let local = v8::Local::new(scope, script_result_or_promise);
        let local = if let Ok(promise) = v8::Local::<v8::Promise>::try_from(local) {
            while promise.state() == v8::PromiseState::Pending {
                scope.perform_microtask_checkpoint();

                // Check if script has been terminated in the meantime.
                if termination_reason.load(Ordering::Relaxed)
                    != ScriptTerminationReason::NotTerminated as usize
                {
                    return Err(handle_error(anyhow::anyhow!(
                        "Script returned a promise that timed out."
                    )));
                }
            }
            promise.result(scope)
        } else {
            local
        };

        // Abort termination thread, if script managed to complete.
        timeout_token.swap(true, Ordering::Relaxed);

        serde_v8::from_v8(scope, local)
            .map(|result| (result, now.elapsed()))
            .with_context(|| "Error deserializing script result")
    }
}
#[cfg(test)]
pub mod tests {
    use super::{JsRuntime, JsRuntimeConfig};
    use deno_core::error::JsError;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_execute_scripts() -> anyhow::Result<()> {
        let config = JsRuntimeConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_script_execution_time: std::time::Duration::from_secs(5),
        };

        #[derive(Deserialize, Serialize, Debug, PartialEq, Eq, Clone)]
        struct ScriptContext {
            arg_num: usize,
            arg_str: String,
            arg_array: Vec<String>,
            arg_buf: Vec<u8>,
        }
        let script_context = ScriptContext {
            arg_num: 115,
            arg_str: "Hello, world!".to_string(),
            arg_array: vec!["one".to_string(), "two".to_string()],
            arg_buf: vec![1, 2, 3],
        };

        // Can access script context.
        let (result, _) = JsRuntime::execute_script_sync::<ScriptContext>(
            &config,
            r#"(() => {{ return context; }})();"#,
            Some(script_context.clone()),
        )?;
        assert_eq!(result, script_context);

        // Can do basic math.
        let (result, _) = JsRuntime::execute_script_sync::<usize>(
            &config,
            r#"((context) => {{ return context.arg_num * 2; }})(context);"#,
            Some(script_context.clone()),
        )?;
        assert_eq!(result, 230);

        // Can use Deno APIs.
        let (result, _) = JsRuntime::execute_script_sync::<serde_bytes::ByteBuf>(
            &config,
            r#"((context) => Deno.core.encode(JSON.stringify({ key: 'value' })))(context);"#,
            Some(script_context.clone()),
        )?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&result)?,
            json!({ "key": "value" })
        );

        // Returns error from scripts
        let result = JsRuntime::execute_script_sync::<()>(
            &config,
            r#"(() => {{ throw new Error("Uh oh."); }})();"#,
            None::<()>,
        )
        .unwrap_err()
        .downcast::<JsError>()?;
        assert_eq!(result.exception_message, "Uncaught Error: Uh oh.");

        // Can access script context (async).
        let (result, _) = JsRuntime::execute_script_sync::<ScriptContext>(
            &config,
            r#"(async () => {{ return new Promise((resolve) => resolve(context)); }})();"#,
            Some(script_context.clone()),
        )?;
        assert_eq!(result, script_context);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_limit_execution_time() -> anyhow::Result<()> {
        let config = JsRuntimeConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_script_execution_time: std::time::Duration::from_secs(5),
        };

        // Limit execution time (async).
        let result = JsRuntime::execute_script_sync::<String>(
            &config,
            r#"
        (async () => {{
            return new Promise((resolve) => {
                Deno.core.queueUserTimer(
                    Deno.core.getTimerDepth() + 1,
                    false,
                    10 * 1000,
                    () => resolve("Done")
                );
            });
        }})();
        "#,
            None::<()>,
        )
        .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded time limit.".to_string()
        );

        // Limit execution time (sync).
        let result = JsRuntime::execute_script_sync::<String>(
            &config,
            r#"
        (() => {{
            while (true) {}
        }})();
        "#,
            None::<()>,
        )
        .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded time limit.".to_string()
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn can_limit_execution_memory() -> anyhow::Result<()> {
        let config = JsRuntimeConfig {
            max_heap_size: 10 * 1024 * 1024,
            max_script_execution_time: std::time::Duration::from_secs(5),
        };

        // Limit memory usage.
        let result = JsRuntime::execute_script_sync::<String>(
            &config,
            r#"
        (() => {{
           let s = "";
           while(true) { s += "Hello"; }
           return "Done";
        }})();
        "#,
            None::<()>,
        )
        .unwrap_err();
        assert_eq!(
            format!("{result}"),
            "Script exceeded memory limit.".to_string()
        );

        Ok(())
    }
}

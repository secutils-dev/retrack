use crate::js_runtime::{Script, ScriptConfig};

/// Represents a task to execute JS script in Deno runtime.
pub struct ScriptTask {
    /// A script to execute including arguments and channel to send result back.
    pub script: Script,
    /// A JS runtime config (max execution time, max memory etc.).
    pub config: ScriptConfig,
}

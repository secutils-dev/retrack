use std::time::Duration;

/// Configuration for the JS runtime (Deno) for the specific script.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ScriptConfig {
    /// The hard limit for the JS runtime heap size in bytes.
    pub max_heap_size: usize,
    /// The maximum duration for a single JS script execution.
    pub max_execution_time: Duration,
}

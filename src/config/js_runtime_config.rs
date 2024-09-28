use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSeconds};
use std::time::Duration;

/// Configuration for the embedded JS Runtime (Deno).
#[serde_as]
#[derive(Deserialize, Serialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct JsRuntimeConfig {
    /// The hard limit for the JS runtime heap size in bytes. Defaults to 10485760 bytes or 10 MB.
    pub max_heap_size: usize,
    /// The maximum duration for a single JS script execution. Defaults to 10 seconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub max_script_execution_time: Duration,
}

impl Default for JsRuntimeConfig {
    fn default() -> Self {
        Self {
            max_heap_size: 10_485_760,
            max_script_execution_time: Duration::from_secs(10),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::JsRuntimeConfig;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        let config = JsRuntimeConfig::default();
        assert_toml_snapshot!(config, @r###"
        max_heap_size = 10485760
        max_script_execution_time = 10000
        "###);
    }

    #[test]
    fn deserialization() {
        let config: JsRuntimeConfig = toml::from_str(
            r#"
        max_heap_size = 10485760
        max_script_execution_time = 10000
    "#,
        )
        .unwrap();
        assert_eq!(config, JsRuntimeConfig::default());
    }
}

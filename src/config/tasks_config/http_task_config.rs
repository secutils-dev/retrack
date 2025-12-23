use crate::config::TaskRetryStrategy;
use serde::{Deserialize, Serialize};

/// Configuration for the HTTP task.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct HttpTaskConfig {
    /// The retry strategy for the HTTP task.
    pub retry_strategy: TaskRetryStrategy,
}

impl Default for HttpTaskConfig {
    fn default() -> Self {
        Self {
            retry_strategy: TaskRetryStrategy::Exponential {
                initial_interval: std::time::Duration::from_secs(60),
                max_interval: std::time::Duration::from_secs(600),
                multiplier: 2,
                max_attempts: 3,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HttpTaskConfig;
    use crate::config::TaskRetryStrategy;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        assert_toml_snapshot!(HttpTaskConfig::default(), @"retry_strategy = { type = 'exponential', initial_interval = 60000, multiplier = 2, max_interval = 600000, max_attempts = 3 }");
    }

    #[test]
    fn deserialization() {
        let config: HttpTaskConfig = toml::from_str(
            r#"
        [retry_strategy]
        type = 'exponential'
        initial_interval = 60000
        multiplier = 2
        max_interval = 600000
        max_attempts = 3
    "#,
        )
        .unwrap();
        assert_eq!(config, HttpTaskConfig::default());

        let config: HttpTaskConfig = toml::from_str(
            r#"
        [retry_strategy]
        type = 'constant'
        interval = 30000
        max_attempts = 3
    "#,
        )
        .unwrap();
        assert_eq!(
            config,
            HttpTaskConfig {
                retry_strategy: TaskRetryStrategy::Constant {
                    interval: std::time::Duration::from_secs(30),
                    max_attempts: 3,
                },
            }
        );
    }
}

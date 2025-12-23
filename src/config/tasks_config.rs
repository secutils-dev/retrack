use serde::{Deserialize, Serialize};

mod email_task_config;
mod http_task_config;
mod task_retry_strategy;

pub use self::{
    email_task_config::EmailTaskConfig, http_task_config::HttpTaskConfig,
    task_retry_strategy::TaskRetryStrategy,
};

/// Configuration for the Retrack tasks.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Default)]
pub struct TasksConfig {
    /// The configuration for the HTTP tasks.
    pub http: HttpTaskConfig,
    /// The configuration for the email tasks.
    pub email: EmailTaskConfig,
}

#[cfg(test)]
mod tests {
    use crate::config::TasksConfig;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        assert_toml_snapshot!(TasksConfig::default(), @r###"
        http = { retry_strategy = { type = 'exponential', initial_interval = 60000, multiplier = 2, max_interval = 600000, max_attempts = 3 } }
        email = { retry_strategy = { type = 'exponential', initial_interval = 60000, multiplier = 2, max_interval = 600000, max_attempts = 3 } }
        "###);
    }

    #[test]
    fn deserialization() {
        let config: TasksConfig = toml::from_str(
            r#"
        [http.retry_strategy]
        type = 'exponential'
        initial_interval = 60000
        multiplier = 2
        max_interval = 600000
        max_attempts = 3

        [email.retry_strategy]
        type = 'exponential'
        initial_interval = 60000
        multiplier = 2
        max_interval = 600000
        max_attempts = 3
    "#,
        )
        .unwrap();
        assert_eq!(config, TasksConfig::default());
    }
}

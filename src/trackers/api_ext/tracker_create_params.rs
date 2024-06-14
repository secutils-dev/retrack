use crate::{scheduler::SchedulerJobConfig, trackers::TrackerSettings};
use serde::Deserialize;
use url::Url;
use utoipa::ToSchema;

/// Parameters for creating a tracker.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerCreateParams {
    /// Arbitrary name of the web page tracker.
    pub name: String,
    /// URL of the web page to track.
    pub url: Url,
    /// Settings of the web page tracker.
    pub settings: TrackerSettings,
    /// Configuration for a job, if tracker needs to be scheduled for automatic change detection.
    pub job_config: Option<SchedulerJobConfig>,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{TrackerCreateParams, TrackerSettings},
    };
    use std::time::Duration;
    use url::Url;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "settings": {
            "revisions": 3,
            "delay": 2000
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: None,
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "settings": {
            "revisions": 3,
            "delay": 2000,
            "extractor":  "return document.body.innerHTML;",
            "headers": {
                "cookie": "my-cookie"
            }
        },
        "jobConfig": {
            "schedule": "0 0 * * *",
            "retryStrategy": {
                "type": "exponential",
                "initialInterval": 1234,
                "multiplier": 2,
                "maxInterval": 120000,
                "maxAttempts": 5
            },
            "notifications": true
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Some("return document.body.innerHTML;".to_string()),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect(),
                    )
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * *".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                        initial_interval: Duration::from_millis(1234),
                        multiplier: 2,
                        max_interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                    notifications: true,
                }),
            }
        );

        Ok(())
    }
}

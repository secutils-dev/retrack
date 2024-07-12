use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Deserialize;
use url::Url;
use utoipa::ToSchema;

/// Parameters for creating a tracker.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerCreateParams {
    /// Arbitrary name of the tracker.
    pub name: String,
    /// URL of the resource to track.
    pub url: Url,
    /// Target of the tracker (web page, API, or file).
    #[serde(default)]
    pub target: TrackerTarget,
    /// Tracker config.
    #[serde(default)]
    pub config: TrackerConfig,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{TrackerConfig, TrackerCreateParams, TrackerTarget, TrackerWebPageTarget},
    };
    use std::time::Duration;
    use url::Url;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"{ "name": "tck", "url": "https://retrack.dev" }"#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: Default::default(),
                config: Default::default()
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "config": {
            "revisions": 10
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 10,
                    ..Default::default()
                },
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "target": { "type": "web:page" },
        "config": {
            "revisions": 3
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: TrackerTarget::WebPage(Default::default()),
                config: TrackerConfig {
                    revisions: 3,
                    ..Default::default()
                },
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "target": {
            "type": "web:page",
            "delay": 2000
        },
        "config": {
            "revisions": 3,
            "extractor":  "return document.body.innerHTML;",
            "headers": {
                "cookie": "my-cookie"
            },
            "job": {
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
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: TrackerTarget::WebPage(TrackerWebPageTarget {
                    delay: Some(Duration::from_millis(2000)),
                }),
                config: TrackerConfig {
                    revisions: 3,
                    extractor: Some("return document.body.innerHTML;".to_string()),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    job: Some(SchedulerJobConfig {
                        schedule: "0 0 * * *".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                            initial_interval: Duration::from_millis(1234),
                            multiplier: 2,
                            max_interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                        notifications: Some(true),
                    }),
                },
            }
        );

        Ok(())
    }
}

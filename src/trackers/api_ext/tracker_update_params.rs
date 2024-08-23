use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Deserialize;
use url::Url;
use utoipa::ToSchema;

/// Parameters for updating a tracker.
#[derive(Deserialize, Debug, Default, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct TrackerUpdateParams {
    /// Arbitrary name of the tracker.
    #[schema(min_length = 1, max_length = 100)]
    pub name: Option<String>,
    /// URL of the resource to track.
    pub url: Option<Url>,
    /// Target of the tracker (web page, API, or file).
    pub target: Option<TrackerTarget>,
    /// Tracker config.
    pub config: Option<TrackerConfig>,
    /// Tracker tags.
    #[schema(max_items = 10, min_length = 1, max_length = 50)]
    pub tags: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            TrackerConfig, TrackerTarget, TrackerUpdateParams, WebPageTarget, WebPageWaitFor,
            WebPageWaitForState,
        },
    };
    use std::time::Duration;
    use url::Url;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerUpdateParams>(
                r#"
    {
        "name": "tck"
    }
              "#
            )?,
            TrackerUpdateParams {
                name: Some("tck".to_string()),
                url: None,
                target: None,
                config: None,
                tags: None
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerUpdateParams>(
                r#"
    {
        "target": {
            "type": "web:page",
            "delay": 3000,
            "waitFor": "div"
        }
    }
              "#
            )?,
            TrackerUpdateParams {
                name: None,
                url: None,
                target: Some(TrackerTarget::WebPage(WebPageTarget {
                    delay: Some(Duration::from_millis(3000)),
                    wait_for: Some("div".parse()?),
                })),
                config: None,
                tags: None
            }
        );
        assert_eq!(
            serde_json::from_str::<TrackerUpdateParams>(
                r#"
    {
        "config": {
            "revisions": 3,
            "extractor": "return document.body.innerHTML;",
            "headers": {
                "cookie": "my-cookie"
            }
        }
    }
              "#
            )?,
            TrackerUpdateParams {
                name: None,
                url: None,
                target: None,
                config: Some(TrackerConfig {
                    revisions: 3,
                    extractor: Some("return document.body.innerHTML;".to_string()),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    job: None
                }),
                tags: None
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerUpdateParams>(
                r#"
    {
        "name": "tck",
        "url": "https://retrack.dev",
        "target": {
            "type": "web:page",
            "delay": 2000,
            "waitFor": {
                "selector": "div",
                "state": "attached",
                "timeout": 5000
            }
        },
        "config": {
            "revisions": 3,
            "extractor": "return document.body.innerHTML;",
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
        },
        "tags": ["tag1", "tag2"]
    }
              "#
            )?,
            TrackerUpdateParams {
                name: Some("tck".to_string()),
                url: Some(Url::parse("https://retrack.dev")?),
                target: Some(TrackerTarget::WebPage(WebPageTarget {
                    delay: Some(Duration::from_millis(2000)),
                    wait_for: Some(WebPageWaitFor {
                        selector: "div".to_string(),
                        state: Some(WebPageWaitForState::Attached),
                        timeout: Some(Duration::from_millis(5000)),
                    }),
                })),
                config: Some(TrackerConfig {
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
                }),
                tags: Some(vec!["tag1".to_string(), "tag2".to_string()])
            }
        );

        Ok(())
    }
}

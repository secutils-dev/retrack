use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Deserialize;
use utoipa::ToSchema;

/// Parameters for updating a tracker.
#[derive(Deserialize, Debug, Default, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct TrackerUpdateParams {
    /// Arbitrary name of the tracker.
    #[schema(min_length = 1, max_length = 100)]
    pub name: Option<String>,
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
        trackers::{TrackerConfig, TrackerTarget, TrackerUpdateParams, WebPageTarget},
    };
    use std::time::Duration;

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
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
            "userAgent": "Retrack/1.0.0",
            "ignoreHTTPSErrors": true
        }
    }
              "#
            )?,
            TrackerUpdateParams {
                name: None,
                target: Some(TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
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
            "timeout": 2000,
            "headers": {
                "cookie": "my-cookie"
            }
        }
    }
              "#
            )?,
            TrackerUpdateParams {
                name: None,
                target: None,
                config: Some(TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
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
        "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
            "userAgent": "Retrack/1.0.0",
            "ignoreHTTPSErrors": true
        },
        "config": {
            "revisions": 3,
            "timeout": 2000,
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
                target: Some(TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                })),
                config: Some(TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
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

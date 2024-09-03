use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Deserialize;
use utoipa::ToSchema;

/// Parameters for creating a tracker.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerCreateParams {
    /// Arbitrary name of the tracker.
    #[schema(min_length = 1, max_length = 100)]
    pub name: String,
    /// Target of the tracker (web page, API, or file).
    pub target: TrackerTarget,
    /// Tracker config.
    #[serde(default)]
    pub config: TrackerConfig,
    /// Tracker tags.
    #[schema(max_items = 10, min_length = 1, max_length = 50)]
    #[serde(default)]
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{TrackerConfig, TrackerCreateParams, TrackerTarget, WebPageTarget},
    };
    use std::time::Duration;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(r#"{ "name": "tck", "target": { "type": "web:page", "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }" } }"#)?,
            TrackerCreateParams {
                name: "tck".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: None,
                    ignore_https_errors: false,
                }),
                config: Default::default(),
                tags: vec![]
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }"
        },
        "config": {
            "revisions": 10
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: None,
                    ignore_https_errors: false,
                }),
                config: TrackerConfig {
                    revisions: 10,
                    ..Default::default()
                },
                tags: vec![]
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }"
        },
        "config": {
            "revisions": 3,
            "timeout": 2000
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: None,
                    ignore_https_errors: false,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
                    ..Default::default()
                },
                tags: vec![]
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
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
        "tags": ["tag"]
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
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
                },
                tags: vec!["tag".to_string()]
            }
        );

        Ok(())
    }
}

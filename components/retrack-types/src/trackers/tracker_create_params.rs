use crate::trackers::{TrackerAction, TrackerConfig, TrackerTarget};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Parameters for creating a tracker.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerCreateParams {
    /// Arbitrary name of the tracker.
    #[schema(min_length = 1, max_length = 100)]
    pub name: String,
    /// Whether the tracker is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Target of the tracker (web page, API, or file).
    pub target: TrackerTarget,
    /// Tracker config.
    #[serde(default)]
    pub config: TrackerConfig,
    /// Tracker tags.
    #[schema(max_items = 10, min_length = 1, max_length = 50)]
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tracker actions.
    #[schema(max_items = 10)]
    #[serde(default)]
    pub actions: Vec<TrackerAction>,
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            ExtractorEngine, PageTarget, TrackerAction, TrackerConfig, TrackerCreateParams,
            TrackerTarget, WebhookAction,
        },
    };
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let params = TrackerCreateParams {
            name: "tck".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            }),
            config: Default::default(),
            tags: vec![],
            actions: vec![],
        };
        assert_eq!(
            serde_json::to_value(&params)?,
            json!({
                "name": "tck",
                "enabled": true,
                "target": {
                    "type": "page",
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
                },
                "config": {
                    "revisions": 3
                },
                "actions": [],
                "tags": []
            })
        );

        let params = TrackerCreateParams {
            name: "tck".to_string(),
            enabled: false,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                engine: None,
                params: None,
                user_agent: None,
                accept_invalid_certificates: false,
            }),
            config: TrackerConfig {
                revisions: 10,
                ..Default::default()
            },
            tags: vec![],
            actions: vec![],
        };
        assert_eq!(
            serde_json::to_value(&params)?,
            json!({
                "name": "tck",
                "enabled": false,
                "target": {
                    "type": "page",
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
                },
                "config": {
                    "revisions": 10
                },
                "actions": [],
                "tags": []
            })
        );

        let params = TrackerCreateParams {
            name: "tck".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            }),
            config: TrackerConfig {
                revisions: 3,
                timeout: Some(Duration::from_millis(2000)),
                ..Default::default()
            },
            tags: vec![],
            actions: vec![],
        };
        assert_eq!(
            serde_json::to_value(&params)?,
            json!({
                "name": "tck",
                "enabled": true,
                "target": {
                    "type": "page",
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
                },
                "config": {
                    "revisions": 3,
                    "timeout": 2000
                },
                "actions": [],
                "tags": []
            })
        );

        let params = TrackerCreateParams {
            name: "tck".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                params: Some(json!({ "param": "value" })),
                engine: Some(ExtractorEngine::Chromium),
                user_agent: Some("Retrack/1.0.0".to_string()),
                accept_invalid_certificates: true,
            }),
            config: TrackerConfig {
                revisions: 3,
                timeout: Some(Duration::from_millis(2000)),
                job: Some(SchedulerJobConfig {
                    schedule: "0 0 * * *".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                        initial_interval: Duration::from_millis(1234),
                        multiplier: 2,
                        max_interval: Duration::from_secs(120),
                        max_attempts: 5,
                    })
                }),
            },
            tags: vec!["tag".to_string()],
            actions: vec![TrackerAction::ServerLog(Default::default()), TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: None,
                formatter: None,
            })],
        };
        assert_eq!(
            serde_json::to_value(&params)?,
            json!({
                "name": "tck",
                "enabled": true,
                "target": {
                    "type": "page",
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
                    "params": { "param": "value" },
                    "engine": { "type": "chromium" },
                    "userAgent": "Retrack/1.0.0",
                    "acceptInvalidCertificates": true
                },
                "config": {
                    "revisions": 3,
                    "timeout": 2000,
                    "job": {
                        "schedule": "0 0 * * *",
                        "retryStrategy": {
                            "type": "exponential",
                            "initialInterval": 1234,
                            "multiplier": 2,
                            "maxInterval": 120000,
                            "maxAttempts": 5
                        }
                    }
                },
                "tags": ["tag"],
                "actions": [{ "type": "log" }, { "type": "webhook", "url": "https://retrack.dev/" }]
            })
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(r#"{ "name": "tck", "target": { "type": "page", "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }" } }"#)?,
            TrackerCreateParams {
                name: "tck".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: None,
                    engine: None,
                    user_agent: None,
                    accept_invalid_certificates: false,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "enabled": false,
        "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
        },
        "config": {
            "revisions": 10
        }
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                enabled: false,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: None,
                    engine: None,
                    user_agent: None,
                    accept_invalid_certificates: false,
                }),
                config: TrackerConfig {
                    revisions: 10,
                    ..Default::default()
                },
                tags: vec![],
                actions: vec![],
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "enabled": true,
        "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
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
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: None,
                    engine: None,
                    user_agent: None,
                    accept_invalid_certificates: false,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
                    ..Default::default()
                },
                tags: vec![],
                actions: vec![],
            }
        );

        assert_eq!(
            serde_json::from_str::<TrackerCreateParams>(
                r#"
    {
        "name": "tck",
        "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "params": { "param": "value" },
            "engine": { "type": "chromium" },
            "userAgent": "Retrack/1.0.0",
            "acceptInvalidCertificates": true
        },
        "config": {
            "revisions": 3,
            "timeout": 2000,
            "job": {
                "schedule": "0 0 * * *",
                "retryStrategy": {
                    "type": "exponential",
                    "initialInterval": 1234,
                    "multiplier": 2,
                    "maxInterval": 120000,
                    "maxAttempts": 5
                }
            }
        },
        "tags": ["tag"],
        "actions": [{ "type": "log" }, { "type": "webhook", "url": "https://retrack.dev" }]
    }
              "#
            )?,
            TrackerCreateParams {
                name: "tck".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: Some(json!({ "param": "value" })),
                    engine: Some(ExtractorEngine::Chromium),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    accept_invalid_certificates: true,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
                    job: Some(SchedulerJobConfig {
                        schedule: "0 0 * * *".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                            initial_interval: Duration::from_millis(1234),
                            multiplier: 2,
                            max_interval: Duration::from_secs(120),
                            max_attempts: 5,
                        })
                    }),
                },
                tags: vec!["tag".to_string()],
                actions: vec![TrackerAction::ServerLog(Default::default()), TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: None,
                    formatter: None,
                })],
            }
        );

        Ok(())
    }
}

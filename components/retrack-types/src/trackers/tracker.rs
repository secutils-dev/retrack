use crate::trackers::{TrackerAction, TrackerConfig, TrackerTarget};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Tracker for a web page, API response, or a file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    /// Unique tracker id (UUIDv7).
    pub id: Uuid,
    /// Arbitrary name of the tracker.
    pub name: String,
    /// Whether the tracker is enabled. Disabled trackers are not scheduled.
    pub enabled: bool,
    /// Target of the tracker (web page, API, file).
    pub target: TrackerTarget,
    /// ID of the optional job that triggers tracker. If not set,then the job is not scheduled yet.
    #[serde(skip_serializing)]
    pub job_id: Option<Uuid>,
    /// Tracker config.
    pub config: TrackerConfig,
    /// Case-insensitive tags to categorize the tracker.
    pub tags: Vec<String>,
    /// List of actions to execute when the tracker fetches new data.
    pub actions: Vec<TrackerAction>,
    /// Date and time when the tracker was created.
    #[serde(with = "time::serde::timestamp")]
    pub created_at: OffsetDateTime,
    /// Date and time when the tracker was last updated.
    #[serde(with = "time::serde::timestamp")]
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        tests::MockTrackerBuilder,
        trackers::{PageTarget, TrackerAction, TrackerTarget, WebhookAction},
    };
    use http::{Method, header::CONTENT_TYPE};
    use insta::assert_json_snapshot;
    use std::{collections::HashMap, time::Duration};
    use url::Url;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: Some(serde_json::json!({ "param": "value" })),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "enabled": true,
          "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "params": {
              "param": "value"
            },
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000
          },
          "tags": [],
          "actions": [
            {
              "type": "log"
            }
          ],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "enabled": true,
          "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000,
            "job": {
              "schedule": "0 0 * * *"
            }
          },
          "tags": [],
          "actions": [
            {
              "type": "log"
            }
          ],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "enabled": true,
          "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000,
            "job": {
              "schedule": "0 0 * * *"
            }
          },
          "tags": [],
          "actions": [
            {
              "type": "log"
            }
          ],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: Some(serde_json::json!("Hello, World!")),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "enabled": true,
          "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "params": "Hello, World!",
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000,
            "job": {
              "schedule": "0 0 * * *"
            }
          },
          "tags": [],
          "actions": [
            {
              "type": "log"
            }
          ],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .with_job_config(SchedulerJobConfig {
            schedule: "0 0 * * *".to_string(),
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(1000),
                max_attempts: 10,
            }),
        })
        .with_tags(vec!["tag".to_string()])
        .with_actions(vec![TrackerAction::ServerLog(Default::default()), TrackerAction::Webhook(WebhookAction {
            url: Url::parse("https://retrack.dev/")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            formatter: Some("(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string()),
        })])
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "enabled": true,
          "target": {
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000,
            "job": {
              "schedule": "0 0 * * *",
              "retryStrategy": {
                "type": "constant",
                "interval": 1000000,
                "maxAttempts": 10
              }
            }
          },
          "tags": [
            "tag"
          ],
          "actions": [
            {
              "type": "log"
            },
            {
              "type": "webhook",
              "url": "https://retrack.dev/",
              "method": "PUT",
              "headers": {
                "content-type": "application/json"
              },
              "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
            }
          ],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        Ok(())
    }
}

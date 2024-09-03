use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Serialize;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Tracker for a web page, API response, or a file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    /// Unique tracker id (UUIDv7).
    pub id: Uuid,
    /// Arbitrary name of the tracker.
    pub name: String,
    /// Target of the tracker (web page, API, file).
    pub target: TrackerTarget,
    /// ID of the optional job that triggers tracker. If not set,then the job is not scheduled yet.
    #[serde(skip_serializing)]
    pub job_id: Option<Uuid>,
    /// Tracker config.
    pub config: TrackerConfig,
    /// Case-insensitive tags to categorize the tracker.
    pub tags: Vec<String>,
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
        tests::MockWebPageTrackerBuilder,
        trackers::{TrackerTarget, WebPageTarget},
    };
    use insta::assert_json_snapshot;
    use std::time::Duration;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
            "userAgent": "Retrack/2.0.0",
            "ignoreHTTPSErrors": true
          },
          "config": {
            "revisions": 3,
            "timeout": 2000
          },
          "tags": [],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
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
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
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
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
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
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/2.0.0".to_string()),
            ignore_https_errors: true,
        }))
        .with_schedule("0 0 * * *")
        .with_job_config(SchedulerJobConfig {
            schedule: "0 0 * * *".to_string(),
            notifications: None,
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(1000),
                max_attempts: 10,
            }),
        })
        .with_tags(vec!["tag".to_string()])
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "target": {
            "type": "web:page",
            "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
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
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        Ok(())
    }
}

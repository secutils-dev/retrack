use crate::trackers::{TrackerConfig, TrackerTarget};
use serde::Serialize;
use time::OffsetDateTime;
use url::Url;
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
    /// URL of the resource (e.g., web page, API, or file) to track.
    pub url: Url,
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
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some("div".parse()?),
        }))
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "target": {
            "type": "web:page",
            "delay": 2500,
            "waitFor": {
              "selector": "div"
            }
          },
          "config": {
            "revisions": 3
          },
          "tags": [],
          "createdAt": 946720800,
          "updatedAt": 946720810
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some("div".parse()?),
        }))
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "target": {
            "type": "web:page",
            "delay": 2500,
            "waitFor": {
              "selector": "div"
            }
          },
          "config": {
            "revisions": 3,
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
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some("div".parse()?),
        }))
        .with_schedule("0 0 * * *")
        .with_extractor("return document.body.innerHTML;".to_string())
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "target": {
            "type": "web:page",
            "delay": 2500,
            "waitFor": {
              "selector": "div"
            }
          },
          "config": {
            "revisions": 3,
            "extractor": "return document.body.innerHTML;",
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
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some("div".parse()?),
        }))
        .with_schedule("0 0 * * *")
        .with_extractor(Default::default())
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "target": {
            "type": "web:page",
            "delay": 2500,
            "waitFor": {
              "selector": "div"
            }
          },
          "config": {
            "revisions": 3,
            "extractor": "",
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
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some("div".parse()?),
        }))
        .with_schedule("0 0 * * *")
        .with_extractor(Default::default())
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
          "url": "http://localhost:1234/my/app?q=2",
          "target": {
            "type": "web:page",
            "delay": 2500,
            "waitFor": {
              "selector": "div"
            }
          },
          "config": {
            "revisions": 3,
            "extractor": "",
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

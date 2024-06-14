use crate::{scheduler::SchedulerJobConfig, trackers::TrackerSettings};
use serde::Serialize;
use time::OffsetDateTime;
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;

/// Tracker for a web page, API response, or a file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    /// Unique web page tracker id (UUIDv7).
    pub id: Uuid,
    /// Arbitrary name of the web page tracker.
    pub name: String,
    /// URL of the web page to track.
    pub url: Url,
    /// ID of the optional job that triggers web page checking. If `None` when `job_config` is set,
    /// then the job is not scheduled it.
    #[serde(skip_serializing)]
    pub job_id: Option<Uuid>,
    /// Configuration of the job that triggers web page checking, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_config: Option<SchedulerJobConfig>,
    /// Settings of the web page tracker.
    pub settings: TrackerSettings,
    /// Date and time when the web page tracker was created.
    #[serde(with = "time::serde::timestamp")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        tests::MockWebPageTrackerBuilder,
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
        .with_delay_millis(2500)
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "settings": {
            "revisions": 3,
            "delay": 2500
          },
          "createdAt": 946720800
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_delay_millis(2500)
        .with_schedule("0 0 * * *")
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "jobConfig": {
            "schedule": "0 0 * * *",
            "notifications": false
          },
          "settings": {
            "revisions": 3,
            "delay": 2500
          },
          "createdAt": 946720800
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_delay_millis(2500)
        .with_schedule("0 0 * * *")
        .with_extractor("return document.body.innerHTML;".to_string())
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "jobConfig": {
            "schedule": "0 0 * * *",
            "notifications": false
          },
          "settings": {
            "revisions": 3,
            "delay": 2500,
            "extractor": "return document.body.innerHTML;"
          },
          "createdAt": 946720800
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_delay_millis(2500)
        .with_schedule("0 0 * * *")
        .with_extractor(Default::default())
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "jobConfig": {
            "schedule": "0 0 * * *",
            "notifications": false
          },
          "settings": {
            "revisions": 3,
            "delay": 2500,
            "extractor": ""
          },
          "createdAt": 946720800
        }
        "###);

        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_delay_millis(2500)
        .with_schedule("0 0 * * *")
        .with_extractor(Default::default())
        .with_job_config(SchedulerJobConfig {
            schedule: "0 0 * * *".to_string(),
            notifications: false,
            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                interval: Duration::from_secs(1000),
                max_attempts: 10,
            }),
        })
        .build();
        assert_json_snapshot!(tracker, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "name": "some-name",
          "url": "http://localhost:1234/my/app?q=2",
          "jobConfig": {
            "schedule": "0 0 * * *",
            "retryStrategy": {
              "type": "constant",
              "interval": 1000000,
              "maxAttempts": 10
            },
            "notifications": false
          },
          "settings": {
            "revisions": 3,
            "delay": 2500,
            "extractor": ""
          },
          "createdAt": 946720800
        }
        "###);

        Ok(())
    }
}

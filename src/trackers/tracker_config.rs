use crate::scheduler::SchedulerJobConfig;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::{collections::HashMap, time::Duration};
use utoipa::ToSchema;

#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerConfig {
    /// A number of revisions of the content to track.
    pub revisions: usize,
    /// Number of milliseconds to wait content extraction is considered failed.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub timeout: Option<Duration>,
    /// Optional list of HTTP headers that should be sent with the tracker requests.
    pub headers: Option<HashMap<String, String>>,
    /// Configuration of the job that triggers tracker, if configured.
    pub job: Option<SchedulerJobConfig>,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            revisions: 3,
            timeout: None,
            headers: None,
            job: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{scheduler::SchedulerJobConfig, trackers::TrackerConfig};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let config = TrackerConfig::default();
        assert_json_snapshot!(config, @r###"
        {
          "revisions": 3
        }
        "###);

        let config = TrackerConfig {
            revisions: 3,
            timeout: Some(Duration::from_millis(2500)),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
            job: Some(SchedulerJobConfig {
                schedule: "1 2 3 4 5 6 2035".to_string(),
                retry_strategy: None,
                notifications: Some(true),
            }),
        };
        assert_json_snapshot!(config, @r###"
        {
          "revisions": 3,
          "timeout": 2500,
          "headers": {
            "cookie": "my-cookie"
          },
          "job": {
            "schedule": "1 2 3 4 5 6 2035",
            "notifications": true
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let config = TrackerConfig {
            revisions: 3,
            timeout: None,
            headers: Default::default(),
            job: None,
        };
        assert_eq!(
            serde_json::from_str::<TrackerConfig>(&json!({ "revisions": 3 }).to_string())?,
            config
        );

        let config = TrackerConfig {
            revisions: 3,
            timeout: Some(Duration::from_millis(2500)),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
            job: Some(SchedulerJobConfig {
                schedule: "1 2 3 4 5 6 2035".to_string(),
                retry_strategy: None,
                notifications: Some(true),
            }),
        };
        assert_eq!(
            serde_json::from_str::<TrackerConfig>(
                &json!({
                    "revisions": 3,
                    "timeout": 2500,
                    "headers": { "cookie": "my-cookie" },
                    "job": { "schedule": "1 2 3 4 5 6 2035", "notifications": true }
                })
                .to_string()
            )?,
            config
        );

        Ok(())
    }
}

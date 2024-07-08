use crate::scheduler::SchedulerJobConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerConfig {
    /// A number of revisions of the web page content to track.
    pub revisions: usize,
    /// Optional custom script to extract data from a page, file, or API response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    /// Optional list of HTTP headers that should be sent with the tracker requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// Configuration of the job that triggers tracker, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<SchedulerJobConfig>,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            revisions: 3,
            extractor: None,
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
            extractor: Some("return document.body.innerHTML;".to_string()),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
            job: Some(SchedulerJobConfig {
                schedule: "1 2 3 4 5 6 2035".to_string(),
                retry_strategy: None,
                notifications: true,
            }),
        };
        assert_json_snapshot!(config, @r###"
        {
          "revisions": 3,
          "extractor": "return document.body.innerHTML;",
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
            extractor: Default::default(),
            headers: Default::default(),
            job: None,
        };
        assert_eq!(
            serde_json::from_str::<TrackerConfig>(
                &json!({ "revisions": 3, "delay": 2000 }).to_string()
            )?,
            config
        );

        let config = TrackerConfig {
            revisions: 3,
            extractor: Some("return document.body.innerHTML;".to_string()),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
            job: Some(SchedulerJobConfig {
                schedule: "1 2 3 4 5 6 2035".to_string(),
                retry_strategy: None,
                notifications: true,
            }),
        };
        assert_eq!(
            serde_json::from_str::<TrackerConfig>(
                &json!({
                    "revisions": 3,
                    "delay": 2000,
                    "extractor": "return document.body.innerHTML;",
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

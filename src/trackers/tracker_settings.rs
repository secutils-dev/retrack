use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSeconds};
use std::{collections::HashMap, time::Duration};
use utoipa::ToSchema;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerSettings {
    /// A number of revisions of the web page content to track.
    pub revisions: usize,
    /// Number of milliseconds to wait after web page enters "idle" state to start tracking.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub delay: Duration,
    /// Optional custom script to extract data from a page, file, or API response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    /// Optional list of HTTP headers that should be sent with the tracker requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerSettings;
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let settings = TrackerSettings {
            revisions: 3,
            delay: Duration::from_millis(2500),
            extractor: Default::default(),
            headers: Default::default(),
        };
        assert_json_snapshot!(settings, @r###"
        {
          "revisions": 3,
          "delay": 2500
        }
        "###);

        let settings = TrackerSettings {
            revisions: 3,
            delay: Duration::from_millis(2500),
            extractor: Some("return document.body.innerHTML;".to_string()),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
        };
        assert_json_snapshot!(settings, @r###"
        {
          "revisions": 3,
          "delay": 2500,
          "extractor": "return document.body.innerHTML;",
          "headers": {
            "cookie": "my-cookie"
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let settings = TrackerSettings {
            revisions: 3,
            delay: Duration::from_millis(2000),
            extractor: Default::default(),
            headers: Default::default(),
        };
        assert_eq!(
            serde_json::from_str::<TrackerSettings>(
                &json!({ "revisions": 3, "delay": 2000 }).to_string()
            )?,
            settings
        );

        let settings = TrackerSettings {
            revisions: 3,
            delay: Duration::from_millis(2000),
            extractor: Some("return document.body.innerHTML;".to_string()),
            headers: Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
        };
        assert_eq!(
            serde_json::from_str::<TrackerSettings>(
                &json!({
                    "revisions": 3,
                    "delay": 2000,
                    "extractor": "return document.body.innerHTML;",
                    "headers": { "cookie": "my-cookie" }
                })
                .to_string()
            )?,
            settings
        );

        Ok(())
    }
}

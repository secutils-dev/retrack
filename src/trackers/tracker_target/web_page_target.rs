use crate::{trackers::WebPageWaitFor, utils::string_or_struct};
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::time::Duration;
use utoipa::ToSchema;

/// Tracker's target for a web page.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebPageTarget {
    /// Number of milliseconds to wait after web page enters "idle" state to start tracking.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub delay: Option<Duration>,

    /// A CSS selector for Number of milliseconds to wait after web page enters "idle" state to start tracking.
    #[serde(deserialize_with = "string_or_struct", default)]
    pub wait_for: Option<WebPageWaitFor>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{WebPageTarget, WebPageWaitFor, WebPageWaitForState};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = WebPageTarget::default();
        assert_json_snapshot!(target, @"{}");

        let target = WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(5000)),
            }),
        };
        assert_json_snapshot!(target, @r###"
        {
          "delay": 2500,
          "waitFor": {
            "selector": "div",
            "state": "attached",
            "timeout": 5000
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = WebPageTarget::default();
        assert_eq!(
            serde_json::from_str::<WebPageTarget>(&json!({}).to_string())?,
            target
        );

        let target = WebPageTarget {
            delay: Some(Duration::from_millis(2000)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: None,
                timeout: None,
            }),
        };
        assert_eq!(
            serde_json::from_str::<WebPageTarget>(
                &json!({ "delay": 2000, "waitFor": "div" }).to_string()
            )?,
            target
        );

        let target = WebPageTarget {
            delay: Some(Duration::from_millis(2000)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(5000)),
            }),
        };
        assert_eq!(
            serde_json::from_str::<WebPageTarget>(
                &json!({
                    "delay": 2000,
                    "waitFor": { "selector": "div", "state": "attached", "timeout": 5000 }
                })
                .to_string()
            )?,
            target
        );

        Ok(())
    }
}

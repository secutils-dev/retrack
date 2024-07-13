mod json_api_target;
mod web_page_target;
mod web_page_wait_for;
mod web_page_wait_for_state;

pub use self::{
    json_api_target::JsonApiTarget, web_page_target::WebPageTarget,
    web_page_wait_for::WebPageWaitFor, web_page_wait_for_state::WebPageWaitForState,
};
use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target (web page, API, or file).
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum TrackerTarget {
    /// Web page target.
    #[serde(rename = "web:page")]
    WebPage(WebPageTarget),
    /// JSON API target.
    #[serde(rename = "api:json")]
    JsonApi(JsonApiTarget),
}

impl Default for TrackerTarget {
    fn default() -> Self {
        Self::WebPage(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::TrackerTarget;
    use crate::trackers::{JsonApiTarget, WebPageTarget, WebPageWaitFor, WebPageWaitForState};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = TrackerTarget::default();
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page"
        }
        "###);

        let target = TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(5000)),
            }),
        });
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page",
          "delay": 2500,
          "waitFor": {
            "selector": "div",
            "state": "attached",
            "timeout": 5000
          }
        }
        "###);

        let target = TrackerTarget::JsonApi(JsonApiTarget);
        assert_json_snapshot!(target, @r###"
        {
          "type": "api:json"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = TrackerTarget::default();
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({ "type": "web:page" }).to_string())?,
            target
        );

        let target = TrackerTarget::WebPage(WebPageTarget {
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: None,
                timeout: None,
            }),
            ..Default::default()
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({ "type": "web:page", "waitFor": "div" }).to_string()
            )?,
            target
        );

        let target = TrackerTarget::WebPage(WebPageTarget {
            delay: Some(Duration::from_millis(2000)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(5000)),
            }),
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({
                    "type": "web:page",
                    "delay": 2000,
                    "waitFor": { "selector": "div", "state": "attached", "timeout": 5000 }
                })
                .to_string()
            )?,
            target
        );

        let target = TrackerTarget::JsonApi(JsonApiTarget);
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({ "type": "api:json" }).to_string())?,
            target
        );

        Ok(())
    }
}

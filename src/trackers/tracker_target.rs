mod tracker_json_api_target;
mod tracker_web_page_target;

pub use self::{
    tracker_json_api_target::TrackerJsonApiTarget, tracker_web_page_target::TrackerWebPageTarget,
};
use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target (web page, API, or file).
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum TrackerTarget {
    /// Web page target.
    #[serde(rename = "web:page")]
    WebPage(TrackerWebPageTarget),
    /// JSON API target.
    #[serde(rename = "api:json")]
    JsonApi(TrackerJsonApiTarget),
}

impl Default for TrackerTarget {
    fn default() -> Self {
        Self::WebPage(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::TrackerTarget;
    use crate::trackers::{TrackerJsonApiTarget, TrackerWebPageTarget};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = TrackerTarget::default();
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page",
          "delay": null
        }
        "###);

        let target = TrackerTarget::WebPage(TrackerWebPageTarget {
            delay: Some(Duration::from_millis(2500)),
        });
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page",
          "delay": 2500
        }
        "###);

        let target = TrackerTarget::JsonApi(TrackerJsonApiTarget);
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

        let target = TrackerTarget::WebPage(TrackerWebPageTarget {
            delay: Some(Duration::from_millis(2000)),
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({ "type": "web:page", "delay": 2000 }).to_string()
            )?,
            target
        );

        let target = TrackerTarget::JsonApi(TrackerJsonApiTarget);
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({ "type": "api:json" }).to_string())?,
            target
        );

        Ok(())
    }
}

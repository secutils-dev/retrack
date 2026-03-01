use crate::trackers::{TrackerAction, TrackerConfig, TrackerDataValue, TrackerTarget};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Parameters for the ad-hoc tracker debug endpoint (`POST /api/trackers/_debug`).
///
/// Accepts the same target/config shape as `TrackerCreateParams` but does not persist
/// anything. Fields irrelevant to a debug run (schedule, revisions count) are accepted
/// but ignored.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDebugParams {
    /// Target of the tracker (web page or API).
    pub target: TrackerTarget,
    /// Tracker config (only `timeout` is meaningful for debug runs).
    #[serde(default)]
    pub config: TrackerConfig,
    /// Tracker tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Actions to simulate (dry-run). No side-effects are produced.
    #[serde(default)]
    pub actions: Vec<TrackerAction>,
    /// Optional previous content to simulate how the tracker behaves with existing data.
    pub previous_content: Option<TrackerDataValue>,
}

#[cfg(test)]
mod tests {
    use super::TrackerDebugParams;
    use crate::trackers::{
        PageTarget, TrackerAction, TrackerConfig, TrackerDataValue, TrackerTarget, WebhookAction,
    };
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn deserialization_minimal() -> anyhow::Result<()> {
        let params: TrackerDebugParams = serde_json::from_value(json!({
            "target": {
                "type": "page",
                "extractor": "export async function execute(p) { return 'ok'; }"
            }
        }))?;

        assert_eq!(
            params.target,
            TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { return 'ok'; }".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            })
        );
        assert_eq!(params.config, TrackerConfig::default());
        assert!(params.tags.is_empty());
        assert!(params.actions.is_empty());
        assert!(params.previous_content.is_none());

        Ok(())
    }

    #[test]
    fn deserialization_full() -> anyhow::Result<()> {
        let params: TrackerDebugParams = serde_json::from_value(json!({
            "target": {
                "type": "page",
                "extractor": "export async function execute(p) { return 'ok'; }"
            },
            "config": { "revisions": 1, "timeout": 5000 },
            "tags": ["t1"],
            "actions": [{ "type": "webhook", "url": "https://retrack.dev/" }],
            "previousContent": { "original": "prev" }
        }))?;

        assert_eq!(params.config.timeout, Some(Duration::from_millis(5000)));
        assert_eq!(params.tags, vec!["t1"]);
        assert_eq!(params.actions.len(), 1);
        assert_eq!(
            params.previous_content,
            Some(TrackerDataValue::new(json!("prev")))
        );

        Ok(())
    }

    #[test]
    fn serialization_roundtrip() -> anyhow::Result<()> {
        let params = TrackerDebugParams {
            target: TrackerTarget::Page(PageTarget {
                extractor: "script".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            }),
            config: TrackerConfig::default(),
            tags: vec!["tag".to_string()],
            actions: vec![TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: None,
                formatter: None,
            })],
            previous_content: Some(TrackerDataValue::new(json!("prev"))),
        };

        assert_json_snapshot!(params, @r###"
        {
          "target": {
            "type": "page",
            "extractor": "script"
          },
          "config": {
            "revisions": 3
          },
          "tags": [
            "tag"
          ],
          "actions": [
            {
              "type": "webhook",
              "url": "https://retrack.dev/"
            }
          ],
          "previousContent": {
            "original": "prev"
          }
        }
        "###);

        let roundtrip: TrackerDebugParams = serde_json::from_value(serde_json::to_value(&params)?)?;
        assert_eq!(roundtrip, params);

        Ok(())
    }
}

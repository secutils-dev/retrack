use crate::{
    trackers::{TrackerAction, TrackerConfig, TrackerDataValue, TrackerTarget},
    utils::{deserialize_opt_byte_from_u64, serialize_opt_byte_as_u64},
};
use byte_unit::Byte;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Options controlling debug behavior such as screenshot capture limits.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DebugOptions {
    /// Maximum cumulative size (in bytes) of screenshots to capture. When omitted the web
    /// scraper applies its own default (5 MB).
    #[serde(
        default,
        serialize_with = "serialize_opt_byte_as_u64",
        deserialize_with = "deserialize_opt_byte_from_u64"
    )]
    #[schema(value_type = Option<u64>)]
    pub max_screenshots_total_size: Option<Byte>,
    /// Whether to automatically capture a screenshot after every significant Playwright action.
    /// Defaults to `true` when omitted.
    pub auto_screenshots: Option<bool>,
}

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
    /// Optional debug options controlling screenshot capture and other debug settings.
    pub debug: Option<DebugOptions>,
}

#[cfg(test)]
mod tests {
    use super::{DebugOptions, TrackerDebugParams};
    use crate::trackers::{
        PageTarget, TrackerAction, TrackerConfig, TrackerDataValue, TrackerTarget, WebhookAction,
    };
    use byte_unit::Byte;
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
        assert!(params.debug.is_none());

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
            "previousContent": { "original": "prev" },
            "debug": {
                "maxScreenshotsTotalSize": 10485760,
                "autoScreenshots": false
            }
        }))?;

        assert_eq!(params.config.timeout, Some(Duration::from_millis(5000)));
        assert_eq!(params.tags, vec!["t1"]);
        assert_eq!(params.actions.len(), 1);
        assert_eq!(
            params.previous_content,
            Some(TrackerDataValue::new(json!("prev")))
        );
        assert_eq!(
            params.debug,
            Some(DebugOptions {
                max_screenshots_total_size: Some(Byte::from_u64(10485760)),
                auto_screenshots: Some(false),
            })
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
            debug: None,
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

    #[test]
    fn serialization_roundtrip_with_debug() -> anyhow::Result<()> {
        let params = TrackerDebugParams {
            target: TrackerTarget::Page(PageTarget {
                extractor: "script".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            }),
            config: TrackerConfig::default(),
            tags: vec![],
            actions: vec![],
            previous_content: None,
            debug: Some(DebugOptions {
                max_screenshots_total_size: Some(Byte::from_u64(5242880)),
                auto_screenshots: Some(true),
            }),
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
          "tags": [],
          "actions": [],
          "debug": {
            "maxScreenshotsTotalSize": 5242880,
            "autoScreenshots": true
          }
        }
        "###);

        let roundtrip: TrackerDebugParams = serde_json::from_value(serde_json::to_value(&params)?)?;
        assert_eq!(roundtrip, params);

        Ok(())
    }
}

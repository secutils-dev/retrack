use crate::trackers::{
    DebugOptions, TrackerAction, TrackerConfig, TrackerDataValue, TrackerTarget,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Parameters for the existing-tracker debug endpoint
/// (`POST /api/trackers/{tracker_id}/_debug`).
///
/// Every field is optional. When provided, it overrides the corresponding field on the
/// stored tracker for this debug run only (nothing is persisted).
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDebugExistingParams {
    /// Override the tracker's target for this debug run.
    pub target: Option<TrackerTarget>,
    /// Override the tracker's config for this debug run.
    pub config: Option<TrackerConfig>,
    /// Override the tracker's tags for this debug run.
    pub tags: Option<Vec<String>>,
    /// Override the tracker's actions for this debug run.
    pub actions: Option<Vec<TrackerAction>>,
    /// Supply previous content instead of using the latest stored revision.
    pub previous_content: Option<TrackerDataValue>,
    /// Optional debug options controlling screenshot capture.
    pub debug: Option<DebugOptions>,
}

#[cfg(test)]
mod tests {
    use super::TrackerDebugExistingParams;
    use crate::trackers::{PageTarget, TrackerDataValue, TrackerTarget};
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn deserialization_empty() -> anyhow::Result<()> {
        let params: TrackerDebugExistingParams = serde_json::from_value(json!({}))?;
        assert_eq!(params, TrackerDebugExistingParams::default());
        Ok(())
    }

    #[test]
    fn deserialization_with_overrides() -> anyhow::Result<()> {
        let params: TrackerDebugExistingParams = serde_json::from_value(json!({
            "target": {
                "type": "page",
                "extractor": "export async function execute(p) { return 'ok'; }"
            },
            "tags": ["override-tag"],
            "previousContent": { "original": "prev" }
        }))?;

        assert!(params.target.is_some());
        assert!(params.config.is_none());
        assert_eq!(params.tags, Some(vec!["override-tag".to_string()]));
        assert!(params.actions.is_none());
        assert_eq!(
            params.previous_content,
            Some(TrackerDataValue::new(json!("prev")))
        );

        Ok(())
    }

    #[test]
    fn serialization_roundtrip() -> anyhow::Result<()> {
        let params = TrackerDebugExistingParams {
            target: Some(TrackerTarget::Page(PageTarget {
                extractor: "script".to_string(),
                params: None,
                engine: None,
                user_agent: None,
                accept_invalid_certificates: false,
            })),
            config: None,
            tags: Some(vec!["t".to_string()]),
            actions: None,
            previous_content: Some(TrackerDataValue::new(json!("prev"))),
            debug: None,
        };

        assert_json_snapshot!(params, @r###"
        {
          "target": {
            "type": "page",
            "extractor": "script"
          },
          "tags": [
            "t"
          ],
          "previousContent": {
            "original": "prev"
          }
        }
        "###);

        let roundtrip: TrackerDebugExistingParams =
            serde_json::from_value(serde_json::to_value(&params)?)?;
        assert_eq!(roundtrip, params);

        Ok(())
    }
}

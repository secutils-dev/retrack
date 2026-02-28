use crate::trackers::{TargetResponse, TrackerDataValue};
use serde::Serialize;
use serde_with::skip_serializing_none;

/// Context available to the "extractor" scripts through the global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorScriptArgs {
    /// Tags associated with the tracker.
    pub tags: Vec<String>,

    /// Optional previous content.
    pub previous_content: Option<TrackerDataValue>,

    /// Optional HTTP body returned from the API.
    #[serde(default)]
    pub responses: Option<Vec<TargetResponse>>,

    /// Optional parameters passed from the target configuration (e.g., secrets).
    pub params: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ExtractorScriptArgs, TargetResponse, TrackerDataValue};
    use http::{StatusCode, header::CONTENT_TYPE};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = ExtractorScriptArgs::default();
        let context_json = json!({ "tags": [] });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let previous_content = TrackerDataValue::new(json!({"key": "value"}));
        let context = ExtractorScriptArgs {
            tags: vec![],
            previous_content: Some(previous_content.clone()),
            responses: None,
            params: None,
        };
        let context_json =
            json!({ "tags": [], "previousContent": { "original": { "key": "value" } } });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let body = json!({ "body": "value" });
        let context = ExtractorScriptArgs {
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            previous_content: Some(previous_content),
            params: Some(json!({ "secrets": { "api_key": "s3cr3t" } })),
            responses: Some(vec![TargetResponse {
                status: StatusCode::OK,
                headers: (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
                body: serde_json::to_vec(&body)?,
            }]),
        };
        let context_json = json!({
            "tags": ["tag1", "tag2"],
            "previousContent": { "original": { "key": "value" } },
            "params": { "secrets": { "api_key": "s3cr3t" } },
            "responses": [{
                "status": 200,
                "headers": { "content-type": "application/json" },
                "body": [123, 34, 98, 111, 100, 121, 34, 58, 34, 118, 97, 108, 117, 101, 34, 125]
            }],
        });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        Ok(())
    }
}

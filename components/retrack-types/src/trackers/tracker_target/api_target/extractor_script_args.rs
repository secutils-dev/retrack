use crate::trackers::TrackerDataValue;
use serde::Serialize;
use serde_with::skip_serializing_none;

/// Context available to the "extractor" scripts through global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorScriptArgs {
    /// Tags associated with the tracker.
    pub tags: Vec<String>,

    /// Optional previous content.
    pub previous_content: Option<TrackerDataValue>,

    /// Optional HTTP body to send with the request. If not specified, the default body of the `api`
    /// target is used.
    #[serde(with = "serde_bytes", default)]
    pub body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ExtractorScriptArgs, TrackerDataValue};
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = ExtractorScriptArgs::default();
        let context_json = json!({ "tags": [] });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let previous_content = TrackerDataValue::new(json!({"key": "value"}));
        let context = ExtractorScriptArgs {
            tags: vec![],
            previous_content: Some(previous_content.clone()),
            body: None,
        };
        let context_json =
            json!({ "tags": [], "previousContent": { "original": { "key": "value" } } });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let body = json!({ "body": "value" });
        let context = ExtractorScriptArgs {
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            previous_content: Some(previous_content),
            body: Some(serde_json::to_vec(&body)?),
        };
        let context_json = json!({
            "tags": ["tag1", "tag2"],
            "previousContent": { "original": { "key": "value" } },
            "body": [123, 34, 98, 111, 100, 121, 34, 58, 34, 118, 97, 108, 117, 101, 34, 125],
        });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        Ok(())
    }
}

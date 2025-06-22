use crate::trackers::{ConfiguratorScriptRequest, TrackerDataValue};
use serde::Serialize;
use serde_with::skip_serializing_none;

/// Context available to the "configurator" scripts through global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguratorScriptArgs {
    /// Tags associated with the tracker.
    pub tags: Vec<String>,

    /// Optional previous content.
    pub previous_content: Option<TrackerDataValue>,

    /// A list of HTTP requests configured for the target.
    pub requests: Vec<ConfiguratorScriptRequest>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ConfiguratorScriptArgs, ConfiguratorScriptRequest, TrackerDataValue};
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = ConfiguratorScriptArgs::default();
        let context_json = json!({ "tags": [], "requests": [] });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let previous_content = TrackerDataValue::new(json!({"key": "value"}));
        let context = ConfiguratorScriptArgs {
            tags: vec![],
            previous_content: Some(previous_content.clone()),
            requests: vec![],
        };
        let context_json = json!({ "tags": [], "previousContent": { "original": { "key": "value" } }, "requests": [] });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let context = ConfiguratorScriptArgs {
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            previous_content: Some(previous_content),
            requests: vec![ConfiguratorScriptRequest {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: None,
                media_type: None,
                body: Some(serde_json::to_vec(&json!({ "body": "value" }))?),
                accept_statuses: None,
                accept_invalid_certificates: None,
            }],
        };
        let context_json = json!({
            "tags": ["tag1", "tag2"],
            "previousContent": { "original": { "key": "value" } },
            "requests": [{ "url": "https://retrack.dev/", "body": [123, 34, 98, 111, 100, 121, 34, 58, 34, 118, 97, 108, 117, 101, 34, 125] }],
        });
        assert_eq!(serde_json::to_value(&context)?, context_json);
        Ok(())
    }
}

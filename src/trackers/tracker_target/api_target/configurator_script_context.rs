use crate::trackers::TrackerDataValue;
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_with::skip_serializing_none;

/// Context available to the "configurator" scripts through global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguratorScriptContext<'a> {
    /// Optional previous contentAn internet socket address of the client that made the request.
    pub previous_content: Option<&'a TrackerDataValue>,

    /// Optional HTTP body configured for the request.
    pub body: Option<&'a JsonValue>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ConfiguratorScriptContext, TrackerDataValue};
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = ConfiguratorScriptContext::default();
        let context_json = json!({});
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let previous_content = TrackerDataValue::new(json!({"key": "value"}));
        let context = ConfiguratorScriptContext {
            previous_content: Some(&previous_content),
            body: None,
        };
        let context_json = json!({ "previousContent": { "original": { "key": "value" } } });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let body = json!({ "body": "value" });
        let context = ConfiguratorScriptContext {
            previous_content: Some(&previous_content),
            body: Some(&body),
        };
        let context_json = json!({
            "previousContent": { "original": { "key": "value" } },
            "body": { "body": "value" },
        });
        assert_eq!(serde_json::to_value(&context)?, context_json);
        Ok(())
    }
}

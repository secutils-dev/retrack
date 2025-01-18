use serde::Serialize;
use serde_with::skip_serializing_none;

/// Context available to the "formatter" scripts through global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FormatterScriptArgs {
    /// Tracker action type the formatter script is invoked for.
    pub action: &'static str,

    /// New content.
    pub new_content: serde_json::Value,

    /// Optional previous content.
    pub previous_content: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::FormatterScriptArgs;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = FormatterScriptArgs {
            action: "log",
            new_content: json!({ "key": "value" }),
            previous_content: None,
        };
        let context_json = json!({ "action": "log", "newContent": { "key": "value" } });
        assert_eq!(serde_json::to_value(&context)?, context_json);

        let context = FormatterScriptArgs {
            action: "log",
            new_content: json!({ "key": "value" }),
            previous_content: Some(json!({ "key": "old-value" })),
        };
        let context_json = json!({
            "action": "log",
            "newContent": { "key": "value" },
            "previousContent": { "key": "old-value" },
        });
        assert_eq!(serde_json::to_value(&context)?, context_json);
        Ok(())
    }
}

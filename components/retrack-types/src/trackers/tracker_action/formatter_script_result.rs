use serde::Deserialize;

/// Result of the "formatter" script execution.
#[derive(Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FormatterScriptResult {
    /// Optional formatted content.
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::FormatterScriptResult;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<FormatterScriptResult>(
                r#"
{
    "content": { "key": "value" }
}
          "#
            )?,
            FormatterScriptResult {
                content: Some(serde_json::json!({ "key": "value" }))
            }
        );

        assert_eq!(
            serde_json::from_str::<FormatterScriptResult>(r#"{}"#)?,
            Default::default()
        );

        Ok(())
    }
}

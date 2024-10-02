use serde::Deserialize;

/// Result of the "extractor" script execution.
#[derive(Deserialize, Default, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorScriptResult;

#[cfg(test)]
mod tests {
    use crate::trackers::ExtractorScriptResult;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ExtractorScriptResult>(r#"null"#)?,
            ExtractorScriptResult
        );

        assert_eq!(
            serde_json::from_str::<ExtractorScriptResult>(r#"null"#)?,
            Default::default()
        );

        Ok(())
    }
}

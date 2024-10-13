use serde::Deserialize;

/// Result of the "extractor" script execution.
#[derive(Deserialize, Default, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorScriptResult {
    /// Optional HTTP body to use as a response.
    #[serde(with = "serde_bytes", default)]
    pub body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::ExtractorScriptResult;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ExtractorScriptResult>(
                r#"
{
    "body": [1, 2 ,3]
}
          "#
            )?,
            ExtractorScriptResult {
                body: Some(vec![1, 2, 3]),
            }
        );

        assert_eq!(
            serde_json::from_str::<ExtractorScriptResult>(r#"{}"#)?,
            Default::default()
        );

        Ok(())
    }
}

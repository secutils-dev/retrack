use serde::Serialize;
use serde_with::skip_serializing_none;

/// Context available to the "extractor" scripts through global `context` variable.
#[skip_serializing_none]
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorScriptArgs;

#[cfg(test)]
mod tests {
    use crate::trackers::ExtractorScriptArgs;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let context = ExtractorScriptArgs;
        let context_json = json!(null);
        assert_eq!(serde_json::to_value(&context)?, context_json);

        Ok(())
    }
}

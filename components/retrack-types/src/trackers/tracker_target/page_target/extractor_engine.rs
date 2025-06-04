use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's page target extractor engine (browser).
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum ExtractorEngine {
    Chromium,
    Camoufox,
}

#[cfg(test)]
mod tests {
    use super::ExtractorEngine;
    use insta::assert_json_snapshot;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(ExtractorEngine::Camoufox, @r###"
        {
          "type": "camoufox"
        }
        "###);
        assert_json_snapshot!(ExtractorEngine::Chromium, @r###"
        {
          "type": "chromium"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ExtractorEngine>("{ \"type\": \"camoufox\" }")?,
            ExtractorEngine::Camoufox
        );

        assert_eq!(
            serde_json::from_str::<ExtractorEngine>("{ \"type\": \"chromium\" }")?,
            ExtractorEngine::Chromium
        );

        Ok(())
    }
}

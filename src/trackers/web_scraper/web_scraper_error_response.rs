use crate::trackers::web_scraper::WebScraperDebugInfo;
use serde::{Deserialize, Serialize};

/// Represents an error returned by the web scraper service.
/// Includes `debug` info when the request had `debug: true`.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperErrorResponse {
    /// Human-readable error description.
    pub message: String,
    /// Raw error string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Debug information, present only when `debug: true` was sent in the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<WebScraperDebugInfo>,
}

#[cfg(test)]
mod tests {
    use super::WebScraperErrorResponse;
    use insta::assert_json_snapshot;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<WebScraperErrorResponse>(
                r#"
{
    "message": "some-error"
}
          "#
            )?,
            WebScraperErrorResponse {
                message: "some-error".to_string(),
                error: None,
                debug: None,
            }
        );

        Ok(())
    }

    #[test]
    fn deserialization_with_error_and_debug() -> anyhow::Result<()> {
        let response: WebScraperErrorResponse = serde_json::from_str(
            r#"{
                "message": "Failed to execute extractor script: timeout",
                "error": "timeout",
                "debug": {
                    "logs": [{ "level": "info", "message": "Connecting..." }]
                }
            }"#,
        )?;

        assert_eq!(response.error, Some("timeout".to_string()));
        let debug = response.debug.unwrap();
        assert_eq!(debug.logs.len(), 1);

        Ok(())
    }

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperErrorResponse {
            message: "some-error".to_string(),
            error: None,
            debug: None,
        }, @r###"
        {
          "message": "some-error"
        }
        "###);

        Ok(())
    }
}

use crate::trackers::web_scraper::WebScraperDebugInfo;
use serde::Deserialize;
use serde_json::Value as JsonValue;

/// Successful response returned by the web scraper service.
/// Always contains the extracted `result`; includes `debug` info when the request had `debug: true`.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperSuccessResponse {
    /// The extracted result value.
    pub result: JsonValue,
    /// Debug information, present only when `debug: true` was sent in the request.
    pub debug: Option<WebScraperDebugInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialization_without_debug() -> anyhow::Result<()> {
        let response: WebScraperSuccessResponse =
            serde_json::from_value(json!({ "result": "extracted content" }))?;

        assert_eq!(response.result, json!("extracted content"));
        assert!(response.debug.is_none());

        Ok(())
    }

    #[test]
    fn deserialization_with_debug() -> anyhow::Result<()> {
        let response: WebScraperSuccessResponse = serde_json::from_value(json!({
            "result": {"key": "value"},
            "debug": {
                "logs": [
                    { "level": "info", "message": "Connecting to browser..." },
                    { "level": "error", "message": "Failed to load resource", "args": [{"url": "https://example.com"}] }
                ]
            }
        }))?;

        assert_eq!(response.result, json!({"key": "value"}));
        let debug = response.debug.unwrap();
        assert_eq!(debug.logs.len(), 2);
        assert_eq!(debug.logs[0].level, "info");
        assert_eq!(debug.logs[1].level, "error");
        assert!(debug.logs[1].args.is_some());

        Ok(())
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Debug payload attached to both success and error responses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperDebugInfo {
    /// Log messages collected from the Playwright worker.
    #[serde(default)]
    pub logs: Vec<WebScraperLogEntry>,
}

/// A single log entry from the web scraper worker.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperLogEntry {
    /// Log level ("info" or "error").
    pub level: String,
    /// Log message text.
    pub message: String,
    /// Optional structured arguments.
    pub args: Option<JsonValue>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::web_scraper::WebScraperDebugInfo;
    use serde_json::json;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let debug: WebScraperDebugInfo = serde_json::from_value(json!({
            "logs": [
                { "level": "info", "message": "Connecting to browser..." },
                { "level": "error", "message": "Failed to load resource", "args": [{"url": "https://example.com"}] }
            ]
        }))?;

        assert_eq!(debug.logs.len(), 2);
        assert_eq!(debug.logs[0].level, "info");
        assert_eq!(debug.logs[1].level, "error");
        assert!(debug.logs[1].args.is_some());

        Ok(())
    }
}

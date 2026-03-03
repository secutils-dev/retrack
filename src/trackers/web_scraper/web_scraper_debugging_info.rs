use retrack_types::trackers::{PageLogEntry, PageScreenshotEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Debug payload attached to both success and error responses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperDebugInfo {
    /// Log messages collected from the Playwright worker.
    #[serde(default)]
    pub logs: Vec<WebScraperLogEntry>,
    /// Screenshots captured during extraction (manual, auto-trace, or on-error).
    #[serde(default)]
    pub screenshots: Vec<WebScraperScreenshotEntry>,
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

/// A single screenshot entry from the web scraper worker.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperScreenshotEntry {
    /// Descriptive label (e.g., "after goto: https://example.com" or "page.screenshot()").
    pub label: String,
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type of the image (e.g., "image/png").
    pub mime_type: String,
}

impl From<WebScraperLogEntry> for PageLogEntry {
    fn from(entry: WebScraperLogEntry) -> Self {
        Self {
            level: entry.level,
            message: entry.message,
            args: entry.args,
        }
    }
}

impl From<WebScraperScreenshotEntry> for PageScreenshotEntry {
    fn from(entry: WebScraperScreenshotEntry) -> Self {
        Self {
            label: entry.label,
            data: entry.data,
            mime_type: entry.mime_type,
        }
    }
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
        assert!(debug.screenshots.is_empty());

        Ok(())
    }

    #[test]
    fn deserialization_with_screenshots() -> anyhow::Result<()> {
        let debug: WebScraperDebugInfo = serde_json::from_value(json!({
            "logs": [
                { "level": "info", "message": "Connected." }
            ],
            "screenshots": [
                { "label": "after goto: https://example.com", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
                { "label": "page.screenshot()", "data": "AAAA", "mimeType": "image/jpeg" }
            ]
        }))?;

        assert_eq!(debug.logs.len(), 1);
        assert_eq!(debug.screenshots.len(), 2);
        assert_eq!(
            debug.screenshots[0].label,
            "after goto: https://example.com"
        );
        assert_eq!(debug.screenshots[0].mime_type, "image/png");
        assert_eq!(debug.screenshots[1].label, "page.screenshot()");
        assert_eq!(debug.screenshots[1].mime_type, "image/jpeg");

        Ok(())
    }
}

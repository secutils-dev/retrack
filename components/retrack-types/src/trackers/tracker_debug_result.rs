use crate::trackers::ExtractorEngine;
use http::{HeaderMap, Method};
use mediatype::MediaTypeBuf;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_with::{DurationMilliSeconds, serde_as, skip_serializing_none};
use std::time::Duration;
use url::Url;
use utoipa::ToSchema;

/// Top-level result returned by the tracker debug endpoints.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDebugResult {
    /// Total wall-clock duration of the debug operation in milliseconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub duration_ms: Duration,
    /// Final extracted result (what would become the revision content).
    pub result: Option<JsonValue>,
    /// Top-level error if the extraction pipeline failed.
    pub error: Option<String>,
    /// Target-specific debug information (API or Page).
    pub target: TrackerDebugTargetResult,
    /// Action dry-run results (one entry per configured + default action).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionDebugInfo>,
}

/// Target-specific debug information, tagged by a tracker target type.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackerDebugTargetResult {
    /// Debug information for an API tracker target.
    #[serde(rename = "api")]
    Api(ApiTrackerDebugResult),
    /// Debug information for a Page tracker target.
    #[serde(rename = "page")]
    Page(PageTrackerDebugResult),
}

/// Debug information for an API tracker target run.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTrackerDebugResult {
    /// The `params` value from the target config (may contain secrets).
    pub params: Option<JsonValue>,
    /// Configurator script debug info (present only when a configurator script is defined).
    pub configurator: Option<ScriptDebugInfo>,
    /// Per-request debug information for every HTTP request executed.
    pub requests: Vec<ApiRequestDebugInfo>,
    /// Extractor script debug info (present only when an extractor script is defined).
    pub extractor: Option<ScriptDebugInfo>,
}

/// Debug information for a Page tracker target run.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageTrackerDebugResult {
    /// The `params` value from the target config (may contain secrets).
    pub params: Option<JsonValue>,
    /// The extractor engine used ("chromium" or "camoufox").
    pub engine: Option<ExtractorEngine>,
    /// The resolved extractor script source (after fetching if a URL was provided).
    pub extractor_source: String,
    /// Log messages collected from the Playwright worker during extraction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<PageLogEntry>,
    /// Screenshots captured during extraction (manual, auto-trace, or on-error).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<PageScreenshotEntry>,
    /// Duration of the extraction in milliseconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub duration_ms: Duration,
    /// Error if the extraction failed.
    pub error: Option<String>,
}

/// A single log entry captured from the web scraper worker.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageLogEntry {
    /// Log level ("info" or "error").
    pub level: String,
    /// Log message text.
    pub message: String,
    /// Optional structured arguments.
    pub args: Option<JsonValue>,
}

/// A single screenshot captured during page extraction debug.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageScreenshotEntry {
    /// Descriptive label (e.g., "after goto: https://example.com" or "page.screenshot()").
    pub label: String,
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type of the image (e.g., "image/png").
    pub mime_type: String,
}

/// Debug information about a script execution stage.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDebugInfo {
    /// Script source (inline content or URL reference).
    pub source: String,
    /// Duration of script execution in milliseconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub duration_ms: Duration,
    /// Script result serialized as JSON.
    pub result: Option<JsonValue>,
    /// Error message if the script execution failed.
    pub error: Option<String>,
}

/// Debug information for a single HTTP request within an API tracker target.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiRequestDebugInfo {
    /// Zero-based request index.
    pub index: usize,
    /// Origin of this request: "original" (from target config) or "configurator" (modified
    /// by configurator script).
    pub source: String,
    /// The URL that was requested.
    pub url: Option<Url>,
    /// The HTTP method used.
    #[serde(with = "http_serde::option::method", default)]
    #[schema(value_type = Option<String>)]
    pub method: Option<Method>,
    /// Request headers as a JSON object.
    #[serde(with = "http_serde::option::header_map")]
    #[schema(value_type = Option<HashMap<String, String>>)]
    pub request_headers: Option<HeaderMap>,
    /// Request body.
    pub request_body: Option<JsonValue>,
    /// HTTP status code of the response.
    pub status_code: Option<u16>,
    /// Response headers as a JSON object.
    #[serde(with = "http_serde::option::header_map")]
    #[schema(value_type = Option<HashMap<String, String>>)]
    pub response_headers: Option<HeaderMap>,
    /// Raw response body as a string (UTF-8 text or base64 for binary, truncated).
    pub response_body_raw: Option<String>,
    /// Total response body size in bytes (before truncation).
    pub response_body_raw_size: Option<u64>,
    /// Parsed response body (after auto-parsing or JSON deserialization).
    pub response_body_parsed: Option<JsonValue>,
    /// Auto-parse debug info (present only if media-type-based parsing was attempted).
    pub auto_parse: Option<AutoParseDebugInfo>,
    /// Duration of this request in milliseconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub duration_ms: Duration,
    /// Error message if this request failed.
    pub error: Option<String>,
}

/// Debug information about media-type-based auto-parsing of a response body.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoParseDebugInfo {
    /// The media type used for parsing (e.g. "text/csv").
    #[schema(value_type = String)]
    pub media_type: MediaTypeBuf,
    /// Whether parsing succeeded.
    pub success: bool,
    /// Error message if parsing failed.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Action dry-run types
// ---------------------------------------------------------------------------

/// Debug information for a single action dry-run.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionDebugInfo {
    /// Action type tag: "email", "webhook", or "log".
    pub type_tag: String,
    /// Zero-based action index.
    pub index: usize,
    /// Formatter script debug info (`None` when no formatter is configured).
    pub formatter: Option<ScriptDebugInfo>,
    /// Whether the action would be skipped (formatter returned `null`).
    pub skipped: bool,
    /// The computed action payload.
    pub payload: Option<JsonValue>,
    /// Where the payload would be sent.
    pub destination: ActionDestinationDebugInfo,
    /// Duration to compute this action's payload in milliseconds.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub duration_ms: Duration,
    /// Error if formatter execution or rendering failed.
    pub error: Option<String>,
}

/// Destination details for an action, tagged by action type.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ActionDestinationDebugInfo {
    /// Email destination.
    #[serde(rename = "email")]
    Email(EmailDestinationDebugInfo),
    /// Webhook destination.
    #[serde(rename = "webhook")]
    Webhook(WebhookDestinationDebugInfo),
    /// Server log destination (no extra fields).
    #[serde(rename = "log")]
    ServerLog,
}

/// Email-specific destination details.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailDestinationDebugInfo {
    /// Recipient email addresses.
    pub to: Vec<String>,
    /// Rendered email preview.
    pub rendered_email: Option<RenderedEmailDebugInfo>,
}

/// Webhook-specific destination details.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDestinationDebugInfo {
    /// The webhook URL.
    pub url: Url,
    /// The HTTP method that would be used.
    #[serde(with = "http_serde::method")]
    #[schema(value_type = String)]
    pub method: Method,
    /// Request headers that would be sent.
    #[serde(with = "http_serde::option::header_map")]
    #[schema(value_type = Option<HashMap<String, String>>)]
    pub headers: Option<HeaderMap>,
}

/// Rendered email preview.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderedEmailDebugInfo {
    /// Email subject line.
    pub subject: String,
    /// Plain-text body.
    pub text: String,
    /// HTML body (rendered from the Handlebars template).
    pub html: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderValue, header::CONTENT_TYPE};
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn api_target_result_serialization() -> anyhow::Result<()> {
        let result = TrackerDebugResult {
            duration_ms: Duration::from_millis(1234),
            result: Some(json!({"key": "value"})),
            error: None,
            target: TrackerDebugTargetResult::Api(ApiTrackerDebugResult {
                params: Some(json!({"secrets": {"api_key": "s3cr3t"}})),
                configurator: None,
                requests: vec![ApiRequestDebugInfo {
                    index: 0,
                    source: "original".to_string(),
                    url: Some(Url::parse("https://api.example.com/data")?),
                    method: Some(Method::GET),
                    request_headers: None,
                    request_body: None,
                    status_code: Some(200),
                    response_headers: Some(HeaderMap::from_iter([(
                        CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    )])),
                    response_body_raw: Some(r#"{"key":"value"}"#.to_string()),
                    response_body_raw_size: Some(15),
                    response_body_parsed: Some(json!({"key": "value"})),
                    auto_parse: None,
                    duration_ms: Duration::from_millis(500),
                    error: None,
                }],
                extractor: None,
            }),
            actions: vec![],
        };

        assert_json_snapshot!(result, @r###"
        {
          "durationMs": 1234,
          "result": {
            "key": "value"
          },
          "target": {
            "type": "api",
            "params": {
              "secrets": {
                "api_key": "s3cr3t"
              }
            },
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "https://api.example.com/data",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-type": "application/json"
                },
                "responseBodyRaw": "{\"key\":\"value\"}",
                "responseBodyRawSize": 15,
                "responseBodyParsed": {
                  "key": "value"
                },
                "durationMs": 500
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn page_target_result_serialization() -> anyhow::Result<()> {
        let result = TrackerDebugResult {
            duration_ms: Duration::from_millis(3000),
            result: Some(json!("<html>content</html>")),
            error: None,
            target: TrackerDebugTargetResult::Page(PageTrackerDebugResult {
                params: Some(json!({"param": "value"})),
                engine: Some(ExtractorEngine::Chromium),
                extractor_source: "export async function execute(p) { return 'ok'; }".to_string(),
                logs: vec![PageLogEntry {
                    level: "info".to_string(),
                    message: "Connecting to browser...".to_string(),
                    args: None,
                }],
                screenshots: vec![],
                duration_ms: Duration::from_millis(2800),
                error: None,
            }),
            actions: vec![],
        };

        assert_json_snapshot!(result, @r###"
        {
          "durationMs": 3000,
          "result": "<html>content</html>",
          "target": {
            "type": "page",
            "params": {
              "param": "value"
            },
            "engine": {
              "type": "chromium"
            },
            "extractorSource": "export async function execute(p) { return 'ok'; }",
            "logs": [
              {
                "level": "info",
                "message": "Connecting to browser..."
              }
            ],
            "durationMs": 2800
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn page_target_result_serialization_with_screenshots() -> anyhow::Result<()> {
        let result = TrackerDebugResult {
            duration_ms: Duration::from_millis(1500),
            result: Some(json!("ok")),
            error: None,
            target: TrackerDebugTargetResult::Page(PageTrackerDebugResult {
                params: None,
                engine: None,
                extractor_source: "script".to_string(),
                logs: vec![],
                screenshots: vec![PageScreenshotEntry {
                    label: "after goto: https://example.com".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                    mime_type: "image/png".to_string(),
                }],
                duration_ms: Duration::from_millis(1200),
                error: None,
            }),
            actions: vec![],
        };

        assert_json_snapshot!(result, @r###"
        {
          "durationMs": 1500,
          "result": "ok",
          "target": {
            "type": "page",
            "extractorSource": "script",
            "screenshots": [
              {
                "label": "after goto: https://example.com",
                "data": "iVBORw0KGgo=",
                "mimeType": "image/png"
              }
            ],
            "durationMs": 1200
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn action_debug_info_serialization() -> anyhow::Result<()> {
        let action = ActionDebugInfo {
            type_tag: "webhook".to_string(),
            index: 0,
            formatter: Some(ScriptDebugInfo {
                source: "(() => ({ content: 'formatted' }))();".to_string(),
                duration_ms: Duration::from_millis(4),
                result: Some(json!({"content": "formatted"})),
                error: None,
            }),
            skipped: false,
            payload: Some(json!({"summary": "3 new items"})),
            destination: ActionDestinationDebugInfo::Webhook(WebhookDestinationDebugInfo {
                url: Url::parse("https://hooks.slack.com/trigger")?,
                method: Method::POST,
                headers: None,
            }),
            duration_ms: Duration::from_millis(5),
            error: None,
        };

        assert_json_snapshot!(action, @r###"
        {
          "typeTag": "webhook",
          "index": 0,
          "formatter": {
            "source": "(() => ({ content: 'formatted' }))();",
            "durationMs": 4,
            "result": {
              "content": "formatted"
            }
          },
          "skipped": false,
          "payload": {
            "summary": "3 new items"
          },
          "destination": {
            "type": "webhook",
            "url": "https://hooks.slack.com/trigger",
            "method": "POST"
          },
          "durationMs": 5
        }
        "###);

        Ok(())
    }

    #[test]
    fn action_debug_info_email_with_preview() -> anyhow::Result<()> {
        let action = ActionDebugInfo {
            type_tag: "email".to_string(),
            index: 1,
            formatter: None,
            skipped: false,
            payload: Some(json!("new data")),
            destination: ActionDestinationDebugInfo::Email(EmailDestinationDebugInfo {
                to: vec!["ops@company.com".to_string()],
                rendered_email: Some(RenderedEmailDebugInfo {
                    subject: "[Retrack] Change detected: \"My Tracker\"".to_string(),
                    text: "\"My Tracker\" tracker detected content changes.".to_string(),
                    html: Some("<html>...</html>".to_string()),
                }),
            }),
            duration_ms: Duration::from_millis(2),
            error: None,
        };

        assert_json_snapshot!(action, @r###"
        {
          "typeTag": "email",
          "index": 1,
          "skipped": false,
          "payload": "new data",
          "destination": {
            "type": "email",
            "to": [
              "ops@company.com"
            ],
            "renderedEmail": {
              "subject": "[Retrack] Change detected: \"My Tracker\"",
              "text": "\"My Tracker\" tracker detected content changes.",
              "html": "<html>...</html>"
            }
          },
          "durationMs": 2
        }
        "###);

        Ok(())
    }

    #[test]
    fn action_debug_info_skipped() -> anyhow::Result<()> {
        let action = ActionDebugInfo {
            type_tag: "log".to_string(),
            index: 0,
            formatter: Some(ScriptDebugInfo {
                source: "(() => null)();".to_string(),
                duration_ms: Duration::from_millis(1),
                result: None,
                error: None,
            }),
            skipped: true,
            payload: None,
            destination: ActionDestinationDebugInfo::ServerLog {},
            duration_ms: Duration::from_millis(1),
            error: None,
        };

        assert_json_snapshot!(action, @r###"
        {
          "typeTag": "log",
          "index": 0,
          "formatter": {
            "source": "(() => null)();",
            "durationMs": 1
          },
          "skipped": true,
          "destination": {
            "type": "log"
          },
          "durationMs": 1
        }
        "###);

        Ok(())
    }

    #[test]
    fn auto_parse_debug_info_serialization() -> anyhow::Result<()> {
        let info = AutoParseDebugInfo {
            media_type: MediaTypeBuf::from_string("text/csv".to_string())?,
            success: true,
            error: None,
        };
        assert_json_snapshot!(info, @r###"
        {
          "mediaType": "text/csv",
          "success": true
        }
        "###);

        let failed = AutoParseDebugInfo {
            media_type: MediaTypeBuf::from_string("application/vnd.ms-excel".to_string())?,
            success: false,
            error: Some("Invalid XLS header".to_string()),
        };

        assert_json_snapshot!(failed, @r###"
        {
          "mediaType": "application/vnd.ms-excel",
          "success": false,
          "error": "Invalid XLS header"
        }
        "###);

        Ok(())
    }

    #[test]
    fn result_with_actions_serialization() -> anyhow::Result<()> {
        let result = TrackerDebugResult {
            duration_ms: Duration::from_millis(100),
            result: Some(json!("data")),
            error: None,
            target: TrackerDebugTargetResult::Api(ApiTrackerDebugResult {
                params: None,
                configurator: None,
                requests: vec![],
                extractor: None,
            }),
            actions: vec![ActionDebugInfo {
                type_tag: "log".to_string(),
                index: 0,
                formatter: None,
                skipped: false,
                payload: Some(json!("data")),
                destination: ActionDestinationDebugInfo::ServerLog {},
                duration_ms: Duration::ZERO,
                error: None,
            }],
        };

        assert_json_snapshot!(result, @r###"
        {
          "durationMs": 100,
          "result": "data",
          "target": {
            "type": "api",
            "requests": []
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "skipped": false,
              "payload": "data",
              "destination": {
                "type": "log"
              },
              "durationMs": 0
            }
          ]
        }
        "###);

        let roundtrip: TrackerDebugResult = serde_json::from_value(serde_json::to_value(&result)?)?;
        assert_eq!(roundtrip, result);

        Ok(())
    }
}

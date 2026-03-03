use byte_unit::Byte;
use retrack_types::{trackers::TrackerDataValue, utils::serialize_opt_byte_as_u64};
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_with::{DurationMilliSeconds, serde_as, skip_serializing_none};
use std::time::Duration;

/// Structured debug options sent to the web scraper to control screenshot capture.
#[skip_serializing_none]
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperDebugOptions {
    pub enabled: bool,
    #[serde(serialize_with = "serialize_opt_byte_as_u64")]
    pub max_screenshots_total_size: Option<Byte>,
    pub auto_screenshots: Option<bool>,
}

/// Represents request to scrap web page content.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperContentRequest<'a> {
    /// A script (Playwright scenario) used to extract web page content that needs to be tracked.
    pub extractor: &'a str,

    /// Optional parameters to pass to the extractor scripts as part of the context.
    pub extractor_params: Option<&'a JsonValue>,

    /// Optionally specifies the backend that Web Scraper should use.
    pub extractor_backend: Option<WebScraperBackend>,

    /// Tags associated with the tracker.
    pub tags: &'a Vec<String>,

    /// Optional user agent string to use for every request at the web page.
    pub user_agent: Option<&'a str>,

    /// Indicates whether to accept invalid server certificates when sending network requests.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub accept_invalid_certificates: bool,

    /// Number of milliseconds to wait until an extractor script finishes processing.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub timeout: Option<Duration>,

    /// Optional content of the web page that has been extracted previously.
    pub previous_content: Option<&'a TrackerDataValue>,

    /// Debug options controlling screenshot capture and structured response format.
    pub debug: Option<WebScraperDebugOptions>,
}

/// Represents engines supported by the Web Scraper component.
#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WebScraperBackend {
    Chromium,
    Firefox,
}

#[cfg(test)]
mod tests {
    use super::{WebScraperBackend, WebScraperContentRequest, WebScraperDebugOptions};
    use crate::tests::MockTrackerBuilder;
    use byte_unit::Byte;
    use insta::assert_json_snapshot;
    use retrack_types::trackers::{ExtractorEngine, PageTarget, TrackerDataValue, TrackerTarget};
    use serde_json::json;
    use std::time::Duration;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperContentRequest {
            extractor: "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }",
            extractor_params: Some(&json!({ "param": "value" })),
            extractor_backend: Some(WebScraperBackend::Chromium),
            tags: &vec!["tag1".to_string(), "tag2".to_string()],
            timeout: Some(Duration::from_millis(100)),
            previous_content: Some(&TrackerDataValue::new(json!("some content"))),
            user_agent: Some("Retrack/1.0.0"),
            accept_invalid_certificates: true,
            debug: None,
        }, @r###"
        {
          "extractor": "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }",
          "extractorParams": {
            "param": "value"
          },
          "extractorBackend": "chromium",
          "tags": [
            "tag1",
            "tag2"
          ],
          "userAgent": "Retrack/1.0.0",
          "acceptInvalidCertificates": true,
          "timeout": 100,
          "previousContent": {
            "original": "some content"
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn serialization_with_debug() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperContentRequest {
            extractor: "script",
            extractor_params: None,
            extractor_backend: None,
            tags: &vec![],
            timeout: None,
            previous_content: None,
            user_agent: None,
            accept_invalid_certificates: false,
            debug: Some(WebScraperDebugOptions {
                enabled: true,
                max_screenshots_total_size: Some(Byte::from_u64(5242880)),
                auto_screenshots: Some(true),
            }),
        }, @r###"
        {
          "extractor": "script",
          "tags": [],
          "debug": {
            "enabled": true,
            "maxScreenshotsTotalSize": 5242880,
            "autoScreenshots": true
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn from_tracker() -> anyhow::Result<()> {
        let target = PageTarget {
            extractor: "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }".to_string(),
            params: Some(json!({ "param": "value" })),
            engine: None,
            user_agent: Some("Retrack/1.0.0".to_string()),
            accept_invalid_certificates: true,
        };
        let tracker = MockTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::Page(target.clone()))
        .with_timeout(Duration::from_millis(2500))
        .with_tags(vec!["tag1".to_string(), "tag2".to_string()])
        .build();

        let request = WebScraperContentRequest::try_from(&tracker)?;

        // Target properties (default engine/backend).
        assert_eq!(request.extractor, target.extractor.as_str());
        assert_eq!(request.extractor_params, target.params.as_ref());
        assert_eq!(request.extractor_backend, Some(WebScraperBackend::Chromium));
        assert_eq!(request.user_agent, target.user_agent.as_deref());
        assert_eq!(
            request.accept_invalid_certificates,
            target.accept_invalid_certificates
        );
        assert_eq!(request.tags, &tracker.tags);

        // Config properties.
        assert_eq!(request.timeout, Some(Duration::from_millis(2500)));

        assert!(request.previous_content.is_none());
        assert!(request.debug.is_none());

        // Explicit engines.
        for (engine, expected_backend) in [
            (ExtractorEngine::Chromium, WebScraperBackend::Chromium),
            (ExtractorEngine::Camoufox, WebScraperBackend::Firefox),
        ] {
            let tracker = MockTrackerBuilder::create(
                uuid!("00000000-0000-0000-0000-000000000001"),
                "some-name",
                3,
            )?
            .with_target(TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }".to_string(),
                params: None,
                engine: Some(engine),
                user_agent: None,
                accept_invalid_certificates: false,
            }))
            .build();

            assert_eq!(
                WebScraperContentRequest::try_from(&tracker)?.extractor_backend,
                Some(expected_backend)
            );
        }

        Ok(())
    }
}

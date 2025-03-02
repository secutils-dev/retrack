use retrack_types::trackers::TrackerDataValue;
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_with::{DurationMilliSeconds, serde_as, skip_serializing_none};
use std::time::Duration;

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

    /// Tags associated with the tracker.
    pub tags: &'a Vec<String>,

    /// Optional user agent string to use for every request at the web page.
    pub user_agent: Option<&'a str>,

    /// Indicates whether to ignore HTTPS errors when sending network requests.
    #[serde(
        rename = "ignoreHTTPSErrors",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub ignore_https_errors: bool,

    /// Number of milliseconds to wait until extractor script finishes processing.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub timeout: Option<Duration>,

    /// Optional content of the web page that has been extracted previously.
    pub previous_content: Option<&'a TrackerDataValue>,
}

#[cfg(test)]
mod tests {
    use super::WebScraperContentRequest;
    use crate::tests::MockTrackerBuilder;
    use insta::assert_json_snapshot;
    use retrack_types::trackers::{PageTarget, TrackerDataValue, TrackerTarget};
    use serde_json::json;
    use std::time::Duration;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperContentRequest {
            extractor: "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }",
            extractor_params: Some(&json!({ "param": "value" })),
            tags: &vec!["tag1".to_string(), "tag2".to_string()],
            timeout: Some(Duration::from_millis(100)),
            previous_content: Some(&TrackerDataValue::new(json!("some content"))),
            user_agent: Some("Retrack/1.0.0"),
            ignore_https_errors: true
        }, @r###"
        {
          "extractor": "export async function execute(p) { await p.goto('http://localhost:1234/my/app?q=2'); return await p.content(); }",
          "extractorParams": {
            "param": "value"
          },
          "tags": [
            "tag1",
            "tag2"
          ],
          "userAgent": "Retrack/1.0.0",
          "ignoreHTTPSErrors": true,
          "timeout": 100,
          "previousContent": {
            "original": "some content"
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
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
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

        // Target properties.
        assert_eq!(request.extractor, target.extractor.as_str());
        assert_eq!(request.extractor_params, target.params.as_ref());
        assert_eq!(request.user_agent, target.user_agent.as_deref());
        assert_eq!(request.ignore_https_errors, target.ignore_https_errors);
        assert_eq!(request.tags, &tracker.tags);

        // Config properties.
        assert_eq!(request.timeout, Some(Duration::from_millis(2500)));

        assert!(request.previous_content.is_none());

        Ok(())
    }
}

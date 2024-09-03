use crate::trackers::{Tracker, TrackerTarget};
use anyhow::bail;
use serde::Serialize;
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::time::Duration;

/// Represents request to scrap web page content.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperContentRequest<'a> {
    /// A script (Playwright scenario) used to extract web page content that needs to be tracked.
    pub extractor: &'a str,

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
    pub previous_content: Option<&'a str>,
}

impl<'a> WebScraperContentRequest<'a> {
    /// Sets the content that has been extracted from the page previously.
    pub fn set_previous_content(self, previous_content: &'a str) -> Self {
        Self {
            previous_content: Some(previous_content),
            ..self
        }
    }
}

impl<'t> TryFrom<&'t Tracker> for WebScraperContentRequest<'t> {
    type Error = anyhow::Error;

    fn try_from(tracker: &'t Tracker) -> Result<Self, Self::Error> {
        let TrackerTarget::WebPage(ref target) = tracker.target else {
            bail!(
                "Tracker ('{}') target is not web page, instead got: {:?}",
                tracker.id,
                tracker.target
            );
        };

        Ok(Self {
            // Target properties.
            extractor: target.extractor.as_str(),
            user_agent: target.user_agent.as_deref(),
            ignore_https_errors: target.ignore_https_errors,
            // Config properties.
            timeout: tracker.config.timeout,
            // Non-tracker properties.
            previous_content: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WebScraperContentRequest;
    use crate::{
        tests::MockWebPageTrackerBuilder,
        trackers::{TrackerTarget, WebPageTarget},
    };
    use insta::assert_json_snapshot;
    use std::time::Duration;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperContentRequest {
            extractor: "export async function execute(p, r) { await p.goto('http://localhost:1234/my/app?q=2'); return r.html(await p.content()); }",
            timeout: Some(Duration::from_millis(100)),
            previous_content: Some("some content"),
            user_agent: Some("Retrack/1.0.0"),
            ignore_https_errors: true
        }, @r###"
        {
          "extractor": "export async function execute(p, r) { await p.goto('http://localhost:1234/my/app?q=2'); return r.html(await p.content()); }",
          "userAgent": "Retrack/1.0.0",
          "ignoreHTTPSErrors": true,
          "timeout": 100,
          "previousContent": "some content"
        }
        "###);

        Ok(())
    }

    #[test]
    fn from_tracker() -> anyhow::Result<()> {
        let target = WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('http://localhost:1234/my/app?q=2'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        };
        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            3,
        )?
        .with_target(TrackerTarget::WebPage(target.clone()))
        .with_timeout(Duration::from_millis(2500))
        .build();

        let request = WebScraperContentRequest::try_from(&tracker)?;

        // Target properties.
        assert_eq!(request.extractor, target.extractor.as_str());
        assert_eq!(request.user_agent, target.user_agent.as_deref());
        assert_eq!(request.ignore_https_errors, target.ignore_https_errors);

        // Config properties.
        assert_eq!(request.timeout, Some(Duration::from_millis(2500)));

        assert!(request.previous_content.is_none());

        Ok(())
    }
}

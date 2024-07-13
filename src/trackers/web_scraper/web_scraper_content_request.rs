use crate::trackers::{Tracker, TrackerTarget, WebPageWaitFor};
use anyhow::bail;
use serde::Serialize;
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::{collections::HashMap, time::Duration};
use url::Url;

/// Represents request to scrap web page content.
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebScraperContentRequest<'a> {
    /// URL of the web page to scrap content for.
    pub url: &'a Url,

    /// Optional script used to extract web page content that needs to be tracked.
    pub extractor: Option<&'a str>,

    /// Optional content of the web page that has been extracted previously.
    pub headers: Option<&'a HashMap<String, String>>,

    /// Number of milliseconds to wait after page enters "idle" state.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub delay: Option<Duration>,

    /// Number of milliseconds to wait until page enters "idle" state.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub timeout: Option<Duration>,

    /// Instructs web scraper to wait for a specified element to reach specified state before
    /// extracting content.
    pub wait_for: Option<&'a WebPageWaitFor>,

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
            // Top-level properties.
            url: &tracker.url,
            // Config properties.
            extractor: tracker.config.extractor.as_deref(),
            headers: tracker.config.headers.as_ref(),
            // Target properties.
            delay: target.delay,
            wait_for: target.wait_for.as_ref(),
            // Non-tracker properties.
            timeout: None,
            previous_content: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WebScraperContentRequest;
    use crate::{
        tests::MockWebPageTrackerBuilder,
        trackers::{TrackerTarget, WebPageTarget, WebPageWaitFor, WebPageWaitForState},
    };
    use insta::assert_json_snapshot;
    use std::time::Duration;
    use url::Url;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(WebScraperContentRequest {
            url: &Url::parse("http://localhost:1234/my/app?q=2")?,
            timeout: Some(Duration::from_millis(100)),
            delay: Some(Duration::from_millis(200)),
            wait_for: Some(&WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(3000))
            }),
            previous_content: Some("some content"),
            extractor: Some("return resource;"),
            headers: Some(
                &[("cookie".to_string(), "my-cookie".to_string())]
                    .into_iter()
                    .collect(),
            ),
        }, @r###"
        {
          "url": "http://localhost:1234/my/app?q=2",
          "extractor": "return resource;",
          "headers": {
            "cookie": "my-cookie"
          },
          "delay": 200,
          "timeout": 100,
          "waitFor": {
            "selector": "div",
            "state": "attached",
            "timeout": 3000
          },
          "previousContent": "some content"
        }
        "###);

        Ok(())
    }

    #[test]
    fn from_tracker() -> anyhow::Result<()> {
        let target = WebPageTarget {
            delay: Some(Duration::from_millis(2500)),
            wait_for: Some(WebPageWaitFor {
                selector: "div".to_string(),
                state: Some(WebPageWaitForState::Attached),
                timeout: Some(Duration::from_millis(5000)),
            }),
        };
        let tracker = MockWebPageTrackerBuilder::create(
            uuid!("00000000-0000-0000-0000-000000000001"),
            "some-name",
            "http://localhost:1234/my/app?q=2",
            3,
        )?
        .with_target(TrackerTarget::WebPage(target.clone()))
        .with_extractor("return resource;".to_string())
        .build();

        let request = WebScraperContentRequest::try_from(&tracker)?;

        // Top-level properties.
        assert_eq!(request.url, &tracker.url);
        assert_eq!(request.extractor, tracker.config.extractor.as_deref());

        // Config properties.
        assert_eq!(request.headers, tracker.config.headers.as_ref());
        assert_eq!(request.delay, Some(Duration::from_millis(2500)));

        // Target properties.
        assert_eq!(request.delay, target.delay);
        assert_eq!(request.wait_for, target.wait_for.as_ref());

        assert!(request.timeout.is_none());
        assert!(request.previous_content.is_none());

        Ok(())
    }
}

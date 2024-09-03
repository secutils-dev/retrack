mod json_api_target;
mod web_page_target;

pub use self::{json_api_target::JsonApiTarget, web_page_target::WebPageTarget};
use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target (web page, API, or file).
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum TrackerTarget {
    /// Web page target.
    #[serde(rename = "web:page")]
    WebPage(WebPageTarget),
    /// JSON API target.
    #[serde(rename = "api:json")]
    JsonApi(JsonApiTarget),
}

#[cfg(test)]
mod tests {
    use super::TrackerTarget;
    use crate::trackers::{JsonApiTarget, WebPageTarget};
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: None,
            ignore_https_errors: false,
        });
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page",
          "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }"
        }
        "###);

        let target = TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        assert_json_snapshot!(target, @r###"
        {
          "type": "web:page",
          "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
          "userAgent": "Retrack/1.0.0",
          "ignoreHTTPSErrors": true
        }
        "###);

        let target = TrackerTarget::JsonApi(JsonApiTarget {
            url: url::Url::parse("https://retrack.dev")?,
        });
        assert_json_snapshot!(target, @r###"
        {
          "type": "api:json",
          "url": "https://retrack.dev/"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: None,
            ignore_https_errors: false,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({
                "type": "web:page",
                "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
            }).to_string())?,
            target
        );

        let target = TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: None,
            ignore_https_errors: true,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({
                "type": "web:page",
                "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
                "ignoreHTTPSErrors": true
            }).to_string())?,
            target
        );

        let target = TrackerTarget::WebPage(WebPageTarget {
            extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({
                    "type": "web:page",
                    "extractor": "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }",
                    "userAgent": "Retrack/1.0.0",
                    "ignoreHTTPSErrors": true
                })
                .to_string()
            )?,
            target
        );

        let target = TrackerTarget::JsonApi(JsonApiTarget {
            url: url::Url::parse("https://retrack.dev")?,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({ "type": "api:json", "url": "https://retrack.dev" }).to_string()
            )?,
            target
        );

        Ok(())
    }
}

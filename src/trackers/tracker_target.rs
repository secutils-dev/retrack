mod api_target;
mod page_target;

pub use self::{api_target::ApiTarget, page_target::PageTarget};
use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target (web page, API, or file).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum TrackerTarget {
    /// Web page target.
    Page(PageTarget),
    /// HTTP API target.
    Api(ApiTarget),
}

#[cfg(test)]
mod tests {
    use super::TrackerTarget;
    use crate::trackers::{ApiTarget, PageTarget};
    use http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        Method,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: None,
            ignore_https_errors: false,
        });
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({
                "type": "page",
                "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
            })
        );

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({
                "type": "page",
                "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
                "userAgent": "Retrack/1.0.0",
                "ignoreHTTPSErrors": true
            })
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev/")?,
            method: None,
            headers: None,
            media_type: None,
        });
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({ "type": "api", "url": "https://retrack.dev/" })
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev/")?,
            method: Some(Method::PUT),
            headers: None,
            media_type: None,
        });
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({ "type": "api", "url": "https://retrack.dev/", "method": "PUT" })
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev/")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[
                    (CONTENT_TYPE, "application/json".to_string()),
                    (AUTHORIZATION, "Bearer token".to_string()),
                ]
                .into_iter()
                .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            media_type: None,
        });
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({
                "type": "api",
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": { "content-type": "application/json", "authorization": "Bearer token" }
            })
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: None,
            ignore_https_errors: false,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({
                "type": "page",
                "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            }).to_string())?,
            target
        );

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: None,
            ignore_https_errors: true,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(&json!({
                "type": "page",
                "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
                "ignoreHTTPSErrors": true
            }).to_string())?,
            target
        );

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({
                    "type": "page",
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
                    "userAgent": "Retrack/1.0.0",
                    "ignoreHTTPSErrors": true
                })
                .to_string()
            )?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
            media_type: None,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({ "type": "api", "url": "https://retrack.dev" }).to_string()
            )?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: None,
            media_type: None,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({ "type": "api", "url": "https://retrack.dev", "method": "PUT" })
                    .to_string()
            )?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            url: url::Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[
                    (CONTENT_TYPE, "application/json".to_string()),
                    (AUTHORIZATION, "Bearer token".to_string()),
                ]
                .into_iter()
                .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            media_type: None,
        });
        assert_eq!(
            serde_json::from_str::<TrackerTarget>(
                &json!({
                    "type": "api",
                    "url": "https://retrack.dev",
                    "method": "PUT",
                    "headers": { "content-type": "application/json", "authorization": "Bearer token" }
                }).to_string()
            )?,
            target
        );

        Ok(())
    }
}

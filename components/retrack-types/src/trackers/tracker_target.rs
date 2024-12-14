mod api_target;
mod page_target;

pub use self::{
    api_target::{
        ApiTarget, ConfiguratorScriptArgs, ConfiguratorScriptRequest, ConfiguratorScriptResult,
        ExtractorScriptArgs, ExtractorScriptResult, TargetRequest,
    },
    page_target::PageTarget,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's target (web page, API, or file).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum TrackerTarget {
    /// Web page target.
    Page(PageTarget),
    /// HTTP API target.
    Api(ApiTarget),
}

#[cfg(test)]
mod tests {
    use super::TrackerTarget;
    use crate::trackers::{ApiTarget, PageTarget, TargetRequest};
    use http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        Method,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn can_serialization_and_deserialize() -> anyhow::Result<()> {
        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            user_agent: None,
            ignore_https_errors: false,
        });
        let target_json = json!({
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(
            serde_json::from_value::<TrackerTarget>(target_json)?,
            target
        );

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: Some(json!({ "param": "value" })),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        let target_json = json!({
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "params": { "param": "value" },
            "userAgent": "Retrack/1.0.0",
            "ignoreHTTPSErrors": true
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(
            serde_json::from_value::<TrackerTarget>(target_json)?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new("https://retrack.dev/".parse()?)],
            configurator: None,
            extractor: None,
        });
        let target_json = json!({ "type": "api", "requests": [{ "url": "https://retrack.dev/" }] });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(
            serde_json::from_value::<TrackerTarget>(target_json)?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest {
                method: Some(Method::PUT),
                ..TargetRequest::new("https://retrack.dev/".parse()?)
            }],
            configurator: None,
            extractor: None,
        });
        let target_json = json!({ "type": "api", "requests": [{ "url": "https://retrack.dev/", "method": "PUT" }] });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(
            serde_json::from_value::<TrackerTarget>(target_json)?,
            target
        );

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest {
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
                ..TargetRequest::new("https://retrack.dev/".parse()?)
            }],
            configurator: None,
            extractor: None,
        });
        let target_json = json!({
            "type": "api",
            "requests": [{
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": { "content-type": "application/json", "authorization": "Bearer token" }
            }]
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(
            serde_json::from_value::<TrackerTarget>(target_json)?,
            target
        );

        Ok(())
    }
}

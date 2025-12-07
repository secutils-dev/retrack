mod api_target;
mod page_target;
mod proxy_config;

pub use self::{
    api_target::{
        ApiTarget, ConfiguratorScriptArgs, ConfiguratorScriptRequest, ConfiguratorScriptResult,
        ExtractorScriptArgs, ExtractorScriptResult, TargetRequest, TargetResponse,
    },
    page_target::{ExtractorEngine, PageTarget},
    proxy_config::{ProxyConfig, ProxyCredentials},
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
    use crate::trackers::{ApiTarget, ExtractorEngine, PageTarget, TargetRequest};
    use http::{
        Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            engine: None,
            user_agent: None,
            accept_invalid_certificates: false,
            proxy: None,
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
            engine: Some(ExtractorEngine::Camoufox),
            user_agent: Some("Retrack/1.0.0".to_string()),
            accept_invalid_certificates: true,
            proxy: None,
        });
        let target_json = json!({
            "type": "page",
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "params": { "param": "value" },
            "engine": { "type": "camoufox" },
            "userAgent": "Retrack/1.0.0",
            "acceptInvalidCertificates": true
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
            proxy: None,
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
            proxy: None,
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
            proxy: None,
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

mod extractor_engine;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

pub use extractor_engine::ExtractorEngine;

/// Tracker's target for a web page.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageTarget {
    /// A custom script (Playwright scenario) to extract data from the page.
    pub extractor: String,

    /// Defines the engine (browser) to use to extract content. By default, the web scraper with
    /// the Chromium browser backend is used.
    pub engine: Option<ExtractorEngine>,

    /// Optional parameters to pass to the scripts as part of the context.
    #[serde(default)]
    pub params: Option<serde_json::Value>,

    /// Specific user agent to use for the browser context.
    pub user_agent: Option<String>,

    /// Whether to ignore HTTPS errors when sending network requests.
    #[serde(
        rename = "ignoreHTTPSErrors",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub ignore_https_errors: bool,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ExtractorEngine, PageTarget};
    use serde_json::json;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let target = PageTarget::default();
        let target_json = json!({ "extractor": "" });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<PageTarget>(target_json)?, target);

        let target = PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            engine: Some(ExtractorEngine::Camoufox),
            params: Some(json!({ "param": "value" })),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        };
        let target_json = json!({
            "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
            "engine": { "type": "camoufox" },
            "params": { "param": "value" },
            "userAgent": "Retrack/1.0.0",
            "ignoreHTTPSErrors": true
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<PageTarget>(target_json)?, target);

        Ok(())
    }
}

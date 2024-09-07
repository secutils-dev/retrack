use serde_derive::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Tracker's target for a web page.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageTarget {
    /// A custom script (Playwright scenario) to extract data from the page.
    pub extractor: String,

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
    use crate::trackers::PageTarget;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let target = PageTarget::default();
        assert_json_snapshot!(target, @r###"
        {
          "extractor": ""
        }
        "###);

        let target = PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        };
        assert_json_snapshot!(target, @r###"
        {
          "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
          "userAgent": "Retrack/1.0.0",
          "ignoreHTTPSErrors": true
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: None,
            ignore_https_errors: false,
        };
        assert_eq!(
            serde_json::from_str::<PageTarget>(&json!({ "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }" }).to_string())?,
            target
        );

        let target = PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        };
        assert_eq!(
            serde_json::from_str::<PageTarget>(
                &json!({
                    "extractor": "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }",
                    "userAgent": "Retrack/1.0.0",
                    "ignoreHTTPSErrors": true
                }).to_string()
            )?,
            target
        );

        Ok(())
    }
}

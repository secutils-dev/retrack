use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use utoipa::ToSchema;

/// Tracker's action to send an HTTP request.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAction {
    /// URL of the API endpoint to send the tracker data (JSON) to.
    pub url: Url,

    /// HTTP method to use for the request. If not specified, during deserialization set to `POST`.
    #[serde(with = "http_serde::option::method", default)]
    pub method: Option<Method>,

    /// Optional headers to include in the request.
    #[serde(with = "http_serde::option::header_map", default)]
    pub headers: Option<HeaderMap>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::WebhookAction;
    use http::{header::CONTENT_TYPE, Method};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::collections::HashMap;
    use url::Url;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
        };
        assert_json_snapshot!(action, @r###"
        {
          "url": "https://retrack.dev/"
        }
        "###);

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::GET),
            headers: None,
        };
        assert_json_snapshot!(action, @r###"
        {
          "url": "https://retrack.dev/",
          "method": "GET"
        }
        "###);

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
        };
        assert_json_snapshot!(action, @r###"
        {
          "url": "https://retrack.dev/",
          "method": "PUT",
          "headers": {
            "content-type": "application/json"
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
        };
        assert_eq!(
            serde_json::from_str::<WebhookAction>(
                &json!({ "url": "https://retrack.dev" }).to_string()
            )?,
            action
        );

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::GET),
            headers: None,
        };
        assert_eq!(
            serde_json::from_str::<WebhookAction>(
                &json!({ "url": "https://retrack.dev", "method": "GET" }).to_string()
            )?,
            action
        );

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
        };
        assert_eq!(
            serde_json::from_str::<WebhookAction>(
                &json!({
                    "url": "https://retrack.dev",
                    "method": "PUT",
                    "headers": { "content-type": "application/json" }
                })
                .to_string()
            )?,
            action
        );

        Ok(())
    }
}

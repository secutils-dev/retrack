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

    /// HTTP method to use for the request (`GET`, `POST`, or `PUT`). If not specified, defaults to
    /// `POST`.
    #[serde(with = "http_serde::option::method", default)]
    #[schema(value_type = String)]
    pub method: Option<Method>,

    /// Optional headers to include in the request.
    #[serde(with = "http_serde::option::header_map", default)]
    #[schema(value_type = HashMap<String, String>)]
    pub headers: Option<HeaderMap>,

    /// Optional custom script (Deno) to format tracker revision content for action. The script
    /// accept both previous and current tracker revision content as arguments and should return
    /// a serializable value that will be consumed by the action. If the script is not provided or
    /// returns `null` or `undefined`, the action will receive the current tracker revision content
    /// as is.
    pub formatter: Option<String>,
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
            formatter: None,
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
            formatter: None,
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
            formatter: None,
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

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_json_snapshot!(action, @r###"
        {
          "url": "https://retrack.dev/",
          "method": "PUT",
          "headers": {
            "content-type": "application/json"
          },
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
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
            formatter: None,
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
            formatter: None,
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
            formatter: None,
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

        let action = WebhookAction {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        };
        assert_eq!(
            serde_json::from_str::<WebhookAction>(
                &json!({
                    "url": "https://retrack.dev",
                    "method": "PUT",
                    "headers": { "content-type": "application/json" },
                    "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                })
                .to_string()
            )?,
            action
        );

        Ok(())
    }
}

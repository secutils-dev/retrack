use http::{HeaderMap, Method};
use mediatype::MediaTypeBuf;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use utoipa::ToSchema;

mod configurator_script_args;
mod configurator_script_result;
mod extractor_script_args;
mod extractor_script_result;

pub use self::{
    configurator_script_args::ConfiguratorScriptArgs,
    configurator_script_result::ConfiguratorScriptResult,
    extractor_script_args::ExtractorScriptArgs, extractor_script_result::ExtractorScriptResult,
};

/// Tracker's target for HTTP API.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTarget {
    /// URL of the API endpoint that returns JSON to track.
    pub url: Url,

    /// The HTTP method to use to send request to.
    #[serde(with = "http_serde::option::method", default)]
    #[schema(value_type = String)]
    pub method: Option<Method>,

    /// Optional headers to include in the request.
    #[serde(with = "http_serde::option::header_map", default)]
    #[schema(value_type = HashMap<String, String>)]
    pub headers: Option<HeaderMap>,

    /// The media type of the content returned by the API. By default, application/json is assumed.
    #[schema(value_type = String)]
    pub media_type: Option<MediaTypeBuf>,

    /// Optional body to include to the request.
    pub body: Option<serde_json::Value>,

    /// Optional custom script (Deno) to configure request.
    pub configurator: Option<String>,

    /// Optional custom script (Deno) to extract only necessary data from the API response.
    pub extractor: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::ApiTarget;
    use http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        Method,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use url::Url;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
            body: None,
            media_type: None,
            configurator: None,
            extractor: None,
        };
        let target_json = json!({ "url": "https://retrack.dev/" });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: None,
            body: None,
            media_type: None,
            configurator: None,
            extractor: None,
        };
        let target_json = json!({ "url": "https://retrack.dev/", "method": "PUT" });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
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
            body: None,
            media_type: None,
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            }
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
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
            body: Some(json!({ "key": "value" })),
            media_type: None,
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            },
            "body": {
                "key": "value"
            }
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
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
            body: Some(json!({ "key": "value" })),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            },
            "body": {
                "key": "value"
            },
            "mediaType": "text/plain; charset=UTF-8"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
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
            body: Some(json!({ "key": "value" })),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
            configurator: Some(
                "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
                    .to_string(),
            ),
            extractor: None,
        };
        let target_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            },
            "body": {
                "key": "value"
            },
            "mediaType": "text/plain; charset=UTF-8",
            "configurator": "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
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
            body: Some(json!({ "key": "value" })),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
            configurator: Some(
                "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
                    .to_string(),
            ),
            extractor: Some(
                "((context) => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
                    .to_string(),
            ),
        };
        let target_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            },
            "body": {
                "key": "value"
            },
            "mediaType": "text/plain; charset=UTF-8",
            "configurator": "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();",
            "extractor": "((context) => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        Ok(())
    }
}

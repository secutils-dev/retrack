use http::{HeaderMap, Method};
use mediatype::MediaTypeBuf;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use utoipa::ToSchema;

/// Request structure for the API target.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetRequest {
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
}

impl TargetRequest {
    /// Creates a new target request with the given URL.
    pub fn new(url: Url) -> Self {
        Self {
            url,
            method: None,
            headers: None,
            media_type: None,
            body: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::trackers::TargetRequest;
    use http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        Method,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let request = TargetRequest::new("https://retrack.dev".parse()?);
        let request_json = json!({ "url": "https://retrack.dev/" });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetRequest>(request_json)?,
            request
        );

        let request = TargetRequest {
            url: "https://retrack.dev".parse()?,
            method: Some(Method::PUT),
            headers: None,
            body: None,
            media_type: None,
        };
        let request_json = json!({ "url": "https://retrack.dev/", "method": "PUT" });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetRequest>(request_json)?,
            request
        );

        let request = TargetRequest {
            url: "https://retrack.dev".parse()?,
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
        };
        let request_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            }
        });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetRequest>(request_json)?,
            request
        );

        let request = TargetRequest {
            url: "https://retrack.dev".parse()?,
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
        };
        let request_json = json!({
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
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetRequest>(request_json)?,
            request
        );

        let request = TargetRequest {
            url: "https://retrack.dev".parse()?,
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
        };
        let request_json = json!({
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
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetRequest>(request_json)?,
            request
        );

        Ok(())
    }
}
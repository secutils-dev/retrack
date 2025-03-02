use crate::trackers::TargetRequest;
use http::{HeaderMap, Method};
use mediatype::MediaTypeBuf;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;

/// Structure of the request representation for the configurator script.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguratorScriptRequest {
    /// URL of the API endpoint that returns JSON to track.
    pub url: Url,

    /// The HTTP method to use to send request to.
    #[serde(with = "http_serde::option::method", default)]
    pub method: Option<Method>,

    /// Optional HTTP headers to send with the request.
    #[serde(with = "http_serde::option::header_map", default)]
    pub headers: Option<HeaderMap>,

    /// The media type of the content returned by the API. By default, application/json is assumed.
    pub media_type: Option<MediaTypeBuf>,

    /// Optional HTTP body configured for the request.
    #[serde(with = "serde_bytes", default)]
    pub body: Option<Vec<u8>>,
}

impl TryFrom<ConfiguratorScriptRequest> for TargetRequest {
    type Error = serde_json::Error;

    fn try_from(request: ConfiguratorScriptRequest) -> Result<Self, Self::Error> {
        Ok(TargetRequest {
            url: request.url,
            method: request.method,
            headers: request.headers,
            media_type: request.media_type,
            body: request
                .body
                .map(|body| serde_json::from_slice(&body))
                .transpose()?,
        })
    }
}

impl TryFrom<TargetRequest> for ConfiguratorScriptRequest {
    type Error = serde_json::Error;

    fn try_from(request: TargetRequest) -> Result<Self, Self::Error> {
        Ok(ConfiguratorScriptRequest {
            url: request.url,
            method: request.method,
            headers: request.headers,
            media_type: request.media_type,
            body: request.body.as_ref().map(serde_json::to_vec).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ConfiguratorScriptRequest, TargetRequest};
    use http::{Method, header::CONTENT_TYPE};
    use serde_json::json;
    use std::collections::HashMap;
    use url::Url;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
            body: None,
            media_type: None,
        };
        let request_json = json!({ "url": "https://retrack.dev/" });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<ConfiguratorScriptRequest>(request_json)?,
            request
        );

        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: None,
            body: None,
            media_type: None,
        };
        let request_json = json!({ "url": "https://retrack.dev/", "method": "PUT" });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<ConfiguratorScriptRequest>(request_json)?,
            request
        );

        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
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
            }
        });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<ConfiguratorScriptRequest>(request_json)?,
            request
        );

        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            body: Some(serde_json::to_vec(&json!({ "key": "value" }))?),
            media_type: None,
        };
        let request_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json",
            },
            "body": serde_json::to_vec(&json!({ "key": "value" }))?
        });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<ConfiguratorScriptRequest>(request_json)?,
            request
        );

        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            body: Some(serde_json::to_vec(&json!({ "key": "value" }))?),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
        };
        let request_json = json!({
            "url": "https://retrack.dev/",
            "method": "PUT",
            "headers": {
                "content-type": "application/json"
            },
            "body": serde_json::to_vec(&json!({ "key": "value" }))?,
            "mediaType": "text/plain; charset=UTF-8"
        });
        assert_eq!(serde_json::to_value(&request)?, request_json);
        assert_eq!(
            serde_json::from_value::<ConfiguratorScriptRequest>(request_json)?,
            request
        );

        Ok(())
    }

    #[test]
    fn can_convert_to_target_request() -> anyhow::Result<()> {
        let request = ConfiguratorScriptRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            body: Some(serde_json::to_vec(&json!({ "key": "value" }))?),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
        };

        assert_eq!(
            TargetRequest::try_from(request)?,
            TargetRequest {
                url: Url::parse("https://retrack.dev")?,
                method: Some(Method::PUT),
                headers: Some(
                    (&[(CONTENT_TYPE, "application/json".to_string())]
                        .into_iter()
                        .collect::<HashMap<_, _>>())
                        .try_into()?,
                ),
                body: Some(json!({ "key": "value" })),
                media_type: Some("text/plain; charset=UTF-8".parse()?)
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_from_target_request() -> anyhow::Result<()> {
        let request = TargetRequest {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
            body: Some(json!({ "key": "value" })),
            media_type: Some("text/plain; charset=UTF-8".parse()?),
        };

        assert_eq!(
            ConfiguratorScriptRequest::try_from(request)?,
            ConfiguratorScriptRequest {
                url: Url::parse("https://retrack.dev")?,
                method: Some(Method::PUT),
                headers: Some(
                    (&[(CONTENT_TYPE, "application/json".to_string())]
                        .into_iter()
                        .collect::<HashMap<_, _>>())
                        .try_into()?,
                ),
                body: Some(serde_json::to_vec(&json!({ "key": "value" }))?),
                media_type: Some("text/plain; charset=UTF-8".parse()?)
            }
        );

        Ok(())
    }
}

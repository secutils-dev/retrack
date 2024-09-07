use http::{HeaderMap, Method};
use mediatype::MediaTypeBuf;
use serde_derive::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use utoipa::ToSchema;

/// Tracker's target for HTTP API.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTarget {
    /// URL of the API endpoint that returns JSON to track.
    pub url: Url,

    /// The HTTP method to use to send request to.
    #[serde(with = "http_serde::option::method", default)]
    pub method: Option<Method>,

    /// Optional headers to include in the request.
    #[serde(with = "http_serde::option::header_map", default)]
    pub headers: Option<HeaderMap>,

    /// The media type of the content returned by the API. By default, application/json is assumed.
    pub media_type: Option<MediaTypeBuf>,
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
    fn serialization() -> anyhow::Result<()> {
        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            method: None,
            headers: None,
            media_type: None,
        };
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({ "url": "https://retrack.dev/" })
        );

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: None,
            media_type: None,
        };
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({ "url": "https://retrack.dev/", "method": "PUT" })
        );

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
            media_type: None,
        };
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": {
                    "content-type": "application/json",
                    "authorization": "Bearer token"
                }
            })
        );

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
            media_type: Some("text/plain; charset=UTF-8".parse()?),
        };
        assert_eq!(
            serde_json::to_value(&target)?,
            json!({
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": {
                    "content-type": "application/json",
                    "authorization": "Bearer token"
                },
                "mediaType": "text/plain; charset=UTF-8"
            })
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            media_type: None,
            method: None,
            headers: None,
        };
        assert_eq!(
            serde_json::from_value::<ApiTarget>(json!({ "url": "https://retrack.dev" }))?,
            target
        );

        let target = ApiTarget {
            url: Url::parse("https://retrack.dev")?,
            method: Some(Method::PUT),
            headers: None,
            media_type: None,
        };
        assert_eq!(
            serde_json::from_value::<ApiTarget>(json!({
                "url": "https://retrack.dev",
                "method": "PUT"
            }))?,
            target
        );

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
            media_type: Some("text/plain; charset=UTF-8".parse()?),
        };
        assert_eq!(
            serde_json::from_value::<ApiTarget>(json!({
                "url": "https://retrack.dev",
                "method": "PUT",
                "headers": { "content-type": "application/json", "authorization": "Bearer token" },
                "mediaType": "text/plain; charset=UTF-8"
            }))?,
            target
        );

        Ok(())
    }
}

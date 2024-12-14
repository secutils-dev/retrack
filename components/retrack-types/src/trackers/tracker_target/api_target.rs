use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

mod configurator_script_args;
mod configurator_script_request;
mod configurator_script_result;
mod extractor_script_args;
mod extractor_script_result;
mod target_request;

pub use self::{
    configurator_script_args::ConfiguratorScriptArgs,
    configurator_script_request::ConfiguratorScriptRequest,
    configurator_script_result::ConfiguratorScriptResult,
    extractor_script_args::ExtractorScriptArgs, extractor_script_result::ExtractorScriptResult,
    target_request::TargetRequest,
};

/// Tracker's target for HTTP API.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTarget {
    /// A list of the requests to the HTTP endpoints to send when retrieving data for the target.
    pub requests: Vec<TargetRequest>,

    /// Optional custom script (Deno) to configure request.
    pub configurator: Option<String>,

    /// Optional custom script (Deno) to extract only necessary data from the API response.
    pub extractor: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ApiTarget, TargetRequest};
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
            requests: vec![TargetRequest::new(Url::parse("https://retrack.dev")?)],
            configurator: None,
            extractor: None,
        };
        let target_json = json!({ "requests": [{ "url": "https://retrack.dev/" }] });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            requests: vec![TargetRequest {
                method: Some(Method::PUT),
                ..TargetRequest::new(Url::parse("https://retrack.dev")?)
            }],
            configurator: None,
            extractor: None,
        };
        let target_json =
            json!({ "requests": [{"url": "https://retrack.dev/", "method": "PUT" }] });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
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
                ..TargetRequest::new(Url::parse("https://retrack.dev")?)
            }],
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "requests": [{
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": {
                    "content-type": "application/json",
                    "authorization": "Bearer token"
                }
            }]
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
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
                body: Some(json!({ "key": "value" })),
                ..TargetRequest::new(Url::parse("https://retrack.dev")?)
            }],
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "requests": [{
                "url": "https://retrack.dev/",
                "method": "PUT",
                "headers": {
                    "content-type": "application/json",
                    "authorization": "Bearer token"
                },
                "body": {
                    "key": "value"
                }
            }]
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            requests: vec![TargetRequest {
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
            }],
            configurator: None,
            extractor: None,
        };
        let target_json = json!({
            "requests": [{
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
            }]
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            requests: vec![TargetRequest {
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
            }],
            configurator: Some(
                "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
                    .to_string(),
            ),
            extractor: None,
        };
        let target_json = json!({
            "requests": [{
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
            }],
            "configurator": "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        let target = ApiTarget {
            requests: vec![TargetRequest {
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
            }],
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
            "requests": [{
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
            }],
            "configurator": "(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();",
            "extractor": "((context) => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();"
        });
        assert_eq!(serde_json::to_value(&target)?, target_json);
        assert_eq!(serde_json::from_value::<ApiTarget>(target_json)?, target);

        Ok(())
    }
}

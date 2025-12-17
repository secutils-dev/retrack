use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use utoipa::ToSchema;

/// Proxy configuration for tracker targets.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    /// Proxy server URL.
    pub url: Url,

    /// Optional credentials for proxy authentication.
    pub credentials: Option<ProxyCredentials>,
}

/// Proxy authentication credentials.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProxyCredentials {
    /// Authentication scheme (e.g., "Basic", "Bearer").
    pub scheme: String,

    /// Authentication value that will be used for Proxy-Authorization HTTP header.
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::{ProxyConfig, ProxyCredentials};
    use serde_json::json;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let proxy = ProxyConfig {
            url: "http://proxy.example.com:8080".parse()?,
            credentials: None,
        };
        let proxy_json = json!({ "url": "http://proxy.example.com:8080/" });
        assert_eq!(serde_json::to_value(&proxy)?, proxy_json);
        assert_eq!(serde_json::from_value::<ProxyConfig>(proxy_json)?, proxy);

        let proxy = ProxyConfig {
            url: "http://proxy.example.com:8080".parse()?,
            credentials: Some(ProxyCredentials {
                scheme: "Basic".to_string(),
                value: "dXNlcjpwYXNz".to_string(),
            }),
        };
        let proxy_json = json!({
            "url": "http://proxy.example.com:8080/",
            "credentials": {
                "scheme": "Basic",
                "value": "dXNlcjpwYXNz"
            }
        });
        assert_eq!(serde_json::to_value(&proxy)?, proxy_json);
        assert_eq!(serde_json::from_value::<ProxyConfig>(proxy_json)?, proxy);

        Ok(())
    }
}

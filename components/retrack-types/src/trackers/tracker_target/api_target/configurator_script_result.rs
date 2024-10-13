use http::HeaderMap;
use serde::Deserialize;

/// Result of the "configurator" script execution.
#[derive(Deserialize, Default, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguratorScriptResult {
    /// Optional HTTP headers to send with the request. If not specified, the default headers of the
    /// `api` target are used.
    #[serde(with = "http_serde::option::header_map", default)]
    pub headers: Option<HeaderMap>,

    /// Optional HTTP body to send with the request. If not specified, the default body of the `api`
    /// target is used.
    #[serde(with = "serde_bytes", default)]
    pub body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::ConfiguratorScriptResult;
    use http::{header::CONTENT_TYPE, HeaderMap, HeaderValue};

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "body": [1, 2 ,3],
    "headers": {
        "Content-Type": "text/plain"
    }
}
          "#
            )?,
            ConfiguratorScriptResult {
                headers: Some(HeaderMap::from_iter([(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            }
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(r#"{}"#)?,
            Default::default()
        );

        Ok(())
    }
}

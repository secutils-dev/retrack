use http::HeaderMap;
use serde::Deserialize;

/// Result of the "configurator" script execution.
#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConfiguratorScriptResult {
    /// Configurator script modifications for the request.
    Request {
        /// Optional HTTP headers to send with the request. If not specified, the default headers of the
        /// `api` target are used.
        #[serde(with = "http_serde::option::header_map", default)]
        headers: Option<HeaderMap>,

        /// Optional HTTP body to send with the request. If not specified, the default body of the `api`
        /// target is used.
        #[serde(with = "serde_bytes", default)]
        body: Option<Vec<u8>>,
    },
    /// Configurator script modifications for the response. If body is provided, the actual request
    /// is not sent and the response is returned immediately.
    Response {
        /// HTTP body that should be treated as response body.
        #[serde(with = "serde_bytes")]
        body: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use crate::trackers::ConfiguratorScriptResult;
    use http::{header::CONTENT_TYPE, HeaderValue};
    use insta::assert_debug_snapshot;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "request": {
        "body": [1, 2 ,3],
        "headers": {
            "Content-Type": "text/plain"
        }
    }
}
          "#
            )?,
            ConfiguratorScriptResult::Request {
                headers: Some(
                    vec![(CONTENT_TYPE, HeaderValue::from_static("text/plain"))]
                        .into_iter()
                        .collect()
                ),
                body: Some(vec![1, 2, 3]),
            }
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "response": {
        "body": [1, 2 ,3]
    }
}
          "#
            )?,
            ConfiguratorScriptResult::Response {
                body: vec![1, 2, 3],
            }
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(r#"{ "request": {} }"#)?,
            ConfiguratorScriptResult::Request {
                headers: None,
                body: None,
            }
        );

        assert_debug_snapshot!(serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "request": {
        "headers": {
            "Content-Type": "text/plain"
        }
    }
    "response": {
        "body": [1, 2 ,3]
    }
}
          "#
            ).unwrap_err(), @r###"Error("expected value", line: 8, column: 4)"###);

        Ok(())
    }
}

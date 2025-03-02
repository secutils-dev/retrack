use crate::trackers::ConfiguratorScriptRequest;
use serde::Deserialize;

/// Result of the "configurator" script execution.
#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConfiguratorScriptResult {
    /// Configurator script modifications for the request.
    Requests(Vec<ConfiguratorScriptRequest>),
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
    use crate::trackers::{ConfiguratorScriptRequest, ConfiguratorScriptResult};
    use http::{HeaderValue, header::CONTENT_TYPE};
    use insta::assert_debug_snapshot;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "requests": [{
        "url": "https://retrack.dev",
        "body": [1, 2 ,3],
        "headers": {
            "Content-Type": "text/plain"
        }
    }]
}
          "#
            )?,
            ConfiguratorScriptResult::Requests(vec![ConfiguratorScriptRequest {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: Some(
                    vec![(CONTENT_TYPE, HeaderValue::from_static("text/plain"))]
                        .into_iter()
                        .collect()
                ),
                media_type: None,
                body: Some(vec![1, 2, 3]),
            }])
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
            serde_json::from_str::<ConfiguratorScriptResult>(r#"{ "requests": [] }"#)?,
            ConfiguratorScriptResult::Requests(vec![])
        );

        assert_debug_snapshot!(serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "requests": [{
        "url": "https://retrack.dev",
        "headers": {
            "Content-Type": "text/plain"
        }
    }],
    "response": {
        "body": [1, 2 ,3]
    }
}
          "#
            ).unwrap_err(), @r###"Error("expected value", line: 8, column: 6)"###);

        Ok(())
    }
}
